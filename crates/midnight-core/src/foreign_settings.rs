//! Settings that a settings file from another operating system brought along.
//!
//! Their paths use the other system's syntax or encoding. They are kept only as
//! data: never resolved, opened, run or turned into native paths, so a stored
//! `/home/a` can never become a folder on the current Windows drive. The settings
//! codec saves and exports their stored records unchanged, and they become ordinary
//! settings again on their own system.
use crate::{
    BookmarkId, MAX_BOOKMARKS, MAX_EDITOR_RULES, MAX_FILE_COMPARATOR_RULES, MAX_USER_COMMANDS,
    UserCommandId, UserPreferences,
};
use std::{collections::HashSet, sync::Arc};

/// Path syntax of an operating system, judged on text alone, without filesystem access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathSystem {
    Windows,
    Unix,
}

impl PathSystem {
    /// The system this build runs on.
    pub const CURRENT: Self = if cfg!(windows) {
        Self::Windows
    } else {
        Self::Unix
    };

    pub const fn other(self) -> Self {
        match self {
            Self::Windows => Self::Unix,
            Self::Unix => Self::Windows,
        }
    }

    /// User-facing name; Linux is the supported Unix-like system.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Unix => "Linux",
        }
    }

    /// The other system, when stored text is an absolute path in its syntax.
    pub fn foreign(text: &str) -> Option<Self> {
        Self::foreign_for(Self::CURRENT, text)
    }

    /// [`Self::foreign`] as seen from `current`. A Unix path begins with `/`, which
    /// Windows would otherwise resolve on the current drive. A Windows path has a
    /// drive letter or begins with `\\`, which Unix would treat as a relative name.
    pub fn foreign_for(current: Self, text: &str) -> Option<Self> {
        if text.contains('\0') {
            return None;
        }
        let other = match current {
            Self::Windows => text.starts_with('/'),
            Self::Unix => {
                let bytes = text.as_bytes();
                text.starts_with("\\\\")
                    || (bytes.len() >= 3
                        && bytes[0].is_ascii_alphabetic()
                        && bytes[1] == b':'
                        && matches!(bytes[2], b'\\' | b'/'))
            }
        };
        other.then_some(current.other())
    }
}

/// An inactive Hot Path whose location belongs to another system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignHotPath {
    pub id: BookmarkId,
    /// A reserved numbered shortcut; using it only reports that the path is unavailable.
    pub slot: Option<u8>,
    pub system: PathSystem,
    /// The other system's location, as display text only.
    pub location: String,
    /// The stored record; only the settings codec interprets it.
    pub stored: Arc<str>,
}

/// Which setting a [`ForeignSetting`] keeps, and where it returns on export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForeignSettingKind {
    /// The default editor program and its arguments.
    Editor,
    /// An editor rule for files matching `pattern`, at this position in the rule list.
    EditorRule { position: usize, pattern: String },
    /// A custom command; its identity stays reserved.
    UserCommand {
        position: usize,
        id: UserCommandId,
        name: String,
    },
    /// A comparator rule for files matching `pattern`, at this position in the rule list.
    FileComparator { position: usize, pattern: String },
    /// The last Make File List output file.
    FileListOutput,
}

/// An editor, custom command, comparator or output file whose path belongs to another
/// system. It is inactive here: matching files and commands behave as if it were absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignSetting {
    pub kind: ForeignSettingKind,
    pub system: PathSystem,
    /// The other system's path, as display text only.
    pub location: String,
    /// The stored record; only the settings codec interprets it.
    pub stored: Arc<str>,
}

impl ForeignSetting {
    /// Short user-facing description, without the location.
    pub fn description(&self) -> String {
        match &self.kind {
            ForeignSettingKind::Editor => "Editor".into(),
            ForeignSettingKind::EditorRule { pattern, .. } => format!("Editor for {pattern}"),
            ForeignSettingKind::UserCommand { name, .. } => format!("Custom command “{name}”"),
            ForeignSettingKind::FileComparator { pattern, .. } => {
                format!("Comparator for {pattern}")
            }
            ForeignSettingKind::FileListOutput => "Make File List output".into(),
        }
    }
}

