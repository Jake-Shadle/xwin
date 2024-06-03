use anyhow::{ensure, Context as _};
use serde::Deserialize;
use std::{cmp, collections::BTreeMap};

use crate::Ctx;

#[derive(Deserialize, Debug, Clone)]
pub struct Payload {
    #[serde(rename = "fileName")]
    pub file_name: String,
    pub sha256: crate::util::Sha256,
    pub size: u64,
    pub url: String,
}

#[derive(Copy, Clone, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Chip {
    X86,
    X64,
    Arm,
    Arm64,
    Neutral,
}

#[derive(Copy, Clone, Deserialize, PartialEq, Eq, Debug)]
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

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct InstallSizes {
    pub target_drive: Option<u64>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ManifestItem {
    pub id: String,
    pub version: String,
    #[serde(rename = "type")]
    pub kind: ItemKind,
    pub chip: Option<Chip>,
    #[serde(default)]
    pub payloads: Vec<Payload>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, serde_json::Value>,
    pub install_sizes: Option<InstallSizes>,
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

#[derive(Deserialize, Debug)]
pub struct Manifest {
    #[serde(rename = "channelItems")]
    channel_items: Vec<ManifestItem>,
}

/// Retrieves the top-level manifest which contains license links as well as the
/// link to the actual package manifest which describes all of the contents
pub fn get_manifest(
    ctx: &Ctx,
    version: &str,
    channel: &str,
    progress: indicatif::ProgressBar,
) -> Result<Manifest, anyhow::Error> {
    let manifest_bytes = ctx.get_and_validate(
        format!("https://aka.ms/vs/{version}/{channel}/channel"),
        &format!("manifest_{version}.json"),
        None,
        progress,
    )?;

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;

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

    #[derive(Deserialize, Debug)]
    struct PkgManifest {
        packages: Vec<ManifestItem>,
    }

    let manifest: PkgManifest =
        serde_json::from_slice(&manifest_bytes).context("unable to parse manifest")?;

    let mut packages = BTreeMap::new();

    for pkg in manifest.packages {
        packages.insert(pkg.id.clone(), pkg);
    }

    Ok(PackageManifest { packages })
}

pub struct PackageManifest {
    pub packages: BTreeMap<String, ManifestItem>,
}
