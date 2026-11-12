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
fn settings_preview_never_requests_the_jobs_popup() {
    let mut p = setup();
    let token = p.user_command_settings.token();
    let form = p.user_command_settings_form().unwrap();
    assert!(p.preview_user_command_setting(&token, &form, &|command| {
        assert!(!matches!(
            command,
            Command::ExecuteUserCommand(_) | Command::SavePreferences { .. }
        ));
        true
    }));
    let job = p.command_launches.jobs.last().unwrap();
    p.apply(Event::UserCommand(Box::new(UserCommandEvent {
        request: job.id,
        command: job.command.clone(),
        update: UserCommandUpdate::Preview(Arc::new(UserCommandPreview {
            processes: Arc::from([]),
            retention: UserCommandRetention::new(()),
        })),
    })));
    assert!(p.command_launches.jobs.last().unwrap().finished);
    assert!(
        p.command_launches.preview_ready.is_none(),
        "settings previews must stay inside Options, never request a Jobs popup"
    );
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
fn settings_preview_tracks_the_exact_draft_and_ignores_late_errors() {
    for transition in ["edit", "select", "cancel"] {
        let mut p = setup();
        let token = p.user_command_settings.token();
        let mut form = p.user_command_settings_form().unwrap();
        assert!(p.preview_user_command_setting(&token, &form, &|_| true));
        let job = p.current_settings_command_preview().unwrap();
        let request = job.id;
        let command = job.command.clone();
        match transition {
            "edit" => {
                form.name = "A newer draft".into();
                p.remember_user_command_form(&token, form);
            }
            "select" => p.select_user_command_setting("second"),
            _ => p.cancel_user_command_setting(),
        }
        assert!(
            p.current_settings_command_preview().is_none(),
            "{transition}"
        );
        p.apply(Event::UserCommand(Box::new(UserCommandEvent {
            request,
            command,
            update: UserCommandUpdate::Rejected(FsError::from_io(
                "preview",
                Path::new("missing-tool"),
                std::io::ErrorKind::NotFound.into(),
            )),
        })));
        assert!(p.user_command_settings.error.is_empty(), "{transition}");
        assert!(p.desktop.notice.is_none(), "{transition}");
        assert!(p.command_launches.preview_ready.is_none(), "{transition}");
    }
}

#[test]
fn settings_preview_failures_stay_in_the_form_and_preserve_transport_details() {
    let mut p = setup();
    let token = p.user_command_settings.token();
    let form = p.user_command_settings_form().unwrap();
    assert!(!p.preview_user_command_setting(&token, &form, &|_| false));
    assert!(p.user_command_settings.error.contains("queue"));
    assert!(
        !p.user_command_settings
            .error
            .contains("application message")
    );
    assert!(p.desktop.notice.is_none());
    assert!(p.preview_user_command_setting(&token, &form, &|_| true));
    let job = p.current_settings_command_preview().unwrap();
    p.apply(Event::UserCommand(Box::new(UserCommandEvent {
        request: job.id,
        command: job.command.clone(),
        update: UserCommandUpdate::Rejected(FsError::from_io(
            "preview",
            Path::new("missing-tool"),
            std::io::ErrorKind::NotFound.into(),
        )),
    })));
    assert!(p.user_command_settings.error.contains("missing-tool"));
    assert!(p.desktop.notice.is_none());
    assert!(p.current_settings_command_preview().unwrap().finished);
}

#[test]
fn older_preview_failure_cannot_replace_a_newer_success_for_the_same_draft() {
    let mut p = setup();
    let token = p.user_command_settings.token();
    let form = p.user_command_settings_form().unwrap();
    assert!(p.preview_user_command_setting(&token, &form, &|_| true));
    let first = p.current_settings_command_preview().unwrap().id;
    assert!(p.preview_user_command_setting(&token, &form, &|_| true));
    let job = p.current_settings_command_preview().unwrap();
    let current = job.id;
    let command = job.command.clone();
    p.apply(Event::UserCommand(Box::new(UserCommandEvent {
        request: current,
        command: command.clone(),
        update: UserCommandUpdate::Preview(Arc::new(UserCommandPreview {
            processes: Arc::from([]),
            retention: UserCommandRetention::new(()),
        })),
    })));
    p.apply(Event::UserCommand(Box::new(UserCommandEvent {
        request: first,
        command,
        update: UserCommandUpdate::Rejected(FsError::from_io(
            "preview",
            Path::new("obsolete-tool"),
            std::io::ErrorKind::NotFound.into(),
        )),
    })));
    assert_eq!(p.current_settings_command_preview().unwrap().id, current);
    assert!(
        p.current_settings_command_preview()
            .unwrap()
            .preview
            .is_some()
    );
    assert!(
        p.user_command_settings.error.is_empty(),
        "obsolete failure reached current draft"
    );
    assert!(p.desktop.notice.is_none());
}

#[test]
fn settings_preview_selection_failures_remain_scoped_to_the_current_form() {
    for stale in [false, true] {
        let mut p = setup();
        let pane = p.active;
        p.panes[pane.index()].selection_revision = SelectionRevision(1);
        let token = p.user_command_settings.token();
        let form = p.user_command_settings_form().unwrap();
        assert!(p.preview_user_command_setting(&token, &form, &|_| true));
        assert!(p.selection_actions[pane.index()].is_some());
        if stale {
            p.cancel_user_command_setting();
        }
        p.dispatch_selection_actions(
            &Event::SelectionFailed {
                pane,
                navigation: NavigationId(p.panes[pane.index()].generation),
                revision: SelectionRevision(1),
                error: FsError::from_io(
                    "selection",
                    Path::new("fixture"),
                    std::io::ErrorKind::Interrupted.into(),
                ),
            },
            &|_| panic!("failed selection cannot launch a preview"),
        );
        assert!(
            p.desktop.notice.is_none(),
            "selection failure opened another popup"
        );
        assert_eq!(p.user_command_settings.error.is_empty(), stale);
        assert!(p.selection_actions[pane.index()].is_none());
    }
}

#[test]
fn edited_preview_waiting_for_selection_is_not_submitted_when_capture_finishes() {
    let mut p = setup();
    let pane = p.active;
    p.panes[pane.index()].selection_revision = SelectionRevision(1);
    let token = p.user_command_settings.token();
    let mut form = p.user_command_settings_form().unwrap();
    assert!(p.preview_user_command_setting(&token, &form, &|_| true));
    assert!(p.user_command_settings.status.contains("selection"));
    form.name = "Changed while waiting".into();
    p.remember_user_command_form(&token, form);
    assert!(p.user_command_settings.status.is_empty());
    p.dispatch_selection_actions(
        &Event::SelectionChanged {
            pane,
            navigation: NavigationId(p.panes[pane.index()].generation),
            revision: SelectionRevision(1),
            selection: p.panes[pane.index()].selected.clone(),
            deferred: Default::default(),
        },
        &|_| panic!("an edited draft's old capture cannot be submitted"),
    );
    assert!(p.selection_actions[pane.index()].is_none());
    assert!(p.command_launches.jobs.is_empty());
    assert!(p.desktop.notice.is_none());
}

#[test]
fn settings_preview_blocked_by_bulk_selection_reports_inside_options() {
    let mut p = setup();
    p.panes[p.active.index()].selection_pending_bulk = true;
    let token = p.user_command_settings.token();
    let form = p.user_command_settings_form().unwrap();
    assert!(!p.preview_user_command_setting(&token, &form, &|_| panic!("selection not ready")));
    assert!(p.user_command_settings.error.contains("selection"));
    assert!(p.desktop.notice.is_none());
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

#[test]
fn settings_waiting_status_is_removed_from_the_rendered_form_after_capture_or_draft_changes() {
    use slint::{
        ComponentHandle,
        platform::{
            Platform, PlatformError, WindowAdapter,
            software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        },
    };
    use std::rc::Rc;
    struct TestPlatform;
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
        }
    }
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    for transition in ["submit", "navigate", "select", "sync"] {
        let window = crate::MainWindow::new().unwrap();
        window.set_options_open(true);
        let mut p = setup();
        let pane = p.active;
        p.panes[pane.index()].selection_revision = SelectionRevision(1);
        let token = p.user_command_settings.token();
        let form = p.user_command_settings_form().unwrap();
        assert!(p.preview_user_command_setting(&token, &form, &|_| true));
        crate::user_command_settings_presentation::refresh(&window, &p);
        let view = window.global::<crate::UserCommandSettingsView>();
        assert!(view.get_status().contains("selection"), "{transition}");
        let waiting_revision = p.user_command_settings.revision;
        match transition {
            "submit" => {
                let sent = RefCell::new(Vec::new());
                p.dispatch_selection_actions(
                    &Event::SelectionChanged {
                        pane,
                        navigation: NavigationId(p.panes[pane.index()].generation),
                        revision: SelectionRevision(1),
                        selection: p.panes[pane.index()].selected.clone(),
                        deferred: Default::default(),
                    },
                    &|command| {
                        sent.borrow_mut().push(command);
                        true
                    },
                );
                assert!(matches!(
                    sent.borrow().as_slice(),
                    [Command::PreviewUserCommand(_)]
                ));
            }
            "navigate" => {
                assert!(p.navigate(
                    pane,
                    std::env::temp_dir().join("next preview folder"),
                    None,
                    &|_| true
                ));
                assert!(p.selection_actions[pane.index()].is_none());
            }
            "select" => p.select_user_command_setting("second"),
            _ => {
                p.preferences.values.user_commands =
                    Arc::new(p.preferences.values.user_commands.as_ref().clone());
                p.sync_user_command_settings();
            }
        }
        crate::user_command_settings_presentation::refresh(&window, &p);
        assert!(
            view.get_status().is_empty(),
            "{transition}: {}",
            view.get_status()
        );
        assert!(p.user_command_settings.status.is_empty(), "{transition}");
        assert!(
            p.user_command_settings.revision > waiting_revision,
            "{transition}: refresh cache was not invalidated"
        );
        assert!(p.desktop.notice.is_none(), "{transition}");
    }
}

