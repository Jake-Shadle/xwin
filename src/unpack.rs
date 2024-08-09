use crate::{download::PayloadContents, Ctx, Error, Path, PathBuf};
use anyhow::Context as _;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct UnpackMeta {
    #[serde(serialize_with = "crate::util::serialize_sha256")]
    pub(crate) sha256: crate::util::Sha256,
    pub(crate) compressed: u64,
    pub(crate) decompressed: u64,
    pub(crate) num_files: u32,
}

#[derive(Debug)]
pub(crate) struct FileTree {
    pub(crate) files: Vec<(PathBuf, u64)>,
    pub(crate) dirs: Vec<(PathBuf, FileTree)>,
}

impl FileTree {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            dirs: Vec::new(),
        }
    }

    fn push(&mut self, path: &Path, size: u64) {
        let fname = path.file_name().unwrap();
        let mut tree = self;

        for comp in path.iter() {
            if comp != fname {
                #[allow(clippy::single_match_else)]
                match tree.dirs.iter().position(|(dir, _tree)| dir == comp) {
                    Some(t) => tree = &mut tree.dirs[t].1,
                    None => {
                        tree.dirs.push((comp.into(), FileTree::new()));
                        tree = &mut tree.dirs.last_mut().unwrap().1;
                    }
                }
            } else {
                tree.files.push((fname.into(), size));
            }
        }
    }

    pub(crate) fn stats(&self) -> (u32, u64) {
        self.dirs.iter().fold(
            (
                self.files.len() as u32,
                self.files.iter().map(|(_, size)| *size).sum(),
            ),
            |(num_files, size), tree| {
                let stats = tree.1.stats();
                (num_files + stats.0, size + stats.1)
            },
        )
    }

    pub(crate) fn subtree(&self, path: &Path) -> Option<&FileTree> {
        let mut tree = self;

        for comp in path.iter() {
            match tree.dirs.iter().find(|dir| dir.0 == comp) {
                Some(t) => tree = &t.1,
                None => return None,
            }
        }

        Some(tree)
    }
}

fn read_unpack_dir(root: PathBuf) -> Result<FileTree, Error> {
    let mut root_tree = FileTree::new();

    fn read(src: PathBuf, tree: &mut FileTree) -> Result<(), Error> {
        for entry in std::fs::read_dir(&src).with_context(|| format!("unable to read {src}"))? {
            let entry = entry.with_context(|| format!("unable to read entry from {src}"))?;

            let src_name = PathBuf::from_path_buf(entry.file_name().into()).map_err(|_pb| {
                anyhow::anyhow!(
                    "src path {} is not a valid utf-8 path",
                    entry.path().display()
                )
            })?;

            if src_name == ".unpack" {
                continue;
            }

            let metadata = entry.metadata().with_context(|| {
                format!("unable to get metadata for {}", entry.path().display())
            })?;

            let ft = metadata.file_type();

            if ft.is_dir() {
                let mut dir_tree = FileTree::new();
                read(src.join(&src_name), &mut dir_tree)?;

                tree.dirs.push((src_name, dir_tree));
            } else if ft.is_file() {
                tree.files.push((src_name, metadata.len()));
            } else if ft.is_symlink() {
                anyhow::bail!(
                    "detected symlink {} in source directory which should be impossible",
                    entry.path().display()
                );
            }
        }

        Ok(())
    }

    read(root, &mut root_tree)?;

    Ok(root_tree)
}

