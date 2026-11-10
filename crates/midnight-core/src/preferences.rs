//! User-visible preferences; native storage belongs to backend platform services.
use crate::{
    Bookmark, CommandHistoryEntry, FsError, MAX_BOOKMARKS, ShortcutBinding, ShortcutMap,
    validate_command_history,
};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

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
    pub networks: BTreeMap<usize, crate::NetworkAddress>,
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
        for (index, address) in &self.networks {
            if *index >= self.paths.len() || self.archives.contains_key(index) {
                return Err("Network tab address must refer to one saved non-archive tab.");
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
    CarbonGreen = 8,
    TokyoNight = 9,
    OneDark = 10,
    RosePine = 11,
    NortonCommander = 12,
    AmberTerminal = 13,
    VerdantLight = 14,
    CatppuccinLatte = 15,
    RosePineDawn = 16,
    PaperContrast = 17,
    SandstoneContrast = 18,
    DeepOcean = 19,
    GraphiteContrast = 20,
    HighContrastDark = 21,
    HighContrastLight = 22,
}

impl PreferenceTheme {
    /// Stable picker order. Replaced names retain a storage migration alias.
    pub const ALL: [Self; 23] = [
        Self::Blue,
        Self::Dark,
        Self::Light,
        Self::Dracula,
        Self::Nord,
        Self::GruvboxDark,
        Self::CatppuccinMocha,
        Self::SolarizedLight,
        Self::CarbonGreen,
        Self::TokyoNight,
        Self::OneDark,
        Self::RosePine,
        Self::NortonCommander,
        Self::AmberTerminal,
        Self::VerdantLight,
        Self::CatppuccinLatte,
        Self::RosePineDawn,
        Self::PaperContrast,
        Self::SandstoneContrast,
        Self::DeepOcean,
        Self::GraphiteContrast,
        Self::HighContrastDark,
        Self::HighContrastLight,
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
            Self::CarbonGreen => "carbon-green",
            Self::TokyoNight => "tokyo-night",
            Self::OneDark => "one-dark",
            Self::RosePine => "rose-pine",
            Self::NortonCommander => "norton-commander",
            Self::AmberTerminal => "amber-terminal",
            Self::VerdantLight => "verdant-light",
            Self::CatppuccinLatte => "catppuccin-latte",
            Self::RosePineDawn => "rose-pine-dawn",
            Self::PaperContrast => "paper-contrast",
            Self::SandstoneContrast => "sandstone-contrast",
            Self::DeepOcean => "deep-ocean",
            Self::GraphiteContrast => "graphite-contrast",
            Self::HighContrastDark => "high-contrast-dark",
            Self::HighContrastLight => "high-contrast-light",
        }
    }

    pub fn from_storage_name(name: &str) -> Option<Self> {
        if name == "dos-commander" {
            return Some(Self::AmberTerminal);
        }
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
    pub user_commands: Arc<Vec<crate::UserCommand>>,
    /// Accepted Make File List templates and output choices; draft edits are not saved.
    pub file_list: crate::FileListPreferences,
    /// Algorithm choices for new checksum windows; recursion is deliberately not persisted.
    pub checksum_algorithms: crate::ChecksumAlgorithms,
    pub window_geometry: Option<crate::WindowGeometry>,
    /// Normal left panel width as 1..=9999 basis points; panel zoom is transient.
    pub panel_split_basis_points: u16,
    /// Per-extension archive browsing choices; absent keys use the catalog default.
    pub archive_association_overrides: BTreeMap<String, bool>,
    /// User-defined literal suffixes mapped to an installed archive decoder format.
    pub archive_custom_extensions: BTreeMap<String, crate::ArchiveFormat>,
    pub disabled_viewer_plugins: Vec<String>,
    pub viewer_associations: Arc<crate::ViewerAssociations>,
    pub viewer: crate::ViewerPreferences,
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
    /// Full SHA-256 for new Copy/Move jobs; false checks length and three64 KiB samples.
    pub copy_checksum_verification: bool,
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
    /// Square thumbnail artwork edge in logical pixels; decoding resolves physical pixels separately.
    pub thumbnail_size: u16,
    /// Only deviations from the application shortcut catalog are persisted.
    pub shortcuts: Vec<ShortcutBinding>,
    pub panel_filters: [crate::PanelFilter; 2],
    /// Accepted edits replace this bounded shared collection atomically.
    pub filename_colors: Arc<Vec<crate::FilenameColorRule>>,
}

pub const FONT_SIZE_MIN: u16 = 8;
pub const FONT_SIZE_MAX: u16 = 32;
pub const MAX_FONT_FAMILY_BYTES: usize = 512;
pub const DEFAULT_PANEL_SPLIT_BASIS_POINTS: u16 = 5000;
pub const DEFAULT_THUMBNAIL_SIZE: u16 = 72;
pub const THUMBNAIL_SIZES: [u16; 7] = [48, 64, 72, 96, 128, 192, 256];

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            user_commands: Arc::new(Vec::new()),
            file_list: crate::FileListPreferences::default(),
            checksum_algorithms: crate::ChecksumAlgorithms::default(),
            window_geometry: None,
            panel_split_basis_points: DEFAULT_PANEL_SPLIT_BASIS_POINTS,
            archive_association_overrides: BTreeMap::new(),
            archive_custom_extensions: BTreeMap::new(),
            disabled_viewer_plugins: Vec::new(),
            viewer_associations: Arc::default(),
            viewer: crate::ViewerPreferences::default(),
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
            copy_checksum_verification: true,
            command_history: Vec::new(),
            bookmarks: Vec::new(),
            tabs: Default::default(),
            panel_view_modes: Default::default(),
            thumbnails_enabled: true,
            thumbnails_network: false,
            thumbnail_size: DEFAULT_THUMBNAIL_SIZE,
            shortcuts: Vec::new(),
            panel_filters: Default::default(),
            filename_colors: crate::default_filename_colors().into(),
        }
    }
}

