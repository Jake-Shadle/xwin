use anyhow::{Context as _, ensure};
use serde::{Deserialize, de};
use std::{cmp, collections::BTreeMap};

use crate::Ctx;

#[derive(Debug, Clone)]
pub struct Payload {
    pub file_name: String,
    pub sha256: crate::util::Sha256,
    pub size: u64,
    pub url: String,
}

impl<'de> Deserialize<'de> for Payload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct V;

        impl<'de> de::Visitor<'de> for V {
            type Value = Payload;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("payload")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut file_name = None;
                let mut sha256 = None;
                let mut size = None;
                let mut url = None;

                while let Some(k) = map.next_key()? {
                    match k {
                        "fileName" => file_name = Some(map.next_value()?),
                        "sha256" => sha256 = Some(map.next_value()?),
                        "size" => size = Some(map.next_value()?),
                        "url" => url = Some(map.next_value()?),
                        _ => _ = map.next_value::<de::IgnoredAny>()?,
                    }
                }

                Ok(Payload {
                    file_name: file_name.ok_or_else(|| de::Error::missing_field("fileName"))?,
                    sha256: sha256.ok_or_else(|| de::Error::missing_field("sha256"))?,
                    size: size.ok_or_else(|| de::Error::missing_field("size"))?,
                    url: url.ok_or_else(|| de::Error::missing_field("url"))?,
                })
            }
        }

        deserializer.deserialize_map(V)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Chip {
    X86,
    X64,
    Arm,
    Arm64,
    Neutral,
}

impl<'de> Deserialize<'de> for Chip {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        #[allow(clippy::enum_glob_use)]
        use Chip::*;

        let s: std::borrow::Cow<'de, str> = de::Deserialize::deserialize(deserializer)?;

        let v = match s.as_ref() {
            "x86" => X86,
            "x64" => X64,
            "arm" => Arm,
            "arm64" => Arm64,
            "neutral" => Neutral,
            _ => {
                return Err(de::Error::unknown_variant(
                    s.as_ref(),
                    &["x86", "x64", "arm", "arm64", "neutral"],
                ));
            }
        };

        Ok(v)
    }
}

impl Chip {
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X64 => "x64",
            Self::Arm => "arm",
            Self::Arm64 => "arm64",
            Self::Neutral => "neutral",
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ItemKind {
    /// Unused.
    Bootstrapper,
    /// Unused.
    Channel,
    /// Unused.
    ChannelProduct,
    /// A composite package, no contents itself. Unused.
    Component,
    /// A single executable. Unused.
    Exe,
    /// Another kind of composite package without contents, and no localization. Unused.
    Group,
    /// Top level manifest
    Manifest,
    /// MSI installer
    Msi,
    /// Unused.
    Msu,
    /// Nuget package. Unused.
    Nupkg,
    /// Unused
    Product,
    /// A glorified zip file
    Vsix,
    /// Windows feature install/toggle. Unused.
    WindowsFeature,
    /// Unused.
    Workload,
    /// Plain zip file (ie not vsix). Unused.
    Zip,
}

impl<'de> Deserialize<'de> for ItemKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        #[allow(clippy::enum_glob_use)]
        use ItemKind::*;

        let s: std::borrow::Cow<'de, str> = de::Deserialize::deserialize(deserializer)?;

        let v = match s.as_ref() {
            "Vsix" => Vsix,
            "Msi" => Msi,
            "Manifest" => Manifest,

            "Bootstrapper" => Bootstrapper,
            "Channel" => Channel,
            "ChannelProduct" => ChannelProduct,
            "Component" => Component,
            "Exe" => Exe,
            "Group" => Group,
            "Msu" => Msu,
            "Nupkg" => Nupkg,
            "Product" => Product,
            "WindowsFeature" => WindowsFeature,
            "Workload" => Workload,
            "Zip" => Zip,
            _ => {
                return Err(de::Error::unknown_variant(
                    s.as_ref(),
                    &["Vsix", "Msi", "Manifest"],
                ));
            }
        };

        Ok(v)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstallSizes {
    pub target_drive: Option<u64>,
}

impl<'de> Deserialize<'de> for InstallSizes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct V;

        impl<'de> de::Visitor<'de> for V {
            type Value = InstallSizes;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("installSizes")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut td = None;

                while let Some(k) = map.next_key::<std::borrow::Cow<'_, str>>()? {
                    if k == "targetDrive" {
                        td = Some(map.next_value()?);
                    } else {
                        _ = map.next_value::<de::IgnoredAny>()?;
                    }
                }

                Ok(InstallSizes { target_drive: td })
            }
        }

        deserializer.deserialize_map(V)
    }
}

#[derive(Debug, Clone)]
pub struct ManifestItem {
    pub id: String,
    pub version: String,
    pub kind: ItemKind,
    pub chip: Option<Chip>,
    pub payloads: Vec<Payload>,
    pub dependencies: BTreeMap<String, serde_json::Value>,
    pub install_sizes: Option<InstallSizes>,
}

