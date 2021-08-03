use crate::{manifest, util::Sha256, Ctx, Error, Payload};
use anyhow::Context as _;
use camino::Utf8PathBuf as PathBuf;
use futures::StreamExt;

#[derive(Debug)]
struct Cab {
    filename: PathBuf,
    sha256: Sha256,
    url: String,
}

pub async fn download(
    ctx: std::sync::Arc<Ctx>,
    pkgs: &std::collections::BTreeMap<String, manifest::ManifestItem>,
    items: Vec<Payload>,
) -> Result<(), Error> {
    let resu = futures::stream::iter(items.into_iter().map(|payload| {
        let cabs = if payload.filename.extension() == Some("msi") {
            match pkgs.values().find(|mi| {
                mi.payloads
                    .iter()
                    .any(|mi_payload| mi_payload.sha256 == payload.sha256)
            }) {
                Some(mi) => {
                    let cabs: Vec<_> = mi
                        .payloads
                        .iter()
                        .filter_map(|pay| {
                            pay.file_name.ends_with(".cab").then(|| Cab {
                                filename: pay
                                    .file_name
                                    .strip_prefix("Installers\\")
                                    .unwrap_or(&pay.file_name)
                                    .into(),
                                sha256: pay.sha256.clone(),
                                url: pay.url.clone(),
                            })
                        })
                        .collect();

                    Some(cabs)
                }
                None => {
                    tracing::error!("unable to find manifest parent for {}", payload.filename);
                    None
                }
            }
        } else {
            None
        };

        (payload, cabs)
    }))
    .map(|(payload, cabs)| {
        let ctx = ctx.clone();
        async move {
            let dl_ctx = ctx.clone();
            let (dl_url, dl_filename, dl_sha) = (
                payload.url.clone(),
                payload.filename.clone(),
                payload.sha256.clone(),
            );

            match async move {
                dl_ctx
                    .get_and_validate(dl_url, &dl_filename, Some(dl_sha))
                    .await
            }
            .await
            {
                Ok(msi_data) => match cabs {
                    Some(cabs) => download_cabs(ctx.clone(), &cabs, &payload, &msi_data).await,
                    None => Ok(()),
                },
                Err(e) => Err(e),
            }
        }
    })
    .buffer_unordered(32);

    resu.fold((), |u, res| async move {
        match res {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("{:#}", e);
                u
            }
        }
    })
    .await;

    Ok(())
}

/// Each SDK MSI has 1 or more cab files associated with it containing the actual
/// data we need that must be downloaded separately and indexed from the MSI
#[tracing::instrument(skip(ctx, cabs, msi_content))]
async fn download_cabs(
    ctx: std::sync::Arc<Ctx>,
    cabs: &[Cab],
    msi: &Payload,
    msi_content: &[u8],
) -> Result<(), Error> {
    let mut msi_pkg = msi::Package::open(std::io::Cursor::new(msi_content))
        .with_context(|| format!("invalid MSI for {}", msi.filename))?;

    // The `Media` table contains the list of cabs by name, which we then need
    // to lookup in the list of payloads.
    // Columns: [DiskId, LastSequence, DiskPrompt, Cabinet, VolumeLabel, Source]
    let cabs: Vec<_> = msi_pkg
        .select_rows(msi::Select::table("Media"))
        .with_context(|| format!("{} does not contain a list of CAB files", msi.filename))?
        .filter_map(|row| {
            for column in 0..row.len() {
                tracing::debug!("{} {:#?}", column, row[column]);
            }

            if row.len() >= 3 {
                // For some reason most/all of the msi files contain a NULL cabinet
                // in the first position which is useless
                row[3].as_str().and_then(|s| {
                    let cab_name = s.trim_matches('"');

                    cabs.iter().find_map(|payload| {
                        (payload.filename == cab_name).then(|| {
                            (
                                PathBuf::from(format!(
                                    "{}/{}",
                                    msi.filename.file_stem().unwrap(),
                                    cab_name
                                )),
                                payload.sha256.clone(),
                                payload.url.clone(),
                            )
                        })
                    })
                })
            } else {
                None
            }
        })
        .collect();

    let cabs = futures::stream::iter(cabs)
        .map(|(cab_name, chksum, url)| {
            let ctx = ctx.clone();

            async move {
                ctx.get_and_validate(url, &cab_name, Some(chksum))
                    .await
                    .map(|cab_bytes| cab_bytes.len())
            }
        })
        .buffer_unordered(4);

    let (count, downloaded) = cabs
        .fold((0, 0), |acc, res| async move {
            match res {
                Ok(size) => (acc.0 + 1, acc.1 + size),
                Err(e) => {
                    tracing::error!("{:#}", e);
                    acc
                }
            }
        })
        .await;

    tracing::debug!(
        "downloaded {} cabs totalling {}",
        count,
        indicatif::HumanBytes(downloaded as u64)
    );
    Ok(())
}
