#![doc = include_str!("../README.md")]

use anyhow::{Context as _, Error};
pub use camino::{Utf8Path as Path, Utf8PathBuf as PathBuf};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

mod ctx;
mod download;
pub mod manifest;
mod minimize;
mod splat;
mod unpack;
pub mod util;

pub use ctx::Ctx;
pub use minimize::MinimizeConfig;
pub use splat::SplatConfig;
pub use ureq;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Arch {
    X86 = 0x1,
    X86_64 = 0x2,
    Aarch = 0x4,
    Aarch64 = 0x8,
}

impl std::str::FromStr for Arch {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "x86" => Self::X86,
            "x86_64" => Self::X86_64,
            "aarch" => Self::Aarch,
            "aarch64" => Self::Aarch64,
            o => anyhow::bail!("unknown architecture '{}'", o),
        })
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Arch {
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X86_64 => "x86_64",
            Self::Aarch => "aarch",
            Self::Aarch64 => "aarch64",
        }
    }

    #[inline]
    pub fn as_ms_str(&self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X86_64 => "x64",
            Self::Aarch => "arm",
            Self::Aarch64 => "arm64",
        }
    }

    pub fn iter(val: u32) -> impl Iterator<Item = Self> {
        [Self::X86, Self::X86_64, Self::Aarch, Self::Aarch64]
            .iter()
            .filter_map(move |arch| {
                if *arch as u32 & val != 0 {
                    Some(*arch)
                } else {
                    None
                }
            })
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Variant {
    Desktop = 0x1,
    OneCore = 0x2,
    Store = 0x4,
    /// All of the variants come in a spectre-safe form as well
    Spectre = 0x8,
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Variant {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "desktop" => Self::Desktop,
            "onecore" => Self::OneCore,
            //"store" => Self::Store,
            "spectre" => Self::Spectre,
            o => anyhow::bail!("unknown variant '{o}'"),
        })
    }
}

impl Variant {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::OneCore => "onecore",
            Self::Store => "store",
            Self::Spectre => "spectre",
        }
    }

    pub fn iter(val: u32) -> impl Iterator<Item = &'static str> {
        [Self::Desktop, Self::OneCore, Self::Store]
            .iter()
            .filter_map(move |var| {
                if *var as u32 & val != 0 {
                    Some(match *var {
                        Self::Desktop => "Desktop",
                        Self::OneCore => "OneCore.Desktop",
                        Self::Store => "Store",
                        Self::Spectre => unreachable!(),
                    })
                } else {
                    None
                }
            })
    }
}

pub enum Ops {
    Download,
    Unpack,
    Splat(SplatConfig),
    Minimize(MinimizeConfig),
}

#[derive(Clone)]
pub struct WorkItem {
    pub progress: indicatif::ProgressBar,
    pub payload: std::sync::Arc<Payload>,
}

#[derive(Clone, Debug)]
pub struct Payload {
    /// The "suggested" filename for the payload when stored on disk
    pub filename: PathBuf,
    /// The sha-256 checksum of the payload
    pub sha256: util::Sha256,
    /// The url from which to acquire the payload
    pub url: String,
    /// The total download size
    pub size: u64,
    /// If a package has a single payload, this will be set to the actual
    /// size it will be on disk when decompressed
    pub install_size: Option<u64>,
    /// The kind of the payload, which determines how we un/pack it
    pub kind: PayloadKind,
    /// Specific architecture this payload targets
    pub target_arch: Option<Arch>,
    /// Specific variant this payload targets
    pub variant: Option<Variant>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    AtlHeaders,
    AtlLibs,
    CrtHeaders,
    CrtLibs,
    SdkHeaders,
    SdkLibs,
    SdkStoreLibs,
    Ucrt,
}

pub struct PrunedPackageList {
    pub crt_version: String,
    pub sdk_version: String,
    pub payloads: Vec<Payload>,
}

