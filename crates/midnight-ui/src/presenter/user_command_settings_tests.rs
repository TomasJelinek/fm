use super::*;
use std::cell::RefCell;

fn command(id: &str, name: &str) -> UserCommand {
    UserCommand {
        id: UserCommandId::new(id).unwrap(),
        name: name.into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::temp_dir().join("fixture-tool.exe"),
            arguments: vec![],
        },
    }
}
fn setup() -> Presenter {
    let root = std::env::temp_dir().join("commands settings 文档");
    let mut p = Presenter::new(root.clone(), root);
    p.preferences.loaded = true;
    p.preferences.values.user_commands = Arc::new(vec![
        command("first", "Duplicate"),
        command("second", "Duplicate"),
    ]);
    p.sync_user_command_settings();
    p
}

#[test]
fn command_edit_apply_preserves_stable_id_and_reorder_delete_retains_other_shortcuts() {
    let mut p = setup();
    let token = p.user_command_settings.token();
    let mut form = p.user_command_settings_form().unwrap();
    form.name = "Renamed 文档".into();
    form.enabled = false;
    p.validate_user_command_setting(&token, &form);
    assert!(p.user_command_settings.can_apply);
    assert!(p.apply_user_command_setting(&token, &form, &|_| true));
    assert_eq!(p.preferences.values.user_commands[0].id.as_str(), "first");
    assert!(!p.preferences.values.user_commands[0].enabled);
    let token = p.user_command_settings.token();
    assert!(p.move_user_command_setting(&token, 1, &|_| true));
    assert_eq!(p.preferences.values.user_commands[1].id.as_str(), "first");
    assert_eq!(
        p.user_command_settings.selected.as_ref().unwrap().as_str(),
        "first"
    );
    let keep = ShortcutBinding {
        action: "files.copy".into(),
        keys: vec![],
    };
    p.preferences.values.shortcuts = vec![
        keep.clone(),
        ShortcutBinding {
            action: "user-command.first".into(),
            keys: vec![],
        },
    ];
    let token = p.user_command_settings.token();
    assert!(p.delete_user_command_setting(&token, &|_| true));
    assert_eq!(p.preferences.values.user_commands.len(), 1);
    assert_eq!(p.preferences.values.user_commands[0].id.as_str(), "second");
    assert_eq!(p.preferences.values.shortcuts, vec![keep]);
}

#[test]
fn invalid_stale_and_cancelled_forms_cannot_change_current_configuration() {
    let mut p = setup();
    let before = p.preferences.values.user_commands.clone();
    let token = p.user_command_settings.token();
    let mut form = p.user_command_settings_form().unwrap();
    form.arguments = "not JSON".into();
    p.validate_user_command_setting(&token, &form);
    assert!(!p.user_command_settings.can_apply);
    assert!(p.user_command_settings.dirty);
    assert!(!p.apply_user_command_setting(&token, &form, &|_| panic!("invalid form persisted")));
    assert!(Arc::ptr_eq(&before, &p.preferences.values.user_commands));
    p.cancel_user_command_setting();
    let current = p.user_command_settings.token();
    assert_ne!(token, current);
    form = p.user_command_settings_form().unwrap();
    form.name = "Changed".into();
    p.validate_user_command_setting(&current, &form);
    p.preferences.values.user_commands =
        Arc::new(p.preferences.values.user_commands.as_ref().clone());
    p.sync_user_command_settings();
    assert!(!p.user_command_settings.can_apply);
    assert!(!p.apply_user_command_setting(&current, &form, &|_| panic!("stale form persisted")));
    p.cancel_user_command_setting();
    assert!(!p.user_command_settings.dirty);
    p.validate_user_command_setting(&token, &form);
    assert!(
        !p.user_command_settings.dirty,
        "cancelled callback is obsolete"
    );
}

