use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::Ctx;

#[derive(Deserialize, Serialize)]
pub struct Payload {
    #[serde(rename = "fileName")]
    pub file_name: String,
    pub sha256: String,
    pub size: usize,
    pub url: String,
}

#[derive(Deserialize, Serialize)]
pub struct ManifestItem {
    id: String,
    version: String,
    #[serde(rename = "type")]
    typ: String,
    chip: Option<String>,
    payloads: Vec<Payload>,
    dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize)]
pub struct Manifest {
    #[serde(rename = "channelItems")]
    channel_items: Vec<ManifestItem>,
}

pub async fn get_manifest(
    ctx: &Ctx,
    version: &str,
    channel: &str,
) -> Result<Manifest, anyhow::Error> {
    let manifest_bytes = ctx
        .get(
            format!("https://aka.ms/vs/{}/{}/channel", version, channel),
            &format!("manifest_{}.json", version),
        )
        .await?;

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;

    Ok(manifest)
}

pub async fn get_package_manifest(
    ctx: &Ctx,
    manifest: &Manifest,
) -> Result<PackageManifest, anyhow::Error> {
    let pkg_manifest = manifest
        .channel_items
        .iter()
        .find(|ci| ci.typ == "Manifest" && !ci.payloads.is_empty())
        .context("Unable to locate package manifest")?;

    // It will always be the first payload?
    let payload = &pkg_manifest.payloads[0];
    let payload_sha256 = payload.sha256.clone();

    let manifest_bytes = ctx
        .get_and_validate(
            payload.url.clone(),
            &format!("pkg_manifest_{}.vsman", payload_sha256),
            move |bytes| match crate::validate_checksum(bytes, &payload_sha256) {
                Ok(_) => true,
                Err(err) => {
                    log::error!("Failed to validate package manifest checksum: {}", err);
                    false
                }
            },
        )
        .await?;

    #[derive(Deserialize)]
    struct PkgManifest {
        packages: Vec<ManifestItem>,
    }

    let manifest: PkgManifest = serde_json::from_slice(&manifest_bytes)?;

    let mut packages = BTreeMap::new();

    for pkg in manifest.packages {
        packages.insert(pkg.id.clone(), pkg);
    }

    Ok(PackageManifest { packages })
}

pub struct PackageManifest {
    packages: BTreeMap<String, ManifestItem>,
}
