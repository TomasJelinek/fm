//! Portable preferences codec and atomic publication; native adapters own path/security policy.
use fm_core::{Bookmark, BookmarkId, PreferenceTheme, UiDensity, UserPreferences, WindowsShell};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

mod native_path;
use native_path::NativePath;

/// An aggregate storage bound, never a native path length restriction.
pub const MAX_PREFERENCES_BYTES: usize = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPreferences {
    version: u32,
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
    if !(1..=4).contains(&stored.version) {
        return Err(invalid("unsupported preferences version"));
    }
    let theme = match stored.theme.as_str() {
        "blue" => PreferenceTheme::Blue,
        "dark" => PreferenceTheme::Dark,
        "light" => PreferenceTheme::Light,
        _ => return Err(invalid("unknown preferences theme")),
    };
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
        2..=4 => (
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
        (3 | 4, Some(bookmarks)) => bookmarks
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
        (3 | 4, None) => return Err(invalid("missing bookmarks")),
        _ => return Err(invalid("bookmarks require preferences version 3")),
    };
    let quick_search_filter = match (stored.version, stored.quick_search_filter) {
        (1..=3, None) => false,
        (4, Some(enabled)) => enabled,
        (4, None) => return Err(invalid("missing quick search filter preference")),
        _ => {
            return Err(invalid(
                "quick search filter requires preferences version 4",
            ));
        }
    };
    let preferences = UserPreferences {
        theme,
        density,
        windows_shell,
        font_family,
        desktop_font_size,
        touch_font_size,
        quick_search_filter,
        bookmarks,
    };
    preferences.validate().map_err(invalid)?;
    Ok(preferences)
}

pub fn encode_preferences(preferences: UserPreferences) -> io::Result<Vec<u8>> {
    preferences.validate().map_err(invalid)?;
    let stored = StoredPreferences {
        version: 4,
        quick_search_filter: Some(preferences.quick_search_filter),
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
        theme: match preferences.theme {
            PreferenceTheme::Blue => "blue",
            PreferenceTheme::Dark => "dark",
            PreferenceTheme::Light => "light",
        }
        .into(),
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
            ".fm-preferences-{}-{sequence}.tmp",
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
    use fm_core::{PreferenceTheme, UiDensity};
    use std::fs::{self, OpenOptions};

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
            4
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
        assert_eq!(stored["version"], 4);
        assert_eq!(stored["font_family"], "文档 é 🧪");
        assert_eq!(stored["desktop_font_size"], 12);
        assert_eq!(stored["touch_font_size"], 19);
    }

    #[test]
    fn legacy_migration_writes_v4_and_preserves_existing_choices() {
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
            4
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
        for theme in [
            PreferenceTheme::Blue,
            PreferenceTheme::Dark,
            PreferenceTheme::Light,
        ] {
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
                    4
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
            windows_shell: fm_core::WindowsShell::Cmd,
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
            fm_core::WindowsShell::PowerShell
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
