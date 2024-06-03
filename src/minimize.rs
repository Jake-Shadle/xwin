use crate::{util::canonicalize, Ctx, Path, PathBuf, SectionKind};
use anyhow::Context as _;

pub struct MinimizeConfig {
    pub include_debug_libs: bool,
    pub include_debug_symbols: bool,
    pub enable_symlinks: bool,
    pub use_winsysroot_style: bool,
    pub preserve_ms_arch_notation: bool,
    pub splat_output: PathBuf,
    pub copy: bool,
    pub minimize_output: Option<PathBuf>,
    pub map: PathBuf,
    pub target: String,
    pub manifest_path: PathBuf,
    pub preserve_strace: bool,
}

#[derive(Default)]
pub struct FileCounts {
    pub bytes: u64,
    pub count: u32,
}

pub struct FileNumbers {
    /// The counts for the total set of files
    pub total: FileCounts,
    /// The counts for the used set of files
    pub used: FileCounts,
}

pub struct MinimizeResults {
    pub crt_headers: FileNumbers,
    pub crt_libs: FileNumbers,
    pub sdk_headers: FileNumbers,
    pub sdk_libs: FileNumbers,
}

pub(crate) fn minimize(
    _ctx: std::sync::Arc<Ctx>,
    config: MinimizeConfig,
    roots: crate::splat::SplatRoots,
    sdk_version: &str,
) -> anyhow::Result<MinimizeResults> {
    let mut used_paths: std::collections::BTreeMap<
        PathBuf,
        (SectionKind, std::collections::BTreeSet<String>),
    > = std::collections::BTreeMap::new();

    let (used, total) = rayon::join(
        || -> anyhow::Result<_> {
            // Clean the output for the package, otherwise we'll miss headers if
            // C/C++ code has already been built
            let mut clean = std::process::Command::new("cargo");

            clean.args([
                "clean",
                "--target",
                &config.target,
                "--manifest-path",
                config.manifest_path.as_str(),
            ]);
            if !clean.status().map_or(false, |s| s.success()) {
                tracing::error!("failed to clean cargo target directory");
            }

            // Use a temporary (hopefully ramdisk) file to store the actual output
            // from strace, and just let the output from the build itself go
            // to stderr as normal
            let td = tempfile::tempdir().context("failed to create strace output file")?;
            let strace_output_path = td.path().join("strace_output.txt");

            if config.preserve_strace {
                let path = td.into_path();
                tracing::info!("strace output {}", path.display());
            }

            let mut strace = std::process::Command::new("strace");
            strace.args([
                // Follow forks, cargo spawns clang/lld
                "-f",
                // We only care about opens
                "-e",
                "trace=openat",
                "-o",
            ]);
            strace.arg(&strace_output_path);
            strace.args([
                "cargo",
                "build",
                "--target",
                &config.target,
                "--manifest-path",
                config.manifest_path.as_str(),
            ]);

            let splat_root = canonicalize(&config.splat_output)?;

            let includes = format!(
                "-Wno-unused-command-line-argument -fuse-ld=lld-link /vctoolsdir {splat_root}/crt /winsdkdir {splat_root}/sdk"
            );

            let mut libs = format!("-C linker=lld-link -Lnative={splat_root}/crt/lib/x86_64 -Lnative={splat_root}/sdk/lib/um/x86_64 -Lnative={splat_root}/sdk/lib/ucrt/x86_64");

            let rust_flags_env = format!(
                "CARGO_TARGET_{}_RUSTFLAGS",
                config.target.replace('-', "_").to_uppercase()
            );

            // Sigh, some people use RUSTFLAGS to enable hidden library features, incredibly annoying
            if let Ok(rf) = std::env::var(&rust_flags_env) {
                libs.push(' ');
                libs.push_str(&rf);
            } else if let Ok(rf) = std::env::var("RUSTFLAGS") {
                libs.push(' ');
                libs.push_str(&rf);
            }

            let triple = config.target.replace('-', "_");

            let cc_env = [
                (format!("CC_{triple}"), "clang-cl"),
                (format!("CXX_{triple}"), "clang-cl"),
                (format!("AR_{triple}"), "llvm-lib"),
                (format!("CFLAGS_{triple}"), &includes),
                (format!("CXXFLAGS_{triple}"), &includes),
                (rust_flags_env, &libs),
            ];

            strace.envs(cc_env);

            tracing::info!("compiling {}", config.manifest_path);

            let mut child = strace.spawn().context("unable to start strace")?;

            let (tx, rx) = crossbeam_channel::unbounded();

            // This should happen quickly
            let strace_output = {
                let start = std::time::Instant::now();
                let max = std::time::Duration::from_secs(10);
                loop {
                    match std::fs::File::open(&strace_output_path) {
                        Ok(f) => break f,
                        Err(err) => {
                            if start.elapsed() > max {
                                anyhow::bail!("failed to open strace output '{}' after waiting for {max:?}: {err}", strace_output_path.display());
                            }

                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }
                }
            };

            let mut output = std::io::BufReader::new(strace_output);

            let (_, counts) = rayon::join(
                move || -> anyhow::Result<()> {
                    use std::io::BufRead;
                    let mut line = String::new();

                    // We cannot use read_line/read_until here as Rust's BufRead
                    // will end a line on either the delimiter OR EOF, and since
                    // the file is being written to while we are reading, it is
                    // almost guaranteed we will hit EOF 1 or more times before
                    // an actual line is completed, given a large enough trace,
                    // so we roll our own
                    let mut read_line = |line: &mut String| -> anyhow::Result<bool> {
                        let buf = unsafe { line.as_mut_vec() };
                        loop {
                            let (done, used) = {
                                let available = match output.fill_buf() {
                                    Ok(n) => n,
                                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                                        continue
                                    }
                                    Err(e) => anyhow::bail!(e),
                                };
                                if let Some(i) = memchr::memchr(b'\n', available) {
                                    buf.extend_from_slice(&available[..=i]);
                                    (true, i + 1)
                                } else {
                                    buf.extend_from_slice(available);
                                    (false, available.len())
                                }
                            };
                            output.consume(used);
                            if done {
                                return Ok(true);
                            } else if used == 0
                                && child.try_wait().context("compile child failed")?.is_some()
                            {
                                return Ok(false);
                            }
                        }
                    };

                    loop {
                        line.clear();
                        if !read_line(&mut line)? {
                            break;
                        }

                        let Some(i) = line.find("openat(AT_FDCWD, \"") else {
                            continue;
                        };
                        let Some(open) = line[i + 18..].split_once('"') else {
                            continue;
                        };

                        // We can immediately skip file that were unable to be opened,
                        // but many file opens will be asynchronous so this won't
                        // catch all of them, but that's fine since we check for
                        // the existence in the other thread
                        if open.1.contains("-1 NOENT (") {
                            continue;
                        }

                        let _ = tx.send(open.0.to_owned());
                    }

                    drop(tx);
                    let status = child.wait()?;
                    anyhow::ensure!(status.success(), "compilation failed");

                    Ok(())
                },
                || {
                    let mut crt_headers = FileCounts::default();
                    let mut crt_libs = FileCounts::default();
                    let mut sdk_headers = FileCounts::default();
                    let mut sdk_libs = FileCounts::default();

                    let sdk_root = canonicalize(&roots.sdk).unwrap();
                    let crt_root = canonicalize(&roots.crt).unwrap();

                    while let Ok(path) = rx.recv() {
                        let path = PathBuf::from(path);
                        let (hdrs, libs, is_sdk) = if path.starts_with(&sdk_root) {
                            (&mut sdk_headers, &mut sdk_libs, true)
                        } else if path.starts_with(&crt_root) {
                            (&mut crt_headers, &mut crt_libs, false)
                        } else {
                            continue;
                        };

                        let (counts, which) = match path.extension() {
                            // Some headers don't have extensions, eg ciso646
                            Some("h" | "hpp") | None => (
                                hdrs,
                                if is_sdk {
                                    SectionKind::SdkHeader
                                } else {
                                    SectionKind::CrtHeader
                                },
                            ),
                            Some("lib" | "Lib") => (
                                libs,
                                if is_sdk {
                                    SectionKind::SdkLib
                                } else {
                                    SectionKind::CrtLib
                                },
                            ),
                            _ => continue,
                        };

                        let mut insert = |path: PathBuf, symlink: Option<String>| {
                            if let Some((_, sls)) = used_paths.get_mut(&path) {
                                sls.extend(symlink);
                                return;
                            }

                            let Ok(md) = std::fs::metadata(&path) else {
                                // clang will probe paths according to the include directories,
                                // and while we filter on NOENT in the strace output, in many
                                // cases the opens are async and this split over multiple lines,
                                // and while we _could_ keep a small buffer to pair the open results
                                // with the original thread that queued it, it's just simpler to
                                // just ignore paths that managed to get here that don't actually exist
                                return;
                            };

                            if !md.is_file() {
                                return;
                            }

                            counts.bytes += md.len();
                            counts.count += 1;

                            used_paths
                                .entry(path)
                                .or_insert_with(|| (which, Default::default()))
                                .1
                                .extend(symlink);
                        };

                        if path.is_symlink() {
                            // We're the ones creating symlinks and they are always utf-8
                            let sl = std::fs::read_link(&path).expect("failed to read symlink");
                            let sl = PathBuf::from_path_buf(sl).expect("symlink path was non-utf8");

                            let resolved = path.parent().unwrap().join(sl);
                            insert(resolved, Some(path.file_name().unwrap().to_owned()));
                        } else {
                            insert(path, None);
                        }
                    }

                    (crt_headers, crt_libs, sdk_headers, sdk_libs)
                },
            );

            Ok(counts)
        },
        || {
            let walk = |root: &Path| {
                let mut hdrs = FileCounts::default();
                let mut libs = FileCounts::default();
                let mut symlinks =
                    std::collections::BTreeMap::<PathBuf, std::collections::BTreeSet<String>>::new(
                    );

                let root = canonicalize(root).unwrap();

                for entry in walkdir::WalkDir::new(root)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let Some(path) = Path::from_path(entry.path()) else {
                        continue;
                    };

                    if entry.file_type().is_dir() {
                        continue;
                    }

                    if entry.path_is_symlink() {
                        let Ok(rp) = std::fs::read_link(path) else {
                            continue;
                        };

                        if !path.is_file() {
                            continue;
                        }

                        let Ok(real_path) = PathBuf::from_path_buf(rp) else {
                            continue;
                        };

                        symlinks
                            .entry(path.parent().unwrap().join(real_path))
                            .or_default()
                            .insert(path.file_name().unwrap().to_owned());
                    } else {
                        let Ok(md) = entry.metadata() else {
                            continue;
                        };

                        let which = match path.extension() {
                            Some("h" | "idl" | "hpp") | None => &mut hdrs,
                            Some("lib" | "Lib") => &mut libs,
                            _ => continue,
                        };

                        which.bytes += md.len();
                        which.count += 1;
                    }
                }

                (hdrs, libs, symlinks)
            };

            let (sdk, crt) = rayon::join(|| walk(&roots.sdk), || walk(&roots.crt));

            let mut symlinks = sdk.2;
            let mut crt_symlinks = crt.2;
            symlinks.append(&mut crt_symlinks);

            (crt.0, crt.1, sdk.0, sdk.1, symlinks)
        },
    );

    let used = used.context("unable to determine used file set")?;
    let symlinks = total.4;
    let root = canonicalize(&roots.root).unwrap();

    let mut additional_symlinks = 0;

    // For libraries, there are cases where strace doesn't actually detect
    // all the symlinks from which they are rereferenced, so just be conservative
    // and add all of them
    for (p, sls) in symlinks {
        if let Some((_, symlinks)) = used_paths.get_mut(&p) {
            let before = symlinks.len();
            symlinks.extend(sls);
            additional_symlinks += symlinks.len() - before;
        }
    }

    tracing::info!("added {additional_symlinks} additional symlinks");

    let (serialize, mv) = rayon::join(
        || -> anyhow::Result<()> {
            let cur_map = if config.map.exists() {
                match std::fs::read_to_string(&config.map) {
                    Ok(contents) => match toml::from_str::<crate::Map>(&contents) {
                        Ok(t) => Some(t),
                        Err(err) => {
                            tracing::error!(
                                path = config.map.as_str(),
                                error = ?err,
                                "failed to deserialize map file"
                            );
                            None
                        }
                    },
                    Err(err) => {
                        tracing::error!(
                            path = config.map.as_str(),
                            error = ?err,
                            "failed to read map file"
                        );
                        None
                    }
                }
            } else {
                None
            };

            let mut map = cur_map.unwrap_or_default();

            // We _could_ keep the original filters, but that would mean that the
            // user could just accumulate things over time that they aren't
            // actually using any longer, if this file is in source control then
            // they can just revert the changes if a file that was previously in
            // the list was removed
            map.clear();

            let crt_hdr_prefix = roots.crt.join("include");
            let crt_lib_prefix = roots.crt.join("lib");
            let sdk_hdr_prefix = {
                let mut sp = roots.sdk.clone();
                sp.push("Include");
                sp.push(sdk_version);
                sp
            };
            let sdk_lib_prefix = roots.sdk.join("lib");

            for (p, (which, sls)) in &used_paths {
                let (prefix, section) = match which {
                    SectionKind::SdkHeader => (&sdk_hdr_prefix, &mut map.sdk.headers),
                    SectionKind::SdkLib => (&sdk_lib_prefix, &mut map.sdk.libs),
                    SectionKind::CrtHeader => (&crt_hdr_prefix, &mut map.crt.headers),
                    SectionKind::CrtLib => (&crt_lib_prefix, &mut map.crt.libs),
                };

                let path = p
                    .strip_prefix(prefix)
                    .with_context(|| {
                        format!("path {p} did not begin with expected prefix {prefix}")
                    })
                    .unwrap()
                    .as_str()
                    .to_owned();

                if sls.is_empty() {
                    section.filter.insert(path);
                    continue;
                }

                section.filter.insert(path.clone());
                section.symlinks.insert(path, sls.iter().cloned().collect());
            }

            let serialized = toml::to_string_pretty(&map).unwrap();

            if let Err(err) = std::fs::write(&config.map, serialized) {
                tracing::error!(
                    path = config.map.as_str(),
                    error = ?err,
                    "failed to write map file"
                );
            }

            Ok(())
        },
        || -> anyhow::Result<()> {
            let Some(od) = config.minimize_output else {
                return Ok(());
            };

            if od.exists() {
                std::fs::remove_dir_all(&od).context("failed to clean output directory")?;
            }

            let mv = |up: &Path| -> anyhow::Result<PathBuf> {
                let np = od.join(up.strip_prefix(&root).unwrap());

                std::fs::create_dir_all(np.parent().unwrap())
                    .context("failed to create directories")?;

                if config.copy {
                    std::fs::copy(up, &np)
                        .with_context(|| format!("failed to copy {up} => {np}"))?;
                } else {
                    std::fs::rename(up, &np)
                        .with_context(|| format!("failed to move {up} => {np}"))?;
                }

                Ok(np)
            };

            for (up, (_, sls)) in &used_paths {
                let np = mv(up)?;

                for sl in sls {
                    let sl = np.parent().unwrap().join(sl);
                    crate::symlink(np.file_name().unwrap(), &sl)
                        .context("failed to create link")?;
                }
            }

            Ok(())
        },
    );

    serialize?;
    mv?;

    Ok(MinimizeResults {
        crt_headers: FileNumbers {
            total: total.0,
            used: used.0,
        },
        crt_libs: FileNumbers {
            total: total.1,
            used: used.1,
        },
        sdk_headers: FileNumbers {
            total: total.2,
            used: used.2,
        },
        sdk_libs: FileNumbers {
            total: total.3,
            used: used.3,
        },
    })
}
