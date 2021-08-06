use crate::util::Sha256;
use anyhow::{Context as _, Error};
use camino::{Utf8Path as Path, Utf8PathBuf as PathBuf};

pub struct Unpack {
    pub compressed: u64,
    pub decompressed: u64,
    pub num_files: u32,
}

pub struct Ctx {
    pub work_dir: PathBuf,
    pub tempdir: Option<tempfile::TempDir>,
    pub client: reqwest::Client,
}

impl Ctx {
    pub fn with_temp() -> Result<Self, Error> {
        let td = tempfile::TempDir::new()?;
        let client = reqwest::ClientBuilder::new().build()?;

        Ok(Self {
            work_dir: PathBuf::from_path_buf(td.path().to_owned()).map_err(|pb| {
                anyhow::anyhow!("tempdir {} is not a valid utf-8 path", pb.display())
            })?,
            tempdir: Some(td),
            client,
        })
    }

    pub fn with_dir(mut work_dir: PathBuf) -> Result<Self, Error> {
        let client = reqwest::ClientBuilder::new().build()?;

        work_dir.push("dl");
        std::fs::create_dir_all(&work_dir)?;
        work_dir.pop();
        work_dir.push("unpack");
        std::fs::create_dir_all(&work_dir)?;
        work_dir.pop();

        Ok(Self {
            work_dir,
            tempdir: None,
            client,
        })
    }

    #[tracing::instrument(skip(self, url, checksum))]
    pub async fn get_and_validate<P>(
        &self,
        url: String,
        path: &P,
        checksum: Option<Sha256>,
    ) -> Result<bytes::Bytes, Error>
    where
        P: AsRef<Path> + std::fmt::Debug,
    {
        let short_path = path.as_ref();
        let cache_path = {
            let mut cp = self.work_dir.clone();
            cp.push("dl");
            cp.push(short_path);
            cp
        };

        let (cache_path, checksum) = if cache_path.exists() {
            tracing::debug!("verifying existing cached dl file");

            let cached = tokio::task::spawn_blocking(move || match std::fs::read(&cache_path) {
                Ok(contents) => match checksum {
                    Some(expected) => {
                        let chksum = Sha256::digest(&contents);

                        if chksum != expected {
                            Err((
                                anyhow::anyhow!(
                                    "checksum mismatch, expected {} != actual {}",
                                    expected,
                                    chksum
                                ),
                                cache_path,
                                Some(expected),
                            ))
                        } else {
                            Ok(contents)
                        }
                    }
                    None => Ok(contents),
                },
                Err(e) => Err((e.into(), cache_path, checksum)),
            })
            .await?;

            match cached {
                Ok(cached) => return Ok(cached.into()),
                Err((e, cp, chksum)) => {
                    tracing::warn!(error = %e, "failed to read cached file");
                    (cp, chksum)
                }
            }
        } else {
            (cache_path, checksum)
        };

        let res = self.client.get(&url).send().await?.error_for_status()?;
        let body = res.bytes().await?;

        let body = tokio::task::spawn_blocking(move || -> Result<_, Error> {
            if let Some(expected) = checksum {
                let chksum = Sha256::digest(&body);

                anyhow::ensure!(
                    chksum == expected,
                    "checksum mismatch, expected {} != actual {}",
                    expected,
                    chksum
                );
            }

            if let Some(parent) = cache_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(cache_path, &body)?;
            Ok(body)
        })
        .await??;

        Ok(body)
    }

