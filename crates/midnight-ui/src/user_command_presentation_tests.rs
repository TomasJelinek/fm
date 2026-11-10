use super::*;
use std::path::PathBuf;

fn configured(count: usize) -> Presenter {
    let directory = std::env::temp_dir().join("command artwork 文档");
    let mut p = Presenter::new(directory.clone(), directory.clone());
    p.preferences.values.user_commands = Arc::new(
        (0..count)
            .map(|index| UserCommand {
                id: UserCommandId::new(format!("command-{index}")).unwrap(),
                name: format!("Command {index}"),
                enabled: true,
                toolbar_label: None,
                icon: Some(directory.join(format!("image-{index}.png"))),
                working_directory: "${active.path}".into(),
                launch: UserCommandLaunch::Single,
                invocation: UserCommandInvocation::Program {
                    executable: directory.join("tool"),
                    arguments: vec![],
                },
            })
            .collect(),
    );
    p.command_toolbar.sync(&p.preferences.values.user_commands);
    p
}

#[test]
fn only_visible_current_command_icons_enter_the_bounded_decoder_request() {
    let mut p = configured(MAX_USER_COMMANDS);
    p.preferences.values.thumbnails_enabled = false;
    let revision = p.command_toolbar.revision.get().to_string();
    assert!(
        p.thumbnail_targets([(0, usize::MAX); 2], [""; 2])
            .is_empty()
    );
    for index in 0..MAX_USER_COMMANDS {
        p.command_toolbar
            .visible
            .borrow_mut()
            .insert((revision.clone(), format!("command-{index}")));
    }
    let targets = p.thumbnail_targets([(0, usize::MAX); 2], [""; 2]);
    assert_eq!(targets.len(), MAX_USER_COMMANDS);
    assert!(
        targets
            .iter()
            .all(|target| !target.selected && !target.hovered)
    );
    let identity = &targets[0].identity;
    assert!(p.receive_thumbnail(identity, 0, 1, 1, &[10, 20, 30, 255]));
    assert!(p.custom_command_icon(&revision, "command-0").is_some());
    assert!(!p.receive_thumbnail(identity, 6, 1, 1, &[10, 20, 30, 255]));
    assert!(!p.receive_thumbnail(identity, 0, 97, 1, &[0; 388]));
    assert_eq!(
        targets[0].edge, 96,
        "command artwork keeps its small request"
    );
    assert!(p.receive_thumbnail_at_resolution(identity, 0, 192, 96, 256, &vec![128; 192 * 96 * 4]));
    assert_eq!(
        p.custom_command_icon(&revision, "command-0")
            .unwrap()
            .size()
            .width,
        192,
        "larger panel cache artifacts also serve the same command artwork source"
    );
    assert!(!p.advance_thumbnail_animation());

    let changed = Arc::new(p.preferences.values.user_commands.as_ref().clone());
    p.preferences.values.user_commands = changed;
    assert!(p.command_toolbar.sync(&p.preferences.values.user_commands));
    assert!(
        p.thumbnail_targets([(0, usize::MAX); 2], [""; 2])
            .is_empty()
    );
    assert!(!p.receive_thumbnail(identity, 0, 1, 1, &[10, 20, 30, 255]));
    assert!(p.custom_command_icon(&revision, "command-0").is_none());
}

#[test]
fn hidden_artwork_is_evicted_and_cache_clear_invalidates_toolbar_images() {
    let mut p = configured(2);
    let revision = p.command_toolbar.revision.get().to_string();
    let key = (revision.clone(), "command-0".into());
    p.command_toolbar.visible.borrow_mut().insert(key.clone());
    let targets = p.thumbnail_targets([(0, 0); 2], [""; 2]);
    assert_eq!(targets.len(), 1);
    assert!(p.receive_thumbnail(&targets[0].identity, 0, 1, 1, &[1, 2, 3, 255]));
    let image_revision = p.command_toolbar.image_revision.get();
    p.clear_thumbnail_images();
    assert!(p.command_toolbar.image_revision.get() > image_revision);
    assert!(p.custom_command_icon(&revision, "command-0").is_none());
    p.command_toolbar.visible.borrow_mut().remove(&key);
    assert!(p.thumbnail_targets([(0, 0); 2], [""; 2]).is_empty());
    assert!(!p.receive_thumbnail(&targets[0].identity, 0, 1, 1, &[1, 2, 3, 255]));
}

#[test]
fn shortcut_edits_refresh_cached_toolbar_hints_without_rebinding_command_identity() {
    let mut p = configured(1);
    let preferences = p.preferences.values.clone();
    p.apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
    let state = Rc::new(RefCell::new(p));
    let rows = Rows {
        state: Rc::downgrade(&state),
        notify: ModelNotify::default(),
        structure: Cell::new((0, 0, 0)),
    };
    rows.refresh(&state.borrow());
    let tracker = Box::pin(i_slint_core::properties::PropertyTracker::<false>::default());
    let old = tracker.as_ref().evaluate(|| {
        rows.model_tracker().track_row_data_changes(0);
        rows.row_data(0).unwrap()
    });
    assert!(!tracker.is_dirty());
    assert!(state.borrow_mut().change_shortcuts(
        "user-command.command-0",
        vec!["Ctrl+Alt+Shift+Q".into()],
        &|_| true,
    ));
    rows.refresh(&state.borrow());
    assert!(
        tracker.is_dirty(),
        "existing Slint delegates must receive the new hint"
    );
    let new = rows.row_data(0).unwrap();
    assert_eq!(old.identity, new.identity);
    assert_eq!(old.revision, new.revision);
    assert!(new.hint.contains("Ctrl+Alt+Shift+Q"));
}

#[test]
fn tooltip_bounds_unicode_folder_templates_and_omits_script_and_argument_values() {
    let p = configured(1);
    let mut command = p.preferences.values.user_commands[0].clone();
    command.working_directory = "文档".repeat(200);
    command.invocation = UserCommandInvocation::Program {
        executable: PathBuf::from("private-executable"),
        arguments: vec!["private-argument".into()],
    };
    let hint = command_hint(&command, "Ctrl+Q");
    assert!(hint.contains("... [display truncated]"));
    assert!(!hint.contains('…'));
    assert!(hint.contains("Program · Once"));
    assert!(hint.len() < 512);
    assert!(!hint.contains("private-"));
    command.invocation = UserCommandInvocation::Terminal {
        shell: ShellFlavor::PowerShell,
        script: "private-script".into(),
        variables: vec![],
    };
    command.launch = UserCommandLaunch::PerItem;
    let hint = command_hint(&command, "");
    assert!(hint.contains("PowerShell · Once per selected item"));
    assert!(!hint.contains("private-script"));
    assert!(!hint.contains("\n\n"));
}
