//! Verifies that the WDK acquired from nuget can actually be used to compile and
//! link a kernel mode driver.
//!
//! Note this is deliberately not part of `compiles.rs`, as it needs its own splat
//! output and downloads an additional ~100MiB.

use std::process::Command;

const KMDF_VERSION: &str = "1.35";

/// The full splat for the host-ish target, plus everything needed to splat the
/// WDK again for another architecture without redoing the CRT and SDK
struct Kit {
    ctx: std::sync::Arc<xwin::Ctx>,
    packages: std::collections::BTreeMap<String, xwin::manifest::ManifestItem>,
    crt_version: String,
    sdk_version: String,
    /// The `x86_64` splat, which is also where the (architecture independent) CRT
    /// and SDK headers any driver build needs come from
    root: xwin::PathBuf,
}

fn wdf_versions() -> xwin::WdfVersions {
    xwin::WdfVersions {
        kmdf: Some(KMDF_VERSION.to_owned()),
        umdf: None,
    }
}

fn splat_config(output: xwin::PathBuf) -> xwin::Ops {
    xwin::Ops::Splat(xwin::SplatConfig {
        include_debug_libs: false,
        include_debug_symbols: false,
        enable_symlinks: true,
        preserve_ms_arch_notation: false,
        use_winsysroot_style: false,
        map: None,
        copy: true,
        output,
    })
}

fn work_items(payloads: Vec<xwin::Payload>) -> Vec<xwin::WorkItem> {
    payloads
        .into_iter()
        .map(|payload| xwin::WorkItem {
            progress: indicatif::ProgressBar::hidden(),
            payload: std::sync::Arc::new(payload),
        })
        .collect()
}

fn splat_root() -> Kit {
    let ctx = xwin::Ctx::with_dir(
        xwin::PathBuf::from(".xwin-cache/driver-test"),
        xwin::util::ProgressTarget::Hidden,
        ureq::agent(),
        0,
    )
    .unwrap();

    let ctx = std::sync::Arc::new(ctx);
    let hidden = indicatif::ProgressBar::hidden();

    // The WDK is only published for recent kits, so unlike the other tests we
    // always use a manifest new enough to have a matching SDK
    let manifest = xwin::manifest::get_manifest(&ctx, 17, "release", hidden.clone()).unwrap();
    let pkg_manifest =
        xwin::manifest::get_package_manifest(&ctx, &manifest, hidden.clone()).unwrap();

    let mut pruned = xwin::prune_pkg_list(
        &pkg_manifest,
        xwin::Arch::X86_64 as u32,
        xwin::Variant::Desktop as u32,
        false,
        false,
        None,
        None,
    )
    .unwrap();

    let wdk =
        xwin::nuget::get_wdk(&ctx, xwin::Arch::X86_64 as u32, None, &pruned.sdk_version).unwrap();

    pruned.payloads.extend(wdk.payloads);

    let output_dir = ctx.work_dir.join("splat");

    let crt_version = pruned.crt_version.clone();
    let sdk_version = pruned.sdk_version.clone();

    ctx.clone()
        .execute(
            pkg_manifest.packages.clone(),
            work_items(pruned.payloads),
            pruned.crt_version,
            pruned.sdk_version,
            pruned.vcr_version,
            wdf_versions(),
            xwin::Arch::X86_64 as u32,
            xwin::Variant::Desktop as u32,
            splat_config(output_dir.clone()),
        )
        .unwrap();

    Kit {
        packages: pkg_manifest.packages,
        crt_version,
        sdk_version,
        root: xwin::util::canonicalize(&output_dir).unwrap(),
        ctx,
    }
}

/// Splats just the WDK for `arch`, reusing the CRT and SDK already acquired
impl Kit {
    fn splat_wdk(&self, arch: xwin::Arch, name: &str) -> xwin::PathBuf {
        let wdk = xwin::nuget::get_wdk(&self.ctx, arch as u32, None, &self.sdk_version).unwrap();

        let output_dir = self.ctx.work_dir.join(name);

        self.ctx
            .clone()
            .execute(
                self.packages.clone(),
                work_items(wdk.payloads),
                self.crt_version.clone(),
                self.sdk_version.clone(),
                None,
                wdf_versions(),
                arch as u32,
                xwin::Variant::Desktop as u32,
                splat_config(output_dir.clone()),
            )
            .unwrap();

        xwin::util::canonicalize(&output_dir).unwrap()
    }
}

