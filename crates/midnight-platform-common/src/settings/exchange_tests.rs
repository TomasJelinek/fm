use super::*;
use midnight_core::*;
use std::path::PathBuf;

fn native(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// Non-default values on Options pages and in the session state outside Options.
fn rich() -> UserPreferences {
    UserPreferences {
        theme: PreferenceTheme::Nord,
        app_icon: AppIcon::OrigamiTrail,
        density: UiDensity::Touch,
        windows_shell: WindowsShell::Cmd,
        font_family: "Noto Sans 文档 e\u{301}".into(),
        desktop_font_size: 14,
        touch_font_size: 18,
        glyph_fitting: GlyphFitting::Subpixel,
        scroll_glide: ScrollGlide::Short,
        wheel_speed: WheelSpeed::Fast,
        updates: UpdatePreferences {
            check_at_startup: false,
            previews: false,
        },
        quick_search_filter: true,
        size_display: SizeDisplay::Bytes,
        disk_io_concurrency: DiskIoConcurrency::Manual(8),
        copy_checksum_verification: false,
        copy_preserve_metadata: false,
        command_history: vec![CommandHistoryEntry {
            command: "echo žluťoučký 🧪".into(),
            shell: ShellFlavor::PowerShell,
        }],
        bookmarks: vec![Bookmark {
            id: BookmarkId(7),
            name: "Projekty 文档".into(),
            path: native("projekty 文档"),
            archive: None,
            network: None,
            slot: Some(3),
        }],
        tabs: [
            PaneTabPreferences {
                paths: vec![native("left 🧪"), native("left e\u{301}")],
                active: 1,
                ..Default::default()
            },
            PaneTabPreferences {
                paths: vec![native("right")],
                active: 0,
                ..Default::default()
            },
        ],
        panel_view_modes: [PanelViewMode::Brief, PanelViewMode::Thumbnails],
        panel_split_basis_points: 3300,
        window_geometry: Some(WindowGeometry {
            normal: WindowRect {
                x: -20,
                y: 40,
                width: 1280,
                height: 720,
            },
            monitor: None,
            maximized: true,
            fullscreen: false,
        }),
        thumbnails_enabled: false,
        thumbnails_network: true,
        thumbnails_archives: false,
        thumbnail_size: 128,
        shortcuts: vec![ShortcutBinding {
            action: "files.rename".into(),
            keys: vec!["Alt+2".into()],
        }],
        panel_filters: [
            PanelFilter {
                enabled: true,
                mode: FilterMode::Masks,
                expression: "*.rs".into(),
                case_sensitive: false,
            },
            PanelFilter::default(),
        ],
        panel_filter_history: vec![PanelFilter {
            enabled: true,
            mode: FilterMode::Regex,
            expression: "^ž.*".into(),
            case_sensitive: true,
        }],
        checksum_algorithms: ChecksumAlgorithms::all(),
        viewer: ViewerPreferences {
            wrap: true,
            line_numbers: false,
            ..Default::default()
        },
        file_list: FileListPreferences {
            template_history: vec!["$(FileName)$(TAB)$(FileSize)$(CRLF)".into()],
            destination: 2,
            path: Some(native("seznam.txt")),
            append: true,
        },
        filename_colors: Vec::new().into(),
        ..UserPreferences::default()
    }
}

fn message(bytes: &[u8]) -> String {
    let error = decode_settings_file(bytes).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    error.to_string()
}

fn document(preferences: UserPreferences) -> serde_json::Value {
    serde_json::from_slice(&encode_settings_file(preferences).unwrap()).unwrap()
}

#[test]
fn whole_settings_roundtrip_in_the_native_settings_serialization() {
    let preferences = rich();
    let bytes = encode_settings_file(preferences.clone()).unwrap();
    assert_eq!(
        bytes,
        super::super::encode_preferences(preferences.clone()).unwrap()
    );
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.starts_with("{\n  \""), "{text}");
    assert!(text.contains("Projekty 文档") && text.ends_with("}\n"));
    assert_eq!(
        decode_settings_file(&bytes).unwrap().preferences,
        preferences
    );
    let defaults = UserPreferences::default();
    let bytes = encode_settings_file(defaults.clone()).unwrap();
    assert_eq!(decode_settings_file(&bytes).unwrap().preferences, defaults);
}