/// Returns the list of packages that are actually needed for cross compilation
pub fn prune_pkg_list(
    pkg_manifest: &manifest::PackageManifest,
    arches: u32,
    variants: u32,
    include_atl: bool,
    sdk_version: Option<String>,
    crt_version: Option<String>,
) -> Result<PrunedPackageList, Error> {
    // We only really need 2 core pieces from the manifest, the CRT (headers + libs)
    // and the Windows SDK
    let pkgs = &pkg_manifest.packages;
    let mut payloads = Vec::new();

    let crt_version = get_crt(
        pkgs,
        arches,
        variants,
        &mut payloads,
        include_atl,
        crt_version,
    )?;
    let sdk_version = get_sdk(pkgs, arches, sdk_version, &mut payloads)?;

    Ok(PrunedPackageList {
        crt_version,
        sdk_version,
        payloads,
    })
}

fn get_crt(
    pkgs: &BTreeMap<String, manifest::ManifestItem>,
    arches: u32,
    variants: u32,
    pruned: &mut Vec<Payload>,
    include_atl: bool,
    crt_version: Option<String>,
) -> Result<String, Error> {
    fn to_payload(mi: &manifest::ManifestItem, payload: &manifest::Payload) -> Payload {
        // These are really the only two we care about
        let kind = if mi.id.contains("Headers") {
            PayloadKind::CrtHeaders
        } else {
            PayloadKind::CrtLibs
        };

        let variant = [
            // Put this one first as Desktop will match OneCore.Desktop otherwise
            ("OneCore", Variant::OneCore),
            ("Desktop", Variant::Desktop),
            ("Store", Variant::Store),
        ]
        .iter()
        .find_map(|(s, var)| payload.file_name.contains(s).then_some(*var));

        // The "chip" in the manifest means "host architecture" but we never need
        // to care about that since we only care about host agnostic artifacts, but
        // we do need to check the name of the payload in case it targets a specific
        // architecture only (eg libs)
        let target_arch = [
            ("x64", Arch::X86_64),
            // Put this one first otherwise "arm" will match it
            ("arm64", Arch::Aarch64),
            ("ARM64", Arch::Aarch64),
            ("arm", Arch::Aarch),
            // Put this last as many names also include the host architecture :p
            ("x86", Arch::X86),
        ]
        .iter()
        .find_map(|(s, arch)| payload.file_name.contains(s).then_some(*arch));

        Payload {
            filename: if let Some(Arch::Aarch64) = target_arch {
                payload.file_name.replace("ARM", "arm").into()
            } else {
                payload.file_name.clone().into()
            },
            sha256: payload.sha256.clone(),
            url: payload.url.clone(),
            size: payload.size,
            kind,
            target_arch,
            variant,
            install_size: (mi.payloads.len() == 1)
                .then_some(mi)
                .and_then(|mi| mi.install_sizes.as_ref().and_then(|is| is.target_drive)),
        }
    }

    let build_tools = pkgs
        .get("Microsoft.VisualStudio.Product.BuildTools")
        .context("unable to find root BuildTools item")?;

    let crt_version = if let Some(user) = crt_version {
        // Ensure it is a valid version and that it actually exists in the manifest
        versions::Version::new(&user)
            .with_context(|| format!("invalid CRT version '{user}' specified"))?;

        build_tools
            .dependencies
            .get(&format!(
                "Microsoft.VisualStudio.Component.VC.{user}.x86.x64"
            ))
            .with_context(|| format!("CRT version '{user}' does not exist in the manifest"))?;

        user
    } else {
        let crt_version_rs_versions = build_tools
            .dependencies
            .keys()
            .filter_map(|key| {
                key.strip_prefix("Microsoft.VisualStudio.Component.VC.")
                    .and_then(|s| s.strip_suffix(".x86.x64"))
                    .and_then(versions::Version::new)
            })
            .max()
            .context("unable to find latest CRT version")?;

        crt_version_rs_versions.to_string()
    };

    // The CRT headers are in the "base" package
    // `Microsoft.VC.<ridiculous_version_numbers>.CRT.Headers.base`
    {
        let header_key = format!("Microsoft.VC.{crt_version}.CRT.Headers.base");

        let crt_headers = pkgs
            .get(&header_key)
            .with_context(|| format!("unable to find CRT headers item '{header_key}'"))?;

        pruned.push(to_payload(crt_headers, &crt_headers.payloads[0]));
    }

    {
        use std::fmt::Write;

        // The CRT libs are each in a separate arch + variant specific package.
        // The spectre versions include both the regular and spectre version of every lib
        let spectre = (variants & Variant::Spectre as u32) != 0;

        // We need to force include the Store version as well, as they
        // include some libraries that are often linked by default, eg oldnames.lib
        let variants = variants | Variant::Store as u32;

        let mut crt_lib_id = String::new();

        for arch in Arch::iter(arches) {
            for variant in Variant::iter(variants) {
                crt_lib_id.clear();

                write!(
                    &mut crt_lib_id,
                    "Microsoft.VC.{crt_version}.CRT.{}.{variant}{}.base",
                    // In keeping with MS's arbitrary casing all across the VS
                    // suite, arm64 is uppercased, but only in the ids of the
                    // CRT libs because...?
                    if arch == Arch::Aarch64 {
                        "ARM64"
                    } else {
                        arch.as_ms_str()
                    },
                    // The Store variant doesn't have a spectre version
                    if spectre && variant != "Store" {
                        ".spectre"
                    } else {
                        ""
                    }
                )
                .unwrap();

                match pkgs.get(&crt_lib_id) {
                    Some(crt_libs) => {
                        pruned.push(to_payload(crt_libs, &crt_libs.payloads[0]));
                    }
                    None => {
                        tracing::warn!("Unable to locate '{crt_lib_id}'");
                    }
                }
            }
        }
        if include_atl {
            get_atl(pkgs, arches, spectre, pruned, &crt_version)?;
        }
    }

    Ok(crt_version)
}

