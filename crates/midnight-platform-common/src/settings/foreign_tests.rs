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
        "shortcuts",
        "file_comparators",
        "file_list",
    ] {
        assert_eq!(document[field], original[field], "{field}");
    }
    // Skipped tabs do not return; the kept command's key binding does, with it.
    assert_eq!(document["tabs"][0]["paths"].as_array().unwrap().len(), 1);
    // Reading the export again keeps the same entries; no tab is skipped twice.
    let again = decode_settings_file(&exported).unwrap();
    assert_eq!(again.preferences, imported.preferences);
    assert_eq!((again.skipped_tabs, again.removed_bindings), (0, 1));
}

/// The shell this system's terminal runs, and one only the other system's runs.
fn shells() -> (ShellFlavor, ShellFlavor) {
    match PathSystem::CURRENT {
        PathSystem::Windows => (ShellFlavor::PowerShell, ShellFlavor::Posix),
        PathSystem::Unix => (ShellFlavor::Posix, ShellFlavor::Cmd),
    }
}

fn terminal(id: &str, shell: ShellFlavor, working_directory: &str) -> UserCommand {
    UserCommand {
        id: UserCommandId::new(id).unwrap(),
        name: format!("{id} 文档"),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: working_directory.into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Terminal {
            shell,
            script: "make 文档".into(),
            variables: Vec::new(),
        },
    }
}

fn binding(id: &str, keys: &[&str]) -> ShortcutBinding {
    ShortcutBinding {
        action: format!("user-command.{id}"),
        keys: keys.iter().map(|key| (*key).to_owned()).collect(),
    }
}

/// A file as the other system writes it: a native program, a terminal command that
/// works in one of its folders and one in its shell, each with a key binding.
fn terminal_document() -> Value {
    let (native_shell, other_shell) = shells();
    let preferences = UserPreferences {
        user_commands: vec![
            program("tool", native("tool")),
            terminal("folder", native_shell, &other("src")),
            terminal("shell", other_shell, "${active.path}"),
        ]
        .into(),
        shortcuts: vec![
            binding("tool", &["Alt+1"]),
            binding("folder", &["Alt+3", "Alt+5"]),
            binding("shell", &["Alt+4"]),
        ],
        ..UserPreferences::default()
    };
    serde_json::from_slice(&encode_settings_file(preferences).unwrap()).unwrap()
}

/// `document` read on the system that wrote it: there its folder and shell are native.
fn on_its_own_system(mut document: Value) -> ImportedPreferences {
    let native_shell = match shells().0 {
        ShellFlavor::PowerShell => "powershell",
        ShellFlavor::Cmd => "cmd",
        ShellFlavor::Posix => "posix",
    };
    for command in document["user_commands"].as_array_mut().unwrap() {
        if command["working_directory"] == other("src") {
            command["working_directory"] = "${active.path}".into();
        }
        if command["invocation"]["mode"] == "terminal" {
            command["invocation"]["shell"] = native_shell.into();
        }
    }
    decode_settings_file(&serde_json::to_vec(&document).unwrap()).unwrap()
}

#[test]
fn terminal_commands_of_the_other_system_are_kept_inactive_with_their_key_bindings() {
    let imported =
        decode_settings_file(&serde_json::to_vec(&terminal_document()).unwrap()).unwrap();
    let preferences = &imported.preferences;
    let ids: Vec<_> = preferences
        .user_commands
        .iter()
        .map(|command| command.id.as_str())
        .collect();
    assert_eq!(ids, ["tool"], "only the native program stays active");
    let kept: Vec<_> = preferences
        .foreign_settings
        .iter()
        .map(|setting| {
            (
                setting.kind.clone(),
                setting.system,
                setting.location.clone(),
            )
        })
        .collect();
    let system = PathSystem::CURRENT.other();
    let shell_name = match system {
        PathSystem::Unix => "POSIX shell",
        PathSystem::Windows => "Command Prompt",
    };
    assert_eq!(
        kept,
        [
            (
                ForeignSettingKind::UserCommand {
                    position: 1,
                    id: UserCommandId::new("folder").unwrap(),
                    name: "folder 文档".into()
                },
                system,
                other("src")
            ),
            (
                ForeignSettingKind::UserCommand {
                    position: 2,
                    id: UserCommandId::new("shell").unwrap(),
                    name: "shell 文档".into()
                },
                system,
                shell_name.into()
            ),
        ]
    );
    // Their key bindings do nothing here, but are not lost.
    assert_eq!(preferences.shortcuts, [binding("tool", &["Alt+1"])]);
    assert_eq!(imported.removed_bindings, 2);
    let exported: Value =
        serde_json::from_slice(&encode_settings_file(preferences.clone()).unwrap()).unwrap();
    assert_eq!(
        exported["user_commands"],
        terminal_document()["user_commands"]
    );
    assert_eq!(exported["shortcuts"], terminal_document()["shortcuts"]);
    // Back on their own system the commands and their keys work again.
    let home = on_its_own_system(exported);
    let shortcuts = &home.preferences.shortcuts;
    for wanted in [
        binding("folder", &["Alt+3", "Alt+5"]),
        binding("shell", &["Alt+4"]),
    ] {
        assert!(shortcuts.contains(&wanted), "{wanted:?} in {shortcuts:?}");
    }
    assert!(
        home.preferences
            .user_commands
            .iter()
            .any(|command| command.id.as_str() == "shell")
    );
}

#[test]
fn a_key_taken_here_leaves_the_kept_binding_so_the_file_stays_valid_there() {
    let imported =
        decode_settings_file(&serde_json::to_vec(&terminal_document()).unwrap()).unwrap();
    let mut preferences = imported.preferences;
    // Alt+3 now runs the native program here, as Alt+1 does.
    preferences.shortcuts = vec![binding("tool", &["Alt+1", "Alt+3"])];
    let exported: Value =
        serde_json::from_slice(&encode_settings_file(preferences).unwrap()).unwrap();
    assert_eq!(
        exported["shortcuts"],
        json!([
            { "action": "user-command.tool", "keys": ["Alt+1", "Alt+3"] },
            { "action": "user-command.folder", "keys": ["Alt+5"] },
            { "action": "user-command.shell", "keys": ["Alt+4"] },
        ])
    );
    // The other system reads it without a conflict.
    let home = on_its_own_system(exported);
    assert!(
        home.preferences
            .shortcuts
            .contains(&binding("folder", &["Alt+5"]))
    );
    // A kept binding left without keys is left out.
    let imported =
        decode_settings_file(&serde_json::to_vec(&terminal_document()).unwrap()).unwrap();
    let mut preferences = imported.preferences;
    preferences.shortcuts = vec![binding("tool", &["Alt+1", "Alt+4"])];
    let exported: Value =
        serde_json::from_slice(&encode_settings_file(preferences).unwrap()).unwrap();
    let actions: Vec<_> = exported["shortcuts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|binding| binding["action"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(actions, ["user-command.tool", "user-command.folder"]);
}

#[test]
fn a_kept_command_bound_twice_is_refused_as_its_own_system_refuses_it() {
    let mut document = terminal_document();
    document["shortcuts"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "action": "user-command.shell", "keys": ["Alt+6"] }));
    let error = decode_settings_file(&serde_json::to_vec(&document).unwrap()).unwrap_err();
    assert!(
        error.to_string().contains("repeated key binding"),
        "{error}"
    );
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
