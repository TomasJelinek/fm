use midnight_core::*;
use std::path::PathBuf;

fn command() -> UserCommand {
    UserCommand {
        id: UserCommandId::new("tool_1").unwrap(),
        name: "My tool".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["${file.path}".into()],
        },
    }
}

#[test]
fn command_models_validate_literal_programs_and_group_per_item_rules() {
    let mut value = command();
    assert!(value.validate().is_ok());
    value.invocation = UserCommandInvocation::Program {
        executable: PathBuf::from("relative"),
        arguments: vec![],
    };
    assert!(value.validate().is_err());
    value.invocation = UserCommandInvocation::Program {
        executable: std::env::current_exe().unwrap(),
        arguments: vec!["${selected.paths}".into()],
    };
    assert!(value.validate().is_ok());
    value.launch = UserCommandLaunch::PerItem;
    assert!(value.validate().is_err());
    value.invocation = UserCommandInvocation::Terminal {
        shell: ShellFlavor::PowerShell,
        script: "Write-Output '${this.is.literal.user.script}'".into(),
        variables: vec![],
    };
    assert!(
        value.validate().is_ok(),
        "Terminal source is literal user-authored script."
    );
    value.working_directory = "${selected.paths}".into();
    assert!(value.validate().is_err());
}

#[test]
fn stable_ids_duplicates_and_exact_orphan_shortcut_cleanup_are_bounded() {
    for id in ["", "a.b", "a b", "a/../../x", "ü", "\0"] {
        assert!(UserCommandId::new(id).is_err());
    }
    let first = command();
    let mut second = first.clone();
    assert!(validate_user_commands(&[first.clone(), second.clone()]).is_err());
    second.id = UserCommandId::new("tool_2").unwrap();
    assert!(validate_user_commands(&[first.clone(), second.clone()]).is_ok());
    let mut shortcuts = vec![
        ShortcutBinding {
            action: first.id.action(),
            keys: vec!["Ctrl+1".into()],
        },
        ShortcutBinding {
            action: second.id.action(),
            keys: vec![],
        },
        ShortcutBinding {
            action: "files.copy".into(),
            keys: vec!["F5".into()],
        },
    ];
    remove_deleted_user_command_shortcuts(&[first.clone(), second], &[first], &mut shortcuts);
    assert_eq!(shortcuts.len(), 2);
    assert_eq!(shortcuts[0].action, "user-command.tool_1");
    assert_eq!(shortcuts[1].action, "files.copy");
    assert!(validate_user_commands(&vec![command(); MAX_USER_COMMANDS + 1]).is_err());
}

#[test]
fn dynamic_shortcuts_accept_only_configured_ids_and_preserve_existing_conflict_rules() {
    let command = command();
    let binding = ShortcutBinding {
        action: command.id.action(),
        keys: vec!["Ctrl+Alt+U".into()],
    };
    assert!(ShortcutMap::new(std::slice::from_ref(&binding)).is_err());
    let configured = ShortcutMap::with_user_commands(
        std::slice::from_ref(&binding),
        std::slice::from_ref(&command),
    )
    .unwrap();
    assert_eq!(
        configured
            .resolve(
                &ShortcutChord::parse("Ctrl+Alt+U").unwrap(),
                ShortcutContext::Files
            )
            .unwrap()
            .id,
        command.id.action()
    );
    assert!(
        ShortcutMap::with_user_commands(
            &[ShortcutBinding {
                action: command.id.action(),
                keys: vec!["F7".into()],
            }],
            std::slice::from_ref(&command)
        )
        .is_err()
    );
    assert!(
        ShortcutMap::with_user_commands(
            &[ShortcutBinding {
                action: "user-command.unknown".into(),
                keys: vec![],
            }],
            &[command]
        )
        .is_err()
    );
}

#[test]
fn accepted_configuration_update_removes_only_deleted_ids_and_failed_edit_preserves_everything() {
    let first = command();
    let mut second = first.clone();
    second.id = UserCommandId::new("second").unwrap();
    let mut preferences = UserPreferences {
        user_commands: vec![first.clone(), second.clone()].into(),
        shortcuts: vec![
            ShortcutBinding {
                action: first.id.action(),
                keys: vec!["Ctrl+Alt+U".into()],
            },
            ShortcutBinding {
                action: second.id.action(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "files.copy".into(),
                keys: vec![],
            },
        ],
        ..Default::default()
    };
    let before = preferences.clone();
    assert!(
        preferences
            .replace_user_commands(vec![first.clone(), first.clone()].into())
            .is_err()
    );
    assert_eq!(preferences, before);
    preferences
        .replace_user_commands(vec![first].into())
        .unwrap();
    assert_eq!(preferences.shortcuts.len(), 2);
    assert_eq!(preferences.shortcuts[0].action, "user-command.tool_1");
    assert_eq!(preferences.shortcuts[1].action, "files.copy");
}
