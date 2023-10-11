use crate::{manifest, util::Sha256, Ctx, Error};
use anyhow::Context as _;
use camino::Utf8PathBuf as PathBuf;
use std::sync::Arc;

#[derive(Debug)]
struct Cab {
    filename: PathBuf,
    sha256: Sha256,
    url: String,
    #[allow(dead_code)]
    size: u64,
}

pub(crate) struct CabContents {
    pub(crate) path: PathBuf,
    pub(crate) content: bytes::Bytes,
    pub(crate) sequence: u32,
}

pub(crate) enum PayloadContents {
    Vsix(bytes::Bytes),
    Msi {
        msi: bytes::Bytes,
        cabs: Vec<CabContents>,
    },
}

pub(crate) fn download(
    ctx: Arc<Ctx>,
    pkgs: Arc<std::collections::BTreeMap<String, manifest::ManifestItem>>,
    item: &crate::WorkItem,
) -> Result<PayloadContents, Error> {
    item.progress.set_message("📥 downloading..");

    let contents = ctx.get_and_validate(
        &item.payload.url,
        &item.payload.filename,
        Some(item.payload.sha256.clone()),
        item.progress.clone(),
    )?;

    let pc = match item.payload.filename.extension() {
        Some("msi") => {
            let cabs: Vec<_> = match pkgs.values().find(|mi| {
                mi.payloads
                    .iter()
                    .any(|mi_payload| mi_payload.sha256 == item.payload.sha256)
            }) {
                Some(mi) => mi
                    .payloads
                    .iter()
                    .filter(|pay| pay.file_name.ends_with(".cab"))
                    .map(|pay| Cab {
                        filename: pay
                            .file_name
                            .strip_prefix("Installers\\")
                            .unwrap_or(&pay.file_name)
                            .into(),
                        sha256: pay.sha256.clone(),
                        url: pay.url.clone(),
                        size: pay.size,
                    })
                    .collect(),
                None => anyhow::bail!(
                    "unable to find manifest parent for {}",
                    item.payload.filename
                ),
            };

            download_cabs(ctx, &cabs, item, contents)
        }
        Some("vsix") => Ok(PayloadContents::Vsix(contents)),
        ext => anyhow::bail!("unknown extension {ext:?}"),
    };

    item.progress.finish_with_message("downloaded");

    pc
}

/// Each SDK MSI has 1 or more cab files associated with it containing the actual
/// data we need that must be downloaded separately and indexed from the MSI
fn download_cabs(
    ctx: Arc<Ctx>,
    cabs: &[Cab],
    msi: &crate::WorkItem,
    msi_content: bytes::Bytes,
) -> Result<PayloadContents, Error> {
    use rayon::prelude::*;

    let msi_filename = &msi.payload.filename;

    let mut msi_pkg = msi::Package::open(std::io::Cursor::new(msi_content.clone()))
        .with_context(|| format!("invalid MSI for {}", msi_filename))?;

    // The `Media` table contains the list of cabs by name, which we then need
    // to lookup in the list of payloads.
    // Columns: [DiskId, LastSequence, DiskPrompt, Cabinet, VolumeLabel, Source]
    let cab_files: Vec<_> = msi_pkg
        .select_rows(msi::Select::table("Media"))
        .with_context(|| format!("{} does not contain a list of CAB files", msi_filename))?
        .filter_map(|row| {
            // Columns:
            // 0 - DiskId
            // 1 - LastSequence
            // 2 - DiskPrompt
            // 3 - Cabinet name
            // ...
            if row.len() >= 3 {
                // For some reason most/all of the msi files contain a NULL cabinet
                // in the first position which is useless
                row[3]
                    .as_str()
                    .and_then(|s| row[1].as_int().map(|seq| (s, seq as u32)))
                    .and_then(|(name, seq)| {
                        let cab_name = name.trim_matches('"');

                        cabs.iter().find_map(|payload| {
                            (payload.filename == cab_name).then(|| {
                                (
                                    PathBuf::from(format!(
                                        "{}/{cab_name}",
                                        msi_filename.file_stem().unwrap(),
                                    )),
                                    payload.sha256.clone(),
                                    payload.url.clone(),
                                    seq,
                                )
                            })
                        })
                    })
            } else {
                None
            }
        })
        .collect();

    let cabs = cab_files
        .into_par_iter()
        .map(
            |(cab_name, chksum, url, sequence)| -> Result<CabContents, Error> {
                let cab_contents =
                    ctx.get_and_validate(url, &cab_name, Some(chksum), msi.progress.clone())?;
                Ok(CabContents {
                    path: cab_name,
                    content: cab_contents,
                    sequence,
                })
            },
        )
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PayloadContents::Msi {
        msi: msi_content,
        cabs,
    })
}
