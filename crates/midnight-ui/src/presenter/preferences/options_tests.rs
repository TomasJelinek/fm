use super::*;
use std::{cell::RefCell, io};

fn loaded() -> Presenter {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(UserPreferences::default()));
    p
}

#[test]
fn verification_preview_cancel_and_ok_only_affect_new_jobs_after_commit() {
    let mut p = loaded();
    assert_eq!(p.transfer_verification(), TransferVerification::FullSha256);
    p.begin_options();
    p.change_copy_verification(false, &|_| panic!("preview cannot save"));
    assert_eq!(p.transfer_verification(), TransferVerification::FullSha256);
    p.cancel_options(&|_| panic!("cancel cannot save"));
    assert!(p.preferences.values.copy_checksum_verification);
    p.begin_options();
    p.change_copy_verification(false, &|_| panic!("preview cannot save"));
    let commands = RefCell::new(Vec::new());
    p.accept_options(&|command| {
        commands.borrow_mut().push(command);
        true
    });
    let writes = saved(&commands.borrow());
    assert!(!writes[0].1.copy_checksum_verification);
    p.preferences_saved(writes[0].0, Ok(()));
    assert_eq!(
        p.transfer_verification(),
        TransferVerification::SizeAndSamples
    );
    p.begin_options();
    p.reset_options(&|_| panic!("reset is only preview"));
    assert!(p.preferences.values.copy_checksum_verification);
    assert_eq!(
        p.transfer_verification(),
        TransferVerification::SizeAndSamples
    );
    p.cancel_options(&|_| panic!("cancel cannot save"));
    assert!(!p.preferences.values.copy_checksum_verification);
}

fn saved(commands: &[Command]) -> Vec<(PreferencesRevision, UserPreferences)> {
    commands
        .iter()
        .filter_map(|command| match command {
            Command::SavePreferences {
                revision,
                preferences,
            } => Some((*revision, *preferences.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn archive_preview_draft_stays_out_of_session_saves_and_cancel_restores_it() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.begin_options();
    let mut draft = p.preferences.values.clone();
    draft.thumbnails_archives = false;
    p.update_preferences(draft, &send);
    assert!(!p.preferences.values.thumbnails_archives);
    assert!(
        saved(&commands.borrow()).is_empty(),
        "archive preview is an Options draft"
    );
    p.change_panel_view_mode(PaneId::Left, 1, &send);
    let writes = saved(&commands.borrow());
    assert_eq!(writes.len(), 1);
    assert!(
        writes[0].1.thumbnails_archives,
        "unrelated session saves retain the committed archive policy"
    );
    p.preferences_saved(writes[0].0, Ok(()));
    p.cancel_options(&send);
    assert!(p.preferences.values.thumbnails_archives);
    assert_eq!(saved(&commands.borrow()).len(), 1, "Cancel must not save");
}

#[test]
fn cancelling_restores_all_options_and_runtime_shortcuts_without_saving() {
    let mut p = loaded();
    let baseline = p.preferences.values.clone();
    p.begin_options();
    let mut draft = baseline.clone();
    draft.theme = PreferenceTheme::from_index(1).unwrap();
    draft.density = UiDensity::Touch;
    draft.font_family = "Consolas".into();
    draft.desktop_font_size = 19;
    draft.touch_font_size = 23;
    draft.quick_search_filter = true;
    draft.size_display = SizeDisplay::Bytes;
    draft.remember_command_history = false;
    draft.copy_checksum_verification = false;
    draft.thumbnails_enabled = false;
    draft.thumbnails_network = true;
    draft.thumbnails_archives = false;
    draft.thumbnail_size = 192;
    draft.disabled_viewer_plugins = vec!["midnight-viewer-source".into()];
    draft
        .archive_association_overrides
        .insert("zip".into(), false);
    draft
        .archive_custom_extensions
        .insert("bundle".into(), ArchiveFormat::Zip);
    draft.filename_colors = Arc::new(Vec::new());
    draft.shortcuts = vec![ShortcutBinding {
        action: "files.rename".into(),
        keys: vec!["Ctrl+Alt+R".into()],
    }];
    p.update_preferences(draft, &|_| panic!("preview must not persist"));
    assert_eq!(p.shortcut_key("files.rename"), "Ctrl+Alt+R");
    assert!(!p.preferences.unsaved);
    assert!(p.pending_preferences_save().is_none());
    p.cancel_options(&|_| panic!("cancel must not persist"));
    assert_eq!(p.preferences.values, baseline);
    assert_eq!(p.shortcut_key("files.rename"), "F2");
    assert!(!p.preferences.options.open);
}

#[test]
fn automatic_session_saves_and_shutdown_never_include_preview_options() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.begin_options();
    p.change_preferences(1, 1, 0, &send);
    p.change_panel_view_mode(PaneId::Left, 1, &send);
    let writes = saved(&commands.borrow());
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].1.theme, PreferenceTheme::default());
    assert_eq!(writes[0].1.density, UiDensity::Desktop);
    assert_eq!(writes[0].1.panel_view_modes[0], PanelViewMode::Brief);
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.theme, PreferenceTheme::default());
    p.preferences_saved(writes[0].0, Ok(()));
    p.cancel_options(&send);
    assert_eq!(
        p.preferences.values.panel_view_modes[0],
        PanelViewMode::Brief
    );
    assert_eq!(saved(&commands.borrow()).len(), 1);
}

