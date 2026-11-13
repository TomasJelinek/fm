use super::*;
use std::cell::RefCell;

fn loaded() -> Presenter {
    let root = std::env::current_dir().unwrap();
    let mut p = Presenter::new(root.clone(), root);
    p.apply(Event::PreferencesLoaded(Ok(Box::default())));
    p
}
fn request(p: &mut Presenter, target: Target) -> DesktopRequestId {
    let sent = RefCell::new(Vec::new());
    p.request_settings_picker(target, PathPickerOwner::None, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    match sent.borrow().first().unwrap() {
        Command::PickPath { request, .. } => *request,
        _ => panic!(),
    }
}

const COMMAND_TARGETS: [Target; 4] = [
    Target::CommandProgram,
    Target::WorkingDirectory,
    Target::CommandIcon,
    Target::CommandIconResource,
];

#[test]
fn new_command_picker_cancel_keeps_pristine_draft_unchanged() {
    for target in COMMAND_TARGETS {
        let mut p = loaded();
        let saved = p.preferences.values.user_commands.clone();
        p.new_user_command_setting();
        let before = p.user_command_settings_form().unwrap();
        let revisions = (
            p.user_command_settings.revision,
            p.user_command_settings.draft_revision,
            p.user_command_settings.edit_revision,
        );
        let id = request(&mut p, target);
        p.settings_picker_context(9, &|_| panic!("current picker cancelled"));
        p.flush_settings_picker(&|_| panic!("current picker cancelled"));
        p.apply(Event::PathPicked {
            request: id,
            result: Ok(None),
        });
        assert!(!p.settings_picker.busy(), "{target:?}");
        assert!(p.user_command_settings.new, "{target:?}");
        assert!(p.user_command_settings.dirty, "{target:?}");
        assert!(!p.user_command_settings.can_apply, "{target:?}");
        assert!(!p.user_command_settings.can_preview, "{target:?}");
        assert!(p.user_command_settings.error.is_empty(), "{target:?}");
        assert!(p.user_command_settings.validation.is_empty(), "{target:?}");
        assert_eq!(
            (
                p.user_command_settings.revision,
                p.user_command_settings.draft_revision,
                p.user_command_settings.edit_revision,
            ),
            revisions,
            "{target:?}"
        );
        let after = p.user_command_settings_form().unwrap();
        assert_eq!(after.id, before.id, "{target:?}");
        assert_eq!(after.name, "New command", "{target:?}");
        assert!(after.executable.is_empty(), "{target:?}");
        assert!(after.icon.is_empty(), "{target:?}");
        assert_eq!(after.working_directory, "${active.path}", "{target:?}");
        assert!(Arc::ptr_eq(&saved, &p.preferences.values.user_commands));
    }
}

#[test]
fn command_picker_cancel_preserves_genuine_validation_and_submission_errors() {
    for target in COMMAND_TARGETS {
        let mut p = loaded();
        let saved = p.preferences.values.user_commands.clone();
        p.new_user_command_setting();
        let token = p.user_command_settings.token();
        let mut form = p.user_command_settings_form().unwrap();
        form.name.clear();
        p.remember_user_command_form(&token, form.clone());
        p.validate_user_command_setting(&token, &form);
        assert!(!p.user_command_settings.validation.is_empty());
        assert!(!p.apply_user_command_setting(&token, &form, &|_| panic!("invalid draft saved")));
        assert!(!p.user_command_settings.error.is_empty());
        let error = p.user_command_settings.error.clone();
        let validation = p.user_command_settings.validation.clone();
        let revision = p.user_command_settings.revision;
        let id = request(&mut p, target);
        p.apply(Event::PathPicked {
            request: id,
            result: Ok(None),
        });
        assert!(!p.settings_picker.busy(), "{target:?}");
        assert_eq!(p.user_command_settings.error, error, "{target:?}");
        assert_eq!(p.user_command_settings.validation, validation, "{target:?}");
        assert_eq!(p.user_command_settings.revision, revision, "{target:?}");
        assert!(!p.user_command_settings.can_apply, "{target:?}");
        assert!(Arc::ptr_eq(&saved, &p.preferences.values.user_commands));
    }
}

#[test]
fn current_command_picker_failure_is_visible_and_survives_cancel() {
    for target in COMMAND_TARGETS {
        let mut p = loaded();
        let saved = p.preferences.values.user_commands.clone();
        p.new_user_command_setting();
        let id = request(&mut p, target);
        p.apply(Event::PathPicked {
            request: id,
            result: Err(FsError::from_io(
                "pick command path",
                Path::new("picker-fixture"),
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Native picker refused access",
                ),
            )),
        });
        assert!(!p.settings_picker.busy(), "{target:?}");
        assert!(
            p.user_command_settings
                .error
                .contains("Native picker refused access")
        );
        assert!(p.user_command_settings.error.contains("picker-fixture"));
        assert!(p.user_command_settings.error.contains("pick command path"));
        assert!(p.user_command_settings.validation.is_empty(), "{target:?}");
        let error = p.user_command_settings.error.clone();
        let id = request(&mut p, target);
        p.settings_path_picked(id, Ok(None));
        assert_eq!(p.user_command_settings.error, error, "{target:?}");
        assert!(!p.user_command_settings.can_apply, "{target:?}");
        assert!(Arc::ptr_eq(&saved, &p.preferences.values.user_commands));
    }
}

