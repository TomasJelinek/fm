//! Export and Import configuration messages. The settings-file codec, its validation
//! and every file access run on backend workers; the UI chooses paths and stages
//! accepted values in its Options transaction.
use crate::{DesktopRequestId, UserPreferences};
use std::path::PathBuf;

/// Editable name the Export configuration chooser suggests.
pub const SETTINGS_FILE_SUGGESTED_NAME: &str = "midnight-explorer-settings.json";

/// Write one complete settings snapshot in the settings-file format, the same
/// versioned serialization as the application's own settings file.
///
/// An existing destination fails with `AlreadyExists` unless `overwrite` records
/// the user's explicit Replace answer for this destination. A confirmed replacement
/// keeps the displaced file for recovery where the filesystem supports it.
#[derive(Clone, Debug)]
pub struct PreferencesExportRequest {
    pub request: DesktopRequestId,
    pub destination: PathBuf,
    pub preferences: UserPreferences,
    pub overwrite: bool,
}

/// A completely validated settings file, and what it brought from another system.
/// Its `foreign_hot_paths` and `foreign_settings` hold the entries it keeps inactive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedPreferences {
    pub preferences: UserPreferences,
    /// Tabs whose folders belong to another system; they are not opened.
    pub skipped_tabs: usize,
    /// Key bindings of custom commands kept from another system. They are removed from
    /// the active bindings and kept, inactive, with those commands' stored records.
    pub removed_bindings: usize,
}

/// A published export whose bytes were read back and verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreferencesExportReceipt {
    pub path: PathBuf,
    pub bytes: u64,
    /// Displaced earlier files that a confirmed replacement retained for recovery.
    pub recovery: Vec<PathBuf>,
}
