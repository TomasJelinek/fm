//! Portable preferences codec and atomic publication; native adapters own path/security policy.
use midnight_core::{
    Bookmark, BookmarkId, CommandHistoryEntry, PaneTabPreferences, PanelViewMode, PreferenceTheme,
    ShellFlavor, ShortcutBinding, SizeDisplay, UiDensity, UserPreferences, WindowsShell,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

mod archive_associations;
use archive_associations::StoredArchiveAssociations;
mod native_path;
use native_path::NativePath;
#[cfg(test)]
mod archives_tests;
#[cfg(test)]
mod previews_tests;
#[cfg(test)]
mod shortcuts_tests;

/// An aggregate storage bound, never a native path length restriction.
pub const MAX_PREFERENCES_BYTES: usize = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPreferences {
    version: u32,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    archive_association_overrides: Option<StoredArchiveAssociations>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    previews: Option<StoredPreviews>,
    #[serde(default, deserialize_with = "present")]
    disabled_viewer_plugins: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    editor: Option<StoredEditor>,
    theme: String,
    density: String,
    #[serde(default = "default_windows_shell")]
    windows_shell: String,
    #[serde(default, deserialize_with = "present")]
    font_family: Option<String>,
    #[serde(default, deserialize_with = "present")]
    desktop_font_size: Option<u16>,
    #[serde(default, deserialize_with = "present")]
    touch_font_size: Option<u16>,
    #[serde(default, deserialize_with = "present")]
    bookmarks: Option<Vec<StoredBookmark>>,
    #[serde(default, deserialize_with = "present")]
    quick_search_filter: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    size_display: Option<String>,
    #[serde(default, deserialize_with = "present")]
    remember_command_history: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    command_history: Option<Vec<StoredCommandHistory>>,
    #[serde(default, deserialize_with = "present")]
    tabs: Option<[StoredPaneTabs; 2]>,
    #[serde(default, deserialize_with = "present")]
    panel_view_modes: Option<[String; 2]>,
    #[serde(default, deserialize_with = "present")]
    shortcuts: Option<Vec<StoredShortcut>>,
    #[serde(default, deserialize_with = "present")]
    panel_filters: Option<[StoredPanelFilter; 2]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPreviews {
    enabled: bool,
    network: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredEditor {
    executable: Option<NativePath>,
    arguments: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPanelFilter {
    enabled: bool,
    mode: String,
    expression: String,
    case_sensitive: bool,
}

impl StoredPanelFilter {
    fn into_filter(self) -> io::Result<midnight_core::PanelFilter> {
        let filter = midnight_core::PanelFilter {
            enabled: self.enabled,
            mode: match self.mode.as_str() {
                "masks" => midnight_core::FilterMode::Masks,
                "regex" => midnight_core::FilterMode::Regex,
                _ => return Err(invalid("unknown panel filter mode")),
            },
            expression: self.expression,
            case_sensitive: self.case_sensitive,
        };
        crate::filter::CompiledFilter::new(&filter).map_err(invalid)?;
        Ok(filter)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredShortcut {
    action: String,
    keys: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPaneTabs {
    paths: Vec<NativePath>,
    active: usize,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    archives: std::collections::BTreeMap<usize, StoredArchiveAddress>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredArchiveAddress {
    version: u32,
    provider: String,
    container: NativePath,
    source_version: Vec<u8>,
    components: Vec<Vec<u8>>,
}
impl StoredArchiveAddress {
    fn into_address(self) -> io::Result<midnight_core::ArchiveAddress> {
        if self.version != 1 {
            return Err(invalid("unsupported archive address version"));
        }
        let address = midnight_core::ArchiveAddress {
            provider: self.provider,
            container: self.container.into_path()?,
            source_version: self.source_version,
            components: self.components,
        };
        address.validate().map_err(invalid)?;
        Ok(address)
    }
    fn from_address(address: midnight_core::ArchiveAddress) -> Self {
        Self {
            version: 1,
            provider: address.provider,
            container: NativePath::from_path(address.container),
            source_version: address.source_version,
            components: address.components,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCommandHistory {
    command: String,
    shell: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredBookmark {
    id: u64,
    name: String,
    path: NativePath,
    slot: Option<u8>,
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

fn default_windows_shell() -> String {
    "powershell".into()
}

fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn decode_preferences(bytes: &[u8]) -> io::Result<UserPreferences> {
    if bytes.len() > MAX_PREFERENCES_BYTES {
        return Err(invalid("preferences exceed 4 MiB"));
    }
    let stored: StoredPreferences = serde_json::from_slice(bytes).map_err(invalid)?;
    if !(1..=13).contains(&stored.version) {
        return Err(invalid("unsupported preferences version"));
    }
    if stored.version < 11
        && stored
            .tabs
            .as_ref()
            .is_some_and(|tabs| tabs.iter().any(|tab| !tab.archives.is_empty()))
    {
        return Err(invalid("archive addresses require preferences version 11"));
    }
    let (thumbnails_enabled, thumbnails_network) = match (stored.version, stored.previews) {
        (1..=11, None) => (true, false),
        (12..=13, Some(previews)) => (previews.enabled, previews.network),
        _ => return Err(invalid("missing or unexpected preview preferences")),
    };
    let theme = PreferenceTheme::from_storage_name(&stored.theme)
        .ok_or_else(|| invalid("unknown preferences theme"))?;
    let density = match stored.density.as_str() {
        "desktop" => UiDensity::Desktop,
        "touch" => UiDensity::Touch,
        _ => return Err(invalid("unknown preferences density")),
    };
    let windows_shell = match stored.windows_shell.as_str() {
        "powershell" => WindowsShell::PowerShell,
        "cmd" => WindowsShell::Cmd,
        _ => return Err(invalid("unknown Windows shell preference")),
    };
    let defaults = UserPreferences::default();
    let (font_family, desktop_font_size, touch_font_size) = match stored.version {
        1 if stored.font_family.is_none()
            && stored.desktop_font_size.is_none()
            && stored.touch_font_size.is_none() =>
        {
            (
                defaults.font_family,
                defaults.desktop_font_size,
                defaults.touch_font_size,
            )
        }
        2..=13 => (
            stored
                .font_family
                .ok_or_else(|| invalid("missing font family"))?,
            stored
                .desktop_font_size
                .ok_or_else(|| invalid("missing desktop font size"))?,
            stored
                .touch_font_size
                .ok_or_else(|| invalid("missing touch font size"))?,
        ),
        _ => return Err(invalid("font preferences require version 2")),
    };
    let bookmarks = match (stored.version, stored.bookmarks) {
        (1 | 2, None) => Vec::new(),
        (3..=13, Some(bookmarks)) => bookmarks
            .into_iter()
            .map(|bookmark| {
                Ok(Bookmark {
                    id: BookmarkId(bookmark.id),
                    name: bookmark.name,
                    path: bookmark.path.into_path()?,
                    slot: bookmark.slot,
                })
            })
            .collect::<io::Result<Vec<_>>>()?,
        (3..=13, None) => return Err(invalid("missing bookmarks")),
        _ => return Err(invalid("bookmarks require preferences version 3")),
    };
    // Validate legacy records before merging duplicate paths so migration cannot
    // hide malformed entries, repeated identities or conflicting slot owners.
    let mut ids = std::collections::HashSet::new();
    let mut slots = std::collections::HashSet::new();
    for bookmark in &bookmarks {
        bookmark.validate().map_err(invalid)?;
        if !ids.insert(bookmark.id) || bookmark.slot.is_some_and(|slot| !slots.insert(slot)) {
            return Err(invalid("bookmark identities and slots must be unique"));
        }
    }
    let quick_search_filter = match (stored.version, stored.quick_search_filter) {
        (1..=3, None) => false,
        (4..=13, Some(enabled)) => enabled,
        (4..=13, None) => return Err(invalid("missing quick search filter preference")),
        _ => {
            return Err(invalid(
                "quick search filter requires preferences version 4",
            ));
        }
    };
    let (size_display, remember_command_history, command_history) = match (
        stored.version,
        stored.size_display,
        stored.remember_command_history,
        stored.command_history,
    ) {
        (1..=4, None, None, None) => (SizeDisplay::Readable, true, Vec::new()),
        (5..=13, Some(size), Some(remember), Some(history)) => {
            let size = match size.as_str() {
                "readable" => SizeDisplay::Readable,
                "bytes" => SizeDisplay::Bytes,
                _ => return Err(invalid("unknown file size display preference")),
            };
            let history = history
                .into_iter()
                .map(|entry| {
                    let shell = match entry.shell.as_str() {
                        "powershell" => ShellFlavor::PowerShell,
                        "cmd" => ShellFlavor::Cmd,
                        "posix" => ShellFlavor::Posix,
                        _ => return Err(invalid("unknown command history shell")),
                    };
                    Ok(CommandHistoryEntry {
                        command: entry.command,
                        shell,
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            (size, remember, history)
        }
        (5..=13, _, _, _) => return Err(invalid("missing panel or command history preferences")),
        _ => {
            return Err(invalid(
                "panel and command history preferences require version 5",
            ));
        }
    };
    let tabs = match (stored.version, stored.tabs) {
        (1..=5, None) => Default::default(),
        (6..=13, Some(tabs)) => {
            let convert = |tabs: StoredPaneTabs| -> io::Result<PaneTabPreferences> {
                Ok(PaneTabPreferences {
                    archives: tabs
                        .archives
                        .into_iter()
                        .map(|(index, address)| Ok((index, address.into_address()?)))
                        .collect::<io::Result<_>>()?,
                    paths: tabs
                        .paths
                        .into_iter()
                        .map(NativePath::into_path)
                        .collect::<io::Result<Vec<_>>>()?,
                    active: tabs.active,
                })
            };
            let [left, right] = tabs;
            [convert(left)?, convert(right)?]
        }
        (6..=13, None) => return Err(invalid("missing tab sessions")),
        _ => return Err(invalid("tab sessions require preferences version 6")),
    };
    let panel_view_modes = match (stored.version, stored.panel_view_modes) {
        (1..=5, None) => Default::default(),
        (6..=13, Some(modes)) => {
            let parse = |mode: String| match mode.as_str() {
                "detailed" => Ok(PanelViewMode::Detailed),
                "brief" => Ok(PanelViewMode::Brief),
                "thumbnails" => Ok(PanelViewMode::Thumbnails),
                _ => Err(invalid("unknown panel view mode")),
            };
            let [left, right] = modes;
            [parse(left)?, parse(right)?]
        }
        (6..=13, None) => return Err(invalid("missing panel view modes")),
        _ => return Err(invalid("panel view modes require preferences version 6")),
    };
    let shortcuts = match (stored.version, stored.shortcuts) {
        (1..=6, None) => Vec::new(),
        (7..=13, Some(bindings)) => bindings
            .into_iter()
            .map(|binding| ShortcutBinding {
                action: binding.action,
                keys: binding.keys,
            })
            .collect(),
        (7..=13, None) => return Err(invalid("missing shortcut overrides")),
        _ => return Err(invalid("shortcut overrides require preferences version 7")),
    };
    let panel_filters = match (stored.version, stored.panel_filters) {
        (1..=7, None) => Default::default(),
        (8..=13, Some([left, right])) => [left.into_filter()?, right.into_filter()?],
        (8..=13, None) => return Err(invalid("missing panel filters")),
        _ => return Err(invalid("panel filters require preferences version 8")),
    };
    let editor = match (stored.version, stored.editor) {
        (1..=8, None) => Default::default(),
        (9..=13, Some(editor)) => midnight_core::EditorPreferences {
            executable: editor.executable.map(NativePath::into_path).transpose()?,
            arguments: editor.arguments,
        },
        (9..=13, None) => return Err(invalid("missing editor preferences")),
        _ => return Err(invalid("editor preferences require version 9")),
    };
    let mut preferences = UserPreferences {
        archive_association_overrides: match (stored.version, stored.archive_association_overrides)
        {
            (1..=12, None) => Default::default(),
            (13, Some(associations)) => associations.0,
            _ => {
                return Err(invalid(
                    "missing or unexpected archive association preferences",
                ));
            }
        },
        disabled_viewer_plugins: match (stored.version, stored.disabled_viewer_plugins) {
            (1..=9, None) => Vec::new(),
            (10..=13, Some(ids)) => ids,
            _ => return Err(invalid("missing or unexpected viewer plugin preferences")),
        },
        editor,
        theme,
        density,
        windows_shell,
        font_family,
        desktop_font_size,
        touch_font_size,
        quick_search_filter,
        size_display,
        remember_command_history,
        command_history,
        bookmarks,
        tabs,
        panel_view_modes,
        thumbnails_enabled,
        thumbnails_network,
        shortcuts,
        panel_filters,
    };
    preferences.deduplicate_bookmarks();
    preferences.validate().map_err(invalid)?;
    Ok(preferences)
}

pub fn encode_preferences(preferences: UserPreferences) -> io::Result<Vec<u8>> {
    preferences.validate().map_err(invalid)?;
    for filter in &preferences.panel_filters {
        crate::filter::CompiledFilter::new(filter).map_err(invalid)?;
    }
    let archive_choices = !preferences.archive_association_overrides.is_empty();
    let store_previews =
        archive_choices || !preferences.thumbnails_enabled || preferences.thumbnails_network;
    let stored = StoredPreferences {
        archive_association_overrides: archive_choices.then_some(StoredArchiveAssociations(
            preferences.archive_association_overrides,
        )),
        version: if archive_choices {
            13
        } else if store_previews {
            12
        } else if preferences
            .tabs
            .iter()
            .any(|tabs| !tabs.archives.is_empty())
        {
            11
        } else {
            10
        },
        previews: store_previews.then_some(StoredPreviews {
            enabled: preferences.thumbnails_enabled,
            network: preferences.thumbnails_network,
        }),
        disabled_viewer_plugins: Some(preferences.disabled_viewer_plugins),
        editor: Some(StoredEditor {
            executable: preferences.editor.executable.map(NativePath::from_path),
            arguments: preferences.editor.arguments,
        }),
        panel_filters: Some(preferences.panel_filters.map(|filter| {
            StoredPanelFilter {
                enabled: filter.enabled,
                mode: match filter.mode {
                    midnight_core::FilterMode::Masks => "masks",
                    midnight_core::FilterMode::Regex => "regex",
                }
                .into(),
                expression: filter.expression,
                case_sensitive: filter.case_sensitive,
            }
        })),
        shortcuts: Some(
            preferences
                .shortcuts
                .into_iter()
                .map(|binding| StoredShortcut {
                    action: binding.action,
                    keys: binding.keys,
                })
                .collect(),
        ),
        panel_view_modes: Some(preferences.panel_view_modes.map(|mode| {
            match mode {
                PanelViewMode::Detailed => "detailed",
                PanelViewMode::Brief => "brief",
                PanelViewMode::Thumbnails => "thumbnails",
            }
            .to_owned()
        })),
        tabs: Some(preferences.tabs.map(|tabs| {
            StoredPaneTabs {
                archives: tabs
                    .archives
                    .into_iter()
                    .map(|(index, address)| (index, StoredArchiveAddress::from_address(address)))
                    .collect(),
                paths: tabs.paths.into_iter().map(NativePath::from_path).collect(),
                active: tabs.active,
            }
        })),
        quick_search_filter: Some(preferences.quick_search_filter),
        size_display: Some(
            match preferences.size_display {
                SizeDisplay::Readable => "readable",
                SizeDisplay::Bytes => "bytes",
            }
            .into(),
        ),
        remember_command_history: Some(preferences.remember_command_history),
        command_history: Some(if preferences.remember_command_history {
            preferences
                .command_history
                .into_iter()
                .map(|entry| StoredCommandHistory {
                    command: entry.command,
                    shell: match entry.shell {
                        ShellFlavor::PowerShell => "powershell",
                        ShellFlavor::Cmd => "cmd",
                        ShellFlavor::Posix => "posix",
                    }
                    .into(),
                })
                .collect()
        } else {
            Vec::new()
        }),
        bookmarks: Some(
            preferences
                .bookmarks
                .into_iter()
                .map(|bookmark| StoredBookmark {
                    id: bookmark.id.0,
                    name: bookmark.name,
                    path: NativePath::from_path(bookmark.path),
                    slot: bookmark.slot,
                })
                .collect(),
        ),
        font_family: Some(preferences.font_family),
        desktop_font_size: Some(preferences.desktop_font_size),
        touch_font_size: Some(preferences.touch_font_size),
        theme: preferences.theme.storage_name().into(),
        windows_shell: match preferences.windows_shell {
            WindowsShell::PowerShell => "powershell",
            WindowsShell::Cmd => "cmd",
        }
        .into(),
        density: match preferences.density {
            UiDensity::Desktop => "desktop",
            UiDensity::Touch => "touch",
        }
        .into(),
    };
    let mut bytes = serde_json::to_vec(&stored).map_err(invalid)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_PREFERENCES_BYTES {
        return Err(invalid("preferences exceed 4 MiB"));
    }
    Ok(bytes)
}

pub fn read_preferences(file: File) -> io::Result<UserPreferences> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(invalid("preferences are not a regular file"));
    }
    if metadata.len() > MAX_PREFERENCES_BYTES as u64 {
        return Err(invalid("preferences exceed 4 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PREFERENCES_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    decode_preferences(&bytes)
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TemporaryFile {
    path: PathBuf,
    file: Option<File>,
    published: bool,
}
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// `create_exclusive` must atomically create a fresh file or return AlreadyExists.
pub fn publish_atomic(
    path: &Path,
    data: &[u8],
    mut create_exclusive: impl FnMut(&Path) -> io::Result<File>,
    mut validate_destination: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    if data.len() > MAX_PREFERENCES_BYTES {
        return Err(invalid("preferences exceed 4 MiB"));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "preferences need a parent directory",
            )
        })?;
    validate_destination(path)?;
    let mut temporary = None;
    for _ in 0..16 {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".midnight-preferences-{}-{sequence}.tmp",
            std::process::id()
        ));
        match create_exclusive(&candidate) {
            Ok(file) => {
                temporary = Some(TemporaryFile {
                    path: candidate,
                    file: Some(file),
                    published: false,
                });
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let mut temporary = temporary.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "preferences temporary-name collision limit reached",
        )
    })?;
    let file = temporary
        .file
        .as_mut()
        .expect("new temporary owns an open file");
    file.write_all(data)?;
    file.flush()?;
    file.sync_all()?;
    drop(temporary.file.take());
    validate_destination(path)?;
    // Same-directory rename atomically replaces an existing regular destination;
    // no existing file is opened with truncation or deleted before publication.
    fs::rename(&temporary.path, path)?;
    temporary.published = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{PreferenceTheme, UiDensity};
    use std::fs::{self, OpenOptions};

    #[test]
    fn stored_duplicate_hot_paths_merge_without_losing_settings_or_native_path() {
        let path = std::env::temp_dir()
            .join("資料 e\u{301} 🧪")
            .join("x".repeat(70_000));
        let mut stored: serde_json::Value = serde_json::from_slice(
            &encode_preferences(UserPreferences {
                theme: PreferenceTheme::Light,
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        stored["bookmarks"] = serde_json::json!([
            {"id":7,"name":"Original name","path":path,"slot":null},
            {"id":8,"name":"Duplicate name","path":path,"slot":3},
            {"id":9,"name":"Other","path":std::env::temp_dir().join("Other"),"slot":10}
        ]);
        let decoded = decode_preferences(&serde_json::to_vec(&stored).unwrap()).unwrap();
        assert_eq!(decoded.theme, PreferenceTheme::Light);
        assert_eq!(decoded.bookmarks.len(), 2);
        assert_eq!(
            decoded.bookmarks[0],
            Bookmark {
                id: BookmarkId(7),
                name: "Original name".into(),
                path,
                slot: Some(3)
            }
        );
        assert_eq!(decoded.bookmarks[1].id, BookmarkId(9));
        assert_eq!(decoded.bookmarks[1].slot, Some(10));
        assert_eq!(
            decode_preferences(&encode_preferences(decoded.clone()).unwrap()).unwrap(),
            decoded
        );
    }

    #[test]
    fn version_three_requires_a_bookmark_array() {
        let json = concat!(
            r#"{"version":3,"theme":"blue","density":"desktop","font_family":"","#,
            r#""desktop_font_size":11,"touch_font_size":15,"bookmarks":[]}"#,
        );
        let preferences = decode_preferences(json.as_bytes()).unwrap();
        let encoded = encode_preferences(preferences).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&encoded).unwrap()["version"],
            10
        );
    }

    #[test]
    fn unicode_bookmarks_roundtrip_readably_without_normalization_or_path_length_cap() {
        let path = std::env::temp_dir()
            .join("資料 e\u{301} 🧪")
            .join("x".repeat(70_000));
        let preferences = UserPreferences {
            bookmarks: vec![Bookmark {
                id: BookmarkId(u64::MAX),
                name: "資料 e\u{301} 🧪".into(),
                path: path.clone(),
                slot: Some(10),
            }],
            ..UserPreferences::default()
        };
        let encoded = encode_preferences(preferences.clone()).unwrap();
        assert!(encoded.len() > 64 * 1024);
        let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(stored["bookmarks"][0]["path"], path.to_str().unwrap());
        assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
    }

    #[test]
    fn bookmark_schema_rejects_invalid_identity_name_slot_and_path() {
        let path = std::env::temp_dir().join("bookmark");
        let valid = serde_json::json!({
            "version":3,"theme":"blue","density":"desktop","font_family":"",
            "desktop_font_size":11,"touch_font_size":15,
            "bookmarks":[{"id":1,"name":"Test","path":path,"slot":1}]
        });
        assert!(decode_preferences(&serde_json::to_vec(&valid).unwrap()).is_ok());
        for (field, value) in [
            ("id", serde_json::json!(0)),
            ("name", serde_json::json!("bad\u{0000}name")),
            ("name", serde_json::json!(" ")),
            ("slot", serde_json::json!(0)),
            ("slot", serde_json::json!(11)),
            ("path", serde_json::json!("relative")),
            ("path", serde_json::Value::Null),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut input = valid.clone();
            input["bookmarks"][0][field] = value;
            assert!(decode_preferences(&serde_json::to_vec(&input).unwrap()).is_err());
        }
        let mut duplicate = valid.clone();
        duplicate["bookmarks"] =
            serde_json::json!([valid["bookmarks"][0].clone(), valid["bookmarks"][0].clone()]);
        assert!(decode_preferences(&serde_json::to_vec(&duplicate).unwrap()).is_err());
        duplicate["bookmarks"][1]["id"] = serde_json::json!(2);
        assert!(decode_preferences(&serde_json::to_vec(&duplicate).unwrap()).is_err());
        duplicate["bookmarks"][1]["slot"] = serde_json::Value::Null;
        assert!(decode_preferences(&serde_json::to_vec(&duplicate).unwrap()).is_ok());
    }

    #[test]
    fn bookmark_fields_are_versioned_and_strict() {
        for version in [1, 2] {
            let mut value: serde_json::Value =
                serde_json::from_slice(&encode_preferences(UserPreferences::default()).unwrap())
                    .unwrap();
            value["version"] = serde_json::json!(version);
            assert!(decode_preferences(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for bookmarks in ["null", "{}", "false"] {
            let json = format!(
                r#"{{"version":3,"theme":"blue","density":"desktop","font_family":"","desktop_font_size":11,"touch_font_size":15,"bookmarks":{bookmarks}}}"#
            );
            assert!(decode_preferences(json.as_bytes()).is_err());
        }
        let duplicate = br#"{"version":3,"theme":"blue","density":"desktop","font_family":"","desktop_font_size":11,"touch_font_size":15,"bookmarks":[],"bookmarks":[]}"#;
        assert!(decode_preferences(duplicate).is_err());
    }

    #[test]
    fn aggregate_storage_limit_rejects_oversized_paths_without_truncating_them() {
        let preferences = UserPreferences {
            bookmarks: vec![Bookmark {
                id: BookmarkId(1),
                name: "Large".into(),
                path: std::env::temp_dir().join("x".repeat(MAX_PREFERENCES_BYTES)),
                slot: None,
            }],
            ..UserPreferences::default()
        };
        assert!(encode_preferences(preferences).is_err());
    }

    #[test]
    fn version_two_preserves_unicode_font_family_and_independent_sizes() {
        let json = r#"{"version":2,"theme":"dark","density":"touch","windows_shell":"cmd","font_family":"文档 é 🧪","desktop_font_size":12,"touch_font_size":19}"#;
        let decoded = decode_preferences(json.as_bytes()).unwrap();
        let encoded = encode_preferences(decoded).unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(stored["version"], 10);
        assert_eq!(stored["font_family"], "文档 é 🧪");
        assert_eq!(stored["desktop_font_size"], 12);
        assert_eq!(stored["touch_font_size"], 19);
    }

    #[test]
    fn legacy_migration_writes_current_schema_and_preserves_existing_choices() {
        let preferences = decode_preferences(
            br#"{"version":1,"theme":"light","density":"touch","windows_shell":"cmd"}"#,
        )
        .unwrap();
        assert_eq!(preferences.font_family, "");
        assert_eq!(preferences.desktop_font_size, 11);
        assert_eq!(preferences.touch_font_size, 15);
        let migrated = encode_preferences(preferences.clone()).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&migrated).unwrap()["version"],
            10
        );
        assert_eq!(decode_preferences(&migrated).unwrap(), preferences);
    }

    #[test]
    fn font_settings_reject_controls_oversized_names_and_out_of_range_sizes() {
        for name in [
            "bad\0name".to_owned(),
            "bad\nname".to_owned(),
            "字".repeat(171),
        ] {
            assert!(
                encode_preferences(UserPreferences {
                    font_family: name.clone(),
                    ..UserPreferences::default()
                })
                .is_err()
            );
            let json = serde_json::json!({ "version":2,"theme":"blue","density":"desktop","font_family":name,"desktop_font_size":11,"touch_font_size":15 });
            assert!(decode_preferences(&serde_json::to_vec(&json).unwrap()).is_err());
        }
        for size in [0, 7, 33, u16::MAX] {
            for desktop in [true, false] {
                let mut preferences = UserPreferences::default();
                if desktop {
                    preferences.desktop_font_size = size;
                } else {
                    preferences.touch_font_size = size;
                }
                assert!(encode_preferences(preferences).is_err());
            }
        }
        for size in [8, 32] {
            let preferences = UserPreferences {
                desktop_font_size: size,
                touch_font_size: size,
                font_family: "字".repeat(170),
                ..UserPreferences::default()
            };
            assert_eq!(
                decode_preferences(&encode_preferences(preferences.clone()).unwrap()).unwrap(),
                preferences
            );
        }
    }

    #[test]
    fn version_fields_reject_null_duplicate_missing_and_cross_version_font_values() {
        for input in [
            r#"{"version":1,"theme":"blue","density":"desktop","font_family":null}"#,
            r#"{"version":1,"theme":"blue","density":"desktop","desktop_font_size":null}"#,
            r#"{"version":1,"theme":"blue","density":"desktop","font_family":"Arial"}"#,
            r#"{"version":2,"theme":"blue","density":"desktop","font_family":"Arial","desktop_font_size":11}"#,
            r#"{"version":2,"theme":"blue","density":"desktop","font_family":"Arial","font_family":"Other","desktop_font_size":11,"touch_font_size":15}"#,
            r#"{"version":2,"theme":"blue","density":"desktop","font_family":null,"desktop_font_size":11,"touch_font_size":15}"#,
            r#"{"version":3,"theme":"blue","density":"desktop"}"#,
        ] {
            assert!(
                decode_preferences(input.as_bytes()).is_err(),
                "accepted {input}"
            );
        }
    }

    #[test]
    fn versioned_json_roundtrips_every_theme_and_density() {
        for theme in PreferenceTheme::ALL {
            for density in [UiDensity::Desktop, UiDensity::Touch] {
                let preferences = UserPreferences {
                    theme,
                    density,
                    ..UserPreferences::default()
                };
                let encoded = encode_preferences(preferences.clone()).unwrap();
                assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&encoded).unwrap()["version"],
                    10
                );
            }
        }
    }

    #[test]
    fn decoder_rejects_malformed_unsupported_duplicate_unknown_and_oversized_json() {
        assert_eq!(
            decode_preferences(br#"{"version":1,"theme":"blue","density":"desktop"}"#).unwrap(),
            UserPreferences::default()
        );
        for input in [
            br#"{}"#.as_slice(),
            br#"{"version":2,"theme":"blue","density":"desktop"}"#,
            br#"{"version":1,"theme":"unknown","density":"desktop"}"#,
            br#"{"version":1,"theme":"blue","density":"unknown"}"#,
            br#"{"version":1,"theme":"blue","theme":"dark","density":"desktop"}"#,
            br#"{"version":1,"theme":"blue","density":"desktop","unexpected":true}"#,
            br#"{"version":1,"theme":"blue","density":"desktop"} true"#,
            &[0xff, 0x00],
        ] {
            assert!(decode_preferences(input).is_err(), "accepted {input:?}");
        }
        assert!(decode_preferences(&vec![b' '; MAX_PREFERENCES_BYTES + 1]).is_err());
    }

    #[test]
    fn version_one_preserves_windows_shell_and_defaults_legacy_files_to_powershell() {
        let preferences = UserPreferences {
            windows_shell: midnight_core::WindowsShell::Cmd,
            ..UserPreferences::default()
        };
        assert_eq!(
            decode_preferences(&encode_preferences(preferences.clone()).unwrap()).unwrap(),
            preferences
        );
        assert_eq!(
            decode_preferences(
                br#"{"version":1,"theme":"blue","density":"desktop","windows_shell":"cmd"}"#
            )
            .unwrap(),
            preferences
        );
        assert_eq!(
            decode_preferences(br#"{"version":1,"theme":"blue","density":"desktop"}"#)
                .unwrap()
                .windows_shell,
            midnight_core::WindowsShell::PowerShell
        );
        assert!(
            decode_preferences(
                br#"{"version":1,"theme":"blue","density":"desktop","windows_shell":"bash"}"#
            )
            .is_err()
        );
    }

    fn create_new(path: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }
    fn validate(path: &Path) -> io::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a regular destination",
            )),
        }
    }

    #[test]
    fn atomic_replace_preserves_exclusive_name_collision_and_leaves_no_own_temp() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("preferences.json");
        fs::write(&target, b"old").unwrap();
        let mut collision = None;
        publish_atomic(
            &target,
            b"new",
            |path| {
                if collision.is_none() {
                    fs::write(path, b"owned by someone else")?;
                    collision = Some(path.to_path_buf());
                }
                create_new(path)
            },
            validate,
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(
            fs::read(collision.unwrap()).unwrap(),
            b"owned by someone else"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn atomic_failure_before_publish_retains_old_file_and_removes_only_own_temp() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("preferences.json");
        fs::write(&target, b"old").unwrap();
        let mut checks = 0;
        let result = publish_atomic(&target, b"new", create_new, |path| {
            checks += 1;
            if checks == 2 {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected publish denial",
                ))
            } else {
                validate(path)
            }
        });
        assert!(result.is_err());
        assert_eq!(checks, 2);
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_collision_retries_are_bounded_without_truncating_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("preferences.json");
        fs::write(&target, b"old").unwrap();
        let mut attempts = 0;
        let result = publish_atomic(
            &target,
            b"new",
            |path| {
                attempts += 1;
                fs::write(path, b"collision")?;
                create_new(path)
            },
            validate,
        );
        assert!(result.is_err());
        assert_eq!(attempts, 16);
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 17);
    }
}