/// The settings file holds one editor program and one Make File List output file,
/// so choosing a native one replaces the one kept from another system.
impl UserPreferences {
    pub fn drop_replaced_foreign_editor(&mut self) {
        if self.editor.executable.is_some() {
            self.drop_foreign(&ForeignSettingKind::Editor);
        }
    }

    pub fn drop_replaced_foreign_output(&mut self) {
        if self.file_list.path.is_some() {
            self.drop_foreign(&ForeignSettingKind::FileListOutput);
        }
    }

    fn drop_foreign(&mut self, kind: &ForeignSettingKind) {
        if self
            .foreign_settings
            .iter()
            .any(|setting| setting.kind == *kind)
        {
            Arc::make_mut(&mut self.foreign_settings).retain(|setting| setting.kind != *kind);
        }
    }
}

/// Hot paths and settings lists include their entries from another system in their
/// limits and identities, so an exported file stays valid on that system too.
pub(crate) fn validate_foreign(preferences: &UserPreferences) -> Result<(), &'static str> {
    let hot_paths = &preferences.foreign_hot_paths;
    if preferences.bookmarks.len() + hot_paths.len() > MAX_BOOKMARKS {
        return Err("At most 64 bookmarks can be saved.");
    }
    let mut ids: HashSet<_> = preferences.bookmarks.iter().map(|path| path.id).collect();
    let mut slots: HashSet<_> = preferences
        .bookmarks
        .iter()
        .filter_map(|path| path.slot)
        .collect();
    for path in hot_paths {
        if path.id.0 == 0 || !ids.insert(path.id) {
            return Err("Bookmark identities must be unique.");
        }
        if path
            .slot
            .is_some_and(|slot| !(1..=10).contains(&slot) || !slots.insert(slot))
        {
            return Err("Bookmark hot path slots must be unique and between 1 and 10.");
        }
        validate_record(path.system, &path.location, &path.stored)?;
    }
    let mut counts = [0usize; 5];
    let mut command_ids: HashSet<_> = preferences
        .user_commands
        .iter()
        .map(|command| &command.id)
        .collect();
    for setting in preferences.foreign_settings.iter() {
        validate_record(setting.system, &setting.location, &setting.stored)?;
        let (index, limit, position) = match &setting.kind {
            ForeignSettingKind::Editor => (0, 1, 0),
            ForeignSettingKind::FileListOutput => (1, 1, 0),
            ForeignSettingKind::EditorRule { position, .. } => (
                2,
                MAX_EDITOR_RULES - preferences.editor.rules.len().min(MAX_EDITOR_RULES),
                *position,
            ),
            ForeignSettingKind::UserCommand { position, id, .. } => {
                if !command_ids.insert(id) {
                    return Err("Custom Command identities must be unique.");
                }
                let native = preferences.user_commands.len().min(MAX_USER_COMMANDS);
                (3, MAX_USER_COMMANDS - native, *position)
            }
            ForeignSettingKind::FileComparator { position, .. } => {
                let native = preferences
                    .file_comparators
                    .rules
                    .len()
                    .min(MAX_FILE_COMPARATOR_RULES);
                (4, MAX_FILE_COMPARATOR_RULES - native, *position)
            }
        };
        counts[index] += 1;
        if counts[index] > limit || position >= MAX_USER_COMMANDS {
            return Err("Settings from another system exceed their list limits.");
        }
    }
    Ok(())
}

fn validate_record(system: PathSystem, location: &str, stored: &str) -> Result<(), &'static str> {
    if system == PathSystem::CURRENT
        || location.is_empty()
        || location.contains('\0')
        || stored.is_empty()
    {
        return Err("Settings from another system need that system's path and stored record.");
    }
    Ok(())
}

#[cfg(test)]
#[path = "foreign_settings_tests.rs"]
mod tests;
