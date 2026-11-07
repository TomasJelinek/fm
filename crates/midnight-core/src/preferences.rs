//! User-visible preferences; native storage belongs to backend platform services.
use crate::{CommandHistoryEntry, FsError, ShortcutBinding, ShortcutMap, validate_command_history};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
};

/// Stable callback identity; names and displayed paths are never identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BookmarkId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

pub const MAX_TABS_PER_PANE: usize = 32;

/// Native return locations and typed provider addresses survive restart. Provider
/// members are never encoded as native paths; runtime IDs and rows are not saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneTabPreferences {
    pub paths: Vec<PathBuf>,
    pub active: usize,
    pub archives: BTreeMap<usize, crate::ArchiveAddress>,
}

impl PaneTabPreferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.paths.len() > MAX_TABS_PER_PANE {
            return Err("At most 32 tabs can be saved in each panel.");
        }
        for (index, address) in &self.archives {
            if *index >= self.paths.len() {
                return Err("Archive tab address must refer to a saved tab.");
            }
            address.validate()?;
        }
        if (self.paths.is_empty() && self.active != 0)
            || (!self.paths.is_empty() && self.active >= self.paths.len())
        {
            return Err("The active tab must refer to a saved location.");
        }
        if self
            .paths
            .iter()
            .any(|path| !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0))
        {
            return Err("Tab paths must be absolute native paths without NUL.");
        }
        Ok(())
    }
}

/// Native locations are stored losslessly. Future providers need a separate
/// location representation rather than interpreting these paths as URLs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub name: String,
    pub path: PathBuf,
    /// Salamander hot path: 1 through 9, and 10 for the zero key.
    pub slot: Option<u8>,
}

pub const MAX_BOOKMARKS: usize = 64;
pub const MAX_BOOKMARK_NAME_BYTES: usize = 512;

