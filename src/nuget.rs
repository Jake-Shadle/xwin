//! The Windows Driver Kit is, unlike the CRT and Windows SDK, not available in
//! the Visual Studio manifests. The only driver related package there is
//! `Microsoft.Windows.DriverKit`, which is a ~2MB vsix containing the Visual
//! Studio integration (msbuild targets and project templates), not a single
//! header or library.
//!
//! Microsoft does however publish the WDK to nuget.org, which is what we use.

use crate::{Arch, Ctx, Error, Payload, PayloadKind, util::Sha512};
use anyhow::Context as _;

/// The nuget "flat container" (aka package base address) resource, which is a
/// simple static file layout, no API keys or content negotiation needed
const FLAT_CONTAINER: &str = "https://api.nuget.org/v3-flatcontainer";
/// The registration resource, used to locate the catalog entry for a specific
/// version, which is the only place the package's checksum is published
const REGISTRATION: &str = "https://api.nuget.org/v3/registration5-gz-semver2";

pub struct WdkPackages {
    /// The resolved WDK version, eg `10.0.26100.6584`
    pub version: String,
    /// The license that has to be accepted to use the WDK
    ///
    /// Note that unlike the Visual Studio packages, there is no permanent link
    /// for this. The nuspec declares the license as a file embedded in the
    /// package itself (its `licenseUrl` is just nuget's "this field is
    /// deprecated" page), so we point at the copy nuget serves of that exact
    /// file, which means it always describes precisely the bytes we downloaded
    pub license_url: String,
    pub payloads: Vec<Payload>,
}

/// A single WDK version published to nuget
pub struct WdkVersion {
    /// The full version, eg `10.0.26100.6584`
    pub version: String,
    /// The kit build the version belongs to, eg `26100`. This is what has to
    /// match the SDK, as the two are released together
    pub build: Option<String>,
    /// Prerelease versions carry a suffix, eg `10.0.28000.1-rtm`. They are never
    /// selected by default, but can still be requested explicitly
    pub prerelease: bool,
}

/// The WDK is only published for these architectures, there are no x86 or arm
/// packages
#[inline]
fn package_id(arch: Arch) -> Option<&'static str> {
    match arch {
        Arch::X86_64 => Some("microsoft.windows.wdk.x64"),
        Arch::Aarch64 => Some("microsoft.windows.wdk.arm64"),
        Arch::X86 | Arch::Aarch => None,
    }
}

/// The first of the requested architectures the WDK is actually published for
fn primary_id(arches: u32) -> Result<&'static str, Error> {
    Arch::iter(arches)
        .find_map(|arch| {
            let id = package_id(arch);

            if id.is_none() {
                tracing::warn!(
                    "the WDK is not published for '{arch}', only x86_64 and aarch64 are available"
                );
            }

            id
        })
        .context("unable to acquire the WDK for any of the requested architectures")
}

/// Retrieves every WDK version published to nuget, oldest first
///
/// `selected` is the one [`get_wdk`] would use for the same arguments, so that
/// callers can show it without having to reimplement the selection rules
pub fn list_versions(
    ctx: &Ctx,
    arches: u32,
    sdk_version: &str,
) -> Result<(Vec<WdkVersion>, Option<String>), Error> {
    let id = primary_id(arches)?;
    let available = get_versions(ctx, id)?;

    let selected = resolve_version(&available, None, sdk_version).ok();

    let mut versions: Vec<_> = available
        .iter()
        .map(|version| WdkVersion {
            build: build_number(version).map(String::from),
            prerelease: version.contains('-'),
            version: version.clone(),
        })
        .collect();

    versions.sort_by_cached_key(|v| versions::Version::new(&v.version));

    Ok((versions, selected))
}

/// Determines which version to acquire, either the one the user asked for, or the
/// newest that pairs with the SDK
fn resolve_version(
    available: &[String],
    wdk_version: Option<String>,
    sdk_version: &str,
) -> Result<String, Error> {
    let Some(user) = wdk_version else {
        // The WDK and SDK are released in lockstep, and the WDK nuspec declares a
        // dependency on the exact SDK build it pairs with, so prefer the WDK that
        // goes with the SDK we already resolved rather than just the newest one
        // published
        let paired = build_number(sdk_version)
            .and_then(|sdk_build| latest_version(available, Some(sdk_build)));

        return match paired {
            Some(paired) => Ok(paired),
            None => latest_version(available, None)
                .context("unable to determine the latest WDK version"),
        };
    };

    versions::Version::new(&user)
        .with_context(|| format!("invalid WDK version '{user}' specified"))?;

    anyhow::ensure!(
        available.contains(&user),
        "WDK version '{user}' does not exist on nuget"
    );

    Ok(user)
}

