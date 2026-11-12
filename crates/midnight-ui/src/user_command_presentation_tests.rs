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
        targets[0].edge, 128,
        "saved command artwork retains touch-mode resolution"
    );
    assert!(p.receive_thumbnail_at_resolution(
        identity,
        0,
        128,
        128,
        128,
        &vec![128; 128 * 128 * 4]
    ));
    assert_eq!(
        p.custom_command_icon(&revision, "command-0")
            .unwrap()
            .size()
            .width,
        128,
        "the toolbar must retain the managed icon pixels without downsampling to 96px"
    );
    assert!(p.receive_thumbnail_at_resolution(
        identity,
        0,
        192,
        192,
        256,
        &vec![128; 192 * 192 * 4]
    ));
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
fn visible_program_without_explicit_artwork_requests_its_executable_icon() {
    let mut p = configured(3);
    let commands = Arc::make_mut(&mut p.preferences.values.user_commands);
    commands[1].icon = None;
    commands[2].icon = None;
    commands[2].invocation = UserCommandInvocation::Terminal {
        shell: ShellFlavor::Posix,
        script: "printf never-run-for-icons".into(),
        variables: vec![],
    };
    p.command_toolbar.sync(&p.preferences.values.user_commands);
    let revision = p.command_toolbar.revision.get().to_string();
    for index in 0..3 {
        p.command_toolbar
            .visible
            .borrow_mut()
            .insert((revision.clone(), format!("command-{index}")));
    }
    let targets = p.thumbnail_targets([(0, 0); 2], [""; 2]);
    assert_eq!(
        targets.len(),
        2,
        "explicit image and Program artwork must both be requested"
    );
    assert_eq!(
        targets[0].identity,
        command_icon_identity(&revision, "command-0")
    );
    assert_eq!(
        targets[1].identity,
        command_icon_identity(&revision, "command-1")
    );
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

#[test]
fn executable_icon_views_retry_admission_and_fence_hidden_edited_and_cleared_results() {
    let mut p = configured(1);
    Arc::make_mut(&mut p.preferences.values.user_commands)[0].icon = None;
    p.command_toolbar.sync(&p.preferences.values.user_commands);
    let revision = p.command_toolbar.revision.get().to_string();
    p.command_toolbar
        .visible
        .borrow_mut()
        .insert((revision.clone(), "command-0".into()));
    let rows = p.thumbnail_targets([(0, 0); 2], [""; 2]);
    let requests = RefCell::new(Vec::new());
    let send = |command| {
        requests.borrow_mut().push(command);
        true
    };
    p.refresh_command_icons(&rows, &|_| false);
    p.refresh_command_icons(&rows, &send);
    p.refresh_command_icons(&rows, &send);
    assert_eq!(
        requests.borrow().len(),
        1,
        "unchanged views must not reload native art"
    );
    let generation = match requests.borrow().first().unwrap() {
        Command::SetCommandIconView {
            generation,
            targets,
        } => {
            assert_eq!(targets.len(), 1);
            *generation
        }
        event => panic!("unexpected {event:?}"),
    };
    let identity = &rows[0].identity;
    let pixels = IconPixels::new(1, 1, vec![1, 2, 3, 255]).unwrap();
    p.receive_command_icon(generation + 1, identity, &pixels);
    assert_eq!(p.thumbnail_coverage(identity, 0), 0);
    p.receive_command_icon(generation, identity, &pixels);
    assert_eq!(p.thumbnail_coverage(identity, 0), 64);
    p.clear_thumbnail_images();
    p.receive_command_icon(generation, identity, &pixels);
    assert_eq!(p.thumbnail_coverage(identity, 0), 0);
    p.refresh_command_icons(&rows, &send);
    p.command_toolbar.visible.borrow_mut().clear();
    p.receive_command_icon(generation + 2, identity, &pixels);
    assert_eq!(p.thumbnail_coverage(identity, 0), 0);
    let hidden = p.thumbnail_targets([(0, 0); 2], [""; 2]);
    p.refresh_command_icons(&hidden, &send);
    assert!(
        matches!(requests.borrow().last(), Some(Command::SetCommandIconView { targets, .. }) if targets.is_empty())
    );
    Arc::make_mut(&mut p.preferences.values.user_commands)[0].icon =
        Some(PathBuf::from("explicit.png"));
    p.command_toolbar.sync(&p.preferences.values.user_commands);
    p.receive_command_icon(generation + 2, identity, &pixels);
    assert_eq!(p.thumbnail_coverage(identity, 0), 0);
}

#[test]
fn separators_group_commands_without_toolbar_edges_icons_or_shortcuts() {
    let mut p = configured(8);
    let commands = Arc::make_mut(&mut p.preferences.values.user_commands);
    for index in [0, 2, 3, 5, 7] {
        commands[index].invocation = UserCommandInvocation::Separator;
    }
    commands[5].enabled = false;
    let ids = toolbar_commands(commands)
        .map(|command| command.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["command-1", "command-2", "command-4", "command-6"]);
    let keys = ShortcutMap::with_user_commands(&[], commands).unwrap();
    assert!(keys.bindings("user-command.command-1").is_some());
    assert!(keys.bindings("user-command.command-2").is_none());
    p.command_toolbar.sync(&p.preferences.values.user_commands);
    let revision = p.command_toolbar.revision.get().to_string();
    for index in 0..8 {
        p.command_toolbar
            .visible
            .borrow_mut()
            .insert((revision.clone(), format!("command-{index}")));
    }
    assert_eq!(
        p.command_icon_targets().len(),
        3,
        "even a stale visible notification cannot request artwork for a separator"
    );
    for command in Arc::make_mut(&mut p.preferences.values.user_commands) {
        command.invocation = UserCommandInvocation::Separator;
    }
    assert_eq!(
        toolbar_commands(&p.preferences.values.user_commands).count(),
        0
    );
}