#[test]
fn settings_waiting_status_survives_cancelled_older_or_unrelated_selection_actions() {
    for old_intent in ["preview", "properties"] {
        let mut p = setup();
        p.panes[PaneId::Left.index()].selection_revision = SelectionRevision(1);
        let token = p.user_command_settings.token();
        let form = p.user_command_settings_form().unwrap();
        if old_intent == "preview" {
            assert!(p.preview_user_command_setting(&token, &form, &|_| true));
        } else {
            p.request_selection_action(
                PaneId::Left,
                selection_actions::SelectionIntent::Properties,
                &|_| true,
            );
        }
        p.active = PaneId::Right;
        p.panes[PaneId::Right.index()].selection_revision = SelectionRevision(1);
        assert!(p.preview_user_command_setting(&token, &form, &|_| true));
        let revision = p.user_command_settings.revision;
        let status = p.user_command_settings.status.clone();
        assert!(status.contains("selection"));
        p.cancel_selection_actions(PaneId::Left);
        assert_eq!(p.user_command_settings.status, status, "{old_intent}");
        assert_eq!(p.user_command_settings.revision, revision, "{old_intent}");
        assert!(p.selection_actions[PaneId::Right.index()].is_some());
        p.cancel_selection_actions(PaneId::Right);
        assert!(p.user_command_settings.status.is_empty());
        assert!(p.user_command_settings.revision > revision);
        let cleared_revision = p.user_command_settings.revision;
        p.cancel_selection_actions(PaneId::Right);
        assert_eq!(p.user_command_settings.revision, cleared_revision);
        assert!(p.desktop.notice.is_none());
    }
}

#[test]
fn separator_settings_apply_preserves_identity_and_refuses_preview_without_dispatch() {
    let mut p = setup();
    let token = p.user_command_settings.token();
    let mut form = p.user_command_settings_form().unwrap();
    form.kind = 2;
    form.working_directory.clear();
    p.validate_user_command_setting(&token, &form);
    assert!(p.user_command_settings.can_apply);
    assert!(!p.user_command_settings.can_preview);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(!p.preview_user_command_setting(&token, &form, &send));
    assert!(sent.borrow().is_empty());
    assert!(p.apply_user_command_setting(&token, &form, &send));
    assert_eq!(p.preferences.values.user_commands[0].id.as_str(), "first");
    assert!(matches!(
        p.preferences.values.user_commands[0].invocation,
        UserCommandInvocation::Separator
    ));
    assert!(!p.user_command_settings.can_preview);
    let token = p.user_command_settings.token();
    assert!(p.move_user_command_setting(&token, 1, &send));
    assert_eq!(p.preferences.values.user_commands[1].id.as_str(), "first");
    assert!(sent.borrow().iter().all(|c| !matches!(
        c,
        Command::ExecuteUserCommand(_) | Command::PreviewUserCommand(_)
    )));
}
