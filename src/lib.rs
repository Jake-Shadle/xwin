mod ctx;
pub mod manifest;

pub use ctx::Ctx;

pub enum Ops {
    Download = 0x1,
    Unpack = 0x2,
    Pack = 0x4,
}

pub async fn execute(ctx: Ctx, ops: u32) -> Result<(), anyhow::Error> {
    let vs_manifest = manifest::get_manifest(&ctx, )
}

pub fn validate_checksum(buffer: &[u8], expected: &str) -> Result<(), anyhow::Error> {
    if expected.len() != 64 {
        anyhow::bail!(
            "hex checksum length is {} instead of expected 64",
            expected.len()
        );
    }

    let content_digest = ring::digest::digest(&ring::digest::SHA256, buffer);
    let digest = content_digest.as_ref();

    for (ind, exp) in expected.as_bytes().chunks(2).enumerate() {
        let mut cur;

        match exp[0] {
            b'A'..=b'F' => cur = exp[0] - b'A' + 10,
            b'a'..=b'f' => cur = exp[0] - b'a' + 10,
            b'0'..=b'9' => cur = exp[0] - b'0',
            c => anyhow::bail!("invalid byte in expected checksum string {}", c),
        }

        cur <<= 4;

        match exp[1] {
            b'A'..=b'F' => cur |= exp[1] - b'A' + 10,
            b'a'..=b'f' => cur |= exp[1] - b'a' + 10,
            b'0'..=b'9' => cur |= exp[1] - b'0',
            c => anyhow::bail!("invalid byte in expected checksum string {}", c),
        }

        if digest[ind] != cur {
            anyhow::bail!("checksum mismatch, expected {}", expected);
        }
    }

    Ok(())
}
