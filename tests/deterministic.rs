use anyhow::Context as _;
use xwin::PathBuf;

#[test]
fn verify_deterministic() {
    let ctx = xwin::Ctx::with_dir(
        PathBuf::from(".xwin-cache/deterministic"),
        xwin::util::ProgressTarget::Hidden,
        None,
    )
    .unwrap();

    let ctx = std::sync::Arc::new(ctx);

    let hidden = indicatif::ProgressBar::hidden();

    let manifest_contents = std::fs::read_to_string("tests/deterministic_manifest.json").unwrap();
    let manifest: xwin::manifest::Manifest = serde_json::from_str(&manifest_contents).unwrap();
    let pkg_manifest =
        xwin::manifest::get_package_manifest(&ctx, &manifest, hidden.clone()).unwrap();

    let pruned = xwin::prune_pkg_list(
        &pkg_manifest,
        xwin::Arch::X86_64 as u32,
        xwin::Variant::Desktop as u32,
        true,
    )
    .unwrap();

    let output_dir = ctx.work_dir.join("splat");
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir).unwrap();
    }

    let op = xwin::Ops::Splat(xwin::SplatConfig {
        include_debug_libs: false,
        include_debug_symbols: false,
        enable_symlinks: true,
        preserve_ms_arch_notation: false,
        copy: true,
        output: output_dir.clone(),
    });

    let output_dir = PathBuf::from_path_buf(output_dir.canonicalize().unwrap()).unwrap();

    ctx.execute(
        pkg_manifest.packages,
        pruned
            .into_iter()
            .map(|payload| xwin::WorkItem {
                progress: hidden.clone(),
                payload: std::sync::Arc::new(payload),
            })
            .collect(),
        xwin::Arch::X86_64 as u32,
        xwin::Variant::Desktop as u32,
        op,
    )
    .unwrap();

    #[inline]
    fn calc_hash(file: &std::fs::File) -> Result<u64, std::io::Error> {
        use std::{hash::Hasher, io::BufRead};

        let mut hasher = twox_hash::XxHash64::with_seed(0);

        let mut reader = std::io::BufReader::new(file);
        loop {
            let len = {
                let buf = reader.fill_buf()?;
                if buf.is_empty() {
                    break;
                }

                hasher.write(buf);
                buf.len()
            };
            reader.consume(len);
        }

        Ok(hasher.finish())
    }

    struct Splatted {
        path: PathBuf,
        link: Option<PathBuf>,
        hash: u64,
    }

    let mut files: Vec<_> = walkdir::WalkDir::new(&output_dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.unwrap();

            if entry.file_type().is_dir() {
                return None;
            }

            let path = PathBuf::from_path_buf(entry.path().to_owned()).unwrap();

            let link = if entry.path_is_symlink() {
                Some(PathBuf::from_path_buf(std::fs::read_link(&path).unwrap()).unwrap())
            } else {
                None
            };

            Some(Splatted {
                path,
                link,
                hash: 0,
            })
        })
        .collect();

    use rayon::prelude::*;
    files.par_iter_mut().for_each(|splat| {
        if splat.link.is_none() {
            let file = std::fs::File::open(&splat.path)
                .with_context(|| format!("failed to open {}", splat.path))
                .unwrap();
            splat.hash = calc_hash(&file)
                .with_context(|| format!("failed to read {}", splat.path))
                .unwrap();
        }
    });

    let mut actual = String::with_capacity(4 * 1024);

    use std::fmt::Write;
    for file in files {
        actual
            .write_str(file.path.strip_prefix(&output_dir).unwrap().as_str())
            .unwrap();

        match file.link {
            Some(link) => {
                actual.write_str(" => ").unwrap();
                actual.write_str(link.as_str()).unwrap();
            }
            None => write!(&mut actual, " @ {:x}", file.hash).unwrap(),
        }

        actual.push('\n');
    }

    insta::assert_snapshot!(actual);
}
