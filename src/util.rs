use crate::{Path, PathBuf};
use anyhow::{Context as _, Error};
use std::fmt;

#[inline]
pub fn canonicalize(path: &Path) -> anyhow::Result<PathBuf> {
    PathBuf::from_path_buf(
        path.canonicalize()
            .with_context(|| format!("unable to canonicalize path '{path}'"))?,
    )
    .map_err(|pb| anyhow::anyhow!("canonicalized path {} is not utf-8", pb.display()))
}

#[derive(Copy, Clone)]
pub enum ProgressTarget {
    Stdout,
    Stderr,
    Hidden,
}

impl From<ProgressTarget> for indicatif::ProgressDrawTarget {
    fn from(pt: ProgressTarget) -> Self {
        match pt {
            ProgressTarget::Stdout => Self::stdout(),
            ProgressTarget::Stderr => Self::stderr(),
            ProgressTarget::Hidden => Self::hidden(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Sha256(pub [u8; 32]);

impl fmt::Debug for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for x in self.0 {
            write!(f, "{:02x}", x)?;
        }

        Ok(())
    }
}

impl<'slice> PartialEq<&'slice [u8]> for Sha256 {
    fn eq(&self, o: &&'slice [u8]) -> bool {
        self.0 == *o
    }
}

impl std::str::FromStr for Sha256 {
    type Err = Error;

    fn from_str(hex_str: &str) -> Result<Self, Self::Err> {
        anyhow::ensure!(
            hex_str.len() == 64,
            "sha256 string length is {} instead of 64",
            hex_str.len()
        );

        let mut digest = [0u8; 32];

        for (ind, chars) in hex_str.as_bytes().chunks(2).enumerate() {
            let mut cur = match chars[0] {
                b'A'..=b'F' => chars[0] - b'A' + 10,
                b'a'..=b'f' => chars[0] - b'a' + 10,
                b'0'..=b'9' => chars[0] - b'0',
                c => anyhow::bail!("invalid byte in hex string {}", c),
            };

            cur <<= 4;

            cur |= match chars[1] {
                b'A'..=b'F' => chars[1] - b'A' + 10,
                b'a'..=b'f' => chars[1] - b'a' + 10,
                b'0'..=b'9' => chars[1] - b'0',
                c => anyhow::bail!("invalid byte in hex checksum string {}", c),
            };

            digest[ind] = cur;
        }

        Ok(Self(digest))
    }
}

impl<'de> serde::Deserialize<'de> for Sha256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Sha256;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("sha256 string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Sha256, E>
            where
                E: serde::de::Error,
            {
                value.parse().map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

pub(crate) fn serialize_sha256<S>(hash: &Sha256, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&hash.to_string())
}

impl Sha256 {
    pub fn digest(buffer: &[u8]) -> Self {
        use sha2::Digest;

        let mut hasher = sha2::Sha256::new();
        hasher.update(buffer);
        let digest = hasher.finalize();

        Self(digest.into())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn sha256() {
        let buffer = [3u8; 11];
        let digest = Sha256::digest(&buffer);

        let hex = digest.to_string();

        assert_eq!(digest, hex.parse::<Sha256>().unwrap());
    }
}
