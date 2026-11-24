//! Schema 41: Find placement and recent filename masks, independent of active searches.
use super::{CURRENT_PREFERENCES_VERSION, StoredWindowGeometry, invalid, present};
use midnight_core::FindPreferences;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredFindPreferences {
    mask_history: Vec<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    window_geometry: Option<StoredWindowGeometry>,
}

pub(super) fn decode(
    version: u32,
    stored: Option<StoredFindPreferences>,
) -> io::Result<FindPreferences> {
    match (version, stored) {
        (41, None) => Err(invalid("missing schema 41 Find preferences")),
        (1..=CURRENT_PREFERENCES_VERSION, None) => Ok(FindPreferences::default()),
        (41..=CURRENT_PREFERENCES_VERSION, Some(stored)) => {
            let preferences = FindPreferences {
                mask_history: stored.mask_history,
                window_geometry: stored.window_geometry.map(Into::into),
            };
            preferences.validate().map_err(invalid)?;
            Ok(preferences)
        }
        _ => Err(invalid("unexpected Find preferences")),
    }
}

impl From<FindPreferences> for StoredFindPreferences {
    fn from(preferences: FindPreferences) -> Self {
        Self {
            mask_history: preferences.mask_history,
            window_geometry: preferences.window_geometry.map(Into::into),
        }
    }
}