#[test]
fn obsolete_command_picker_successes_and_failures_cannot_change_the_current_draft() {
    for target in COMMAND_TARGETS {
        for transition in [
            "edit",
            "discard",
            "configuration",
            "context",
            "close",
            "foreign",
        ] {
            for failure in [false, true] {
                let mut p = loaded();
                p.new_user_command_setting();
                let id = request(&mut p, target);
                match transition {
                    "edit" => {
                        let token = p.user_command_settings.token();
                        let mut form = p.user_command_settings_form().unwrap();
                        form.name = "Changed while browsing".into();
                        p.remember_user_command_form(&token, form);
                    }
                    "discard" => {
                        p.cancel_user_command_setting();
                        p.new_user_command_setting();
                    }
                    "configuration" => {
                        p.preferences.values.user_commands =
                            Arc::new(p.preferences.values.user_commands.as_ref().clone());
                    }
                    "context" => p.settings_picker_context(-1, &|_| false),
                    "close" => p.operations.closing = true,
                    _ => {}
                }
                let before = p.user_command_settings_form().unwrap();
                let error = p.user_command_settings.error.clone();
                let validation = p.user_command_settings.validation.clone();
                let revisions = (
                    p.user_command_settings.revision,
                    p.user_command_settings.draft_revision,
                    p.user_command_settings.edit_revision,
                );
                let saved = p.preferences.values.user_commands.clone();
                let result = if failure {
                    Err(FsError::from_io(
                        "obsolete picker",
                        Path::new("obsolete-picker-fixture"),
                        std::io::ErrorKind::PermissionDenied.into(),
                    ))
                } else {
                    Ok(Some(std::env::temp_dir().join("obsolete-picked-path.exe")))
                };
                p.apply(Event::PathPicked {
                    request: if transition == "foreign" {
                        DesktopRequestId(id.0 + 1)
                    } else {
                        id
                    },
                    result,
                });
                assert_eq!(p.settings_picker.busy(), transition == "foreign");
                let after = p.user_command_settings_form().unwrap();
                assert_eq!(after.id, before.id, "{target:?}: {transition}");
                assert_eq!(after.name, before.name, "{target:?}: {transition}");
                assert_eq!(
                    after.executable, before.executable,
                    "{target:?}: {transition}"
                );
                assert_eq!(after.icon, before.icon, "{target:?}: {transition}");
                assert_eq!(after.working_directory, before.working_directory);
                assert_eq!(
                    p.user_command_settings.error, error,
                    "{target:?}: {transition}"
                );
                assert_eq!(p.user_command_settings.validation, validation);
                assert_eq!(
                    (
                        p.user_command_settings.revision,
                        p.user_command_settings.draft_revision,
                        p.user_command_settings.edit_revision,
                    ),
                    revisions,
                    "{target:?}: {transition}"
                );
                assert!(Arc::ptr_eq(&saved, &p.preferences.values.user_commands));
            }
        }
    }
}

