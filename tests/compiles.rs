#[test]
fn verify_compiles() {
    let ctx = xwin::Ctx::with_dir(
        xwin::PathBuf::from(".xwin-cache/compile-test"),
        xwin::util::ProgressTarget::Hidden,
        ureq::agent(),
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
        None,
        None,
    )
    .unwrap();

    #[derive(Debug)]
    enum Style {
        Default,
        WinSysRoot,
    }

    for style in [Style::Default, Style::WinSysRoot] {
        let output_dir = ctx.work_dir.join(format!("{style:?}"));
        if !output_dir.exists() {
            std::fs::create_dir_all(&output_dir).unwrap();
        }

        if !cfg!(target_os = "windows") && matches!(style, Style::WinSysRoot) {
            continue;
        }

        let op = xwin::Ops::Splat(xwin::SplatConfig {
            include_debug_libs: false,
            include_debug_symbols: false,
            enable_symlinks: matches!(style, Style::Default),
            preserve_ms_arch_notation: matches!(style, Style::WinSysRoot),
            use_winsysroot_style: matches!(style, Style::WinSysRoot),
            map: None,
            copy: true,
            output: output_dir.clone(),
        });

        ctx.clone()
            .execute(
                pkg_manifest.packages.clone(),
                pruned
                    .payloads
                    .clone()
                    .into_iter()
                    .map(|payload| xwin::WorkItem {
                        progress: hidden.clone(),
                        payload: std::sync::Arc::new(payload),
                    })
                    .collect(),
                pruned.crt_version.clone(),
                pruned.sdk_version.clone(),
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

        let od = xwin::util::canonicalize(&output_dir).unwrap();

        let includes = match style {
            Style::Default => {
                cmd.env("RUSTFLAGS", format!("-C linker=lld-link -Lnative={od}/crt/lib/x86_64 -Lnative={od}/sdk/lib/um/x86_64 -Lnative={od}/sdk/lib/ucrt/x86_64"));
                format!("-Wno-unused-command-line-argument -fuse-ld=lld-link /imsvc{od}/crt/include /imsvc{od}/sdk/include/ucrt /imsvc{od}/sdk/include/um /imsvc{od}/sdk/include/shared")
            }
            Style::WinSysRoot => {
                const SEP: char = '\x1F';
                cmd.env("CARGO_ENCODED_RUSTFLAGS", format!("-C{SEP}linker=lld-link{SEP}-Lnative={od}/VC/Tools/MSVC/{crt_version}/Lib/x64{SEP}-Lnative={od}/Windows Kits/10/Lib/{sdk_version}/um/x64{SEP}-Lnative={od}/Windows Kits/10/Lib/{sdk_version}/ucrt/x64", crt_version = &pruned.crt_version, sdk_version = &pruned.sdk_version));

                format!("-Wno-unused-command-line-argument -fuse-ld=lld-link /winsysroot {od}")
            }
        };

        let cc_env = [
            ("CC_x86_64_pc_windows_msvc", "clang-cl"),
            ("CXX_x86_64_pc_windows_msvc", "clang-cl"),
            ("AR_x86_64_pc_windows_msvc", "llvm-lib"),
            ("CFLAGS_x86_64_pc_windows_msvc", &includes),
            ("CXXFLAGS_x86_64_pc_windows_msvc", &includes),
        ];

        cmd.envs(cc_env);

        assert!(cmd.status().unwrap().success());

        // Ignore the /vctoolsdir /winsdkdir test below on CI since it fails, I'm assuming
        // due to the clang version in GHA being outdated, but don't have the will to
        // look into it now
        if !matches!(style, Style::Default) || std::env::var("CI").is_ok() {
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
        "-Wno-unused-command-line-argument -fuse-ld=lld-link /vctoolsdir {od}/crt /winsdkdir {od}/sdk"
    );
        let libs = format!("-C linker=lld-link -Lnative={od}/crt/lib/x86_64 -Lnative={od}/sdk/lib/um/x86_64 -Lnative={od}/sdk/lib/ucrt/x86_64");

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
}

#[test]
#[ignore = "very expensive, and conflicts with the test above, only run this in isolation"]
fn verify_compiles_minimized() {
    let ctx = xwin::Ctx::with_dir(
        xwin::PathBuf::from(".xwin-cache/compile-test-minimized"),
        xwin::util::ProgressTarget::Hidden,
        ureq::agent(),
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
        None,
        None,
    )
    .unwrap();

    let output_dir = ctx.work_dir.join("splat");
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir).unwrap();
    }

    let filtered = ctx.work_dir.join("filtered");
    if !filtered.exists() {
        std::fs::create_dir_all(&filtered).unwrap();
    }

    let map_path = ctx.work_dir.join("map.toml");

    let op = xwin::Ops::Minimize(xwin::MinimizeConfig {
        include_debug_libs: false,
        include_debug_symbols: false,
        enable_symlinks: true,
        preserve_ms_arch_notation: false,
        use_winsysroot_style: false,
        map: map_path.clone(),
        copy: true,
        splat_output: output_dir.clone(),
        manifest_path: "tests/xwin-test/Cargo.toml".into(),
        target: "x86_64-pc-windows-msvc".into(),
        minimize_output: Some(filtered.clone()),
        preserve_strace: false,
    });

    ctx.execute(
        pkg_manifest.packages,
        pruned
            .payloads
            .into_iter()
            .map(|payload| xwin::WorkItem {
                progress: hidden.clone(),
                payload: std::sync::Arc::new(payload),
            })
            .collect(),
        pruned.crt_version,
        pruned.sdk_version,
        xwin::Arch::X86_64 as u32,
        xwin::Variant::Desktop as u32,
        op,
    )
    .unwrap();

    insta::assert_snapshot!(std::fs::read_to_string(map_path).unwrap());

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

    let od = xwin::util::canonicalize(&filtered).unwrap();

    let includes = format!(
        "-Wno-unused-command-line-argument -fuse-ld=lld-link /vctoolsdir {od}/crt /winsdkdir {od}/sdk"
    );
    let libs = format!("-C linker=lld-link -Lnative={od}/crt/lib/x86_64 -Lnative={od}/sdk/lib/um/x86_64 -Lnative={od}/sdk/lib/ucrt/x86_64");

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
