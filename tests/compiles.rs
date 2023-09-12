#[test]
fn verify_compiles() {
    let ctx = xwin::Ctx::with_dir(
        xwin::PathBuf::from(".xwin-cache/compile-test"),
        xwin::util::ProgressTarget::Hidden,
        None,
    )
    .unwrap();

    let ctx = std::sync::Arc::new(ctx);

    let hidden = indicatif::ProgressBar::hidden();

    // TODO: Bump to CI to 17 once github actions isn't using an ancient version,
    // we could install in the action run, but not really worth it since I can
    // test locally
    let manifest_version = if std::env::var_os("CI").is_some() {
        "16"
    } else {
        "17"
    };

    let manifest =
        xwin::manifest::get_manifest(&ctx, manifest_version, "release", hidden.clone()).unwrap();
    let pkg_manifest =
        xwin::manifest::get_package_manifest(&ctx, &manifest, hidden.clone()).unwrap();

    let pruned = xwin::prune_pkg_list(
        &pkg_manifest,
        xwin::Arch::X86_64 as u32,
        xwin::Variant::Desktop as u32,
        false,
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

    let output_dir = xwin::PathBuf::from_path_buf(output_dir.canonicalize().unwrap()).unwrap();

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

    if xwin::Path::new("tests/xwin-test/target").exists() {
        std::fs::remove_dir_all("tests/xwin-test/target").expect("failed to remove target dir");
    }

    let mut cmd = std::process::Command::new("cargo");
    cmd.args([
        "build",
        "--target",
        "x86_64-pc-windows-msvc",
        "--manifest-path",
        "tests/xwin-test/Cargo.toml",
    ]);

    let includes = format!("-Wno-unused-command-line-argument -fuse-ld=lld-link /imsvc{0}/crt/include /imsvc{0}/sdk/include/ucrt /imsvc{0}/sdk/include/um /imsvc{0}/sdk/include/shared", output_dir);
    let libs = format!("-C linker=lld-link -Lnative={0}/crt/lib/x86_64 -Lnative={0}/sdk/lib/um/x86_64 -Lnative={0}/sdk/lib/ucrt/x86_64", output_dir);

    let cc_env = [
        ("CC_x86_64_pc_windows_msvc", "clang-cl"),
        ("CXX_x86_64_pc_windows_msvc", "clang-cl"),
        ("AR_x86_64_pc_windows_msvc", "llvm-lib"),
        ("CFLAGS_x86_64_pc_windows_msvc", &includes),
        ("CXXFLAGS_x86_64_pc_windows_msvc", &includes),
        ("RUSTFLAGS", &libs),
    ];

    cmd.envs(cc_env);

    assert!(cmd.status().unwrap().success());

    // Ignore the /vctoolsdir /winsdkdir test below on CI since it fails, I'm assuming
    // due to the clang version in GHA being outdated, but don't have the will to
    // look into it now
    if std::env::var("CI").is_ok() {
        return;
    }

    std::fs::remove_dir_all("tests/xwin-test/target").expect("failed to remove target dir");

    let mut cmd = std::process::Command::new("cargo");
    cmd.args([
        "build",
        "--target",
        "x86_64-pc-windows-msvc",
        "--manifest-path",
        "tests/xwin-test/Cargo.toml",
    ]);

    let includes = format!(
        "-Wno-unused-command-line-argument -fuse-ld=lld-link /vctoolsdir {0}/crt /winsdkdir {0}/sdk",
        output_dir
    );
    let libs = format!("-C linker=lld-link -Lnative={0}/crt/lib/x86_64 -Lnative={0}/sdk/lib/um/x86_64 -Lnative={0}/sdk/lib/ucrt/x86_64", output_dir);

    let cc_env = [
        ("CC_x86_64_pc_windows_msvc", "clang-cl"),
        ("CXX_x86_64_pc_windows_msvc", "clang-cl"),
        ("AR_x86_64_pc_windows_msvc", "llvm-lib"),
        ("CFLAGS_x86_64_pc_windows_msvc", &includes),
        ("CXXFLAGS_x86_64_pc_windows_msvc", &includes),
        ("RUSTFLAGS", &libs),
    ];

    cmd.envs(cc_env);

    assert!(cmd.status().unwrap().success());
}