#[test]
fn cancelled_history_clear_preserves_commands_completed_during_options() {
    let mut p = loaded();
    let old = CommandHistoryEntry {
        command: "old".into(),
        shell: ShellFlavor::PowerShell,
    };
    let new = CommandHistoryEntry {
        command: "new".into(),
        shell: ShellFlavor::PowerShell,
    };
    p.preferences.values.command_history = vec![old.clone()];
    p.begin_options();
    p.clear_command_history(&|_| panic!("clear is staged"));
    p.record_command_history(new.clone());
    assert_eq!(p.preferences.values.command_history, vec![new.clone()]);
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.command_history, vec![new.clone(), old.clone()]);
    p.cancel_options(&|_| panic!("cancel doesn't save"));
    assert_eq!(p.preferences.values.command_history, vec![new, old]);
}

#[test]
fn failed_ok_keeps_transaction_open_retry_is_new_and_only_success_closes() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.begin_options();
    p.change_preferences(1, 0, 0, &send);
    p.accept_options(&send);
    let first = saved(&commands.borrow())[0].0;
    p.cancel_options(&send);
    assert!(
        p.preferences.options.open,
        "can't undo a write already accepted by OK"
    );
    p.preferences_saved(
        first,
        Err(FsError::from_io(
            "save",
            Path::new("preferences"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        )),
    );
    assert!(p.preferences.options.open);
    assert!(!p.preferences.options.saving);
    p.accept_options(&send);
    let second = saved(&commands.borrow())[1].0;
    assert!(second > first);
    p.preferences_saved(first, Ok(()));
    assert!(
        p.preferences.options.open,
        "old acknowledgement cannot close retry"
    );
    p.preferences_saved(second, Ok(()));
    assert!(!p.preferences.options.open);
    assert_eq!(p.preferences.values.theme.index(), 1);
}

#[test]
fn ok_acknowledgement_closes_even_when_a_newer_session_save_is_pending() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.begin_options();
    p.change_preferences(1, 0, 0, &send);
    p.accept_options(&send);
    let commit = saved(&commands.borrow())[0].0;
    p.change_panel_view_mode(PaneId::Left, 1, &send);
    assert!(p.preferences.revision > commit);
    p.preferences_saved(commit, Ok(()));
    assert!(!p.preferences.options.open);
    assert!(
        p.preferences.saving,
        "the later session write is still pending"
    );
    assert_eq!(p.preferences.values.theme.index(), 1);
}

#[test]
fn failed_admission_can_be_cancelled_and_initial_load_becomes_the_baseline() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.begin_options();
    p.accept_options(&|_| panic!("must wait for initial load"));
    let saved_theme = PreferenceTheme::from_index(2).unwrap();
    p.preferences_loaded(Ok(UserPreferences {
        theme: saved_theme,
        ..Default::default()
    }));
    p.change_preferences(1, 0, 0, &|_| panic!("preview"));
    p.accept_options(&|_| false);
    assert!(p.preferences.options.open);
    assert!(!p.preferences.options.saving);
    p.cancel_options(&|_| panic!("cancel"));
    assert_eq!(p.preferences.values.theme, saved_theme);
}

