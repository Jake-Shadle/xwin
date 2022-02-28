#[test]
fn verify_compiles() {
    let ctx = xwin::Ctx::with_dir(
        xwin::PathBuf::from(".xwin-cache/compile-test"),
        xwin::util::ProgressTarget::Hidden,
    )
    .unwrap();

    let ctx = std::sync::Arc::new(ctx);

    let hidden = indicatif::ProgressBar::hidden();

    let manifest = xwin::manifest::get_manifest(&ctx, "16", "release", hidden.clone()).unwrap();
    let pkg_manifest =
        xwin::manifest::get_package_manifest(&ctx, &manifest, hidden.clone()).unwrap();

    let pruned = xwin::prune_pkg_list(
        &pkg_manifest,
        xwin::Arch::X86_64 as u32,
        xwin::Variant::Desktop as u32,
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

    let mut cmd = std::process::Command::new("cargo");
    cmd.args(&[
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
}
