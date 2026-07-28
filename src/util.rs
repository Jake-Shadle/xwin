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
        write!(f, "{self}")
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for x in self.0 {
            write!(f, "{x:02x}")?;
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

        impl serde::de::Visitor<'_> for Visitor {
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

pub(crate) fn serialize_checksum<S>(chksum: &Checksum, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&chksum.to_string())
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

/// Sha-512 digests are only used by nuget, which unlike the VS manifests,
/// publishes base64 encoded sha-512 checksums rather than hex encoded sha-256 ones
#[derive(Clone, PartialEq, Eq)]
pub struct Sha512(pub [u8; 64]);

impl fmt::Debug for Sha512 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for Sha512 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for x in self.0 {
            write!(f, "{x:02x}")?;
        }

        Ok(())
    }
}

impl Sha512 {
    pub fn digest(buffer: &[u8]) -> Self {
        use sha2::Digest;

        let mut hasher = sha2::Sha512::new();
        hasher.update(buffer);
        let digest = hasher.finalize();

        Self(digest.into())
    }

    /// Decodes a standard (ie not url-safe) base64 encoded sha-512 digest, the
    /// form used by the nuget catalog. We do this by hand rather than pulling in
    /// a dependency for the ~15 lines it takes
    pub fn from_base64(b64: &str) -> Result<Self, Error> {
        #[inline]
        fn sextet(c: u8) -> Result<u32, Error> {
            Ok(match c {
                b'A'..=b'Z' => (c - b'A') as u32,
                b'a'..=b'z' => (c - b'a') as u32 + 26,
                b'0'..=b'9' => (c - b'0') as u32 + 52,
                b'+' => 62,
                b'/' => 63,
                c => anyhow::bail!("invalid byte '{c}' in base64 string"),
            })
        }

        // 64 bytes encodes to 86 characters + 2 padding
        let b64 = b64.trim_end_matches('=');
        anyhow::ensure!(
            b64.len() == 86,
            "base64 sha512 string length is {} instead of 86",
            b64.len()
        );

        let mut digest = [0u8; 64];
        let mut ind = 0;

        for chunk in b64.as_bytes().chunks(4) {
            let mut acc = 0u32;
            for (i, c) in chunk.iter().enumerate() {
                acc |= sextet(*c)? << (18 - i * 6);
            }

            // The final chunk only has 2 characters, which encode the last byte
            for i in 0..chunk.len() - 1 {
                digest[ind] = ((acc >> (16 - i * 8)) & 0xff) as u8;
                ind += 1;
            }
        }

        Ok(Self(digest))
    }
}

/// The checksum used to validate a downloaded payload. The VS manifests use
/// sha-256, nuget uses sha-512.
#[derive(Clone, PartialEq, Eq)]
pub enum Checksum {
    Sha256(Sha256),
    Sha512(Sha512),
}

impl Checksum {
    /// Hashes the buffer with the same algorithm as `self`, so that the two can
    /// be compared
    pub fn digest_like(&self, buffer: &[u8]) -> Self {
        match self {
            Self::Sha256(_) => Self::Sha256(Sha256::digest(buffer)),
            Self::Sha512(_) => Self::Sha512(Sha512::digest(buffer)),
        }
    }

    /// The sha-256 digest, if this is one. Used to correlate MSI payloads with
    /// the manifest item that owns them, which is always sha-256
    #[inline]
    pub fn as_sha256(&self) -> Option<&Sha256> {
        match self {
            Self::Sha256(sha) => Some(sha),
            Self::Sha512(_) => None,
        }
    }
}

impl From<Sha256> for Checksum {
    fn from(sha: Sha256) -> Self {
        Self::Sha256(sha)
    }
}

impl From<Sha512> for Checksum {
    fn from(sha: Sha512) -> Self {
        Self::Sha512(sha)
    }
}

impl fmt::Debug for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Note the sha-256 form is unprefixed so that the paths and messages it
        // is embedded in are unchanged from previous versions
        match self {
            Self::Sha256(sha) => write!(f, "{sha}"),
            Self::Sha512(sha) => write!(f, "sha512-{sha}"),
        }
    }
}

impl std::str::FromStr for Checksum {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some(hex_str) = s.strip_prefix("sha512-") else {
            return Ok(Self::Sha256(s.parse()?));
        };

        anyhow::ensure!(
            hex_str.len() == 128,
            "sha512 string length is {} instead of 128",
            hex_str.len()
        );

        let mut digest = [0u8; 64];

        for (ind, chars) in hex_str.as_bytes().chunks(2).enumerate() {
            let mut cur = hex_nibble(chars[0])?;
            cur <<= 4;
            cur |= hex_nibble(chars[1])?;

            digest[ind] = cur;
        }

        Ok(Self::Sha512(Sha512(digest)))
    }
}

#[inline]
fn hex_nibble(c: u8) -> Result<u8, Error> {
    Ok(match c {
        b'A'..=b'F' => c - b'A' + 10,
        b'a'..=b'f' => c - b'a' + 10,
        b'0'..=b'9' => c - b'0',
        c => anyhow::bail!("invalid byte in hex checksum string {c}"),
    })
}

impl<'de> serde::Deserialize<'de> for Checksum {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Checksum;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("checksum string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Checksum, E>
            where
                E: serde::de::Error,
            {
                value.parse().map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
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

    #[test]
    fn checksums_roundtrip() {
        let buffer = [3u8; 11];

        for digest in [
            Checksum::Sha256(Sha256::digest(&buffer)),
            Checksum::Sha512(Sha512::digest(&buffer)),
        ] {
            assert_eq!(digest, digest.to_string().parse::<Checksum>().unwrap());
            assert_eq!(digest, digest.digest_like(&buffer));
        }
    }

    /// The base64 digest is the one published by the nuget catalog for
    /// `Microsoft.Windows.WDK.x64` 10.0.26100.6584
    #[test]
    fn sha512_from_base64() {
        let b64 = "jhddaBnhMDqt3GVr32RVTtaR0KHmZDjY0JCTMn10OQpk4+AShXCK9QA09vBazp8ni1Mju84qM5SGXfIfnyOJ+g==";
        let digest = Sha512::from_base64(b64).unwrap();

        assert_eq!(
            digest.to_string(),
            concat!(
                "8e175d6819e1303aaddc656bdf64554ed691d0a1e66438d8d09093327d74390a",
                "64e3e01285708af50034f6f05ace9f278b5323bbce2a3394865df21f9f2389fa",
            )
        );

        assert!(Sha512::from_base64("nope").is_err());
    }
}