fn get_atl(
    pkgs: &BTreeMap<String, manifest::ManifestItem>,
    arches: u32,
    spectre: bool,
    pruned: &mut Vec<Payload>,
    crt_version: &str,
) -> Result<(), Error> {
    fn to_payload(mi: &manifest::ManifestItem, payload: &manifest::Payload) -> Payload {
        // These are really the only two we care about
        let kind = if mi.id.contains("Headers") {
            PayloadKind::AtlHeaders
        } else {
            PayloadKind::AtlLibs
        };

        let filename = payload.file_name.to_lowercase();

        // The "chip" in the manifest means "host architecture" but we never need
        // to care about that since we only care about host agnostic artifacts, but
        // we do need to check the name of the payload in case it targets a specific
        // architecture only (eg libs)
        let target_arch = [
            ("x64", Arch::X86_64),
            // Put this one first otherwise "arm" will match it
            ("arm64", Arch::Aarch64),
            ("arm", Arch::Aarch),
            // Put this last as many names also include the host architecture :p
            ("x86", Arch::X86),
        ]
        .iter()
        .find_map(|(s, arch)| filename.contains(s).then_some(*arch));

        Payload {
            filename: if let Some(Arch::Aarch64) = target_arch {
                payload.file_name.replace("ARM", "arm").into()
            } else {
                payload.file_name.clone().into()
            },
            sha256: payload.sha256.clone(),
            url: payload.url.clone(),
            size: payload.size,
            kind,
            target_arch,
            variant: None,
            install_size: (mi.payloads.len() == 1)
                .then_some(mi)
                .and_then(|mi| mi.install_sizes.as_ref().and_then(|is| is.target_drive)),
        }
    }

    // The ATL headers are in the "base" package
    // `Microsoft.VC.<ridiculous_version_numbers>.ATL.Headers.base`
    {
        let header_key = format!("Microsoft.VC.{crt_version}.ATL.Headers.base");

        let atl_headers = pkgs
            .get(&header_key)
            .with_context(|| format!("unable to find ATL headers item '{header_key}'"))?;

        pruned.push(to_payload(atl_headers, &atl_headers.payloads[0]));
    }

    {
        use std::fmt::Write;

        let mut crt_lib_id = String::new();
        for variant_spectre in [false, true] {
            if variant_spectre && !spectre {
                continue;
            }

            for arch in Arch::iter(arches) {
                crt_lib_id.clear();

                write!(
                    &mut crt_lib_id,
                    "Microsoft.VC.{}.ATL.{}{}.base",
                    crt_version,
                    arch.as_ms_str().to_uppercase(), // ATL is uppercased for some reason
                    if variant_spectre { ".spectre" } else { "" }
                )
                .unwrap();

                match pkgs.get(&crt_lib_id) {
                    Some(crt_libs) => {
                        pruned.push(to_payload(crt_libs, &crt_libs.payloads[0]));
                    }
                    None => {
                        tracing::warn!("Unable to locate '{}'", crt_lib_id);
                    }
                }
            }
        }
    }

    Ok(())
}