    #[tracing::instrument(skip(self))]
    pub async fn unpack(&self, payload: &crate::Payload) -> Result<Unpack, Error> {
        let mut unpack_dir = {
            let mut pb = self.work_dir.clone();
            pb.push("unpack");
            pb.push(&payload.filename);
            pb
        };

        unpack_dir.push(".unpack");

        #[derive(serde::Serialize, serde::Deserialize)]
        struct UnpackMeta {
            #[serde(serialize_with = "crate::util::serialize_sha256")]
            sha256: Sha256,
            compressed: u64,
            decompressed: u64,
            num_files: u32,
        }

        if let Ok(unpack) = std::fs::read(&unpack_dir) {
            if let Ok(um) = serde_json::from_slice::<UnpackMeta>(&unpack) {
                if payload.sha256 == &unpack[..] {
                    tracing::debug!("already unpacked");
                    return Ok(Unpack {
                        compressed: um.compressed,
                        decompressed: um.decompressed,
                        num_files: um.num_files,
                    });
                }
            }
        }

        unpack_dir.pop();

        // If we didn't validate the .unpack file, ensure that we clean up anything
        // that might be leftover from a failed unpack
        if unpack_dir.exists() {
            std::fs::remove_dir_all(&unpack_dir)
                .with_context(|| format!("unable to remove invalid unpack dir '{}'", unpack_dir))?;
        }

        std::fs::create_dir_all(&unpack_dir)
            .with_context(|| format!("unable to create unpack dir '{}'", unpack_dir))?;

        let output_dir = unpack_dir.clone();
        let pkg = {
            let mut pb = self.work_dir.clone();
            pb.push("dl");
            pb.push(&payload.filename);
            pb
        };

        let mut up = Unpack {
            compressed: 0,
            decompressed: 0,
            num_files: 0,
        };

        let unpack_task = match payload.filename.extension() {
            Some("vsix") => {
                tokio::task::spawn_blocking(move || -> Result<Unpack, Error> {
                    let vsix =
                        std::fs::read(&pkg).with_context(|| format!("unable to read {}", pkg))?;
                    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(vsix))
                        .with_context(|| format!("invalid zip {}", pkg))?;

                    // VSIX files are just a "specially" formatted zip file, all
                    // of the actual files we want are under "Contents"
                    let to_extract: Vec<_> = zip
                        .file_names()
                        .filter_map(|fname| {
                            (fname.starts_with("Contents/")
                                && (fname.contains("lib") || fname.contains("include")))
                            .then(|| fname.to_owned())
                        })
                        .collect();

                    for fname in to_extract {
                        let mut file = zip
                            .by_name(&fname)
                            .with_context(|| format!("no file '{}' in zip '{}'", fname, pkg))?;
                        let zip_path = Path::new(&fname);
                        let mut fs_path = output_dir.clone();

                        for comp in zip_path
                            .components()
                            .skip_while(|comp| comp.as_str() != "lib" && comp.as_str() != "include")
                        {
                            fs_path.push(comp);
                        }

                        if let Some(parent) = fs_path.parent() {
                            if !parent.exists() {
                                std::fs::create_dir_all(parent).with_context(|| {
                                    format!("unable to create unpack dir '{}'", parent)
                                })?;
                            }
                        }

                        let mut dest = std::fs::File::create(&fs_path).with_context(|| {
                            format!(
                                "unable to create {} to decompress {} from {}",
                                fs_path, fname, pkg
                            )
                        })?;

                        let decompressed =
                            std::io::copy(&mut file, &mut dest).with_context(|| {
                                format!(
                                    "unable to decompress {} from {} to {}",
                                    fname, pkg, fs_path
                                )
                            })?;

                        up.num_files += 1;
                        up.compressed += file.compressed_size();
                        up.decompressed += decompressed;
                    }

                    Ok(up)
                })
            }
            Some("msi") => tokio::task::spawn_blocking(move || -> Result<Unpack, Error> {
                let msi = std::fs::read(&pkg).with_context(|| format!("unable to read {}", pkg))?;

                up.compressed += msi.len() as u64;

                let mut msi = msi::Package::open(std::io::Cursor::new(msi))
                    .with_context(|| format!("unable to read MSI from {}", pkg))?;

                // Open source ftw https://gitlab.gnome.org/GNOME/msitools/-/blob/master/tools/msiextract.vala

                // For some reason many filenames in the table(s) have a weird
                // checksum(?) filename with an extension separated from the
                // _actual_ filename with a `|` so we need to detect that and
                // strip off just the real name we want
                #[inline]
                fn fix_name(name: &msi::Value) -> Result<&str, Error> {
                    let name = name.as_str().context("filename is not a string")?;

                    Ok(match name.find('|') {
                        Some(ind) => &name[ind + 1..],
                        None => name,
                    })
                }

                let components = {
                    #[derive(Debug)]
                    struct Dir {
                        id: String,
                        parent: Option<String>,
                        path: PathBuf,
                    }

                    // Collect the directories that can be referenced by a component
                    // that are reference by files. Ugh.
                    let mut directories: Vec<_> = msi
                        .select_rows(msi::Select::table("Directory"))
                        .with_context(|| format!("MSI {} has no 'Directory' table", pkg))?
                        .map(|row| -> Result<_, _> {
                            // Columns:
                            // 0 - Directory (name)
                            // 1 - Directory_Parent (name of parent)
                            // 2 - DefaultDir (location of directory on disk)
                            // ...
                            anyhow::ensure!(row.len() >= 3, "invalid row in 'Directory'");

                            Ok(Dir {
                                id: row[0]
                                    .as_str()
                                    .context("directory name is not a string")?
                                    .to_owned(),
                                // This can be `null`
                                parent: row[1].as_str().map(String::from),
                                path: fix_name(&row[2])?.into(),
                            })
                        })
                        .collect::<Result<_, _>>()
                        .with_context(|| format!("unable to read directories for {}", pkg))?;

                    directories.sort_by(|a, b| a.id.cmp(&b.id));

                    let components: std::collections::BTreeMap<_, _> = msi
                        .select_rows(msi::Select::table("Component"))
                        .with_context(|| format!("MSI {} has no 'Directory' table", pkg))?
                        .map(|row| -> Result<_, _> {
                            // Columns:
                            // 0 - Component (name, really, id)
                            // 1 - ComponentId
                            // 2 - Directory_ (directory id)
                            anyhow::ensure!(row.len() >= 3, "invalid row in 'Component'");

                            // The recursion depth for directory lookup is quite shallow
                            // typically, the full path to a file would be something like
                            // `Program Files/Windows Kits/10/Lib/10.0.19041.0/um/x64`
                            // but this a terrible path, so we massage it to instead be
                            // `lib/um/x64`
                            fn build_dir(dirs: &[Dir], id: &str, dir: &mut PathBuf) {
                                let cur_dir = match dirs.binary_search_by(|d| d.id.as_str().cmp(id))
                                {
                                    Ok(i) => &dirs[i],
                                    Err(_) => {
                                        tracing::warn!("unable to find directory {}", id);
                                        return;
                                    }
                                };

                                match cur_dir.path.file_name() {
                                    Some("Lib") => {
                                        dir.push("lib");
                                    }
                                    Some("Include") => {
                                        dir.push("include");
                                    }
                                    other => {
                                        if let Some(parent) = &cur_dir.parent {
                                            build_dir(dirs, parent, dir);
                                        }

                                        if let Some(other) = other {
                                            // Ignore the SDK version directory between
                                            // Lib/Include and the actual subdirs we care about
                                            if !other.starts_with(|c: char| c.is_digit(10)) {
                                                dir.push(other);
                                            }
                                        }
                                    }
                                }
                            }

                            let component_id = row[0]
                                .as_str()
                                .context("component id is not a string")?
                                .to_owned();

                            let mut dir = output_dir.clone();
                            build_dir(
                                &directories,
                                row[2]
                                    .as_str()
                                    .context("component directory is not a string")?,
                                &mut dir,
                            );

                            Ok((component_id, dir))
                        })
                        .collect::<Result<_, _>>()
                        .with_context(|| format!("unable to read components for {}", pkg))?;

                    components
                };

                struct Cab {
                    /// The max sequence number, each `File` in an MSI has a
                    /// sequence number that maps to exactly one CAB file
                    sequence: u32,
                    path: PathBuf,
                    cab: cab::Cabinet<std::io::Cursor<Vec<u8>>>,
                }

                // Load the cab file(s) from disk
                let mut cabs: Vec<_> = msi
                    .select_rows(msi::Select::table("Media"))
                    .with_context(|| format!("MSI {} has no 'Media' table", pkg))?
                    .filter_map(|row| -> Option<Result<_, _>> {
                        // Columns:
                        // 0 - DiskId
                        // 1 - LastSequence
                        // 2 - DiskPrompt
                        // 3 - Cabinet name
                        // ...
                        if row.len() < 4 {
                            Some(Err(anyhow::anyhow!("invalid row in 'Media'")))
                        } else {
                            row[1].as_int().filter(|seq| *seq > 0).and_then(|seq| {
                                row[3].as_str().map(|name| -> Result<_, Error> {
                                    let cab_path = {
                                        let mut pb = pkg.clone();
                                        pb.set_extension("");
                                        pb.push(name.trim_matches('"'));
                                        pb
                                    };

                                    let cab_contents =
                                        std::fs::read(&cab_path).with_context(|| {
                                            format!("unable to read CAB from path {}", cab_path)
                                        })?;

                                    up.compressed += cab_contents.len() as u64;

                                    let cab = cab::Cabinet::new(std::io::Cursor::new(cab_contents))
                                        .with_context(|| format!("CAB {} is invalid", cab_path))?;

                                    Ok(Cab {
                                        sequence: seq as u32,
                                        path: cab_path,
                                        cab,
                                    })
                                })
                            })
                        }
                    })
                    .collect::<Result<Vec<_>, Error>>()
                    .with_context(|| format!("unable to read CAB files for {}", pkg))?;

                anyhow::ensure!(!cabs.is_empty(), "no cab files were referenced by the MSI");

                // They are usually always sorted correctly, but you never know
                cabs.sort_by(|a, b| a.sequence.cmp(&b.sequence));

                struct CabFile {
                    id: String,
                    name: PathBuf,
                    //size: u64,
                    sequence: u32,
                }

                let mut files: Vec<_> = msi
                    .select_rows(msi::Select::table("File"))
                    .with_context(|| format!("MSI {} has no 'File' table", pkg))?
                    .filter_map(|row| -> Option<Result<_, Error>> {
                        // Columns:
                        // 0 - File Id (lookup in CAB)
                        // 1 - Component_ (target directory)
                        // 2 - FileName
                        // 3 - FileSize
                        // 4 - Version
                        // 5 - Language
                        // 6 - Attributes
                        // 7 - Sequence (determines which CAB file)
                        if row.len() < 8 {
                            return Some(Err(anyhow::anyhow!("invalid row in 'File'")));
                        }

                        let (dir, fname, id, seq) = match || -> Result<_, Error> {
                            let fname = fix_name(&row[2])?;
                            let dir = components
                                .get(row[1].as_str().context("component id was not a string")?)
                                .with_context(|| {
                                    format!("file {} referenced an unknown component", row[2])
                                })?;

                            let id = row[0].as_str().context("File (id) is not a string")?;

                            let seq = row[7].as_int().context("sequence is not an integer")? as u32;

                            Ok((dir, fname, id, seq))
                        }() {
                            Ok(items) => items,
                            Err(e) => return Err(e).transpose(),
                        };

                        if let Some(camino::Utf8Component::Normal(first)) = dir
                            .strip_prefix(&output_dir)
                            .ok()
                            .and_then(|rel| rel.components().nth(0))
                        {
                            match first {
                                "Catalogs" | "bin" | "Source" | "SourceDir" => {
                                    tracing::debug!("ignoring {}/{}", dir, fname);
                                    return None;
                                }
                                _ => {}
                            }
                        }

                        let cf = CabFile {
                            id: id.to_owned(),
                            name: dir.join(fname),
                            sequence: seq,
                        };

                        Some(Ok(cf))
                    })
                    .collect::<Result<Vec<_>, Error>>()
                    .with_context(|| format!("unable to read 'File' metadata for {}", pkg))?;

                files.sort_by(|a, b| a.sequence.cmp(&b.sequence));

                let mut file_skip = 0;

                for cabinet in &mut cabs {
                    let cab_sequence = cabinet.sequence;
                    for file in files
                        .iter()
                        .skip_while(|f| f.sequence <= file_skip)
                        .take_while(|f| f.sequence <= cab_sequence)
                    {
                        let mut cab_file = match cabinet.cab.read_file(file.id.as_str()) {
                            Ok(cf) => cf,
                            Err(e) => Err(e).with_context(|| {
                                format!("unable to read '{}' from {}", file.name, cabinet.path)
                            })?,
                        };

                        let unpack_path = output_dir.join(&file.name);

                        if let Some(parent) = unpack_path.parent() {
                            if !parent.exists() {
                                std::fs::create_dir_all(parent)?;
                            }
                        }

                        let mut unpacked_file = std::fs::File::create(&unpack_path)?;
                        up.decompressed += std::io::copy(&mut cab_file, &mut unpacked_file)?;
                    }

                    file_skip = cab_sequence;
                }

                Ok(up)
            }),
            _ => anyhow::bail!("unsupported package kind {:?}", payload.kind),
        };

        let up = unpack_task.await??;

        unpack_dir.push(".unpack");
        let um = serde_json::to_vec(&UnpackMeta {
            sha256: payload.sha256.clone(),
            compressed: up.compressed,
            decompressed: up.decompressed,
            num_files: up.num_files,
        })?;

        std::fs::write(&unpack_dir, &um)
            .with_context(|| format!("unable to write {}", unpack_dir))?;

        Ok(up)
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        if let Some(td) = self.tempdir.take() {
            let path = td.path().to_owned();
            if let Err(e) = td.close() {
                tracing::warn!(
                    path = ?path,
                    error = %e,
                    "unable to delete temporary directory",
                );
            }
        }
    }
}