/// Asserts the WDK tree at `wdk_root` is complete for `arch`, then compiles and
/// links a KMDF driver against it, with the CRT and SDK headers coming from
/// `base` (they are architecture independent, and a driver links neither)
fn build_driver(base: &xwin::Path, wdk_root: &xwin::Path, arch: xwin::Arch, defines: &[&str]) {
    let a = arch.as_str();

    // Every one of these must exist for a driver build to work at all
    for expected in [
        "wdk/include/km/wdm.h".to_owned(),
        "wdk/include/km/ntddk.h".to_owned(),
        "wdk/include/shared".to_owned(),
        "wdk/include/um".to_owned(),
        format!("wdk/include/wdf/kmdf/{KMDF_VERSION}/wdf.h"),
        format!("wdk/lib/km/{a}/ntoskrnl.lib"),
        format!("wdk/lib/km/{a}/hal.lib"),
        format!("wdk/lib/wdf/kmdf/{a}/{KMDF_VERSION}/wdfldr.lib"),
    ] {
        assert!(
            wdk_root.join(&expected).exists(),
            "{expected} was not splatted for {arch}"
        );
    }

    // The WDK's msbuild props never link these by the casing they have on disk,
    // so if these are missing the casing fixups have regressed
    for canonical in [
        format!("wdk/lib/km/{a}/BufferOverflowFastFailK.lib"),
        format!("wdk/lib/wdf/kmdf/{a}/{KMDF_VERSION}/WdfLdr.lib"),
        format!("wdk/lib/wdf/kmdf/{a}/{KMDF_VERSION}/WdfDriverEntry.lib"),
    ] {
        assert!(
            wdk_root.join(&canonical).exists(),
            "{canonical} symlink was not created for {arch}"
        );
    }

    // None of the msbuild plumbing or windows executables should have survived
    for unwanted in [
        "wdk/bin",
        "wdk/build",
        "wdk/tools",
        "wdk/include/wdf/kmdf/1.15",
    ] {
        assert!(
            !wdk_root.join(unwanted).exists(),
            "{unwanted} should not have been splatted"
        );
    }

    let build_dir = wdk_root.parent().unwrap().join(format!("driver-build-{a}"));
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir).unwrap();
    }
    std::fs::create_dir_all(&build_dir).unwrap();

    let obj = build_dir.join("driver.obj");

    let mut cc = Command::new("clang-cl");
    cc.arg(format!("--target={a}-pc-windows-msvc"))
        .args(["/c", "/kernel", "/GS-", "/WX"])
        // These are normally set by the WDK's msbuild props, not the compiler
        .args(defines)
        .args([
            "-D_KERNEL_MODE",
            "-DNTDDI_VERSION=0x0A000010",
            "-D_WIN32_WINNT=0x0A00",
            "-DWINVER=0x0A00",
        ]);

    for include in [
        "crt/include",
        "sdk/include/ucrt",
        "sdk/include/shared",
        "sdk/include/um",
    ] {
        cc.arg("/imsvc").arg(base.join(include));
    }

    for include in [
        "wdk/include/km".to_owned(),
        "wdk/include/shared".to_owned(),
        format!("wdk/include/wdf/kmdf/{KMDF_VERSION}"),
    ] {
        cc.arg("/imsvc").arg(wdk_root.join(include));
    }

    cc.arg(format!("/Fo{obj}"))
        .arg("tests/xwin-driver-test/driver.c");

    assert!(
        cc.status().unwrap().success(),
        "failed to compile the driver for {arch}"
    );

    let sys = build_dir.join("xwin-driver-test.sys");

    // Note the libraries are deliberately named with the casing that Microsoft's
    // own props use, which is neither the casing on disk nor an all upper or
    // lower case version of it
    let mut link = Command::new("lld-link");
    link.args([
        "/NOLOGO",
        "/SUBSYSTEM:NATIVE",
        "/DRIVER",
        "/ENTRY:FxDriverEntry",
        "/NODEFAULTLIB",
    ])
    .arg(format!("/OUT:{sys}"))
    .arg(format!("/LIBPATH:{wdk_root}/wdk/lib/km/{a}"))
    .arg(format!(
        "/LIBPATH:{wdk_root}/wdk/lib/wdf/kmdf/{a}/{KMDF_VERSION}"
    ))
    .arg(obj)
    .args([
        "ntoskrnl.lib",
        "hal.lib",
        "wmilib.lib",
        "BufferOverflowFastFailK.lib",
        "WdfLdr.lib",
        "WdfDriverEntry.lib",
    ]);

    assert!(
        link.status().unwrap().success(),
        "failed to link the driver for {arch}"
    );

    // A native subsystem PE is the actual deliverable, so check we produced one
    let pe = std::fs::read(&sys).unwrap();
    assert_eq!(&pe[..2], b"MZ", "{sys} is not a PE image");
}

#[test]
fn verify_driver_compiles() {
    let kit = splat_root();

    build_driver(
        &kit.root,
        &kit.root,
        xwin::Arch::X86_64,
        &["-D_AMD64_", "-DAMD64", "-D_WIN64"],
    );

    // The arm64 package spells the architecture directory `ARM64` where the x64
    // one spells it `x64`, so this is not just the same code path twice. Only the
    // WDK is splatted again, the CRT and SDK headers are shared with the x86_64
    // splat above and a driver links neither of them
    let arm = kit.splat_wdk(xwin::Arch::Aarch64, "splat-aarch64");

    build_driver(
        &kit.root,
        &arm,
        xwin::Arch::Aarch64,
        &["-D_ARM64_", "-DARM64", "-D_WIN64"],
    );
}
