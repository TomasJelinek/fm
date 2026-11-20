//! Strict Make File List schema; native output names use the shared lossless path codec.
use super::{NativePath, invalid, present};
use midnight_core::{FileListPreferences, ForeignSetting, ForeignSettingKind};
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

/// An output file of another system is kept inactive; the dialog then starts without one.
pub(super) fn decode(
    version: u32,
    stored: Option<StoredFileListPreferences>,
) -> io::Result<(FileListPreferences, Option<ForeignSetting>)> {
    match (version, stored) {
        (1..=25, None) => Ok((FileListPreferences::default(), None)),
        (26.., Some(mut stored)) => {
            let mut foreign = None;
            if let Some(path) = &stored.path
                && let Some((system, location)) = path.foreign()?
            {
                foreign = Some(ForeignSetting {
                    kind: ForeignSettingKind::FileListOutput,
                    system,
                    location,
                    stored: serde_json::to_string(path).map_err(invalid)?.into(),
                });
                stored.path = None;
            }
            let preferences = FileListPreferences {
                template_history: stored.template_history,
                destination: stored.destination,
                path: stored.path.map(NativePath::into_path).transpose()?,
                append: stored.append,
            };
            preferences.validate().map_err(invalid)?;
            Ok((preferences, foreign))
        }
        _ => Err(invalid("missing or unexpected Make File List preferences")),
    }
}

/// A kept output file returns only while no native output file replaced it.
pub(super) fn merge(
    mut stored: StoredFileListPreferences,
    foreign: &[ForeignSetting],
) -> io::Result<StoredFileListPreferences> {
    if stored.path.is_none()
        && let Some(setting) = foreign
            .iter()
            .find(|setting| setting.kind == ForeignSettingKind::FileListOutput)
    {
        stored.path = Some(serde_json::from_str(&setting.stored).map_err(invalid)?);
    }
    Ok(stored)
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
