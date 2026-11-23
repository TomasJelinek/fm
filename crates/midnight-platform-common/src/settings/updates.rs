//! Schema 40: Options › General › Updates. Written only when a choice is off; every
//! earlier file reads as both on, so existing settings get the startup check.
use super::{CURRENT_PREFERENCES_VERSION, invalid};
use midnight_core::UpdatePreferences;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredUpdates {
    check_at_startup: bool,
    previews: bool,
}

impl From<UpdatePreferences> for StoredUpdates {
    fn from(preferences: UpdatePreferences) -> Self {
        Self {
            check_at_startup: preferences.check_at_startup,
            previews: preferences.previews,
        }
    }
}

pub(super) fn decode(version: u32, stored: Option<StoredUpdates>) -> io::Result<UpdatePreferences> {
    match (version, stored) {
        // Version 40 is written only for an update choice.
        (40, None) => Err(invalid("missing schema 40 preferences")),
        (1..=CURRENT_PREFERENCES_VERSION, None) => Ok(UpdatePreferences::default()),
        (40..=CURRENT_PREFERENCES_VERSION, Some(stored)) => Ok(UpdatePreferences {
            check_at_startup: stored.check_at_startup,
            previews: stored.previews,
        }),
        _ => Err(invalid("unexpected update preferences")),
    }
}