#[test]
fn byte_order_mark_is_ignored_and_older_schemas_migrate_like_saved_settings() {
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend(encode_settings_file(rich()).unwrap());
    assert_eq!(decode_settings_file(&bytes).unwrap().preferences, rich());
    let legacy = br#"{"version": 1, "theme": "dark", "density": "touch"}"#;
    assert_eq!(
        decode_settings_file(legacy).unwrap().preferences,
        UserPreferences {
            theme: PreferenceTheme::Dark,
            density: UiDensity::Touch,
            ..UserPreferences::default()
        }
    );
}

#[test]
fn refuses_documents_that_are_not_settings_files() {
    assert!(message(b"").contains("not a JSON settings document"));
    assert!(message(b"theme = 'dark'").contains("not a JSON settings document"));
    assert!(message(b"[1, 2]").contains("not a Midnight Explorer settings file"));
    assert!(message(br#"{"theme": "dark"}"#).contains("has no settings version"));
    for version in [r#""34""#, "0", "-1", "1.5", "null"] {
        let text = format!(r#"{{"version": {version}, "theme": "dark", "density": "touch"}}"#);
        assert!(
            message(text.as_bytes()).contains("whole number from 1 to"),
            "{text}"
        );
    }
}

#[test]
fn refuses_newer_versions_before_reporting_their_unknown_fields() {
    let mut newer = document(rich());
    newer["version"] = (SETTINGS_FILE_VERSION + 1).into();
    newer["future_setting"] = true.into();
    let text = message(&serde_json::to_vec(&newer).unwrap());
    assert!(
        text.contains(&format!(
            "settings version {} from a newer Midnight Explorer",
            SETTINGS_FILE_VERSION + 1
        )),
        "{text}"
    );
    assert!(text.contains(&format!("versions 1 to {SETTINGS_FILE_VERSION}")));
}

#[test]
fn refuses_unknown_fields_and_invalid_values_with_the_offending_name() {
    let mut unknown = document(rich());
    unknown["colour_scheme"] = "dark".into();
    let text = message(&serde_json::to_vec(&unknown).unwrap());
    assert!(text.contains("unknown field `colour_scheme`"), "{text}");
    assert!(!text.contains("expected one of"), "{text}");
    for (field, value, expected) in [
        (
            "theme",
            serde_json::json!("plaid"),
            "unknown preferences theme",
        ),
        (
            "density",
            serde_json::json!("roomy"),
            "unknown preferences density",
        ),
        (
            "desktop_font_size",
            serde_json::json!(99),
            "Font size must be between",
        ),
        (
            "thumbnail_size",
            serde_json::json!(100),
            "Thumbnail size must be",
        ),
        (
            "panel_split_basis_points",
            serde_json::json!(0),
            "Panel split",
        ),
    ] {
        let mut invalid = document(rich());
        invalid[field] = value;
        let text = message(&serde_json::to_vec(&invalid).unwrap());
        assert!(text.contains(expected), "{field}: {text}");
    }
    let mut duplicate = document(rich());
    let bookmark = duplicate["bookmarks"][0].clone();
    duplicate["bookmarks"]
        .as_array_mut()
        .unwrap()
        .push(bookmark);
    assert!(message(&serde_json::to_vec(&duplicate).unwrap()).contains("bookmark"));
}

#[test]
fn refuses_oversized_files_before_parsing() {
    let mut bytes = encode_settings_file(rich()).unwrap();
    bytes.resize(MAX_PREFERENCES_BYTES, b' ');
    assert_eq!(decode_settings_file(&bytes).unwrap().preferences, rich());
    bytes.push(b' ');
    assert!(message(&bytes).contains("larger than the 4 MiB settings limit"));
}
