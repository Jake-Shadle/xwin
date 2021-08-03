use anyhow::{Context as _, Error};
use camino::Utf8PathBuf as PathBuf;
use std::{collections::BTreeMap, fmt};

mod ctx;
mod download;
pub mod manifest;
mod unpack;
pub mod util;

pub use ctx::Ctx;
pub use download::download;
pub use unpack::unpack;

pub enum Ops {
    Download = 0x1,
    Unpack = 0x2,
    Pack = 0x4,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Arch {
    X86 = 0x1,
    X86_64 = 0x2,
    Aarch = 0x4,
    Aarch64 = 0x8,
}

impl Arch {
    #[inline]
    fn from_chip(chip: Option<manifest::Chip>) -> Option<Self> {
        use manifest::Chip;

        chip.and_then(|chip| {
            Some(match chip {
                Chip::X86 => Self::X86,
                Chip::X64 => Self::X86_64,
                Chip::Arm => Self::Aarch,
                Chip::Arm64 => Self::Aarch64,
                Chip::Neutral => return None,
            })
        })
    }
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
        f.write_str(match self {
            Self::X86 => "x86",
            Self::X86_64 => "x86_64",
            Self::Aarch => "aarch",
            Self::Aarch64 => "aarch64",
        })
    }
}

impl Arch {
    pub fn iter(val: u32) -> impl Iterator<Item = (Self, &'static str)> {
        [Self::X86, Self::X86_64, Self::Aarch, Self::Aarch64]
            .iter()
            .filter_map(move |arch| {
                if *arch as u32 & val != 0 {
                    Some(match *arch {
                        Self::X86 => (Self::X86, "x86"),
                        Self::X86_64 => (Self::X86_64, "x64"),
                        Self::Aarch => (Self::Aarch, "arm"),
                        Self::Aarch64 => (Self::Aarch64, "arm64"),
                    })
                } else {
                    None
                }
            })
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Variant {
    Desktop = 0x1,
    OneCore = 0x2,
    Store = 0x4,
    /// All of the variants come in a spectre-safe form as well
    Spectre = 0x8,
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Desktop => "desktop",
            Self::OneCore => "onecore",
            Self::Store => "store",
            Self::Spectre => "spectre",
        })
    }
}

impl std::str::FromStr for Variant {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "desktop" => Self::Desktop,
            "onecore" => Self::OneCore,
            "store" => Self::Store,
            "spectre" => Self::Spectre,
            o => anyhow::bail!("unknown variant '{}'", o),
        })
    }
}

impl Variant {
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

pub async fn get_pkg_manifest(
    ctx: &Ctx,
    version: &str,
    channel: &str,
) -> Result<manifest::PackageManifest, Error> {
    let vs_manifest = manifest::get_manifest(&ctx, version, channel).await?;
    manifest::get_package_manifest(&ctx, &vs_manifest).await
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

#[derive(Copy, Clone, Debug)]
pub enum PayloadKind {
    CrtHeaders,
    CrtLibs,
    SdkHeaders,
    SdkLibs,
}

/// Returns the list of packages that are actually needed for cross compilation
pub fn prune_pkg_list(
    pkg_manifest: &manifest::PackageManifest,
    arches: u32,
    variants: u32,
) -> Result<Vec<Payload>, Error> {
    // We only really need 2 core pieces from the manifest, the CRT (headers + libs)
    // and the Windows SDK
    let pkgs = &pkg_manifest.packages;
    let mut pruned = Vec::new();

    get_crt(pkgs, arches, variants, &mut pruned)?;
    get_sdk(pkgs, arches, &mut pruned)?;

    Ok(pruned)
}

fn get_crt(
    pkgs: &BTreeMap<String, manifest::ManifestItem>,
    arches: u32,
    variants: u32,
    pruned: &mut Vec<Payload>,
) -> Result<(), Error> {
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
        .find_map(|(s, var)| payload.file_name.contains(s).then(|| *var));

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
        .find_map(|(s, arch)| payload.file_name.contains(s).then(|| *arch));

        Payload {
            filename: payload.file_name.clone().into(),
            sha256: payload.sha256.clone(),
            url: payload.url.clone(),
            size: payload.size,
            kind,
            target_arch,
            variant,
            install_size: (mi.payloads.len() == 1)
                .then(|| mi)
                .and_then(|mi| mi.install_sizes.as_ref().and_then(|is| is.target_drive)),
        }
    }

    let build_tools = pkgs
        .get("Microsoft.VisualStudio.Product.BuildTools")
        .context("unable to find root BuildTools item")?;

    let crt_version = build_tools
        .dependencies
        .keys()
        .filter_map(|key| {
            key.strip_prefix("Microsoft.VisualStudio.Component.VC.")
                .and_then(|s| s.strip_suffix(".x86.x64"))
        })
        .last()
        .context("unable to find latest CRT version")?;

    // The CRT headers are in the "base" package
    // `Microsoft.VC.<ridiculous_version_numbers>.CRT.Headers.base`
    {
        let header_key = format!("Microsoft.VC.{}.CRT.Headers.base", crt_version);

        let crt_headers = pkgs
            .get(&header_key)
            .with_context(|| format!("unable to find CRT headers item '{}'", header_key))?;

        pruned.push(to_payload(crt_headers, &crt_headers.payloads[0]));
    }

    {
        use std::fmt::Write;

        // The CRT libs are each in a separate arch + variant specific package.
        // The spectre versions include both the regular and spectre version of every lib
        let spectre = (variants & Variant::Spectre as u32) != 0;

        let mut crt_lib_id = String::new();

        for (arch, arch_str) in Arch::iter(arches) {
            for variant in Variant::iter(variants) {
                crt_lib_id.clear();

                write!(
                    &mut crt_lib_id,
                    "Microsoft.VC.{}.CRT.{}.{}{}.base",
                    crt_version,
                    // In keeping with MS's arbitrary casing all across the VS
                    // suite, arm64 is uppercased, but only in the ids of the
                    // CRT libs because...?
                    if arch == Arch::Aarch64 {
                        "ARM64"
                    } else {
                        arch_str
                    },
                    variant,
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
                        tracing::warn!("Unable to locate '{}'", crt_lib_id);
                    }
                }
            }
        }
    }

    Ok(())
}

fn get_sdk(
    pkgs: &BTreeMap<String, manifest::ManifestItem>,
    arches: u32,
    pruned: &mut Vec<Payload>,
) -> Result<(), Error> {
    let sdk = pkgs
        .values()
        .filter(|mi| mi.id.starts_with("Win10SDK_10."))
        .max()
        .context("unable to find latest Win10SDK version")?;

    // For some reason, their are multiple `Windows SDK Desktop Headers`, but
    // all of them other than x86-x86 appear to not actually have files in them
    // (though they are still 300KiB+)? Very confused.
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
            // Unfortunately can't predetermine install size due to how many payloads there are
            install_size: None,
            kind: PayloadKind::SdkHeaders,
            variant: None,
            target_arch: None,
        });
    }

    // Each target architecture has its own separate installer
    {
        for (arch, arch_str) in Arch::iter(arches) {
            let lib = sdk
                .payloads
                .iter()
                .find(|payload| {
                    payload
                        .file_name
                        .strip_prefix("Installers\\Windows SDK Desktop Libs ")
                        .and_then(|fname| fname.strip_suffix("-x86_en-us.msi"))
                        .map(|arch_id| arch_id == arch_str)
                        .unwrap_or(false)
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
    }

    Ok(())
}