#[test]
fn text_edits_close_and_foreign_results_cannot_replace_a_draft() {
    let mut p = loaded();
    let id = request(&mut p, Target::Editor);
    p.settings_path_picked(
        DesktopRequestId(id.0 + 1),
        Ok(Some(std::env::current_exe().unwrap())),
    );
    assert!(p.settings_picker.busy());
    p.edit_editor_draft("changed while picker was open".into(), false);
    p.settings_path_picked(id, Ok(Some(std::env::current_exe().unwrap())));
    assert_eq!(p.editor_draft().executable, "changed while picker was open");
    let id = request(&mut p, Target::Editor);
    p.settings_picker_context(-1, &|_| false);
    assert!(
        p.settings_picker.busy(),
        "full queue retains bounded cancellation"
    );
    p.settings_path_picked(id, Ok(Some(std::env::current_exe().unwrap())));
    assert_eq!(p.editor_draft().executable, "changed while picker was open");
}
#[test]
fn cancel_is_noop_and_raw_pick_is_saved_without_display_roundtrip() {
    let mut p = loaded();
    let id = request(&mut p, Target::Editor);
    p.settings_path_picked(id, Ok(None));
    assert!(!p.editor.dirty);
    let path = raw_path();
    let id = request(&mut p, Target::Editor);
    p.settings_path_picked(id, Ok(Some(path.clone())));
    assert!(p.editor.dirty);
    p.save_editor_draft(&|_| true);
    assert_eq!(p.preferences.values.editor.executable, Some(path));
}
#[test]
fn refused_picker_admission_never_leaves_a_busy_draft() {
    let mut p = loaded();
    p.request_settings_picker(Target::Editor, PathPickerOwner::None, &|_| false);
    assert!(!p.settings_picker.busy());
    assert!(!p.editor.error.is_empty());
    assert!(!p.editor.dirty);
}
#[test]
fn command_program_and_icon_picks_keep_raw_paths_and_stale_configuration_rejects_output() {
    let mut p = loaded();
    p.preferences.values.user_commands = Arc::new(vec![UserCommand {
        id: UserCommandId::new("test").unwrap(),
        name: "Test".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec![],
        },
    }]);
    p.sync_user_command_settings();
    let path = raw_path();
    let id = request(&mut p, Target::CommandProgram);
    p.settings_path_picked(id, Ok(Some(path.clone())));
    let id = request(&mut p, Target::CommandIcon);
    p.settings_path_picked(id, Ok(Some(path.clone())));
    let token = p.user_command_settings.token();
    let form = p.user_command_settings_form().unwrap();
    assert!(p.apply_user_command_setting(&token, &form, &|_| true));
    assert_eq!(
        p.preferences.values.user_commands[0].icon,
        Some(path.clone())
    );
    let UserCommandInvocation::Program { executable, .. } =
        &p.preferences.values.user_commands[0].invocation
    else {
        panic!()
    };
    assert_eq!(executable, &path);
    let id = request(&mut p, Target::CommandProgram);
    p.preferences.values.user_commands =
        Arc::new(p.preferences.values.user_commands.as_ref().clone());
    p.settings_path_picked(id, Ok(Some(std::env::current_exe().unwrap())));
    assert!(!p.user_command_settings.dirty);
}
#[test]
fn accepted_cancel_releases_pending_capture_and_late_reply_has_no_effect() {
    let mut p = loaded();
    let id = request(&mut p, Target::Editor);
    p.settings_picker_context(-1, &|_| false);
    assert!(p.settings_picker.busy());
    p.flush_settings_picker(&|command| {
        assert!(matches!(command, Command::CancelPathPicker { .. }));
        true
    });
    assert!(!p.settings_picker.busy());
    p.settings_path_picked(id, Ok(Some(raw_path())));
    assert!(!p.editor.dirty);
}
#[cfg(windows)]
fn raw_path() -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let mut text = std::env::current_dir()
        .unwrap()
        .as_os_str()
        .encode_wide()
        .collect::<Vec<_>>();
    text.extend([
        b'\\' as u16,
        0xd800,
        b'.' as u16,
        b'e' as u16,
        b'x' as u16,
        b'e' as u16,
    ]);
    std::ffi::OsString::from_wide(&text).into()
}
#[cfg(unix)]
fn raw_path() -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let mut text = std::env::current_dir()
        .unwrap()
        .as_os_str()
        .as_encoded_bytes()
        .to_vec();
    text.extend(b"/raw-\xff");
    std::ffi::OsString::from_vec(text).into()
}

#[test]
fn selected_rule_program_and_directory_keep_raw_paths_and_reject_late_selection() {
    let mut p = loaded();
    p.editor_rule_action(0);
    let path = raw_path();
    let id = request(&mut p, Target::Editor);
    p.settings_path_picked(id, Ok(Some(path.clone())));
    let id = request(&mut p, Target::EditorWorkingDirectory);
    p.settings_path_picked(id, Ok(Some(path.clone())));
    p.save_editor_draft(&|_| true);
    assert_eq!(
        p.preferences.values.editor.rules[0].executable,
        Some(path.clone())
    );
    assert_eq!(
        p.preferences.values.editor.rules[0].working_directory,
        Some(path.clone())
    );
    p.select_editor_rule(0);
    let id = request(&mut p, Target::EditorWorkingDirectory);
    p.editor_rule_action(0);
    p.settings_path_picked(id, Ok(Some(std::env::current_dir().unwrap())));
    assert!(
        p.editor_draft()
            .selected_form()
            .native_working_directory
            .is_none()
    );
    assert_eq!(
        p.editor_draft().rules[0].native_working_directory,
        Some(path)
    );
}

#[test]
fn command_icon_resource_uses_native_picker_and_keeps_original_draft_guards() {
    let mut p = loaded();
    p.new_user_command_setting();
    let sent = RefCell::new(Vec::new());
    p.request_settings_picker(
        Target::CommandIconResource,
        PathPickerOwner::None,
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let id = match &sent.borrow()[0] {
        Command::PickPath { request, input } => {
            assert_eq!(input.kind, PathPickerKind::CommandIconResource);
            assert!(input.initial_directory.is_none());
            *request
        }
        _ => panic!(),
    };
    let path = raw_path();
    p.settings_path_picked(id, Ok(Some(path.clone())));
    assert_eq!(p.user_command_settings.picked_icon, Some(path));
    let id = request(&mut p, Target::CommandIconResource);
    let token = p.user_command_settings.token();
    let mut form = p.user_command_settings_form().unwrap();
    form.icon = "changed while chooser is open".into();
    form.icon_edited = true;
    p.remember_user_command_form(&token, form);
    p.settings_path_picked(id, Ok(Some(raw_path())));
    assert_eq!(
        p.user_command_settings_form().unwrap().icon,
        "changed while chooser is open"
    );
}