#[test]
fn new_command_ids_are_not_reused_and_rejected_save_keeps_explicit_unsaved_preference() {
    let mut p = setup();
    p.new_user_command_setting();
    let first = p.user_command_settings.selected.clone();
    p.cancel_user_command_setting();
    p.new_user_command_setting();
    assert_ne!(first, p.user_command_settings.selected);
    let token = p.user_command_settings.token();
    let mut form = p.user_command_settings_form().unwrap();
    form.executable = std::env::temp_dir()
        .join("tool.exe")
        .to_string_lossy()
        .into_owned();
    p.validate_user_command_setting(&token, &form);
    assert!(p.user_command_settings.can_apply);
    assert!(p.apply_user_command_setting(&token, &form, &|_| false));
    assert!(p.preferences.unsaved);
    assert_eq!(p.preferences.values.user_commands.len(), 3);
}

#[test]
fn unsaved_preview_uses_exact_validated_configuration_without_saving_or_executing() {
    let mut p = setup();
    let before = p.preferences.values.user_commands.clone();
    let token = p.user_command_settings.token();
    let mut form = p.user_command_settings_form().unwrap();
    form.name = "Unsaved preview".into();
    form.enabled = false;
    let commands = RefCell::new(Vec::new());
    assert!(p.preview_user_command_setting(&token, &form, &|command| {
        commands.borrow_mut().push(command);
        true
    }));
    assert!(Arc::ptr_eq(&before, &p.preferences.values.user_commands));
    let sent = commands.borrow();
    let request = sent
        .iter()
        .find_map(|command| match command {
            Command::PreviewUserCommand(request) => Some(request),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.configuration[0].name, "Unsaved preview");
    assert!(!request.configuration[0].enabled);
    assert!(sent.iter().all(|command| !matches!(
        command,
        Command::ExecuteUserCommand(_) | Command::SavePreferences { .. }
    )));
}

#[test]
fn keyboard_editor_uses_dynamic_catalog_and_invalidates_on_config_without_binding_changes() {
    let mut p = setup();
    let mut values = p.preferences.values.clone();
    p.update_preferences(values.clone(), &|_| true);
    assert!(
        p.keyboard
            .snapshot
            .map
            .bindings("user-command.first")
            .is_some()
    );
    let first = p.keyboard.snapshot.clone();
    let position = p
        .keyboard
        .snapshot
        .map
        .definitions()
        .iter()
        .position(|definition| definition.id == "user-command.first")
        .unwrap();
    p.select_shortcut(position as i32);
    let mut commands = values.user_commands.as_ref().clone();
    commands.reverse();
    commands[1].name = "Renamed action".into();
    values.replace_user_commands(Arc::new(commands)).unwrap();
    p.update_preferences(values, &|_| true);
    assert!(!Arc::ptr_eq(&first, &p.keyboard.snapshot));
    let selected = &p.keyboard.snapshot.map.definitions()[p.keyboard.selected];
    assert_eq!(selected.id, "user-command.first");
    assert_eq!(selected.label, "Renamed action");
}

#[test]
fn loaded_dynamic_command_can_assign_and_remove_alias_without_changing_other_commands() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
        user_commands: Arc::new(vec![command("first", "Command 文档")]),
        ..Default::default()
    }))));
    let index = p
        .keyboard
        .snapshot
        .map
        .definitions()
        .iter()
        .position(|action| action.id == "user-command.first")
        .unwrap();
    p.select_shortcut(index as i32);
    p.capture_editor_shortcut(ShortcutChord::parse("Ctrl+Alt+Y").unwrap());
    assert!(p.inspect_editor_shortcut().can_assign);
    assert!(p.assign_editor_shortcut("user-command.first", "Ctrl+Alt+Y", &|_| true));
    let map = &p.keyboard.snapshot.map;
    assert_eq!(
        map.resolve(
            &ShortcutChord::parse("Ctrl+Alt+Y").unwrap(),
            ShortcutContext::Files
        )
        .unwrap()
        .id,
        "user-command.first"
    );
    assert!(
        map.resolve(
            &ShortcutChord::parse("Ctrl+Alt+Y").unwrap(),
            ShortcutContext::Search
        )
        .is_none()
    );
    assert!(p.remove_editor_shortcut("user-command.first", "Ctrl+Alt+Y", &|_| true));
    assert_eq!(p.keyboard.snapshot.editor_text("user-command.first"), "");
    assert_eq!(p.keyboard.snapshot.editor_text("files.copy"), "Ctrl+C");
}