/// Retrieves the list of WDK payloads to acquire, one per requested architecture
///
/// Note that unlike the CRT and SDK, this requires network access even when just
/// listing packages, as the set of available versions can only be determined by
/// querying nuget
pub fn get_wdk(
    ctx: &Ctx,
    arches: u32,
    wdk_version: Option<String>,
    sdk_version: &str,
) -> Result<WdkPackages, Error> {
    // Every architecture is published in lockstep, so we only need to resolve
    // the version once, from whichever package we're going to download anyway
    let id = primary_id(arches)?;
    let available = get_versions(ctx, id)?;

    let version = resolve_version(&available, wdk_version, sdk_version)?;

    if let (Some(wdk_build), Some(sdk_build)) = (build_number(&version), build_number(sdk_version))
        && wdk_build != sdk_build
    {
        tracing::warn!(
            "the WDK version '{version}' does not match the SDK version '{sdk_version}', \
             they are meant to be used together, consider passing --sdk-version or --wdk-version"
        );
    }

    let payloads = Arch::iter(arches)
        .filter(|arch| package_id(*arch).is_some())
        .map(|arch| -> Result<Payload, Error> {
            let id = package_id(arch).unwrap();
            let filename = format!("{id}.{version}.nupkg");
            let url = format!("{FLAT_CONTAINER}/{id}/{version}/{filename}");

            let (checksum, size) = get_package_hash(ctx, id, &version)
                .with_context(|| format!("unable to retrieve the checksum for {id} {version}"))?;

            Ok(Payload {
                filename: filename.into(),
                checksum: checksum.into(),
                url,
                size,
                // The nupkg is a single archive, so the install size we report is
                // just what we're going to keep out of it, which we can't know
                // until it is unpacked
                install_size: None,
                kind: PayloadKind::Wdk,
                target_arch: Some(arch),
                variant: None,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let license_url = format!("https://www.nuget.org/packages/{id}/{version}/License");

    Ok(WdkPackages {
        version,
        license_url,
        payloads,
    })
}

/// The `10.0.<build>.<revision>` build number, used to check the WDK and SDK
/// actually belong to the same kit
fn build_number(version: &str) -> Option<&str> {
    version.split('.').nth(2)
}

/// Retrieves every published version of the package
///
/// Note we deliberately don't use [`Ctx::get_and_validate`] here, as it returns
/// any previously cached file as is when there is no checksum to validate it
/// against, which would pin the WDK to whatever version was the latest the first
/// time xwin was run
fn get_versions(ctx: &Ctx, id: &str) -> Result<Vec<String>, Error> {
    let url = format!("{FLAT_CONTAINER}/{id}/index.json");

    #[derive(serde::Deserialize)]
    struct Index {
        versions: Vec<String>,
    }

    let index: Index = get_json(ctx, &url)?;
    anyhow::ensure!(!index.versions.is_empty(), "{id} has no published versions");

    Ok(index.versions)
}

/// Retrieves the checksum and size of a specific version
///
/// The flat container doesn't publish checksums, they're only available from the
/// catalog entry that the registration index points at, and unlike the VS
/// manifests, they are base64 encoded sha-512 rather than hex encoded sha-256
fn get_package_hash(ctx: &Ctx, id: &str, version: &str) -> Result<(Sha512, u64), Error> {
    #[derive(serde::Deserialize)]
    struct Registration {
        #[serde(rename = "catalogEntry")]
        catalog_entry: String,
    }

    #[derive(serde::Deserialize)]
    struct CatalogEntry {
        #[serde(rename = "packageHash")]
        package_hash: String,
        #[serde(rename = "packageHashAlgorithm")]
        package_hash_algorithm: String,
        #[serde(rename = "packageSize")]
        package_size: u64,
    }

    let registration: Registration = get_json(ctx, &format!("{REGISTRATION}/{id}/{version}.json"))?;
    let entry: CatalogEntry = get_json(ctx, &registration.catalog_entry)?;

    anyhow::ensure!(
        entry.package_hash_algorithm.eq_ignore_ascii_case("sha512"),
        "expected a sha512 package hash, but nuget published a '{}' one",
        entry.package_hash_algorithm
    );

    let checksum = Sha512::from_base64(&entry.package_hash)
        .with_context(|| format!("{id} {version} has an invalid package hash"))?;

    Ok((checksum, entry.package_size))
}

fn get_json<T>(ctx: &Ctx, url: &str) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned,
{
    let body = ctx
        .client
        .get(url)
        .call()
        .with_context(|| format!("HTTP GET request for {url} failed"))?
        .into_body()
        .read_to_vec()
        .with_context(|| format!("failed to retrieve body for {url}"))?;

    serde_json::from_slice(&body)
        .with_context(|| format!("failed to deserialize response of {url}"))
}

/// Determines the latest stable version, ie ignoring any that carry a prerelease
/// suffix such as `10.0.28000.1-rtm` or `10.0.28000.1761-preview`, optionally
/// restricted to a single kit build
fn latest_version(versions: &[String], build: Option<&str>) -> Option<String> {
    versions
        .iter()
        .filter(|v| !v.contains('-'))
        .filter(|v| build.is_none_or(|build| build_number(v) == Some(build)))
        .filter_map(|v| Some((versions::Version::new(v)?, v)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, v)| v.clone())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn ignores_prereleases() {
        let versions: Vec<_> = [
            "10.0.26100.2454",
            "10.0.26100.6584",
            "10.0.28000.1-rtm",
            "10.0.28000.1761-preview",
            "10.0.28000.1839",
            "10.0.28000.2526",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        assert_eq!(latest_version(&versions, None).unwrap(), "10.0.28000.2526");

        let only_pre = vec!["10.0.28000.1-rtm".to_owned()];
        assert!(latest_version(&only_pre, None).is_none());
    }

    #[test]
    fn pairs_with_the_sdk_build() {
        let versions: Vec<_> = [
            "10.0.26100.2454",
            "10.0.26100.6584",
            "10.0.28000.1839",
            "10.0.28000.2526",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        // The newest WDK of the same build as the SDK, not the newest overall
        assert_eq!(
            latest_version(&versions, Some("26100")).unwrap(),
            "10.0.26100.6584"
        );
        assert!(latest_version(&versions, Some("22621")).is_none());
    }

    #[test]
    fn compares_kit_builds() {
        assert_eq!(build_number("10.0.26100.6584"), Some("26100"));
        assert_eq!(
            build_number("10.0.26100.6584"),
            build_number("10.0.26100.0")
        );
        assert_ne!(
            build_number("10.0.28000.2526"),
            build_number("10.0.26100.0")
        );
    }
}
