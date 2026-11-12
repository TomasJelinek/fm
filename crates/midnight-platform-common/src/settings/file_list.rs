//! Strict Make File List schema; native output names use the shared lossless path codec.
use super::{NativePath, invalid, present};
use midnight_core::FileListPreferences;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredFileListPreferences {
    template_history: Vec<String>,
    destination: u8,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    path: Option<NativePath>,
    append: bool,
}

pub(super) fn decode(
    version: u32,
    stored: Option<StoredFileListPreferences>,
) -> io::Result<FileListPreferences> {
    match (version, stored) {
        (1..=25, None) => Ok(FileListPreferences::default()),
        (26.., Some(stored)) => {
            let preferences = FileListPreferences {
                template_history: stored.template_history,
                destination: stored.destination,
                path: stored.path.map(NativePath::into_path).transpose()?,
                append: stored.append,
            };
            preferences.validate().map_err(invalid)?;
            Ok(preferences)
        }
        _ => Err(invalid("missing or unexpected Make File List preferences")),
    }
}

impl From<FileListPreferences> for StoredFileListPreferences {
    fn from(preferences: FileListPreferences) -> Self {
        Self {
            template_history: preferences.template_history,
            destination: preferences.destination,
            path: preferences.path.map(NativePath::from_path),
            append: preferences.append,
        }
    }
}