pub(crate) fn unpack(
    ctx: std::sync::Arc<Ctx>,
    item: &crate::WorkItem,
    contents: PayloadContents,
) -> Result<FileTree, Error> {
    item.progress.reset();
    item.progress.set_message("📂 unpacking...");

    let output_dir = match ctx.prep_unpack(&item.payload)? {
        crate::ctx::Unpack::Present { output_dir, .. } => {
            return read_unpack_dir(output_dir);
        }
        crate::ctx::Unpack::Needed(od) => od,
    };

    let pkg = &item.payload.filename;

    let (tree, compressed) = match contents {
        PayloadContents::Vsix(vsix) => {
            let mut tree = FileTree::new();

            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(vsix))
                .with_context(|| format!("invalid zip {pkg}"))?;

            // VSIX files are just a "specially" formatted zip file, all
            // of the actual files we want are under "Contents"
            let mut to_extract = Vec::new();
            let mut total_uncompressed = 0;

            for findex in 0..zip.len() {
                let file = zip.by_index_raw(findex)?;

                let fname = file.name();

                if fname.starts_with("Contents/")
                    && (fname.contains("lib") || fname.contains("include"))
                {
                    to_extract.push(findex);
                    total_uncompressed += file.size();
                }
            }

            item.progress.set_length(total_uncompressed);

            let mut total_compressed = 0;

            for findex in to_extract {
                let mut file = zip.by_index(findex).unwrap();
                let zip_path = Path::new(file.name());
                let mut fs_path = output_dir.clone();

                for comp in zip_path
                    .components()
                    .skip_while(|comp| comp.as_str() != "lib" && comp.as_str() != "include")
                {
                    fs_path.push(comp);
                }

                if let Some(parent) = fs_path.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent)
                            .with_context(|| format!("unable to create unpack dir '{parent}'"))?;
                    }
                }

                let mut dest = std::fs::File::create(&fs_path).with_context(|| {
                    format!(
                        "unable to create {fs_path} to decompress {} from {pkg}",
                        file.name(),
                    )
                })?;

                let decompressed = std::io::copy(&mut file, &mut dest).with_context(|| {
                    format!(
                        "unable to decompress {} from {pkg} to {fs_path}",
                        file.name(),
                    )
                })?;

                item.progress.inc(decompressed);

                let tree_path = fs_path.strip_prefix(&output_dir).unwrap();
                tree.push(tree_path, decompressed);

                total_compressed += file.compressed_size();
            }

            (tree, total_compressed)
        }
        PayloadContents::Msi { msi, cabs } => {
            let mut msi = msi::Package::open(std::io::Cursor::new(msi))
                .with_context(|| format!("unable to read MSI from {pkg}"))?;

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
                    .with_context(|| format!("unable to read directories for {pkg}"))?;

                directories.sort_by(|a, b| a.id.cmp(&b.id));

                let components: std::collections::BTreeMap<_, _> = msi
                    .select_rows(msi::Select::table("Component"))
                    .with_context(|| format!("MSI {pkg} has no 'Directory' table"))?
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
                            #[allow(clippy::single_match_else)]
                            let cur_dir = match dirs.binary_search_by(|d| d.id.as_str().cmp(id)) {
                                Ok(i) => &dirs[i],
                                Err(_) => {
                                    tracing::warn!("unable to find directory {id}");
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
                                        if !other.starts_with(|c: char| c.is_ascii_digit()) {
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

                        let mut dir = PathBuf::new();
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
                    .with_context(|| format!("unable to read components for {pkg}"))?;

                components
            };

            struct Cab {
                /// The max sequence number, each `File` in an MSI has a
                /// sequence number that maps to exactly one CAB file
                sequence: u32,
                path: PathBuf,
                cab: bytes::Bytes,
            }

            let cabs = {
                let mut cab_contents = Vec::with_capacity(cabs.len());

                for cab in cabs {
                    // Validate the cab file
                    cab::Cabinet::new(std::io::Cursor::new(cab.content.clone()))
                        .with_context(|| format!("CAB {} is invalid", cab.path))?;

                    cab_contents.push(Cab {
                        sequence: cab.sequence,
                        path: cab.path,
                        cab: cab.content,
                    });
                }

                // They are usually always sorted correctly, but you never know
                cab_contents.sort_by(|a, b| a.sequence.cmp(&b.sequence));
                cab_contents
            };

            anyhow::ensure!(!cabs.is_empty(), "no cab files were referenced by the MSI");

            struct CabFile {
                id: String,
                name: PathBuf,
                size: u64,
                sequence: u32,
            }

            let (files, uncompressed) = {
                let mut uncompressed = 0u64;
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

                        #[allow(clippy::blocks_in_conditions)]
                        let (dir, fname, id, seq, size) = match || -> Result<_, Error> {
                            let fname = fix_name(&row[2])?;
                            let dir = components
                                .get(row[1].as_str().context("component id was not a string")?)
                                .with_context(|| {
                                    format!("file {} referenced an unknown component", row[2])
                                })?;

                            let size = row[3].as_int().context("size is not an integer")? as u64;
                            let id = row[0].as_str().context("File (id) is not a string")?;
                            let seq = row[7].as_int().context("sequence is not an integer")? as u32;

                            Ok((dir, fname, id, seq, size))
                        }() {
                            Ok(items) => items,
                            Err(e) => return Err(e).transpose(),
                        };

                        if let Some(camino::Utf8Component::Normal(
                            "Catalogs" | "bin" | "Source" | "SourceDir",
                        )) = dir
                            .strip_prefix(&output_dir)
                            .ok()
                            .and_then(|rel| rel.components().next())
                        {
                            return None;
                        }

                        uncompressed += size;

                        let cf = CabFile {
                            id: id.to_owned(),
                            name: dir.join(fname),
                            sequence: seq,
                            size,
                        };

                        Some(Ok(cf))
                    })
                    .collect::<Result<Vec<_>, Error>>()
                    .with_context(|| format!("unable to read 'File' metadata for {pkg}"))?;

                files.sort_by(|a, b| a.sequence.cmp(&b.sequence));

                (files, uncompressed)
            };

            item.progress.set_length(uncompressed);

            // Some MSIs have a lot of cabs and take an _extremely_ long time to
            // decompress, so we just split the files into roughly equal sized
            // chunks and decompress in parallel to reduce wall time
            let mut chunks = Vec::new();

            struct Chunk {
                cab: bytes::Bytes,
                cab_index: usize,
                files: Vec<CabFile>,
                chunk_size: u64,
            }

            chunks.push(Chunk {
                cab: cabs[0].cab.clone(),
                cab_index: 0,
                files: Vec::new(),
                chunk_size: 0,
            });

            let mut cur_chunk = 0;
            let mut cur_cab = 0;
            const CHUNK_SIZE: u64 = 1024 * 1024;

            for file in files {
                let chunk = &mut chunks[cur_chunk];

                if chunk.chunk_size + file.size < CHUNK_SIZE
                    && file.sequence <= cabs[cur_cab].sequence
                {
                    chunk.chunk_size += file.size;
                    chunk.files.push(file);
                } else {
                    let cab = if file.sequence <= cabs[cur_cab].sequence {
                        chunk.cab.clone()
                    } else {
                        match cabs[cur_cab + 1..]
                            .iter()
                            .position(|cab| file.sequence <= cab.sequence)
                        {
                            Some(i) => cur_cab += i + 1,
                            None => anyhow::bail!(
                                "unable to find cab file containing {} {}",
                                file.name,
                                file.sequence
                            ),
                        }

                        cabs[cur_cab].cab.clone()
                    };

                    cur_chunk += 1;
                    chunks.push(Chunk {
                        cab,
                        cab_index: cur_cab,
                        chunk_size: file.size,
                        files: vec![file],
                    });
                }
            }

            let mut results = Vec::new();

            use rayon::prelude::*;

            let tree = parking_lot::Mutex::new(FileTree::new());

            chunks
                .into_par_iter()
                .map(|chunk| -> Result<(), Error> {
                    let mut cab = cab::Cabinet::new(std::io::Cursor::new(chunk.cab)).unwrap();

                    let cab_path = &cabs[chunk.cab_index].path;

                    for file in chunk.files {
                        let mut cab_file = match cab.read_file(file.id.as_str()) {
                            Ok(cf) => cf,
                            Err(e) => Err(e).with_context(|| {
                                format!("unable to read '{}' from {cab_path}", file.name)
                            })?,
                        };

                        let unpack_path = output_dir.join(&file.name);

                        if let Some(parent) = unpack_path.parent() {
                            if !parent.exists() {
                                std::fs::create_dir_all(parent)?;
                            }
                        }

                        let unpacked_file = std::fs::File::create(&unpack_path)?;

                        struct Wrapper<'pb> {
                            pb: &'pb indicatif::ProgressBar,
                            uf: std::fs::File,
                        }

                        impl<'pb> std::io::Write for Wrapper<'pb> {
                            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                                self.pb.inc(buf.len() as u64);
                                self.uf.write(buf)
                            }

                            fn flush(&mut self) -> std::io::Result<()> {
                                self.uf.flush()
                            }
                        }

                        let size = std::io::copy(
                            &mut cab_file,
                            &mut Wrapper {
                                pb: &item.progress,
                                uf: unpacked_file,
                            },
                        )?;

                        tree.lock().push(&file.name, size);
                    }

                    Ok(())
                })
                .collect_into_vec(&mut results);

            (tree.into_inner(), uncompressed)
        }
    };

    let tree_path = format!("{output_dir}/tree.txt");

    std::fs::write(&tree_path, format!("{tree:#?}").as_bytes())
        .with_context(|| format!("failed to write {tree_path}"))?;

    item.progress.finish_with_message("unpacked");

    let (num_files, decompressed) = tree.stats();

    ctx.finish_unpack(
        output_dir,
        UnpackMeta {
            sha256: item.payload.sha256.clone(),
            compressed,
            decompressed,
            num_files,
        },
    )?;

    Ok(tree)
}
