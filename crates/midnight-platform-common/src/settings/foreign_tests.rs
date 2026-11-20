use super::super::{decode_settings_file, encode_settings_file};
use midnight_core::*;
use serde_json::{Value, json};
use std::path::PathBuf;

fn native(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// An absolute path written by the other operating system.
fn other(name: &str) -> String {
    match PathSystem::CURRENT.other() {
        PathSystem::Unix => format!("/home/tom/{name}"),
        PathSystem::Windows => format!("C:\\Users\\tom\\{name}"),
    }
}

fn program(id: &str, executable: PathBuf) -> UserCommand {
    UserCommand {
        id: UserCommandId::new(id).unwrap(),
        name: format!("{id} 文档"),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable,
            arguments: vec!["${selected.paths}".into()],
        },
    }
}

fn bookmark(id: u64, name: &str, slot: u8) -> Bookmark {
    Bookmark {
        id: BookmarkId(id),
        name: name.into(),
        path: native(name),
        archive: None,
        network: None,
        slot: Some(slot),
    }
}

/// Settings with one path of each kind; `foreign_document` moves some to the other system.
fn native_settings() -> UserPreferences {
    UserPreferences {
        bookmarks: vec![bookmark(1, "kept", 1), bookmark(2, "projekty 文档", 2)],
        tabs: [
            PaneTabPreferences {
                paths: vec![native("levý"), native("vzdálený")],
                active: 1,
                ..Default::default()
            },
            PaneTabPreferences {
                paths: vec![native("pravý")],
                active: 0,
                ..Default::default()
            },
        ],
        editor: EditorPreferences {
            executable: Some(native("editor.exe")),
            arguments: vec!["--wait".into(), "{file}".into()],
            rules: vec![
                EditorRule {
                    expression: "*.txt".into(),
                    executable: Some(native("code")),
                    ..EditorRule::default()
                },
                EditorRule {
                    expression: "*.md".into(),
                    ..EditorRule::default()
                },
            ],
        },
        user_commands: vec![
            program("kept", native("tool")),
            program("build", native("make")),
        ]
        .into(),
        shortcuts: vec![ShortcutBinding {
            action: "user-command.build".into(),
            keys: vec!["Alt+2".into()],
        }],
        file_comparators: FileComparatorPreferences {
            rules: vec![FileComparatorRule {
                enabled: true,
                mode: FilterMode::Masks,
                expression: "*.png".into(),
                case_sensitive: false,
                target: FileComparatorTarget::External(FileComparatorProgram {
                    executable: native("meld"),
                    arguments: vec!["{left}".into(), "{right}".into()],
                    working_directory: None,
                }),
            }],
        },
        file_list: FileListPreferences {
            path: Some(native("seznam.txt")),
            ..FileListPreferences::default()
        },
        ..UserPreferences::default()
    }
}

/// The same document as the other system would have written some of its paths.
fn foreign_document() -> Value {
    let bytes = encode_settings_file(native_settings()).unwrap();
    let mut document: Value = serde_json::from_slice(&bytes).unwrap();
    document["bookmarks"][1]["path"] = other("projekty 文档").into();
    document["tabs"][0]["paths"][1] = other("vzdálený").into();
    document["editor"]["executable"] = other("editor").into();
    document["editor"]["rules"][0]["executable"] = other("code").into();
    document["user_commands"][1]["invocation"]["executable"] = other("make").into();
    document["file_comparators"]["rules"][0]["target"]["executable"] = other("meld").into();
    document["file_list"]["path"] = other("seznam.txt").into();
    document
}

#[test]
fn another_systems_paths_are_kept_inactive_and_tabs_at_them_are_skipped() {
    let imported = decode_settings_file(&serde_json::to_vec(&foreign_document()).unwrap()).unwrap();
    assert_eq!((imported.skipped_tabs, imported.removed_bindings), (1, 1));
    let preferences = imported.preferences;
    let system = PathSystem::CURRENT.other();
    assert_eq!(preferences.bookmarks, vec![bookmark(1, "kept", 1)]);
    let [hot_path] = preferences.foreign_hot_paths.as_slice() else {
        panic!("one inactive hot path");
    };
    assert_eq!(
        (hot_path.id, hot_path.slot, hot_path.system),
        (BookmarkId(2), Some(2), system)
    );
    assert_eq!(hot_path.location, other("projekty 文档"));
    assert_eq!(preferences.tabs[0].paths, [native("levý")]);
    assert_eq!(preferences.tabs[0].active, 0);
    assert_eq!(preferences.tabs[1].paths, [native("pravý")]);
    assert_eq!(preferences.editor.executable, None);
    assert_eq!(preferences.editor.arguments, ["{file}"]);
    assert_eq!(preferences.editor.rules.len(), 1);
    assert_eq!(preferences.editor.rules[0].expression, "*.md");
    assert_eq!(preferences.user_commands.len(), 1);
    assert_eq!(preferences.user_commands[0].id.as_str(), "kept");
    assert!(
        preferences.shortcuts.is_empty(),
        "the kept command's binding"
    );
    assert!(preferences.file_comparators.rules.is_empty());
    assert_eq!(preferences.file_list.path, None);
    let kept: Vec<_> = preferences
        .foreign_settings
        .iter()
        .map(|setting| (setting.kind.clone(), setting.location.clone()))
        .collect();
    assert_eq!(
        kept,
        [
            (ForeignSettingKind::Editor, other("editor")),
            (
                ForeignSettingKind::EditorRule {
                    position: 0,
                    pattern: "*.txt".into()
                },
                other("code")
            ),
            (
                ForeignSettingKind::UserCommand {
                    position: 1,
                    id: UserCommandId::new("build").unwrap(),
                    name: "build 文档".into()
                },
                other("make")
            ),
            (
                ForeignSettingKind::FileComparator {
                    position: 0,
                    pattern: "*.png".into()
                },
                other("meld")
            ),
            (ForeignSettingKind::FileListOutput, other("seznam.txt")),
        ]
    );
    assert!(
        preferences
            .foreign_settings
            .iter()
            .all(|setting| setting.system == system)
    );
}

