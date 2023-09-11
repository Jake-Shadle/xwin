use std::time::Duration;

use crate::{
    splat::SdkHeaders,
    util::{ProgressTarget, Sha256},
    Path, PathBuf, WorkItem,
};
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

pub struct Ctx {
    pub work_dir: PathBuf,
    pub tempdir: Option<tempfile::TempDir>,
    pub client: ureq::Agent,
    pub draw_target: ProgressTarget,
}

impl Ctx {
    fn http_client(read_timeout: Option<Duration>) -> Result<ureq::Agent, Error> {
        let mut builder = ureq::builder();

        #[cfg(feature = "native-tls")]
        {
            use std::sync::Arc;
            builder = builder.tls_connector(Arc::new(native_tls_crate::TlsConnector::new()?));
        }

        // Allow user to specify timeout values in the case of bad/slow proxies
        // or MS itself being terrible, but default to a minute, which is _far_
        // more than it should take in normal situations, as by default ureq
        // sets no timeout on the response
        builder = builder.timeout_read(read_timeout.unwrap_or(Duration::from_secs(60)));

        if let Ok(proxy) = std::env::var("https_proxy") {
            let proxy = ureq::Proxy::new(proxy)?;
            builder = builder.proxy(proxy);
        }
        Ok(builder.build())
    }

    pub fn with_temp(dt: ProgressTarget, read_timeout: Option<Duration>) -> Result<Self, Error> {
        let td = tempfile::TempDir::new()?;
        let client = Self::http_client(read_timeout)?;

        Ok(Self {
            work_dir: PathBuf::from_path_buf(td.path().to_owned()).map_err(|pb| {
                anyhow::anyhow!("tempdir {} is not a valid utf-8 path", pb.display())
            })?,
            tempdir: Some(td),
            client,
            draw_target: dt,
        })
    }

    pub fn with_dir(
        mut work_dir: PathBuf,
        dt: ProgressTarget,
        read_timeout: Option<Duration>,
    ) -> Result<Self, Error> {
        let client = Self::http_client(read_timeout)?;

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
            draw_target: dt,
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

        let res = self.client.get(url.as_ref()).call()?;

        let content_length = res
            .header("content-length")
            .and_then(|header| header.parse().ok())
            .unwrap_or_default();
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

        std::io::copy(&mut res.into_reader(), &mut pc)?;

        let body = pc.inner.into_inner().freeze();

        if let Some(expected) = checksum {
            let chksum = Sha256::digest(&body);

            anyhow::ensure!(
                chksum == expected,
                "checksum mismatch, expected {expected} != actual {chksum}"
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

        let (splat_roots, enable_symlinks) = if let crate::Ops::Splat(config) = &ops {
            (
                Some(crate::splat::prep_splat(self.clone(), config)?),
                config.enable_symlinks,
            )
        } else {
            (None, false)
        };

        let mut results = Vec::new();
        let crt_ft = parking_lot::Mutex::new(None);
        let atl_ft = parking_lot::Mutex::new(None);

        payloads
            .into_par_iter()
            .map(|wi| -> Result<Option<SdkHeaders>, Error> {
                let payload_contents =
                    crate::download::download(self.clone(), packages.clone(), &wi)?;

                if let crate::Ops::Download = ops {
                    return Ok(None);
                }

                let ft = crate::unpack::unpack(self.clone(), &wi, payload_contents)?;

                if let crate::Ops::Unpack = ops {
                    return Ok(None);
                }

                let sdk_headers = if let crate::Ops::Splat(config) = &ops {
                    crate::splat::splat(
                        config,
                        splat_roots.as_ref().unwrap(),
                        &wi,
                        &ft,
                        arches,
                        variants,
                    )
                    .with_context(|| format!("failed to splat {}", wi.payload.filename))?
                } else {
                    None
                };

                match wi.payload.kind {
                    crate::PayloadKind::CrtHeaders => *crt_ft.lock() = Some(ft),
                    crate::PayloadKind::AtlHeaders => *atl_ft.lock() = Some(ft),
                    _ => {}
                }

                Ok(sdk_headers)
            })
            .collect_into_vec(&mut results);

        let sdk_headers = results.into_iter().collect::<Result<Vec<_>, _>>()?;
        let sdk_headers = sdk_headers.into_iter().flatten().collect();

        if let Some(roots) = splat_roots {
            if enable_symlinks {
                let crt_ft = crt_ft.lock().take();
                let atl_ft = atl_ft.lock().take();

                crate::splat::finalize_splat(&self, &roots, sdk_headers, crt_ft, atl_ft)?;
            }
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
                .with_context(|| format!("unable to remove invalid unpack dir '{unpack_dir}'"))?;
        }

        std::fs::create_dir_all(&unpack_dir)
            .with_context(|| format!("unable to create unpack dir '{unpack_dir}'"))?;

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

        std::fs::write(&unpack_dir, um).with_context(|| format!("unable to write {unpack_dir}"))?;
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