impl<'de> Deserialize<'de> for ManifestItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;

        impl<'de> de::Visitor<'de> for V {
            type Value = ManifestItem;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("manifest item")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut id = None;
                let mut version = None;
                let mut kind = None;
                let mut chip = None;
                let mut payloads = Vec::new();
                let mut dependencies = BTreeMap::default();
                let mut install_sizes = None;

                while let Some(k) = map.next_key()? {
                    match k {
                        "id" => id = Some(map.next_value()?),
                        "version" => version = Some(map.next_value()?),
                        "type" => kind = Some(map.next_value()?),
                        "chip" => chip = Some(map.next_value()?),
                        "payloads" => payloads = map.next_value()?,
                        "dependencies" => dependencies = map.next_value()?,
                        "installSizes" => install_sizes = Some(map.next_value()?),
                        _ => {
                            _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }

                Ok(ManifestItem {
                    id: id.ok_or_else(|| de::Error::missing_field("id"))?,
                    version: version.ok_or_else(|| de::Error::missing_field("version"))?,
                    kind: kind.ok_or_else(|| de::Error::missing_field("type"))?,
                    chip,
                    payloads,
                    dependencies,
                    install_sizes,
                })
            }
        }

        deserializer.deserialize_map(V)
    }
}

impl PartialEq for ManifestItem {
    #[inline]
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == cmp::Ordering::Equal
    }
}

impl Eq for ManifestItem {}

impl cmp::Ord for ManifestItem {
    #[inline]
    fn cmp(&self, o: &Self) -> cmp::Ordering {
        self.id.cmp(&o.id)
    }
}

impl cmp::PartialOrd for ManifestItem {
    #[inline]
    fn partial_cmp(&self, o: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(o))
    }
}

#[derive(Debug)]
pub struct Manifest {
    channel_items: Vec<ManifestItem>,
}

impl<'de> Deserialize<'de> for Manifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;

        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Manifest;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("list of channel items")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut channel_items = None;

                while let Some(k) = map.next_key::<std::borrow::Cow<'_, str>>()? {
                    if k == "channelItems" {
                        channel_items = Some(map.next_value()?);
                    } else {
                        _ = map.next_value::<de::IgnoredAny>()?;
                    }
                }

                Ok(Manifest {
                    channel_items: channel_items
                        .ok_or_else(|| de::Error::missing_field("channelItems"))?,
                })
            }
        }

        deserializer.deserialize_map(V)
    }
}

/// Retrieves the top-level manifest which contains license links as well as the
/// link to the actual package manifest which describes all of the contents
pub fn get_manifest(
    ctx: &Ctx,
    version: u8,
    mut channel: &str,
    progress: indicatif::ProgressBar,
) -> Result<Manifest, anyhow::Error> {
    // MS gonna MS
    if version >= 18 {
        if channel == "release" {
            channel = "stable";
        } else if channel == "pre" {
            channel = "insiders";
        }
    }

    let url = format!("https://aka.ms/vs/{version}/{channel}/channel");

    let manifest_bytes =
        ctx.get_and_validate(&url, &format!("manifest_{version}.json"), None, progress)?;

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("failed to deserialize manifest from {url}"))?;

    Ok(manifest)
}

/// Retrieves the package manifest specified in the input manifest
pub fn get_package_manifest(
    ctx: &Ctx,
    manifest: &Manifest,
    progress: indicatif::ProgressBar,
) -> Result<PackageManifest, anyhow::Error> {
    let pkg_manifest = manifest
        .channel_items
        .iter()
        .find(|ci| ci.kind == ItemKind::Manifest && !ci.payloads.is_empty())
        .context("Unable to locate package manifest")?;

    // This always just a single payload, but ensure it stays that way in the future
    ensure!(
        pkg_manifest.payloads.len() == 1,
        "VS package manifest should have exactly 1 payload"
    );

    // While the payload includes a sha256 checksum for the payload it is actually
    // never correct (even though it is part of the url!) so we have to just download
    // it without checking, which is terrible but...¯\_(ツ)_/¯
    let payload = &pkg_manifest.payloads[0];

    let manifest_bytes = ctx.get_and_validate(
        payload.url.clone(),
        &format!("pkg_manifest_{}.vsman", payload.sha256),
        None,
        progress,
    )?;

    #[derive(Debug)]
    struct PkgManifest {
        packages: Vec<ManifestItem>,
    }

    impl<'de> de::Deserialize<'de> for PkgManifest {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: de::Deserializer<'de>,
        {
            struct V;

            impl<'de> de::Visitor<'de> for V {
                type Value = PkgManifest;

                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("package manifest")
                }

                fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                where
                    A: de::MapAccess<'de>,
                {
                    let mut packages = None;

                    while let Some(k) = map.next_key::<std::borrow::Cow<'_, str>>()? {
                        if k == "packages" {
                            packages = Some(map.next_value()?);
                        } else {
                            _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }

                    Ok(PkgManifest {
                        packages: packages.ok_or_else(|| de::Error::missing_field("packages"))?,
                    })
                }
            }

            deserializer.deserialize_map(V)
        }
    }

    let manifest: PkgManifest =
        serde_json::from_slice(&manifest_bytes).context("unable to parse manifest")?;

    let mut packages = BTreeMap::new();
    let mut package_counts = BTreeMap::new();

    for pkg in manifest.packages {
        // built a unqiue key for each duplicate id to prevent overriding distinct packages
        let pkg_id = if packages.contains_key(&pkg.id) {
            let count = package_counts.get(&pkg.id).unwrap_or(&0) + 1;
            package_counts.insert(pkg.id.clone(), count);
            format!("{}#{}", pkg.id, count)
        } else {
            pkg.id.clone()
        };

        packages.insert(pkg_id, pkg);
    }

    Ok(PackageManifest { packages })
}

pub struct PackageManifest {
    pub packages: BTreeMap<String, ManifestItem>,
}