#[test]
fn failed_ok_unlocks_when_a_newer_session_save_was_not_admitted() {
    let mut p = loaded();
    p.begin_options();
    p.change_preferences(1, 0, 0, &|_| true);
    p.accept_options(&|_| true);
    let commit = p.preferences.revision;
    p.change_panel_view_mode(PaneId::Left, 1, &|_| false);
    assert!(p.preferences.revision > commit);
    p.preferences_saved(commit, Err(save_failure()));
    assert!(p.preferences.options.open);
    assert!(!p.preferences.options.saving);
    assert!(!p.preferences.saving);
    assert!(!p.preferences.save_error.is_empty());
    p.cancel_options(&|_| panic!("cancel must not save"));
    assert!(!p.preferences.options.open);
    assert_eq!(p.preferences.values.theme, PreferenceTheme::default());
    assert_eq!(
        p.preferences.values.panel_view_modes[0],
        PanelViewMode::Brief
    );
    let Command::SavePreferences { preferences, .. } = p.pending_preferences_save().unwrap() else {
        panic!()
    };
    assert_eq!(preferences.theme, PreferenceTheme::default());
}

#[test]
fn failed_ok_waits_only_for_admitted_successors_then_allows_retry() {
    let mut p = loaded();
    p.begin_options();
    p.change_preferences(1, 0, 0, &|_| true);
    p.accept_options(&|_| true);
    let commit = p.preferences.revision;
    p.change_panel_view_mode(PaneId::Left, 1, &|_| true);
    let admitted = p.preferences.revision;
    p.change_panel_view_mode(PaneId::Right, 1, &|_| false);
    p.preferences_saved(commit, Err(save_failure()));
    assert!(p.preferences.options.saving);
    p.cancel_options(&|_| panic!("a write containing Options is still pending"));
    assert!(p.preferences.options.open);
    p.preferences_saved(admitted, Err(save_failure()));
    assert!(!p.preferences.options.saving);
    p.accept_options(&|_| true);
    let retry = p.preferences.revision;
    assert!(retry > admitted);
    p.preferences_saved(admitted, Ok(()));
    assert!(
        p.preferences.options.open,
        "stale result cannot complete retry"
    );
    p.preferences_saved(retry, Ok(()));
    assert!(!p.preferences.options.open);
}

#[test]
fn rejected_ok_is_not_kept_locked_by_an_older_session_save() {
    let mut p = loaded();
    p.begin_options();
    p.change_panel_view_mode(PaneId::Left, 1, &|_| true);
    let older = p.preferences.revision;
    p.change_preferences(1, 0, 0, &|_| true);
    p.accept_options(&|_| false);
    assert!(!p.preferences.options.saving);
    p.cancel_options(&|_| panic!("cancel must not save"));
    p.preferences_saved(older, Ok(()));
    assert!(!p.preferences.options.open);
    assert_eq!(p.preferences.values.theme, PreferenceTheme::default());
}

fn save_failure() -> FsError {
    FsError::from_io(
        "save",
        Path::new("preferences"),
        io::Error::from(io::ErrorKind::PermissionDenied),
    )
}

#[test]
fn cancelling_discards_forms_and_late_color_validation() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.begin_options();
    p.edit_editor_draft("unfinished".into(), false);
    p.new_user_command_setting();
    p.validate_colors(Vec::new(), &send);
    let request = commands
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    p.cancel_options(&send);
    p.apply(Event::FilenameColorsValidated {
        request,
        result: Ok(()),
    });
    p.flush_filename_colors(&send);
    assert!(!p.editor.dirty);
    assert!(!p.user_command_settings.dirty);
    assert!(!p.colors.editor.dirty);
    assert!(saved(&commands.borrow()).is_empty());
    assert_eq!(
        p.preferences.values.filename_colors,
        UserPreferences::default().filename_colors
    );
}

