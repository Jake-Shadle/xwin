use crate::{symlink, Arch, Ctx, Error, Path, PathBuf, PayloadKind, SectionKind, Variant};
use anyhow::Context as _;
use rayon::prelude::*;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct SplatConfig {
    pub include_debug_libs: bool,
    pub include_debug_symbols: bool,
    pub enable_symlinks: bool,
    pub preserve_ms_arch_notation: bool,
    pub use_winsysroot_style: bool,
    pub output: PathBuf,
    pub map: Option<PathBuf>,
    pub copy: bool,
    //pub isolated: bool,
}

/// There is a massive amount of duplication between SDK headers for the Desktop
/// and Store variants, so we keep track of them so we only splat one unique file
pub(crate) struct SdkHeaders {
    pub(crate) inner: BTreeMap<u64, PathBuf>,
    pub(crate) root: PathBuf,
}

impl SdkHeaders {
    fn new(root: PathBuf) -> Self {
        Self {
            inner: BTreeMap::new(),
            root,
        }
    }

    #[inline]
    fn get_relative_path<'path>(&self, path: &'path Path) -> anyhow::Result<&'path Path> {
        let mut rel = path.strip_prefix(&self.root)?;

        // Skip the first directory, which directly follows the "include", as it
        // is the one that includes are actually relative to
        if let Some(first) = rel.iter().next() {
            rel = rel.strip_prefix(first)?;
        }

        Ok(rel)
    }
}

pub(crate) struct SplatRoots {
    pub root: PathBuf,
    pub crt: PathBuf,
    pub sdk: PathBuf,
    src: PathBuf,
}