fn get_latest_sdk_version<'keys>(
    keys: impl Iterator<Item = &'keys String>,
) -> Option<(String, versions::Version)> {
    // Normally I would consider regex overkill for this, but we already use
    // it for include scanning so...meh, this is only called once so there is
    // no need to do one time initialization or the like (except in tests where it doesn't matter)
    let regex = regex::Regex::new(r"^Win(\d+)SDK_(.+)").ok()?;
    let (major, full) = keys
        .filter_map(|key| {
            let caps = regex.captures(key)?;
            // So the SDK versions are, as usual for Microsoft, fucking stupid.
            // A Win11 SDK still (currently) have a 10.* version...so we can't just
            // assume that they will actually be ordered above a Win10 SDK? (though
            // probably...but better to NOT assume, never trust Microsoft versions numbers)
            let sdk_major: u8 = caps[1].parse().ok()?;
            let version = versions::Version::new(&caps[2])?;
            Some((sdk_major, version))
        })
        .max()?;

    Some((format!("Win{major}SDK_{full}"), full))
}

fn get_sdk(
    pkgs: &BTreeMap<String, manifest::ManifestItem>,
    arches: u32,
    sdk_version: Option<String>,
    pruned: &mut Vec<Payload>,
) -> Result<String, Error> {
    let (sdk, sdk_version) = if let Some(sdk_version) = sdk_version {
        let sv = versions::Version::new(&sdk_version)
            .with_context(|| format!("invalid SDK version '{sdk_version}'"))?;

        let (_, mi) = pkgs
            .iter()
            .find(|(key, _)| key.ends_with(&sdk_version))
            .with_context(|| format!("unable to locate SDK '{sdk_version}'"))?;

        (mi, sv)
    } else {
        let (full, sdk_version) =
            get_latest_sdk_version(pkgs.keys()).context("unable to find latest WinSDK version")?;

        let sdk = pkgs
            .get(&full)
            .with_context(|| format!("unable to locate SDK {sdk_version}"))?;

        (sdk, sdk_version)
    };

    // So. There are multiple SDK Desktop Headers, one per architecture. However,
    // all of the non-x86 ones include either 0 or few files, with x86 containing
    // the vast majority of the actual needed headers. However, it also doesn't
    // have all of them, as there are even more required headers in the completely
    // separate `Windows Store Apps Headers-x86` package as well. Incredibly annoying.
    {
        let header_payload = sdk
            .payloads
            .iter()
            .find(|payload| {
                payload
                    .file_name
                    .ends_with("Windows SDK Desktop Headers x86-x86_en-us.msi")
            })
            .with_context(|| format!("unable to find headers for {}", sdk.id))?;

        pruned.push(Payload {
            filename: format!("{}_headers.msi", sdk.id).into(),
            sha256: header_payload.sha256.clone(),
            url: header_payload.url.clone(),
            size: header_payload.size,
            install_size: None,
            kind: PayloadKind::SdkHeaders,
            variant: None,
            target_arch: None,
        });

        // https://github.com/Jake-Shadle/xwin/pull/134
        // This was previously just part of the regular SDK headers
        let header_payload = sdk.payloads.iter().find(|payload| {
            payload
                .file_name
                .ends_with("Windows SDK OnecoreUap Headers x86-x86_en-us.msi")
        });

        if let Some(header_payload) = header_payload {
            pruned.push(Payload {
                filename: format!("{}_uap_headers.msi", sdk.id).into(),
                sha256: header_payload.sha256.clone(),
                url: header_payload.url.clone(),
                size: header_payload.size,
                install_size: None,
                kind: PayloadKind::SdkHeaders,
                variant: None,
                target_arch: None,
            });
        }

        let header_payload = sdk
            .payloads
            .iter()
            .find(|payload| {
                payload
                    .file_name
                    .ends_with("Windows SDK for Windows Store Apps Headers-x86_en-us.msi")
            })
            .with_context(|| format!("unable to find Windows SDK for Windows Store Apps Headers-x86_en-us.msi for {}", sdk.id))?;

        pruned.push(Payload {
            filename: format!("{}_store_headers.msi", sdk.id).into(),
            sha256: header_payload.sha256.clone(),
            url: header_payload.url.clone(),
            size: header_payload.size,
            install_size: None,
            kind: PayloadKind::SdkHeaders,
            variant: Some(Variant::Store),
            target_arch: None,
        });

        // https://github.com/Jake-Shadle/xwin/issues/128
        let header_payload = sdk.payloads.iter().find(|payload| {
            payload
                .file_name
                .ends_with("Windows SDK for Windows Store Apps Headers OnecoreUap-x86_en-us.msi")
        });

        if let Some(header_payload) = header_payload {
            pruned.push(Payload {
                filename: format!("{}_store_headers_onecoreuap.msi", sdk.id).into(),
                sha256: header_payload.sha256.clone(),
                url: header_payload.url.clone(),
                size: header_payload.size,
                install_size: None,
                kind: PayloadKind::SdkHeaders,
                variant: Some(Variant::Store),
                target_arch: None,
            });
        }

        for arch in Arch::iter(arches) {
            if arch == Arch::X86 {
                continue;
            }

            let header_payload = sdk
                .payloads
                .iter()
                .find(|payload| {
                    payload
                        .file_name
                        .strip_prefix("Installers\\Windows SDK Desktop Headers ")
                        .and_then(|fname| fname.strip_suffix("-x86_en-us.msi"))
                        .map_or(false, |fname| fname == arch.as_ms_str())
                })
                .with_context(|| format!("unable to find {} headers for {}", arch, sdk.id))?;

            pruned.push(Payload {
                filename: format!("{}_{}_headers.msi", sdk.id, arch.as_ms_str()).into(),
                sha256: header_payload.sha256.clone(),
                url: header_payload.url.clone(),
                size: header_payload.size,
                install_size: None,
                kind: PayloadKind::SdkHeaders,
                variant: None,
                target_arch: Some(arch),
            });
        }
    }

    // Each target architecture has its own separate installer. Oh, and we also
    // have to get the Windows Store Apps Libs, which has such libraries as
    // kernel32 etc. :p
    {
        for arch in Arch::iter(arches) {
            let lib = sdk
                .payloads
                .iter()
                .find(|payload| {
                    payload
                        .file_name
                        .strip_prefix("Installers\\Windows SDK Desktop Libs ")
                        .and_then(|fname| fname.strip_suffix("-x86_en-us.msi"))
                        .map_or(false, |arch_id| arch_id == arch.as_ms_str())
                })
                .with_context(|| format!("unable to find SDK libs for '{}'", arch))?;

            pruned.push(Payload {
                filename: format!("{}_libs_{}.msi", sdk.id, arch).into(),
                sha256: lib.sha256.clone(),
                url: lib.url.clone(),
                size: lib.size,
                install_size: None,
                kind: PayloadKind::SdkLibs,
                variant: None,
                target_arch: Some(arch),
            });
        }

        let lib_payload = sdk
            .payloads
            .iter()
            .find(|payload| {
                payload
                    .file_name
                    .ends_with("Windows SDK for Windows Store Apps Libs-x86_en-us.msi")
            })
            .with_context(|| {
                format!(
                    "unable to find Windows SDK for Windows Store Apps Libs-x86_en-us.msi for {}",
                    sdk.id
                )
            })?;

        pruned.push(Payload {
            filename: format!("{}_store_libs.msi", sdk.id).into(),
            sha256: lib_payload.sha256.clone(),
            url: lib_payload.url.clone(),
            size: lib_payload.size,
            install_size: None,
            kind: PayloadKind::SdkStoreLibs,
            variant: None,
            target_arch: None,
        });
    }

    // We also need the Universal CRT, which is luckily all just in a single MSI
    {
        let ucrt = pkgs
            .get("Microsoft.Windows.UniversalCRT.HeadersLibsSources.Msi")
            .context("unable to find Universal CRT")?;

        let msi = ucrt
            .payloads
            .iter()
            .find(|payload| {
                payload.file_name == "Universal CRT Headers Libraries and Sources-x86_en-us.msi"
            })
            .context("unable to find Universal CRT MSI")?;

        pruned.push(Payload {
            filename: "ucrt.msi".into(),
            sha256: msi.sha256.clone(),
            url: msi.url.clone(),
            size: msi.size,
            install_size: None,
            kind: PayloadKind::Ucrt,
            variant: None,
            target_arch: None,
        });
    }

    Ok(sdk_version.to_string())
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Map {
    pub crt: Block,
    pub sdk: Block,
}

impl Map {
    fn clear(&mut self) {
        self.crt.clear();
        self.sdk.clear();
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Block {
    pub headers: Section,
    pub libs: Section,
}

impl Block {
    fn clear(&mut self) {
        self.headers.clear();
        self.libs.clear();
    }
}

#[derive(Copy, Clone)]
pub enum SectionKind {
    CrtHeader,
    CrtLib,
    SdkHeader,
    SdkLib,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Section {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub filter: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symlinks: BTreeMap<String, Vec<String>>,
}

impl Section {
    fn clear(&mut self) {
        self.filter.clear();
        self.symlinks.clear();
    }
}

#[cfg(unix)]
#[inline]
fn symlink(original: &str, link: &Path) -> Result<(), Error> {
    std::os::unix::fs::symlink(original, link)
        .with_context(|| format!("unable to symlink from {link} to {original}"))
}

#[cfg(windows)]
#[inline]
fn symlink(_original: &str, _link: &Path) -> Result<(), Error> {
    Ok(())
}

#[inline]
fn symlink_on_windows_too(original: &str, link: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        symlink(original, link)
    }

    #[cfg(windows)]
    {
        let full_path = link.parent().unwrap().join(original);
        if full_path.is_dir() {
            std::os::windows::fs::symlink_dir(original, link)
                .with_context(|| format!("unable to symlink from {link} to {original}"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod test {
    use super::get_latest_sdk_version as glsv;

    #[test]
    fn sdk_versions() {
        let just_10 = [
            "Win10SDK_10.0.1629".to_owned(),
            "Win10SDK_10.0.17763".to_owned(),
            "Win10SDK_10.0.17134".to_owned(),
        ];

        let (full, vers) = glsv(just_10.iter()).unwrap();

        assert_eq!(just_10[1], full);
        assert_eq!("10.0.17763", vers.to_string());

        let just_11 = [
            "Win11SDK_10.0.22001".to_owned(),
            "Win11SDK_10.0.22000".to_owned(),
        ];

        assert_eq!(just_11[0], glsv(just_11.iter()).unwrap().0);

        assert_eq!(
            just_11[0],
            glsv(just_11.iter().chain(just_10.iter())).unwrap().0
        );
    }
}
