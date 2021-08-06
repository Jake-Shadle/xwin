use crate::{Arch, Ctx, Error, Payload, PayloadKind, Variant};
use anyhow::Context as _;
use camino::Utf8PathBuf as PathBuf;

pub struct PackConfig {
    pub include_debug_libs: bool,
    pub include_debug_symbols: bool,
    pub disable_symlinks: bool,
    pub preserve_ms_arch_notation: bool,
    pub output: PathBuf,
    //pub isolated: bool,
}

pub fn pack(
    ctx: std::sync::Arc<Ctx>,
    config: PackConfig,
    items: Vec<Payload>,
    arches: u32,
    variants: u32,
) -> Result<(), Error> {
    struct Mapping {
        src: PathBuf,
        //payload: Payload,
        kind: PayloadKind,
        variant: Option<Variant>,
        target: PathBuf,
    }

    let crt_root = config.output.join("crt");
    let sdk_root = config.output.join("sdk");

    if crt_root.exists() {
        std::fs::remove_dir_all(&crt_root)
            .with_context(|| format!("unable to delete existing CRT directory {}", crt_root))?;
    }

    if sdk_root.exists() {
        std::fs::remove_dir_all(&sdk_root)
            .with_context(|| format!("unable to delete existing SDK directory {}", sdk_root))?;
    }

    std::fs::create_dir_all(&crt_root)
        .with_context(|| format!("unable to create CRT directory {}", crt_root))?;
    std::fs::create_dir_all(&sdk_root)
        .with_context(|| format!("unable to create SDK directory {}", sdk_root))?;

    let src_root = ctx.work_dir.join("unpack");

    let mappings = items
        .into_iter()
        .map(|payload| -> Result<_, Error> {
            match payload.kind {
                PayloadKind::CrtLibs => {
                    payload
                        .variant
                        .context("CRT libs didn't specify a variant")?;
                    payload
                        .target_arch
                        .context("CRT libs didnt' specify an architecture")?;
                }
                PayloadKind::SdkLibs => {
                    payload
                        .target_arch
                        .context("SDK libs didn't specify an architecture")?;
                }
                _ => {}
            }

            Ok(payload)
        })
        .flat_map(|payload| {
            let payload = match payload {
                Ok(payload) => payload,
                Err(e) => return vec![Err(e)],
            };

            let mut src = src_root.join(&payload.filename);
            let variant = payload.variant;
            let kind = payload.kind;

            match payload.kind {
                PayloadKind::CrtHeaders => {
                    src.push("include");

                    vec![Ok(Mapping {
                        src,
                        target: crt_root.join("include"),
                        kind,
                        variant,
                    })]
                }
                PayloadKind::CrtLibs => {
                    src.push("lib");
                    let mut target = crt_root.join("lib");

                    let spectre = (variants & Variant::Spectre as u32) != 0;

                    match payload.variant.unwrap() {
                        Variant::Desktop => {
                            if spectre {
                                src.push("spectre");
                                target.push("spectre");
                            }
                        }
                        Variant::OneCore => {
                            if spectre {
                                src.push("spectre");
                                target.push("spectre");
                            }

                            src.push("onecore");
                            target.push("onecore");
                        }
                        Variant::Store => {}
                        Variant::Spectre => unreachable!(),
                    }

                    let arch = payload.target_arch.unwrap();
                    src.push(arch.as_ms_str());
                    target.push(if config.preserve_ms_arch_notation {
                        arch.as_ms_str()
                    } else {
                        arch.as_str()
                    });

                    vec![Ok(Mapping {
                        src,
                        target,
                        kind,
                        variant,
                    })]
                }
                PayloadKind::SdkHeaders => {
                    src.push("include");

                    vec![Ok(Mapping {
                        src,
                        target: sdk_root.join("include"),
                        kind,
                        variant,
                    })]
                }
                PayloadKind::SdkLibs => {
                    src.push("lib/um");
                    let mut target = sdk_root.join("lib/um");

                    let arch = payload.target_arch.unwrap();
                    src.push(arch.as_ms_str());
                    target.push(if config.preserve_ms_arch_notation {
                        arch.as_ms_str()
                    } else {
                        arch.as_str()
                    });

                    vec![Ok(Mapping {
                        src,
                        target,
                        kind,
                        variant,
                    })]
                }
                PayloadKind::SdkStoreLibs => {
                    src.push("lib/um");
                    let target = sdk_root.join("lib/um");
                    Arch::iter(arches)
                        .map(|arch| {
                            Ok(Mapping {
                                src: src.join(arch.as_ms_str()),
                                target: target.join(if config.preserve_ms_arch_notation {
                                    arch.as_ms_str()
                                } else {
                                    arch.as_str()
                                }),
                                kind,
                                variant,
                            })
                        })
                        .collect()
                }
                PayloadKind::Ucrt => {
                    let mut mappings = vec![Ok(Mapping {
                        src: src.join("include/ucrt"),
                        target: sdk_root.join("include/ucrt"),
                        kind,
                        variant,
                    })];

                    src.push("lib/ucrt");
                    let target = sdk_root.join("lib/ucrt");
                    mappings.extend(Arch::iter(arches).map(|arch| {
                        Ok(Mapping {
                            src: src.join(arch.as_ms_str()),
                            target: target.join(if config.preserve_ms_arch_notation {
                                arch.as_ms_str()
                            } else {
                                arch.as_str()
                            }),
                            kind,
                            variant,
                        })
                    }));

                    mappings
                }
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .context("failed to map sources to targets")?;

    let include_debug_libs = config.include_debug_libs;
    let include_debug_symbols = config.include_debug_symbols;

    let sdk_files = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        u64,
        PathBuf,
    >::new()));

    #[inline]
    fn calc_lower_hash(path: &str) -> u64 {
        use std::hash::Hasher;
        let mut hasher = twox_hash::XxHash64::with_seed(0);

        for c in path.chars().map(|c| c.to_ascii_lowercase() as u8) {
            hasher.write_u8(c);
        }

        hasher.finish()
    }

    let handles: Vec<_> = mappings.into_iter().map(|mapping| {
        let sdk_files = sdk_files.clone();
        std::thread::spawn(move || -> Result<_, _> {
            let mut create_stack = vec![(mapping.src.clone(), mapping.target.clone())];

            let mut copied = 0;
            let mut total_symlinks= 0;
            let mut files = Vec::with_capacity(1024);

            while let Some((src, mut tar)) = create_stack.pop() {
                std::fs::create_dir_all(&tar).with_context(|| format!("unable to create {}", tar))?;

                for entry in std::fs::read_dir(&src).with_context(|| format!("unable to read {}", src))? {
                    let entry = entry.with_context(|| format!("unable to read entry from {}", src))?;

                    let src_path = PathBuf::from_path_buf(entry.path()).map_err(|pb| {
                        anyhow::anyhow!("src path {} is not a valid utf-8 path", pb.display())
                    })?;

                    // Entries are guaranteed to have a filename
                    let fname = src_path.file_name().unwrap();

                    let ft = entry.file_type().with_context(|| format!("unable to get file type for {}", entry.path().display()))?;

                    if ft.is_dir() {
                        // Due to some libs from the CRT Store libs variant being needed
                        // by the regular Desktop variant, if we are not actually
                        // targetting the Store we can avoid adding the additional
                        // uwp and store subdirectories
                        if mapping.kind == PayloadKind::CrtLibs && mapping.variant == Some(Variant::Store) && (variants & Variant::Store as u32) == 0 {
                            tracing::debug!("skipping CRT subdir {}", fname);
                            continue;
                        }

                        create_stack.push((src.join(fname), tar.join(fname)));
                    } else if ft.is_file() {
                        if mapping.kind == PayloadKind::CrtLibs || mapping.kind == PayloadKind::Ucrt {
                            if !include_debug_symbols && fname.ends_with(".pdb") {
                                tracing::debug!("skipping {}", fname);
                                continue;
                            }

                            if !include_debug_libs {
                                if let Some(stripped) = fname.strip_suffix(".lib") {
                                    if stripped.ends_with("d") ||
                                        stripped.ends_with("d_netcore") ||
                                        stripped.strip_suffix(|c: char| c.is_digit(10))
                                            .map_or(false, |fname| fname.ends_with("d")) {
                                            tracing::debug!("skipping {}", fname);
                                            continue;
                                        }
                                }
                            }
                        }

                        tar.push(fname);

                        // There is a massive amount of duplication between the 
                        // Desktop and Store headers
                        let write = if mapping.kind == PayloadKind::SdkHeaders {
                            let name_hash = calc_lower_hash(fname);

                            let mut lock = sdk_files.lock().unwrap();
                            if !lock.contains_key(&name_hash) {
                                lock.insert(name_hash, tar.clone());
                                true
                            } else {
                                false
                            }
                        } else {
                            true
                        };

                        if write {
                            copied += std::fs::copy(&src_path, &tar).with_context(|| format!("failed to copy {} to {}", src_path, tar))?;
                            files.push(tar.clone());
                        }

                        tar.pop();
                    } else if ft.is_symlink() {
                        anyhow::bail!("detected symlink {} in source directory which should be impossible", entry.path().display());
                    }
                }
            }

            files.sort();

            let total_files = files.len();

            let mut symlink = |original: &str, link: &camino::Utf8Path| -> Result<(), Error> {
                std::os::unix::fs::symlink(original, link).with_context(|| format!("unable to symlink from {} to {}", link, original))?;
                total_symlinks += 1;
                Ok(())
            };

            match mapping.kind {
                // These are all internally consistent and lowercased, so if
                // a library is including them with different casing that is
                // kind of on them
                PayloadKind::CrtHeaders | PayloadKind::Ucrt => {}
                PayloadKind::CrtLibs => {
                    // While _most_ of the libs *stares at Microsoft.VisualC.STLCLR.dll*,
                    // sometimes when they are specified as linker arguments libs
                    // will use SCREAMING_SNAKE_CASE as if they are angry at the
                    // linker this list is probably not completely, but that's
                    // what PRs are for

                    for path in files {
                        if let Some(fname) = path.file_name() {
                            let angry_lib = match fname.strip_suffix(".lib") {
                                Some("libcmt") => "LIBCMT.lib",
                                Some("msvcrt") => "MSVCRT.lib",
                                Some("oldnames") => "OLDNAMES.lib",
                                _ => continue,
                            };

                            let angry = {
                                let mut pb = path.clone();
                                pb.pop();
                                pb.push(angry_lib);
                                pb
                            };

                            symlink(fname, &angry)?;
                        }
                    }
                }
                PayloadKind::SdkHeaders => {
                    // The SDK headers are again all over the place with casing
                    // as well as being internally inconsistent, so we scan
                    // them all for includes and add those that are referenced
                    // incorrectly, but we wait until after all the of headers
                    // have been unpacked before fixing them
                }
                PayloadKind::SdkLibs | PayloadKind::SdkStoreLibs => {
                    // The SDK libraries are just completely inconsistent, but
                    // all usage I have ever seen just links them with lowercase
                    // names, so we just fix all of them to be lowercase.
                    // Note that we need to not only fix the name but also the
                    // extension, as for some inexplicable reason about half of
                    // them use an uppercase L for the extension. WTF. This also
                    // applies to the tlb files, so at least they are consistently
                    // inconsistent
                    for path in files {
                        if let Some(fname) = path.file_name() {
                            // Some libs are already correct
                            if fname.contains(|c: char| c.is_ascii_uppercase()) {
                                let correct = {
                                    let mut pb = path.clone();
                                    pb.pop();
                                    pb.push(fname.to_ascii_lowercase());
                                    pb
                                };

                                symlink(fname, &correct)?;
                            }
                        }
                    }
                }
            }

            Ok((copied, total_files, total_symlinks))
        })
    }).collect();

    let mut total_copied = 0;
    let mut total_files = 0;
    let mut total_symlinks = 0;

    for handle in handles {
        let (copied, files, symlinks) = handle
            .join()
            .map_err(|_e| anyhow::anyhow!("unable to spawn thread"))??;

        total_copied += copied;
        total_files += files;
        total_symlinks += symlinks;
    }

    {
        let mut symlink = |original: &str, link: &camino::Utf8Path| -> Result<(), Error> {
            std::os::unix::fs::symlink(original, link)
                .with_context(|| format!("unable to symlink from {} to {}", link, original))?;
            total_symlinks += 1;
            Ok(())
        };

        let files = std::sync::Arc::try_unwrap(sdk_files)
            .unwrap()
            .into_inner()
            .unwrap();
        let mut includes: std::collections::HashSet<
            _,
            std::hash::BuildHasherDefault<twox_hash::XxHash64>,
        > = Default::default();

        let regex = regex::bytes::Regex::new(r#"#include\s+(?:"|<)([^">]+)(?:"|>)?"#).unwrap();

        // Scan all of the files in the include directory for includes so that
        // we can add symlinks to at least make the SDK headers internally consistent
        for file in files.values() {
            // Of course, there are files with non-utf8 encoding :p
            let contents =
                std::fs::read(file).with_context(|| format!("unable to read {}", file))?;

            for caps in regex.captures_iter(&contents) {
                let name = std::str::from_utf8(&caps[1]).with_context(|| {
                    format!("{} contained an include with non-utf8 characters", file)
                })?;

                let name = match name.rfind('/') {
                    Some(i) => &name[i + 1..],
                    None => name,
                };

                if !includes.contains(name) {
                    includes.insert(name.to_owned());
                }
            }
        }

        // Many headers won't necessarily be referenced internally by an all
        // lower case filename, even when that is common from outside the sdk
        // for basically all files (eg windows.h, psapi.h etc)
        includes.extend(files.values().filter_map(|fpath| {
            fpath.file_name().and_then(|fname| {
                fname
                    .contains(|c: char| c.is_ascii_uppercase())
                    .then(|| fname.to_ascii_lowercase())
            })
        }));

        for include in includes {
            let lower_hash = calc_lower_hash(&include);

            match files.get(&lower_hash) {
                Some(disk_name) => {
                    if let Some(fname) = disk_name.file_name() {
                        if fname != include {
                            let mut link = disk_name.clone();
                            link.pop();
                            link.push(include);
                            symlink(fname, &link)?;
                        }
                    }
                }
                None => {
                    tracing::debug!(
                        "SDK include for '{}' was not found in the SDK headers",
                        include
                    );
                }
            }
        }

        // There is a um/gl directory, but of course there is an include for GL/
        // instead, so fix that as well :p
        symlink("gl", &sdk_root.join("include/um/GL"))?;
    }

    tracing::info!(
        copied = %indicatif::HumanBytes(total_copied),
        files = total_files,
        symlinks = total_symlinks,
        "packed files"
    );

    Ok(())
}
