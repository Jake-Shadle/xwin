use crate::PathBuf;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct VfsOverlay {
    pub version: u32,
    #[serde(with = "string_bool")]
    pub case_sensitive: bool,
    #[serde(with = "string_bool")]
    pub overlay_relative: bool,
    pub root_relative: &'static str,
    pub roots: Vec<VfsEntry>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum VfsEntry {
    File {
        #[serde(with = "pathbuf_serializer")]
        name: PathBuf,
        #[serde(with = "pathbuf_serializer")]
        #[serde(rename = "external-contents")]
        external_contents: PathBuf,
    },
    #[serde(rename = "directory-remap")]
    DirectoryRemap {
        #[serde(with = "pathbuf_serializer")]
        name: PathBuf,
        #[serde(with = "pathbuf_serializer")]
        #[serde(rename = "external-contents")]
        external_contents: PathBuf,
    },
}

mod pathbuf_serializer {
    use crate::PathBuf;
    use serde::{self, Serializer};

    pub fn serialize<S>(value: &PathBuf, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.as_str().replace('\\', "/"))
    }
}

impl VfsOverlay {
    pub fn new() -> Self {
        Self {
            version: 0,
            case_sensitive: false,
            overlay_relative: true,
            root_relative: "overlay-dir",
            roots: Vec::new(),
        }
    }
}

impl Default for VfsOverlay {
    fn default() -> Self {
        Self::new()
    }
}

mod string_bool {
    use serde::{self, Serializer};

    pub fn serialize<S>(value: &bool, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string().to_lowercase())
    }
}
