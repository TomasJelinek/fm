//! What the update check remembers between runs, in the user's private update folder:
//! when it last asked GitHub, when GitHub allows the next request, and the version the
//! user chose to skip. Losing the file only means one extra check.
use crate::install::write_json;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckState {
    /// Unix seconds of the last answered check.
    pub last_check: u64,
    /// No request before this Unix second (GitHub's rate limit or a backoff).
    pub retry_after: u64,
    /// Failed automatic checks in a row; the backoff doubles with each.
    pub failures: u32,
    /// "Skip this version": the startup check stays quiet about exactly this version.
    pub skipped: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(folder: &Path) -> Self {
        Self {
            path: folder.join("update-state.json"),
        }
    }

    pub fn load(&self) -> CheckState {
        fs::read(&self.path)
            .ok()
            .filter(|text| text.len() < 64 * 1024)
            .and_then(|text| serde_json::from_slice(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, state: &CheckState) {
        if let Some(folder) = self.path.parent()
            && fs::create_dir_all(folder).is_ok()
        {
            let _ = write_json(&self.path, state);
        }
    }
}