#[test]
fn kept_records_return_unchanged_on_export_at_their_positions() {
    let original = foreign_document();
    let imported = decode_settings_file(&serde_json::to_vec(&original).unwrap()).unwrap();
    let exported = encode_settings_file(imported.preferences.clone()).unwrap();
    let document: Value = serde_json::from_slice(&exported).unwrap();
    for field in [
        "version",
        "bookmarks",
        "editor",
        "user_commands",
        "file_comparators",
        "file_list",
    ] {
        assert_eq!(document[field], original[field], "{field}");
    }
    // Skipped tabs and the kept command's key binding do not return.
    assert_eq!(document["tabs"][0]["paths"].as_array().unwrap().len(), 1);
    assert_eq!(document["shortcuts"], json!([]));
    // Reading the export again keeps the same entries; nothing is skipped twice.
    let again = decode_settings_file(&exported).unwrap();
    assert_eq!(again.preferences, imported.preferences);
    assert_eq!((again.skipped_tabs, again.removed_bindings), (0, 0));
}

#[test]
fn a_native_replacement_wins_over_the_kept_editor_and_output_file() {
    let imported = decode_settings_file(&serde_json::to_vec(&foreign_document()).unwrap()).unwrap();
    let mut preferences = imported.preferences;
    preferences.editor.executable = Some(native("notepad.exe"));
    preferences.file_list.path = Some(native("output.txt"));
    let document: Value =
        serde_json::from_slice(&encode_settings_file(preferences).unwrap()).unwrap();
    assert_eq!(
        document["editor"]["executable"],
        Value::from(native("notepad.exe").to_str().unwrap())
    );
    assert_eq!(
        document["file_list"]["path"],
        Value::from(native("output.txt").to_str().unwrap())
    );
}

#[test]
fn malformed_records_from_another_system_are_still_refused() {
    let refuse = |document: Value, expected: &str| {
        let error = decode_settings_file(&serde_json::to_vec(&document).unwrap()).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    };
    let mut empty_name = foreign_document();
    empty_name["bookmarks"][1]["name"] = " ".into();
    refuse(empty_name, "invalid hot path from another system");
    let mut slot = foreign_document();
    slot["bookmarks"][1]["slot"] = 1.into();
    refuse(slot, "slots must be unique");
    let mut command = foreign_document();
    command["user_commands"][1]["id"] = "not valid!".into();
    refuse(command, "Command IDs");
    let mut arguments = foreign_document();
    arguments["editor"]["arguments"] = json!(["--wait"]);
    refuse(arguments, "{file}");
    let mut relative = foreign_document();
    relative["bookmarks"][1]["path"] = "relative/path".into();
    refuse(relative, "absolute native path");
}

#[test]
fn encoded_paths_of_the_other_system_are_described_lossily_and_kept_exactly() {
    let (encoding, hex, location) = match PathSystem::CURRENT.other() {
        PathSystem::Unix => ("unix-bytes", "2f746d702fff", "/tmp/\u{fffd}"),
        PathSystem::Windows => ("windows-utf16", "0043003a005c0078d800", "C:\\x\u{fffd}"),
    };
    let mut document = foreign_document();
    document["bookmarks"][1]["path"] = json!({ "encoding": encoding, "hex": hex });
    let imported = decode_settings_file(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(imported.preferences.foreign_hot_paths[0].location, location);
    let exported: Value =
        serde_json::from_slice(&encode_settings_file(imported.preferences).unwrap()).unwrap();
    assert_eq!(exported["bookmarks"], document["bookmarks"]);
    document["bookmarks"][1]["path"] = json!({ "encoding": encoding, "hex": "2f7" });
    assert!(decode_settings_file(&serde_json::to_vec(&document).unwrap()).is_err());
}
