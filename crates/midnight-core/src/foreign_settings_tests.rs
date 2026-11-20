use super::*;
use crate::Bookmark;

fn hot_path(id: u64, slot: Option<u8>) -> ForeignHotPath {
    ForeignHotPath {
        id: BookmarkId(id),
        slot,
        system: PathSystem::CURRENT.other(),
        location: "/home/tom/projekty 文档".into(),
        stored: "{}".into(),
    }
}

fn setting(kind: ForeignSettingKind) -> ForeignSetting {
    ForeignSetting {
        kind,
        system: PathSystem::CURRENT.other(),
        location: "/usr/bin/meld".into(),
        stored: "{}".into(),
    }
}

#[test]
fn path_syntax_of_the_other_system_is_recognized_without_filesystem_access() {
    use PathSystem::{Unix, Windows};
    for text in ["/home/tom", "/", "//server/share", "/usr/bin/文档 e\u{301}"] {
        assert_eq!(PathSystem::foreign_for(Windows, text), Some(Unix), "{text}");
        assert_eq!(PathSystem::foreign_for(Unix, text), None, "{text}");
    }
    for text in [
        "C:\\Users\\tom",
        "d:/data",
        "\\\\server\\share\\x",
        "\\\\?\\C:\\long",
    ] {
        assert_eq!(PathSystem::foreign_for(Unix, text), Some(Windows), "{text}");
        assert_eq!(PathSystem::foreign_for(Windows, text), None, "{text}");
    }
    for text in ["relative", "C:relative", "", "a/b", "/home/\0x", "C:\\\0"] {
        assert_eq!(PathSystem::foreign_for(Windows, text), None, "{text:?}");
        assert_eq!(PathSystem::foreign_for(Unix, text), None, "{text:?}");
    }
    assert_eq!(PathSystem::CURRENT.other().other(), PathSystem::CURRENT);
    assert_eq!(Unix.name(), "Linux");
}

#[test]
fn inactive_hot_paths_share_identities_slots_and_the_hot_path_limit() {
    let native = Bookmark {
        id: BookmarkId(1),
        name: "native".into(),
        path: std::env::temp_dir(),
        archive: None,
        network: None,
        slot: Some(1),
    };
    let mut preferences = UserPreferences {
        bookmarks: vec![native],
        foreign_hot_paths: vec![hot_path(2, Some(2))],
        ..UserPreferences::default()
    };
    assert!(preferences.validate().is_ok());
    preferences.foreign_hot_paths[0].slot = Some(1);
    assert!(preferences.validate().is_err(), "slots are shared");
    preferences.foreign_hot_paths[0] = hot_path(1, None);
    assert!(preferences.validate().is_err(), "identities are shared");
    preferences.foreign_hot_paths = (2..=64).map(|id| hot_path(id, None)).collect();
    assert!(preferences.validate().is_ok());
    preferences.foreign_hot_paths.push(hot_path(65, None));
    assert!(preferences.validate().is_err(), "64 hot paths in total");
    let mut native_location = hot_path(2, None);
    native_location.system = PathSystem::CURRENT;
    preferences.foreign_hot_paths = vec![native_location];
    assert!(
        preferences.validate().is_err(),
        "only the other system's paths"
    );
}

#[test]
fn kept_settings_reserve_command_identities_and_list_limits() {
    let id = UserCommandId::new("custom-1").unwrap();
    let mut preferences = UserPreferences {
        foreign_settings: Arc::new(vec![
            setting(ForeignSettingKind::Editor),
            setting(ForeignSettingKind::UserCommand {
                position: 0,
                id: id.clone(),
                name: "Build".into(),
            }),
            setting(ForeignSettingKind::FileComparator {
                position: 3,
                pattern: "*.png".into(),
            }),
        ]),
        ..UserPreferences::default()
    };
    assert!(preferences.validate().is_ok());
    let mut duplicate = (*preferences.foreign_settings).clone();
    duplicate.push(setting(ForeignSettingKind::Editor));
    preferences.foreign_settings = duplicate.into();
    assert!(preferences.validate().is_err(), "one default editor");
    let mut commands = vec![setting(ForeignSettingKind::UserCommand {
        position: 0,
        id: id.clone(),
        name: "Build".into(),
    })];
    commands.push(commands[0].clone());
    preferences.foreign_settings = commands.into();
    assert!(
        preferences.validate().is_err(),
        "command identities are unique"
    );
    let rules = (0..=MAX_EDITOR_RULES)
        .map(|position| {
            setting(ForeignSettingKind::EditorRule {
                position,
                pattern: "*.txt".into(),
            })
        })
        .collect::<Vec<_>>();
    preferences.foreign_settings = rules.into();
    assert!(preferences.validate().is_err(), "64 editor rules in total");
}

#[test]
fn a_native_editor_or_output_file_replaces_the_kept_one() {
    let rule = setting(ForeignSettingKind::EditorRule {
        position: 0,
        pattern: "*.txt".into(),
    });
    let kept = vec![
        setting(ForeignSettingKind::Editor),
        rule.clone(),
        setting(ForeignSettingKind::FileListOutput),
    ];
    let mut preferences = UserPreferences {
        foreign_settings: kept.clone().into(),
        ..UserPreferences::default()
    };
    preferences.drop_replaced_foreign_editor();
    preferences.drop_replaced_foreign_output();
    assert_eq!(
        *preferences.foreign_settings, kept,
        "nothing replaces them yet"
    );
    preferences.editor.executable = Some(std::env::temp_dir().join("editor"));
    preferences.file_list.path = Some(std::env::temp_dir().join("list.txt"));
    preferences.drop_replaced_foreign_editor();
    assert_eq!(*preferences.foreign_settings, kept[1..], "only the editor");
    preferences.drop_replaced_foreign_output();
    assert_eq!(
        *preferences.foreign_settings,
        [rule],
        "rules are separate entries"
    );
}

#[test]
fn descriptions_name_the_setting_without_its_path() {
    let command = setting(ForeignSettingKind::UserCommand {
        position: 2,
        id: UserCommandId::new("custom-7").unwrap(),
        name: "Sestavit 文档".into(),
    });
    assert_eq!(command.description(), "Custom command “Sestavit 文档”");
    assert_eq!(
        setting(ForeignSettingKind::FileListOutput).description(),
        "Make File List output"
    );
}
