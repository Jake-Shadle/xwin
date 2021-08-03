use crate::{Ctx, Error, Payload};
use futures::StreamExt;

pub async fn unpack(ctx: std::sync::Arc<Ctx>, items: Vec<Payload>) -> Result<(), Error> {
    let resu = futures::stream::iter(items)
        .map(|payload| {
            let ctx = ctx.clone();
            async move { ctx.unpack(&payload).await }
        })
        .buffer_unordered(32);

    let stats = resu
        .fold(
            crate::ctx::Unpack {
                compressed: 0,
                decompressed: 0,
                num_files: 0,
            },
            |mut acc, res| async move {
                match res {
                    Ok(up) => {
                        acc.num_files += up.num_files;
                        acc.compressed += up.compressed;
                        acc.decompressed += up.decompressed;
                        acc
                    }
                    Err(e) => {
                        tracing::error!("{:#}", e);
                        acc
                    }
                }
            },
        )
        .await;

    tracing::info!(
        total = stats.num_files,
        compressed = %indicatif::HumanBytes(stats.compressed),
        decompressed = %indicatif::HumanBytes(stats.decompressed),
        "unpacked files"
    );

    Ok(())
}