#[test]
fn deleting_selected_dynamic_action_clears_candidate_and_rejects_stale_assignment() {
    let mut p = setup();
    p.update_preferences(p.preferences.values.clone(), &|_| true);
    let index = p
        .keyboard
        .snapshot
        .map
        .definitions()
        .iter()
        .position(|action| action.id == "user-command.first")
        .unwrap();
    p.select_shortcut(index as i32);
    p.capture_editor_shortcut(ShortcutChord::parse("Ctrl+Alt+Y").unwrap());
    let mut values = p.preferences.values.clone();
    values
        .replace_user_commands(Arc::new(vec![values.user_commands[1].clone()]))
        .unwrap();
    p.update_preferences(values, &|_| true);
    assert!(p.keyboard.editor.candidate.is_none());
    assert!(
        p.keyboard
            .snapshot
            .map
            .bindings("user-command.first")
            .is_none()
    );
    assert!(
        !p.assign_editor_shortcut("user-command.first", "Ctrl+Alt+Y", &|_| panic!(
            "stale binding saved"
        ))
    );
}

#[test]
fn command_drag_commits_once_preserves_id_and_no_op_or_replayed_drop_does_not_save() {
    let mut p = setup();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let token = p.begin_user_command_drag("first");
    assert!(!token.is_empty());
    assert!(p.finish_user_command_drag(&token, 2, true, &send));
    assert_eq!(p.preferences.values.user_commands[0].id.as_str(), "second");
    assert_eq!(p.preferences.values.user_commands[1].id.as_str(), "first");
    assert_eq!(
        p.user_command_settings.selected.as_ref().unwrap().as_str(),
        "first"
    );
    assert_eq!(
        sent.borrow()
            .iter()
            .filter(|c| matches!(c, Command::SavePreferences { .. }))
            .count(),
        1
    );
    assert!(!p.finish_user_command_drag(&token, 0, true, &send));
    let token = p.begin_user_command_drag("first");
    assert!(p.finish_user_command_drag(&token, 2, true, &send));
    assert_eq!(sent.borrow().len(), 1, "same-position drop does not save");
}

#[test]
fn dirty_command_draft_refuses_drag_without_changing_selection_or_configuration() {
    let mut p = setup();
    let original = p.preferences.values.user_commands.clone();
    let token = p.user_command_settings.token();
    p.mark_user_command_setting_dirty(&token);
    assert!(p.begin_user_command_drag("second").is_empty());
    assert!(p.user_command_settings.dirty);
    assert!(p.user_command_settings.error.contains("draft"));
    assert_eq!(
        p.user_command_settings.selected.as_ref().unwrap().as_str(),
        "first"
    );
    assert!(Arc::ptr_eq(&original, &p.preferences.values.user_commands));
}

#[test]
fn cancelled_replaced_deleted_or_obsolete_command_drags_cannot_commit() {
    let mut p = setup();
    let send = |_: Command| panic!("stale command drag saved");
    let token = p.begin_user_command_drag("first");
    p.cancel_user_command_drag();
    assert!(!p.finish_user_command_drag(&token, 2, true, &send));
    let old = p.begin_user_command_drag("first");
    let current = p.begin_user_command_drag("second");
    assert!(!p.finish_user_command_drag(&old, 2, true, &send));
    assert_eq!(p.user_command_drag_token(), current);
    p.user_command_drag_cancel_handle().set(0);
    assert!(!p.finish_user_command_drag(&current, 0, true, &send));
    let token = p.begin_user_command_drag("first");
    p.preferences.values.user_commands =
        Arc::new(p.preferences.values.user_commands.as_ref().clone());
    assert!(
        !p.finish_user_command_drag(&token, 2, true, &send),
        "final validation checks the actual immutable configuration even before sync"
    );
    p.sync_user_command_settings();
    let token = p.begin_user_command_drag("first");
    p.preferences.values.user_commands =
        Arc::new(vec![p.preferences.values.user_commands[1].clone()]);
    p.sync_user_command_settings();
    assert!(!p.finish_user_command_drag(&token, 0, true, &send));
}