pub(crate) fn prep_splat(
    ctx: std::sync::Arc<Ctx>,
    root: &Path,
    winroot: Option<&str>,
) -> Result<SplatRoots, Error> {
    // Ensure we create the path first, you can't canonicalize a non-existant path
    if !root.exists() {
        std::fs::create_dir_all(root)
            .with_context(|| format!("unable to create splat directory {root}"))?;
    }

    let root = crate::util::canonicalize(root)?;

    let (crt_root, sdk_root) = if let Some(crt_version) = winroot {
        let mut crt = root.join("VC/Tools/MSVC");
        crt.push(crt_version);

        let mut sdk = root.join("Windows Kits");
        sdk.push("10");

        (crt, sdk)
    } else {
        (root.join("crt"), root.join("sdk"))
    };

    if crt_root.exists() {
        std::fs::remove_dir_all(&crt_root)
            .with_context(|| format!("unable to delete existing CRT directory {crt_root}"))?;
    }

    if sdk_root.exists() {
        std::fs::remove_dir_all(&sdk_root)
            .with_context(|| format!("unable to delete existing SDK directory {sdk_root}"))?;
    }

    std::fs::create_dir_all(&crt_root)
        .with_context(|| format!("unable to create CRT directory {crt_root}"))?;
    std::fs::create_dir_all(&sdk_root)
        .with_context(|| format!("unable to create SDK directory {sdk_root}"))?;

    let src_root = ctx.work_dir.join("unpack");

    Ok(SplatRoots {
        root,
        crt: crt_root,
        sdk: sdk_root,
        src: src_root,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn splat(
    config: &SplatConfig,
    roots: &SplatRoots,
    item: &crate::WorkItem,
    tree: &crate::unpack::FileTree,
    map: Option<&crate::Map>,
    sdk_version: &str,
    arches: u32,
    variants: u32,
) -> Result<Option<SdkHeaders>, Error> {
    struct Mapping<'ft> {
        src: PathBuf,
        target: PathBuf,
        tree: &'ft crate::unpack::FileTree,
        kind: PayloadKind,
        variant: Option<Variant>,
        section: SectionKind,
    }

    let mut src = roots.src.join(&item.payload.filename);

    // If we're moving files from the unpack directory, invalidate it immediately
    // so it is recreated in a future run if anything goes wrong
    if !config.copy {
        src.push(".unpack");
        if let Err(e) = std::fs::remove_file(&src) {
            tracing::warn!("Failed to remove {src}: {e}");
        }
        src.pop();
    }

    let get_tree = |src_path: &Path| -> Result<&crate::unpack::FileTree, Error> {
        let src_path = src_path
            .strip_prefix(&roots.src)
            .context("incorrect src root")?;
        let src_path = src_path
            .strip_prefix(&item.payload.filename)
            .context("incorrect src subdir")?;

        tree.subtree(src_path)
            .with_context(|| format!("missing expected subtree '{src_path}'"))
    };

    let push_arch = |src: &mut PathBuf, target: &mut PathBuf, arch: Arch| {
        src.push(arch.as_ms_str());
        target.push(if config.preserve_ms_arch_notation {
            arch.as_ms_str()
        } else {
            arch.as_str()
        });
    };

    let variant = item.payload.variant;
    let kind = item.payload.kind;

    let mappings = match kind {
        PayloadKind::CrtHeaders | PayloadKind::AtlHeaders => {
            src.push("include");
            let tree = get_tree(&src)?;

            vec![Mapping {
                src,
                target: roots.crt.join("include"),
                tree,
                kind,
                variant,
                section: SectionKind::CrtHeader,
            }]
        }
        PayloadKind::AtlLibs => {
            src.push("lib");
            let mut target = roots.crt.join("lib");

            let spectre = (variants & Variant::Spectre as u32) != 0;
            if spectre {
                src.push("spectre");
                target.push("spectre");
            }

            push_arch(
                &mut src,
                &mut target,
                item.payload
                    .target_arch
                    .context("ATL libs didn't specify an architecture")?,
            );

            let tree = get_tree(&src)?;

            vec![Mapping {
                src,
                target,
                tree,
                kind,
                variant,
                section: SectionKind::CrtLib,
            }]
        }

        PayloadKind::CrtLibs => {
            src.push("lib");
            let mut target = roots.crt.join("lib");

            let spectre = (variants & Variant::Spectre as u32) != 0;

            match item
                .payload
                .variant
                .context("CRT libs didn't specify a variant")?
            {
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

            push_arch(
                &mut src,
                &mut target,
                item.payload
                    .target_arch
                    .context("CRT libs didn't specify an architecture")?,
            );

            let tree = get_tree(&src)?;

            vec![Mapping {
                src,
                target,
                tree,
                kind,
                variant,
                section: SectionKind::CrtLib,
            }]
        }
        PayloadKind::SdkHeaders => {
            src.push("include");
            let tree = get_tree(&src)?;

            let target = if map.is_some() {
                let mut inc = roots.sdk.clone();
                inc.push("Include");
                inc.push(sdk_version);
                inc
            } else {
                let mut target = roots.sdk.join("include");

                if config.use_winsysroot_style {
                    target.push(sdk_version);
                }

                target
            };

            vec![Mapping {
                src,
                target,
                tree,
                kind,
                variant,
                section: SectionKind::SdkHeader,
            }]
        }
        PayloadKind::SdkLibs => {
            src.push("lib/um");

            let mut target = roots.sdk.join("lib");

            if config.use_winsysroot_style {
                target.push(sdk_version);
            }

            target.push("um");

            push_arch(
                &mut src,
                &mut target,
                item.payload
                    .target_arch
                    .context("SDK libs didn't specify an architecture")?,
            );

            let tree = get_tree(&src)?;

            vec![Mapping {
                src,
                target,
                tree,
                kind,
                variant,
                section: SectionKind::SdkLib,
            }]
        }
        PayloadKind::SdkStoreLibs => {
            src.push("lib/um");

            let mut target = roots.sdk.join("lib");

            if config.use_winsysroot_style {
                target.push(sdk_version);
            }

            target.push("um");

            Arch::iter(arches)
                .map(|arch| -> Result<Mapping<'_>, Error> {
                    let mut src = src.clone();
                    let mut target = target.clone();

                    push_arch(&mut src, &mut target, arch);

                    let tree = get_tree(&src)?;

                    Ok(Mapping {
                        src,
                        target,
                        tree,
                        kind,
                        variant,
                        section: SectionKind::SdkLib,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        PayloadKind::Ucrt => {
            let inc_src = src.join("include/ucrt");
            let tree = get_tree(&inc_src)?;

            let mut target = if map.is_some() {
                let mut inc = roots.sdk.join("Include");
                inc.push(sdk_version);
                inc
            } else {
                let mut target = roots.sdk.join("include");
                if config.use_winsysroot_style {
                    target.push(sdk_version);
                }
                target
            };

            target.push("ucrt");

            let mut mappings = vec![Mapping {
                src: inc_src,
                target,
                tree,
                kind,
                variant,
                section: SectionKind::SdkHeader,
            }];

            src.push("lib/ucrt");

            let mut target = roots.sdk.join("lib");

            if config.use_winsysroot_style {
                target.push(sdk_version);
            }

            target.push("ucrt");

            for arch in Arch::iter(arches) {
                let mut src = src.clone();
                let mut target = target.clone();

                push_arch(&mut src, &mut target, arch);

                let tree = get_tree(&src)?;

                mappings.push(Mapping {
                    src,
                    target,
                    tree,
                    kind,
                    variant,
                    section: SectionKind::SdkLib,
                });
            }

            mappings
        }
    };

    let mut results = Vec::new();

    item.progress.reset();
    item.progress
        .set_length(mappings.iter().map(|map| map.tree.stats().1).sum());
    item.progress.set_message("📦 splatting");

    struct Dir<'ft> {
        src: PathBuf,
        tar: PathBuf,
        tree: &'ft crate::unpack::FileTree,
    }

    if let Some(map) = map {
        mappings
            .into_par_iter()
            .map(|mapping| -> Result<Option<SdkHeaders>, Error> {
                let (prefix, section) = match mapping.section {
                    SectionKind::SdkHeader => {
                        // All ucrt headers are in the ucrt subdir, but we have a flat
                        // list in the mapping file, so we need to drop that from the prefix
                        // so they match like all the other paths

                        (
                            if matches!(mapping.kind, PayloadKind::Ucrt) {
                                mapping.target.parent().unwrap().to_owned()
                            } else {
                                mapping.target.clone()
                            },
                            &map.sdk.headers,
                        )
                    }
                    SectionKind::SdkLib => (roots.sdk.join("lib"), &map.sdk.libs),
                    SectionKind::CrtHeader => (mapping.target.clone(), &map.crt.headers),
                    SectionKind::CrtLib => {
                        (
                            // Pop the arch directory, it's part of the prefix in
                            // the filter
                            mapping.target.parent().unwrap().to_owned(),
                            &map.crt.libs,
                        )
                    }
                };

                let mut dir_stack = vec![Dir {
                    src: mapping.src,
                    tar: mapping.target,
                    tree: mapping.tree,
                }];

                while let Some(Dir { src, mut tar, tree }) = dir_stack.pop() {
                    let mut created_dir = false;

                    for (fname, size) in &tree.files {
                        // Even if we don't splat 100% of the source files, we still
                        // want to show that we processed them all
                        item.progress.inc(*size);

                        tar.push(fname);

                        let unprefixed = tar.strip_prefix(&prefix).with_context(|| {
                            format!("invalid path {tar}: doesn't begin with prefix {prefix}")
                        })?;

                        if !section.filter.contains(unprefixed.as_str()) {
                            tar.pop();
                            continue;
                        }

                        let src_path = src.join(fname);

                        if !created_dir {
                            std::fs::create_dir_all(tar.parent().unwrap())
                                .with_context(|| format!("unable to create {tar}"))?;
                            created_dir = true;
                        }

                        if config.copy {
                            std::fs::copy(&src_path, &tar)
                                .with_context(|| format!("failed to copy {src_path} to {tar}"))?;
                        } else {
                            std::fs::rename(&src_path, &tar)
                                .with_context(|| format!("failed to move {src_path} to {tar}"))?;
                        }

                        // Create any associated symlinks, these are always going to be symlinks
                        // in the same target directory
                        if let Some(symlinks) = section.symlinks.get(unprefixed.as_str()) {
                            for sl in symlinks {
                                tar.pop();
                                tar.push(sl);
                                symlink(fname.as_str(), &tar)?;
                            }
                        }

                        tar.pop();
                    }

                    for (dir, dtree) in &tree.dirs {
                        dir_stack.push(Dir {
                            src: src.join(dir),
                            tar: tar.join(dir),
                            tree: dtree,
                        });
                    }
                }

                // This is only if we are outputting symlinks, which we don't do when the user
                // has specified an exact mapping
                Ok(None)
            })
            .collect_into_vec(&mut results);
    } else {
        let include_debug_libs = config.include_debug_libs;
        let include_debug_symbols = config.include_debug_symbols;
        let filter_store = variants & Variant::Store as u32 == 0;

        mappings
            .into_par_iter()
            .map(|mapping| -> Result<Option<SdkHeaders>, Error> {
                let mut sdk_headers = (mapping.kind == PayloadKind::SdkHeaders)
                    .then(|| SdkHeaders::new(mapping.target.clone()));

                let mut dir_stack = vec![Dir {
                    src: mapping.src,
                    tar: mapping.target,
                    tree: mapping.tree,
                }];

                while let Some(Dir { src, mut tar, tree }) = dir_stack.pop() {
                    std::fs::create_dir_all(&tar)
                        .with_context(|| format!("unable to create {tar}"))?;

                    for (fname, size) in &tree.files {
                        // Even if we don't splat 100% of the source files, we still
                        // want to show that we processed them all
                        item.progress.inc(*size);

                        if !include_debug_symbols && fname.extension() == Some("pdb") {
                            tracing::debug!("skipping {fname}");
                            continue;
                        }

                        let fname_str = fname.as_str();
                        if !include_debug_libs
                            && (mapping.kind == PayloadKind::CrtLibs
                                || mapping.kind == PayloadKind::Ucrt)
                        {
                            if let Some(stripped) = fname_str.strip_suffix(".lib") {
                                if stripped.ends_with('d')
                                    || stripped.ends_with("d_netcore")
                                    || stripped
                                        .strip_suffix(|c: char| c.is_ascii_digit())
                                        .map_or(false, |fname| fname.ends_with('d'))
                                {
                                    tracing::debug!("skipping {fname}");
                                    continue;
                                }
                            }
                        }

                        tar.push(fname);

                        let src_path = src.join(fname);

                        if config.copy {
                            std::fs::copy(&src_path, &tar)
                                .with_context(|| format!("failed to copy {src_path} to {tar}"))?;
                        } else {
                            std::fs::rename(&src_path, &tar)
                                .with_context(|| format!("failed to move {src_path} to {tar}"))?;
                        }

                        let kind = mapping.kind;

                        let mut add_symlinks = || -> Result<(), Error> {
                            match kind {
                                // These are all internally consistent and lowercased, so if
                                // a library is including them with different casing that is
                                // kind of on them
                                //
                                // The SDK headers are also all over the place with casing
                                // as well as being internally inconsistent, so we scan
                                // them all for includes and add those that are referenced
                                // incorrectly, but we wait until after all the of headers
                                // have been unpacked before fixing them
                                PayloadKind::CrtHeaders
                                | PayloadKind::AtlHeaders
                                | PayloadKind::Ucrt
                                | PayloadKind::AtlLibs => {}

                                PayloadKind::SdkHeaders => {
                                    if let Some(sdk_headers) = &mut sdk_headers {
                                        let rel_target_path =
                                            sdk_headers.get_relative_path(&tar)?;

                                        let rel_hash = calc_lower_hash(rel_target_path.as_str());

                                        if sdk_headers.inner.insert(rel_hash, tar.clone()).is_some()
                                        {
                                            anyhow::bail!(
                                                "found duplicate relative path when hashed"
                                            );
                                        }

                                        if let Some(additional_name) = match fname_str {
                                            // https://github.com/zeromq/libzmq/blob/3070a4b2461ec64129062907d915ed665d2ac126/src/precompiled.hpp#L73
                                            "mstcpip.h" => Some("Mstcpip.h"),
                                            // https://github.com/ponylang/ponyc/blob/8d41d6650b48b9733cd675df199588e6fccc6346/src/common/platform.h#L191
                                            "basetsd.h" => Some("BaseTsd.h"),
                                            _ => None,
                                        } {
                                            tar.pop();
                                            tar.push(additional_name);

                                            symlink(fname_str, &tar)?;
                                        }
                                    }
                                }
                                PayloadKind::CrtLibs => {
                                    // While _most_ of the libs *stares at Microsoft.VisualC.STLCLR.dll* are lower case,
                                    // sometimes when they are specified as linker arguments, crates will link with
                                    // SCREAMING as if they are angry at the linker, so fix this in the few "common" cases.
                                    // This list is probably not complete, but that's what PRs are for
                                    if let Some(angry_lib) = match fname_str.strip_suffix(".lib") {
                                        Some("libcmt") => Some("LIBCMT.lib"),
                                        Some("msvcrt") => Some("MSVCRT.lib"),
                                        Some("oldnames") => Some("OLDNAMES.lib"),
                                        _ => None,
                                    } {
                                        tar.pop();
                                        tar.push(angry_lib);

                                        symlink(fname_str, &tar)?;
                                    }
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
                                    if fname_str.contains(|c: char| c.is_ascii_uppercase()) {
                                        tar.pop();
                                        tar.push(fname_str.to_ascii_lowercase());

                                        symlink(fname_str, &tar)?;
                                    }

                                    // There is also this: https://github.com/time-rs/time/blob/v0.3.2/src/utc_offset.rs#L454
                                    // And this: https://github.com/webrtc-rs/util/blob/main/src/ifaces/ffi/windows/mod.rs#L33
                                    if let Some(additional_name) = match fname_str {
                                        "kernel32.Lib" => Some("Kernel32.lib"),
                                        "iphlpapi.lib" => Some("Iphlpapi.lib"),
                                        _ => None,
                                    } {
                                        tar.pop();
                                        tar.push(additional_name);

                                        symlink(fname_str, &tar)?;
                                    }

                                    // We also need to support SCREAMING case for the library names
                                    // due to...reasons https://github.com/microsoft/windows-rs/blob/a27a74784ccf304ab362bf2416f5f44e98e5eecd/src/bindings.rs#L3772
                                    if tar.extension() == Some("lib") {
                                        tar.pop();
                                        tar.push(fname_str.to_ascii_uppercase());
                                        tar.set_extension("lib");

                                        symlink(fname_str, &tar)?;
                                    }
                                }
                            }

                            Ok(())
                        };

                        if config.enable_symlinks {
                            add_symlinks()?;
                        }

                        tar.pop();
                    }

                    // Due to some libs from the CRT Store libs variant being needed
                    // by the regular Desktop variant, if we are not actually
                    // targeting the Store we can avoid adding the additional
                    // uwp and store subdirectories
                    if mapping.variant == Some(Variant::Store)
                        && filter_store
                        && mapping.kind == PayloadKind::CrtLibs
                    {
                        tracing::debug!("skipping CRT subdirs");

                        item.progress
                            .inc(tree.dirs.iter().map(|(_, ft)| ft.stats().1).sum());
                        continue;
                    }

                    for (dir, dtree) in &tree.dirs {
                        dir_stack.push(Dir {
                            src: src.join(dir),
                            tar: tar.join(dir),
                            tree: dtree,
                        });
                    }
                }

                Ok(sdk_headers)
            })
            .collect_into_vec(&mut results);

        if !config.use_winsysroot_style {
            match kind {
                PayloadKind::SdkLibs => {
                    // Symlink sdk/lib/{sdkversion} -> sdk/lib, regardless of filesystem case sensitivity.
                    let mut versioned_linkname = roots.sdk.clone();
                    versioned_linkname.push("lib");
                    versioned_linkname.push(sdk_version);

                    // Multiple architectures both have a lib dir,
                    // but we only need to create this symlink once.
                    if !versioned_linkname.exists() {
                        crate::symlink_on_windows_too(".", &versioned_linkname)?;
                    }

                    // https://github.com/llvm/llvm-project/blob/release/14.x/clang/lib/Driver/ToolChains/MSVC.cpp#L1102
                    if config.enable_symlinks {
                        let mut title_case = roots.sdk.clone();
                        title_case.push("Lib");
                        if !title_case.exists() {
                            symlink("lib", &title_case)?;
                        }
                    }
                }
                PayloadKind::SdkHeaders => {
                    // Symlink sdk/include/{sdkversion} -> sdk/include, regardless of filesystem case sensitivity.
                    let mut versioned_linkname = roots.sdk.clone();
                    versioned_linkname.push("include");
                    versioned_linkname.push(sdk_version);

                    // Desktop and Store variants both have an include dir,
                    // but we only need to create this symlink once.
                    if !versioned_linkname.exists() {
                        crate::symlink_on_windows_too(".", &versioned_linkname)?;
                    }

                    // https://github.com/llvm/llvm-project/blob/release/14.x/clang/lib/Driver/ToolChains/MSVC.cpp#L1340-L1346
                    if config.enable_symlinks {
                        let mut title_case = roots.sdk.clone();
                        title_case.push("Include");
                        if !title_case.exists() {
                            symlink("include", &title_case)?;
                        }
                    }
                }
                _ => (),
            };
        }
    }

    item.progress.finish_with_message("📦 splatted");

    let headers = results.into_iter().collect::<Result<Vec<_>, _>>()?;

    Ok(headers.into_iter().find_map(|headers| headers))
}

pub(crate) fn finalize_splat(
    ctx: &Ctx,
    sdk_version: Option<&str>,
    roots: &SplatRoots,
    sdk_headers: Vec<SdkHeaders>,
    crt_headers: Option<crate::unpack::FileTree>,
    atl_headers: Option<crate::unpack::FileTree>,
) -> Result<(), Error> {
    let mut files: std::collections::HashMap<
        _,
        Header<'_>,
        std::hash::BuildHasherDefault<twox_hash::XxHash64>,
    > = Default::default();

    struct Header<'root> {
        root: &'root SdkHeaders,
        path: PathBuf,
    }

    fn compare_hashes(existing: &Path, new: &PathBuf) -> anyhow::Result<()> {
        use crate::util::Sha256;

        let existing_hash = Sha256::digest(&std::fs::read(existing)?);
        let new_hash = Sha256::digest(&std::fs::read(new)?);

        anyhow::ensure!(
            existing_hash == new_hash,
            "2 files with same relative path were not equal: '{existing}' != '{new}'"
        );

        Ok(())
    }

    for hdrs in &sdk_headers {
        for (k, v) in &hdrs.inner {
            if let Some(existing) = files.get(k) {
                // We already have a file with the same path, if they're the same
                // as each other it's fine, but if they differ we have an issue
                compare_hashes(&existing.path, v)?;
                tracing::debug!("skipped {v}, a matching path already exists");
            } else {
                files.insert(
                    k,
                    Header {
                        root: hdrs,
                        path: v.clone(),
                    },
                );
            }
        }
    }

    let mut includes: std::collections::HashMap<
        _,
        _,
        std::hash::BuildHasherDefault<twox_hash::XxHash64>,
    > = Default::default();

    // Many headers won't necessarily be referenced internally by an all
    // lower case filename, even when that is common from outside the sdk
    // for basically all files (eg windows.h, psapi.h etc)
    includes.extend(files.values().filter_map(|fpath| {
        fpath
            .root
            .get_relative_path(&fpath.path)
            .ok()
            .and_then(|rel_path| {
                let rp = rel_path.as_str();

                // Ignore the 2 opengl includes, since they are the one exception
                // that all subdirectories are lowercased
                if rel_path.starts_with("gl/") {
                    return None;
                }

                rp.contains(|c: char| c.is_ascii_uppercase())
                    .then(|| (PathBuf::from(rp.to_ascii_lowercase()), true))
            })
    }));

    let regex = regex::bytes::Regex::new(r#"#include\s+(?:"|<)([^">]+)(?:"|>)?"#).unwrap();

    let pb =
        indicatif::ProgressBar::with_draw_target(Some(files.len() as u64), ctx.draw_target.into())
            .with_style(
                indicatif::ProgressStyle::default_bar()
                    .template(
                        "{spinner:.green} {prefix:.bold} [{elapsed}] {wide_bar:.green} {pos}/{len}",
                    )?
                    .progress_chars("█▇▆▅▄▃▂▁  "),
            );

    pb.set_prefix("symlinks");
    pb.set_message("🔍 SDK includes");

    // Scan all of the files in the include directory for includes so that
    // we can add symlinks to at least make the SDK headers internally consistent
    for file in files.values() {
        // Of course, there are files with non-utf8 encoding :p
        let contents =
            std::fs::read(&file.path).with_context(|| format!("unable to read {}", file.path))?;

        for caps in regex.captures_iter(&contents) {
            let rel_path = std::str::from_utf8(&caps[1]).with_context(|| {
                format!(
                    "{} contained an include with non-utf8 characters",
                    file.path
                )
            })?;

            // TODO: Some includes, particularly in [wrl](https://docs.microsoft.com/en-us/cpp/cppcx/wrl/windows-runtime-cpp-template-library-wrl?view=msvc-170)
            // use incorrect `\` path separators, this is hopefully not an issue
            // since no one cares about that target? But if it is a problem
            // we'll need to actually modify the include to fix the path. :-/
            if !includes.contains_key(Path::new(rel_path)) {
                includes.insert(PathBuf::from(rel_path), true);
            }
        }

        pb.inc(1);
    }

    if let Some(crt) = crt_headers
        .as_ref()
        .and_then(|crt| crt.subtree(Path::new("include")))
    {
        pb.set_message("🔍 CRT includes");
        let cr = roots.crt.join("include");

        for (path, _) in &crt.files {
            // Of course, there are files with non-utf8 encoding :p
            let path = cr.join(path);
            let contents =
                std::fs::read(&path).with_context(|| format!("unable to read CRT {path}"))?;

            for caps in regex.captures_iter(&contents) {
                let rel_path = std::str::from_utf8(&caps[1]).with_context(|| {
                    format!("{path} contained an include with non-utf8 characters")
                })?;

                if !includes.contains_key(Path::new(rel_path)) {
                    includes.insert(PathBuf::from(rel_path), false);
                }
            }

            pb.inc(1);
        }
    }

    if let Some(atl) = atl_headers
        .as_ref()
        .and_then(|atl| atl.subtree(Path::new("include")))
    {
        pb.set_message("🔍 ATL includes");
        let cr = roots.crt.join("include");

        for (path, _) in &atl.files {
            // Of course, there are files with non-utf8 encoding :p
            let path = cr.join(path);
            let contents =
                std::fs::read(&path).with_context(|| format!("unable to read ATL {path}"))?;

            for caps in regex.captures_iter(&contents) {
                let rel_path = std::str::from_utf8(&caps[1]).with_context(|| {
                    format!("{path} contained an include with non-utf8 characters")
                })?;

                if !includes.contains_key(Path::new(rel_path)) {
                    includes.insert(PathBuf::from(rel_path), false);
                }
            }

            pb.inc(1);
        }
    }

    pb.finish();

    for (include, is_sdk) in includes {
        let lower_hash = calc_lower_hash(include.as_str());

        match files.get(&lower_hash) {
            Some(disk_file) => match (disk_file.path.file_name(), include.file_name()) {
                (Some(disk_name), Some(include_name)) if disk_name != include_name => {
                    let mut link = disk_file.path.clone();
                    link.pop();
                    link.push(include_name);
                    symlink(disk_name, &link)?;
                }
                _ => {}
            },
            None => {
                if is_sdk {
                    tracing::debug!("SDK include for '{include}' was not found in the SDK headers");
                }
            }
        }
    }

    // There is a um/gl directory, but of course there is an include for GL/
    // instead, so fix that as well :p
    if let Some(_sdk_version) = sdk_version {
        // let mut target = roots.sdk.join("Include");
        // target.push(sdk_version);
        // target.push("um/GL");
        // symlink("gl", &target)?;
    } else {
        symlink("gl", &roots.sdk.join("include/um/GL"))?;
    }

    Ok(())
}

use std::hash::Hasher;

#[inline]
fn calc_lower_hash(path: &str) -> u64 {
    let mut hasher = twox_hash::XxHash64::with_seed(0);

    for c in path.chars().map(|c| c.to_ascii_lowercase() as u8) {
        hasher.write_u8(c);
    }

    hasher.finish()
}