impl Bookmark {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.0 == 0 {
            return Err("Bookmark identity must be nonzero.");
        }
        if self.name.trim().is_empty()
            || self.name.len() > MAX_BOOKMARK_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err("Bookmark name must contain 1 to 512 UTF-8 bytes without controls.");
        }
        if !self.path.is_absolute() || self.path.as_os_str().as_encoded_bytes().contains(&0) {
            return Err("Bookmark path must be an absolute native path without NUL.");
        }
        if self.slot.is_some_and(|slot| !(1..=10).contains(&slot)) {
            return Err("Bookmark hot path slot must be between 1 and 10.");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum PreferenceTheme {
    #[default]
    Blue = 0,
    Dark = 1,
    Light = 2,
    Dracula = 3,
    Nord = 4,
    GruvboxDark = 5,
    CatppuccinMocha = 6,
    SolarizedLight = 7,
}

impl PreferenceTheme {
    /// Stable picker order. Existing indices and persisted names must not change.
    pub const ALL: [Self; 8] = [
        Self::Blue,
        Self::Dark,
        Self::Light,
        Self::Dracula,
        Self::Nord,
        Self::GruvboxDark,
        Self::CatppuccinMocha,
        Self::SolarizedLight,
    ];

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub fn from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }

    pub const fn storage_name(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::GruvboxDark => "gruvbox-dark",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::SolarizedLight => "solarized-light",
        }
    }

    pub fn from_storage_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|theme| theme.storage_name() == name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiDensity {
    #[default]
    Desktop,
    Touch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowsShell {
    #[default]
    PowerShell,
    Cmd,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SizeDisplay {
    #[default]
    Readable,
    Bytes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelViewMode {
    #[default]
    Detailed,
    Brief,
    Thumbnails,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserPreferences {
    pub disabled_viewer_plugins: Vec<String>,
    pub editor: crate::EditorPreferences,
    pub theme: PreferenceTheme,
    pub density: UiDensity,
    pub windows_shell: WindowsShell,
    /// Empty selects the system default. This is a Unicode name, never a path.
    pub font_family: String,
    pub desktop_font_size: u16,
    pub touch_font_size: u16,
    /// Show only matching names during panel type-to-search instead of jumping.
    pub quick_search_filter: bool,
    pub size_display: SizeDisplay,
    pub remember_command_history: bool,
    /// Newest first; exact command text is unique even across different shells.
    pub command_history: Vec<CommandHistoryEntry>,
    pub bookmarks: Vec<Bookmark>,
    /// Left and right panel locations, in their displayed order.
    pub tabs: [PaneTabPreferences; 2],
    pub panel_view_modes: [PanelViewMode; 2],
    /// Preview decoding is restricted to installed helper processes; disabling cancels visible work.
    pub thumbnails_enabled: bool,
    /// Network sources require an explicit user choice independently of the master enable switch.
    pub thumbnails_network: bool,
    /// Only deviations from the application shortcut catalog are persisted.
    pub shortcuts: Vec<ShortcutBinding>,
    pub panel_filters: [crate::PanelFilter; 2],
}

pub const FONT_SIZE_MIN: u16 = 8;
pub const FONT_SIZE_MAX: u16 = 32;
pub const MAX_FONT_FAMILY_BYTES: usize = 512;

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            disabled_viewer_plugins: Vec::new(),
            editor: crate::EditorPreferences::default(),
            theme: PreferenceTheme::default(),
            density: UiDensity::default(),
            windows_shell: WindowsShell::default(),
            font_family: String::new(),
            desktop_font_size: 11,
            touch_font_size: 15,
            quick_search_filter: false,
            size_display: SizeDisplay::default(),
            remember_command_history: true,
            command_history: Vec::new(),
            bookmarks: Vec::new(),
            tabs: Default::default(),
            panel_view_modes: Default::default(),
            thumbnails_enabled: true,
            thumbnails_network: false,
            shortcuts: Vec::new(),
            panel_filters: Default::default(),
        }
    }
}

impl UserPreferences {
    /// Merge legacy duplicate native paths in saved order without filesystem IO.
    /// Keep the first raw path, identity and name, recovering a later slot only
    /// when the retained entry has none. Path equality never folds case or text.
    pub fn deduplicate_bookmarks(&mut self) -> bool {
        let mut unique: Vec<Bookmark> = Vec::with_capacity(self.bookmarks.len());
        let mut changed = false;
        for bookmark in self.bookmarks.drain(..) {
            if let Some(existing) = unique.iter_mut().find(|old| old.path == bookmark.path) {
                if existing.slot.is_none() {
                    existing.slot = bookmark.slot;
                }
                changed = true;
            } else {
                unique.push(bookmark);
            }
        }
        self.bookmarks = unique;
        changed
    }

    pub fn validate(&self) -> Result<(), FsError> {
        ShortcutMap::new(&self.shortcuts).map_err(|message| {
            FsError::from_io(
                "validate preferences",
                std::path::Path::new(""),
                std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
            )
        })?;
        let message = if self.font_family.len() > MAX_FONT_FAMILY_BYTES {
            Some("Font family exceeds 512 UTF-8 bytes.")
        } else if self.font_family.chars().any(char::is_control) {
            Some("Font family cannot contain control characters.")
        } else if !(FONT_SIZE_MIN..=FONT_SIZE_MAX).contains(&self.desktop_font_size)
            || !(FONT_SIZE_MIN..=FONT_SIZE_MAX).contains(&self.touch_font_size)
        {
            Some("Font size must be between 8 and 32 logical pixels.")
        } else {
            None
        };
        let mut message = message;
        if let Err(error) = crate::validate_disabled_viewers(&self.disabled_viewer_plugins) {
            message = Some(error);
        }
        if let Err(error) = self.editor.validate() {
            message = Some(error);
        }
        for filter in &self.panel_filters {
            if let Err(error) = filter.validate_bounds() {
                message = Some(error);
            }
        }
        for tabs in &self.tabs {
            if let Err(error) = tabs.validate() {
                message = Some(error);
                break;
            }
        }
        if let Err(error) = validate_command_history(&self.command_history) {
            message = Some(error);
        }
        if self.bookmarks.len() > MAX_BOOKMARKS {
            message = Some("At most 64 bookmarks can be saved.");
        }
        let mut ids = HashSet::new();
        let mut slots = HashSet::new();
        let mut paths = HashSet::new();
        for bookmark in &self.bookmarks {
            if let Err(error) = bookmark.validate() {
                message = Some(error);
                break;
            }
            if !ids.insert(bookmark.id) {
                message = Some("Bookmark identities must be unique.");
                break;
            }
            if bookmark.slot.is_some_and(|slot| !slots.insert(slot)) {
                message = Some("Bookmark hot path slots must be unique.");
                break;
            }
            if !paths.insert(&bookmark.path) {
                message = Some("Bookmark paths must be unique.");
                break;
            }
        }
        match message {
            Some(message) => Err(FsError::from_io(
                "validate preferences",
                std::path::Path::new(""),
                std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
            )),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct PreferencesRevision(pub u64);

pub trait SettingsService: Send + Sync + 'static {
    /// Enumerate installed family names on a backend worker, never on the UI.
    fn font_families(&self) -> Result<Vec<String>, FsError> {
        Ok(Vec::new())
    }
    /// Load bounded, versioned settings; absence means the defaults.
    fn load_preferences(&self) -> Result<UserPreferences, FsError>;
    /// Publish settings atomically; no truncation of the existing file on failure.
    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError>;
}
