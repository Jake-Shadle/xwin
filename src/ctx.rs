use crate::{
    splat::SdkHeaders,
    util::{ProgressTarget, Sha256},
    Path, PathBuf, WorkItem,
};
use anyhow::{Context as _, Error};

#[allow(dead_code)]
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
    pub fn with_temp(dt: ProgressTarget, client: ureq::Agent) -> Result<Self, Error> {
        let td = tempfile::TempDir::new()?;

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
        client: ureq::Agent,
    ) -> Result<Self, Error> {
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

    #[allow(clippy::too_many_arguments)]
    pub fn execute(
        self: std::sync::Arc<Self>,
        packages: std::collections::BTreeMap<String, crate::manifest::ManifestItem>,
        payloads: Vec<WorkItem>,
        crt_version: String,
        sdk_version: String,
        arches: u32,
        variants: u32,
        ops: crate::Ops,
    ) -> Result<(), Error> {
        use rayon::prelude::*;

        let packages = std::sync::Arc::new(packages);

        let mut results = Vec::new();
        let crt_ft = parking_lot::Mutex::new(None);
        let atl_ft = parking_lot::Mutex::new(None);

        let mut splat_config = match &ops {
            crate::Ops::Splat(config) => {
                let splat_roots = crate::splat::prep_splat(
                    self.clone(),
                    &config.output,
                    config.use_winsysroot_style.then_some(&crt_version),
                )?;
                let mut config = config.clone();
                config.output = splat_roots.root.clone();

                Some((splat_roots, config))
            }
            crate::Ops::Minimize(config) => {
                let splat_roots = crate::splat::prep_splat(
                    self.clone(),
                    &config.splat_output,
                    config.use_winsysroot_style.then_some(&crt_version),
                )?;

                let config = crate::SplatConfig {
                    preserve_ms_arch_notation: config.preserve_ms_arch_notation,
                    include_debug_libs: config.include_debug_libs,
                    include_debug_symbols: config.include_debug_symbols,
                    enable_symlinks: config.enable_symlinks,
                    use_winsysroot_style: config.use_winsysroot_style,
                    output: splat_roots.root.clone(),
                    map: Some(config.map.clone()),
                    copy: config.copy,
                };

                Some((splat_roots, config))
            }
            _ => None,
        };

        // Detect if the output root directory is case sensitive or not,
        // if it's not, disable symlinks as they won't work
        let enable_symlinks = if let Some((root, sc_enable_symlinks)) =
            splat_config.as_mut().and_then(|(sr, c)| {
                c.enable_symlinks
                    .then_some((&sr.root, &mut c.enable_symlinks))
            }) {
            let test_path = root.join("BIG.xwin");
            std::fs::write(&test_path, "").with_context(|| {
                format!("failed to write case-sensitivity test file {test_path}")
            })?;

            let enable_symlinks = if std::fs::read(root.join("big.xwin")).is_ok() {
                tracing::warn!("detected splat root '{root}' is on a case-sensitive file system, disabling symlinks");
                false
            } else {
                true
            };

            // Will be ugly but won't harm anything if file is left
            let _ = std::fs::remove_file(test_path);
            *sc_enable_symlinks = enable_symlinks;
            enable_symlinks
        } else {
            false
        };

        let map = if let Some(map) = splat_config.as_ref().and_then(|(_, sp)| sp.map.as_ref()) {
            match std::fs::read_to_string(map) {
                Ok(m) => Some(
                    toml::from_str::<crate::Map>(&m)
                        .with_context(|| format!("failed to deserialize '{map}'"))?,
                ),
                Err(err) => {
                    if !matches!(err.kind(), std::io::ErrorKind::NotFound) {
                        tracing::error!("unable to read mapping from '{map}': {err}");
                    }
                    None
                }
            }
        } else {
            None
        };

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

                let sdk_headers = if let Some((splat_roots, config)) = &splat_config {
                    crate::splat::splat(
                        config,
                        splat_roots,
                        &wi,
                        &ft,
                        map.as_ref()
                            .filter(|_m| !matches!(ops, crate::Ops::Minimize(_))),
                        &sdk_version,
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

        let Some((roots, sc)) = splat_config else {
            return Ok(());
        };

        let splat_links = || -> anyhow::Result<()> {
            if enable_symlinks {
                let crt_ft = crt_ft.lock().take();
                let atl_ft = atl_ft.lock().take();

                crate::splat::finalize_splat(
                    &self,
                    sc.use_winsysroot_style.then_some(&sdk_version),
                    &roots,
                    sdk_headers,
                    crt_ft,
                    atl_ft,
                )?;
            }

            Ok(())
        };

        match ops {
            crate::Ops::Minimize(config) => {
                splat_links()?;
                let results = crate::minimize::minimize(self, config, roots, &sdk_version)?;

                fn emit(name: &str, num: crate::minimize::FileNumbers) {
                    fn hb(bytes: u64) -> String {
                        let mut bytes = bytes as f64;

                        for unit in ["B", "KiB", "MiB", "GiB"] {
                            if bytes > 1024.0 {
                                bytes /= 1024.0;
                            } else {
                                return format!("{bytes:.1}{unit}");
                            }
                        }

                        "this seems bad".to_owned()
                    }

                    let ratio = (num.used.bytes as f64 / num.total.bytes as f64) * 100.0;

                    println!(
                        "  {name}: {}({}) / {}({}) => {ratio:.02}%",
                        num.used.count,
                        hb(num.used.bytes),
                        num.total.count,
                        hb(num.total.bytes),
                    );
                }

                emit("crt headers", results.crt_headers);
                emit("crt libs", results.crt_libs);
                emit("sdk headers", results.sdk_headers);
                emit("sdk libs", results.sdk_libs);
            }
            crate::Ops::Splat(_config) => {
                if map.is_none() {
                    splat_links()?;
                }
            }
            _ => {}
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
