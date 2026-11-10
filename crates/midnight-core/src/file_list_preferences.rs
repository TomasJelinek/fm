//! Accepted Make File List choices and bounded, exact template history.
use crate::{FileListTemplate, MAX_FILE_LIST_TEMPLATE_BYTES};
use std::{collections::HashSet, path::PathBuf};

pub const MAX_FILE_LIST_HISTORY: usize = 30;
pub const DEFAULT_FILE_LIST_TEMPLATE: &str = "$(FileName)$(CRLF)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileListPreferences {
    /// Newest accepted template first; comparisons preserve exact Unicode text.
    pub template_history: Vec<String>,
    /// Stable storage values: 0 clipboard, 1 internal viewer, 2 file.
    pub destination: u8,
    /// Last accepted native output path, independent of the currently open pane.
    pub path: Option<PathBuf>,
    pub append: bool,
}

impl Default for FileListPreferences {
    fn default() -> Self {
        Self {
            template_history: vec![DEFAULT_FILE_LIST_TEMPLATE.into()],
            destination: 0,
            path: None,
            append: false,
        }
    }
}

impl FileListPreferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.destination > 2 {
            return Err("Unknown Make File List destination.");
        }
        if self.template_history.len() > MAX_FILE_LIST_HISTORY {
            return Err("At most 30 file-list templates can be remembered.");
        }
        let mut seen = HashSet::new();
        for template in &self.template_history {
            if template.is_empty() || template.len() > MAX_FILE_LIST_TEMPLATE_BYTES {
                return Err("Saved file-list templates must contain 1 through 65536 UTF-8 bytes.");
            }
            if !seen.insert(template) {
                return Err("Saved file-list templates must be unique.");
            }
            if FileListTemplate::parse(template).is_err() {
                return Err("A saved Make File List template is invalid.");
            }
        }
        if self.path.as_ref().is_some_and(|path| {
            !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0)
        }) {
            return Err("The saved file-list path must be absolute and contain no NUL.");
        }
        Ok(())
    }

    /// Call only after an accepted operation, never when editing or canceling.
    /// Validation failure leaves the history unchanged.
    pub fn remember(&mut self, template: &str) -> Result<(), String> {
        self.validate().map_err(str::to_owned)?;
        if template.is_empty() {
            return Err("A file-list template cannot be empty.".into());
        }
        FileListTemplate::parse(template)?;
        self.template_history
            .retain(|previous| previous != template);
        self.template_history.insert(0, template.to_owned());
        self.template_history.truncate(MAX_FILE_LIST_HISTORY);
        Ok(())
    }

    /// Reset behavioral choices while retaining history and the last output path.
    pub fn reset_options(&mut self) {
        self.destination = 0;
        self.append = false;
    }
}

#[cfg(test)]
#[path = "file_list_preferences_tests.rs"]
mod tests;