impl UserPreferences {
    /// Whether a known canonical extension opens inside the file manager.
    pub fn archive_association_enabled(&self, extension: &str) -> bool {
        crate::ARCHIVE_ASSOCIATIONS
            .iter()
            .find(|association| association.extension == extension)
            .is_some_and(|association| {
                self.archive_association_overrides
                    .get(extension)
                    .copied()
                    .unwrap_or(association.enabled_by_default)
            })
    }

    /// Merge legacy duplicate native paths in saved order without filesystem IO.
    /// Keep the first raw path, identity and name, recovering a later slot only
    /// when the retained entry has none. Path equality never folds case or text.
    pub fn deduplicate_bookmarks(&mut self) -> bool {
        let mut unique: Vec<Bookmark> = Vec::with_capacity(self.bookmarks.len());
        let mut changed = false;
        for bookmark in self.bookmarks.drain(..) {
            if let Some(existing) = unique.iter_mut().find(|old| old.same_location(&bookmark)) {
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
        ShortcutMap::with_user_commands(&self.shortcuts, &self.user_commands).map_err(
            |message| {
                FsError::from_io(
                    "validate preferences",
                    std::path::Path::new(""),
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
                )
            },
        )?;
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
        if let Err(error) = self.file_list.validate() {
            message = Some(error);
        }
        if !THUMBNAIL_SIZES.contains(&self.thumbnail_size) {
            message =
                Some("Thumbnail size must be 48, 64, 72, 96, 128, 192 or 256 logical pixels.");
        }
        if !(1..10_000).contains(&self.panel_split_basis_points) {
            message = Some("Panel split must be between 1 and 9999 basis points.");
        }
        if let Err(error) = self.viewer.validate() {
            message = Some(error);
        }
        if let Err(error) =
            crate::validate_archive_custom_extensions(&self.archive_custom_extensions)
        {
            message = Some(error);
        }
        if let Some(geometry) = &self.window_geometry
            && let Err(error) = geometry.validate()
        {
            message = Some(error);
        }
        if self.archive_association_overrides.keys().any(|extension| {
            !crate::ARCHIVE_ASSOCIATIONS
                .iter()
                .any(|association| association.extension == extension)
        }) {
            message = Some(
                "Archive associations must use known lowercase extensions without a leading dot.",
            );
        }
        if let Err(error) = self.viewer_associations.validate() {
            message = Some(error);
        }
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
        if let Err(error) = crate::validate_filename_colors(&self.filename_colors) {
            message = Some(error);
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
        let mut locations: Vec<&Bookmark> = Vec::new();
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
            if locations.iter().any(|old| old.same_location(bookmark)) {
                message = Some("Bookmark paths must be unique.");
                break;
            }
            locations.push(bookmark);
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