#[test]
fn reset_preserves_user_content_and_commits_defaults_only_after_ok() {
    let mut p = loaded();
    p.preferences.values.tabs[0].paths = vec![std::env::temp_dir().join("saved tab 文档")];
    p.preferences.values.command_history = vec![CommandHistoryEntry {
        command: "echo preserved 文档".into(),
        shell: ShellFlavor::PowerShell,
    }];
    p.preferences.values.bookmarks = vec![Bookmark {
        network: None,
        id: BookmarkId(1),
        name: "Saved".into(),
        path: std::env::temp_dir().join("saved hot path"),
        archive: None,
        slot: Some(3),
    }];
    p.preferences.values.user_commands = Arc::new(vec![UserCommand {
        id: UserCommandId::new("preserved").unwrap(),
        name: "Preserved command".into(),
        enabled: true,
        toolbar_label: Some("Run".into()),
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::temp_dir().join("fixture-tool"),
            arguments: vec!["${file.name}".into()],
        },
    }]);
    p.preferences.values.validate().unwrap();
    let preserved = p.preferences.values.clone();
    let mut custom = preserved.clone();
    custom.theme = PreferenceTheme::from_index(1).unwrap();
    custom.font_family = "Consolas".into();
    custom.thumbnails_enabled = false;
    custom.thumbnails_archives = false;
    custom.thumbnail_size = 256;
    custom.remember_command_history = false;
    custom.filename_colors = Arc::new(Vec::new());
    custom.disabled_viewer_plugins = vec!["midnight-viewer-source".into()];
    custom
        .archive_custom_extensions
        .insert("bundle".into(), ArchiveFormat::Zip);
    custom.shortcuts = vec![ShortcutBinding {
        action: "files.rename".into(),
        keys: vec!["Ctrl+Alt+R".into()],
    }];
    p.preferences.values = custom.clone();
    p.preferences.values.validate().unwrap();
    p.begin_options();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.reset_options(&send);
    assert_eq!(p.preferences.values, preserved);
    assert_eq!(p.shortcut_key("files.rename"), "F2");
    assert!(saved(&commands.borrow()).is_empty());
    p.cancel_options(&send);
    assert_eq!(p.preferences.values, custom);
    assert_eq!(p.shortcut_key("files.rename"), "Ctrl+Alt+R");
    p.begin_options();
    p.reset_options(&send);
    p.accept_options(&send);
    let writes = saved(&commands.borrow());
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].1, preserved);
    p.preferences_saved(writes[0].0, Ok(()));
    assert!(!p.preferences.options.open);
}

#[test]
fn reset_discards_editor_and_late_color_validation_but_keeps_command_draft() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.begin_options();
    p.edit_editor_draft("unfinished".into(), false);
    p.new_user_command_setting();
    let token = p.user_command_settings.token();
    p.validate_colors(Vec::new(), &send);
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    p.reset_options(&send);
    p.apply(Event::FilenameColorsValidated {
        request,
        result: Ok(()),
    });
    p.flush_filename_colors(&send);
    assert!(!p.editor.dirty);
    assert!(!p.colors.editor.dirty);
    assert!(p.user_command_settings.dirty);
    assert_eq!(p.user_command_settings.token(), token);
    assert_eq!(p.preferences.values, UserPreferences::default());
    assert!(saved(&commands.borrow()).is_empty());
}

#[test]
fn reset_cannot_race_loading_or_an_accepted_settings_write() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.begin_options();
    p.reset_options(&|_| panic!("reset before loading must do nothing"));
    p.preferences_loaded(Ok(UserPreferences {
        theme: PreferenceTheme::from_index(1).unwrap(),
        ..Default::default()
    }));
    p.accept_options(&|_| true);
    let accepted = p.preferences.values.clone();
    p.reset_options(&|_| panic!("reset during save must do nothing"));
    assert_eq!(p.preferences.values, accepted);
    assert!(p.preferences.options.saving);
}

#[test]
fn reset_waits_for_an_open_native_picker_without_discarding_its_result() {
    let mut p = loaded();
    p.preferences.values.theme = PreferenceTheme::from_index(1).unwrap();
    p.begin_options();
    let request = RefCell::new(None);
    p.request_settings_picker(
        settings_picker::Target::Editor,
        PathPickerOwner::None,
        &|command| {
            if let Command::PickPath { request: id, .. } = command {
                *request.borrow_mut() = Some(id);
            }
            true
        },
    );
    assert!(p.settings_picker.busy());
    p.reset_options(&|_| panic!("reset cannot interfere with an open picker"));
    assert_eq!(p.preferences.values.theme.index(), 1);
    let selected = std::env::temp_dir().join("editor fixture");
    p.settings_path_picked(request.borrow().unwrap(), Ok(Some(selected.clone())));
    assert_eq!(p.editor_draft().native, Some(selected));
    p.reset_options(&|_| panic!("reset only stages defaults"));
    assert!(!p.editor.dirty);
    assert_eq!(p.preferences.values.theme, PreferenceTheme::default());
}
