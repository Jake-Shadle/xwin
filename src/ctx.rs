use crate::{util::Sha256, Path, PathBuf, WorkItem};
use anyhow::{Context as _, Error};

pub enum Unpack {
    Present {
        output_dir: PathBuf,
        compressed: u64,
        decompressed: u64,
        num_files: u32,
    },
    Needed(PathBuf),
}

pub struct Stats {}

pub struct Ctx {
    pub work_dir: PathBuf,
    pub tempdir: Option<tempfile::TempDir>,
    pub client: reqwest::blocking::Client,
}

impl Ctx {
    pub fn with_temp() -> Result<Self, Error> {
        let td = tempfile::TempDir::new()?;
        let client = reqwest::blocking::ClientBuilder::new().build()?;

        Ok(Self {
            work_dir: PathBuf::from_path_buf(td.path().to_owned()).map_err(|pb| {
                anyhow::anyhow!("tempdir {} is not a valid utf-8 path", pb.display())
            })?,
            tempdir: Some(td),
            client,
        })
    }

    pub fn with_dir(mut work_dir: PathBuf) -> Result<Self, Error> {
        let client = reqwest::blocking::ClientBuilder::new().build()?;

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

    pub fn get_and_validate<P>(
        &self,
        url: impl AsRef<str>,
        path: &P,
        checksum: Option<Sha256>,
        progress: indicatif::ProgressBar,
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

        if cache_path.exists() {
            tracing::debug!("verifying existing cached dl file");

            match std::fs::read(&cache_path) {
                Ok(contents) => match &checksum {
                    Some(expected) => {
                        let chksum = Sha256::digest(&contents);

                        if chksum != *expected {
                            tracing::warn!(
                                "checksum mismatch, expected {} != actual {}",
                                expected,
                                chksum
                            );
                        } else {
                            progress.inc_length(contents.len() as u64);
                            progress.inc(contents.len() as u64);
                            return Ok(contents.into());
                        }
                    }
                    None => {
                        progress.inc_length(contents.len() as u64);
                        progress.inc(contents.len() as u64);
                        return Ok(contents.into());
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read cached file");
                }
            }
        }

        let mut res = self.client.get(url.as_ref()).send()?.error_for_status()?;

        let content_length = res.content_length().unwrap_or_default();
        progress.inc_length(content_length);

        let body = bytes::BytesMut::with_capacity(content_length as usize);

        struct ProgressCopy {
            progress: indicatif::ProgressBar,
            inner: bytes::buf::Writer<bytes::BytesMut>,
        }

        impl std::io::Write for ProgressCopy {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.progress.inc(buf.len() as u64);
                self.inner.write(buf)
            }

            fn flush(&mut self) -> std::io::Result<()> {
                self.inner.flush()
            }
        }

        use bytes::BufMut;

        let mut pc = ProgressCopy {
            progress,
            inner: body.writer(),
        };

        res.copy_to(&mut pc)?;

        let body = pc.inner.into_inner().freeze();

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
    }

    pub fn execute(
        self: std::sync::Arc<Self>,
        packages: std::collections::BTreeMap<String, crate::manifest::ManifestItem>,
        payloads: Vec<WorkItem>,
        arches: u32,
        variants: u32,
        ops: crate::Ops,
    ) -> Result<(), Error> {
        use rayon::prelude::*;

        let packages = std::sync::Arc::new(packages);

        let splat_roots = if let crate::Ops::Splat(config) = &ops {
            Some(crate::splat::prep_splat(self.clone(), config)?)
        } else {
            None
        };

        let mut results = Vec::new();
        let sdk_files =
            std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));

        payloads
            .into_par_iter()
            .map(|wi| -> Result<Stats, Error> {
                let payload_contents =
                    crate::download::download(self.clone(), packages.clone(), &wi)?;

                if let crate::Ops::Download = ops {
                    return Ok(Stats {});
                }

                let ft = crate::unpack::unpack(self.clone(), &wi, payload_contents)?;

                if let crate::Ops::Unpack = ops {
                    return Ok(Stats {});
                }

                if let crate::Ops::Splat(config) = &ops {
                    crate::splat::splat(
                        config,
                        splat_roots.as_ref().unwrap(),
                        &wi,
                        ft,
                        arches,
                        variants,
                        sdk_files.clone(),
                    )
                    .with_context(|| format!("failed to splat {}", wi.payload.filename))?;
                }

                Ok(Stats {})
            })
            .collect_into_vec(&mut results);

        results.into_iter().collect::<Result<Vec<_>, _>>()?;

        if let Some(roots) = splat_roots {
            crate::splat::finalize_splat(&roots, sdk_files)?;
        }

        Ok(())
    }

    pub(crate) fn prep_unpack(&self, payload: &crate::Payload) -> Result<Unpack, Error> {
        let mut unpack_dir = {
            let mut pb = self.work_dir.clone();
            pb.push("unpack");
            pb.push(&payload.filename);
            pb
        };

        unpack_dir.push(".unpack");

        if let Ok(unpack) = std::fs::read(&unpack_dir) {
            if let Ok(um) = serde_json::from_slice::<crate::unpack::UnpackMeta>(&unpack) {
                if payload.sha256 == um.sha256 {
                    tracing::debug!("already unpacked");
                    unpack_dir.pop();
                    return Ok(Unpack::Present {
                        output_dir: unpack_dir,
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

        Ok(Unpack::Needed(unpack_dir))
    }

    #[allow(clippy::unused_self)]
    pub(crate) fn finish_unpack(
        &self,
        mut unpack_dir: PathBuf,
        um: crate::unpack::UnpackMeta,
    ) -> Result<(), Error> {
        unpack_dir.push(".unpack");
        let um = serde_json::to_vec(&um)?;

        std::fs::write(&unpack_dir, &um)
            .with_context(|| format!("unable to write {}", unpack_dir))?;
        Ok(())
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
