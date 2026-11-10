use super::*;

#[test]
fn parent_navigation_cannot_select_a_real_id_and_offsets_entry_opens() {
    let folder = test_root().join("home");
    let child = folder.join("child");
    let mut p = Presenter::new(folder.clone(), test_root());
    let generation = begin(&mut p, PaneId::Left, folder.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![Entry::new(EntryId(0), child.clone(), EntryKind::Directory)],
    );
    select_row(&mut p, PaneId::Left, 0, true);
    move_cursor(&mut p, PaneId::Left, 8);
    assert!(p.panes[0].selected.is_empty());
    assert!(p.open(PaneId::Left, 0, &|command| {
        assert!(matches!(command, Command::Navigate { path, .. } if path == test_root()));
        true
    }));
    finish(&mut p, PaneId::Left, NavigationId(2), vec![]);
    assert!(p.panes[0].rows().is_empty());

    let generation = begin(&mut p, PaneId::Left, folder.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![Entry::new(EntryId(0), child.clone(), EntryKind::Directory)],
    );
    select_row(&mut p, PaneId::Left, 0, false);
    move_cursor(&mut p, PaneId::Left, 9);
    assert!(p.panes[0].selected.is_empty());
    assert_eq!(p.panes[0].cursor_index(), Some(1));
    assert!(p.open(PaneId::Left, 1, &|command| {
        assert!(matches!(command, Command::Navigate { path, .. } if path == child));
        true
    }));
}

#[test]
fn failed_and_sorted_navigation_restore_the_parent_cursor() {
    let folder = test_root().join("home");
    let mut p = Presenter::new(folder.clone(), test_root());
    let generation = begin(&mut p, PaneId::Left, folder.to_str().unwrap());
    finish(&mut p, PaneId::Left, generation, vec![entry(1, "child")]);
    select_row(&mut p, PaneId::Left, 0, false);
    let generation = begin(&mut p, PaneId::Left, folder.to_str().unwrap());
    finish(&mut p, PaneId::Left, generation, vec![entry(2, "child")]);
    assert_eq!(p.panes[0].cursor_index(), Some(0));
    let generation = begin(&mut p, PaneId::Left, "missing");
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: generation,
        error: FsError::from_io(
            "list",
            Path::new("missing"),
            io::Error::from(io::ErrorKind::NotFound),
        ),
    });
    assert_eq!(p.panes[0].cursor_index(), Some(0));
}

#[test]
fn location_buttons_restore_a_drive_independently_for_each_pane() {
    let drive = test_root().join("drive");
    let home = test_root().join("home");
    let mut p = Presenter::new(drive.clone(), drive.clone());
    p.apply(Event::DesktopContext {
        network_position: 0,
        shell: ShellFlavor::Posix,
        locations: vec![
            NavigationLocation {
                id: "drive".into(),
                label: "D:".into(),
                path: drive.clone(),
                kind: LocationKind::Volume,
                drive_letter: Some('d'),
            },
            NavigationLocation {
                id: "home".into(),
                label: "Home".into(),
                path: home.clone(),
                kind: LocationKind::Home,
                drive_letter: None,
            },
        ],
    });
    for (pane, path) in [
        (PaneId::Left, drive.join("left")),
        (PaneId::Right, drive.join("right")),
    ] {
        let generation = begin(&mut p, pane, path.to_str().unwrap());
        finish(&mut p, pane, generation, vec![]);
    }
    let generation = begin(&mut p, PaneId::Left, home.to_str().unwrap());
    finish(&mut p, PaneId::Left, generation, vec![]);
    let commands = RefCell::new(vec![]);
    p.choose_location(PaneId::Left, 0, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(
        matches!(&commands.borrow()[0], Command::Navigate { pane: PaneId::Left, path, .. } if path == &drive.join("left"))
    );
    commands.borrow_mut().clear();
    p.drive_letter(PaneId::Right, "D", &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(
        matches!(&commands.borrow()[0], Command::Refresh { pane: PaneId::Right, path, .. } if path == &drive.join("right"))
    );
}

#[test]
fn submitted_command_freezes_cwd_and_blocks_repeat_until_result() {
    let left = test_root().join("left");
    let right = test_root().join("文档");
    let mut p = Presenter::new(left, right.clone());
    p.active = PaneId::Right;
    p.apply(Event::DesktopContext {
        network_position: 0,
        shell: ShellFlavor::Posix,
        locations: vec![],
    });
    let sent = RefCell::new(vec![]);
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(!p.submit_command("   ", &send));
    assert!(p.submit_command("echo '👩🏽‍💻'", &send));
    assert!(!p.submit_command("echo twice", &send));
    p.active = PaneId::Left;
    assert_eq!(sent.borrow().len(), 1);
    let request = match &sent.borrow()[0] {
        Command::LaunchTerminal { request, terminal } => {
            assert_eq!(terminal.working_directory, right);
            assert_eq!(terminal.command, "echo '👩🏽‍💻'");
            *request
        }
        _ => panic!(),
    };
    p.apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::TerminalStarted {
            process_id: 5,
            history: CommandHistoryEntry {
                command: "echo '👩🏽‍💻'".into(),
                shell: ShellFlavor::Posix,
            },
        }),
    });
    assert!(p.submit_command("echo next", &send));
}

#[test]
fn properties_use_selection_instead_of_parent_and_ignore_stale_results() {
    let folder = test_root().join("home");
    let raw = folder.join("文档👩🏽‍💻");
    let mut p = Presenter::new(folder.clone(), test_root());
    let generation = begin(&mut p, PaneId::Left, folder.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![Entry::new(EntryId(0), raw.clone(), EntryKind::File)],
    );
    select_row(&mut p, PaneId::Left, 0, false);
    p.properties(PaneId::Left, &|_| {
        panic!("bare parent must not open properties")
    });
    set_selected(&mut p, PaneId::Left, &[EntryId(0)]);
    let sent = RefCell::new(None);
    p.properties(PaneId::Left, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    let request = match sent.into_inner().unwrap() {
        Command::ShowSelectionProperties {
            request,
            selection,
            view,
            ..
        } => {
            assert_eq!(selection.len(), 1);
            assert!(selection.contains_path(&raw));
            assert!(view.by_path.contains_key(&raw));
            request
        }
        _ => panic!(),
    };
    p.apply(Event::DesktopActionFinished {
        request: DesktopRequestId(request.0 + 1),
        result: Ok(DesktopActionResult::PropertiesInfo {
            title: "stale".into(),
            body: "stale".into(),
        }),
    });
    assert!(p.desktop.notice.is_none());
    p.apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::PropertiesInfo {
            title: "Properties".into(),
            body: "Unicode metadata".into(),
        }),
    });
    assert_eq!(p.desktop.notice.as_ref().unwrap().1, "Unicode metadata");
}

#[test]
fn progressive_folder_sizes_ignore_other_scans_and_cancel_on_navigation() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    p.folder_sizes(PaneId::Left, &|_| {
        panic!("cannot measure an unfinished listing")
    });
    finish(&mut p, PaneId::Left, generation, vec![entry(7, "folder")]);
    let sent = RefCell::new(vec![]);
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.folder_sizes(PaneId::Left, &send);
    let scan = match sent.borrow()[0] {
        Command::ComputeFolderSizes { scan, .. } => scan,
        _ => panic!(),
    };
    let size = FolderSize {
        bytes: 4096,
        files: 1,
        finished: false,
        ..Default::default()
    };
    p.apply(Event::FolderSizeUpdated {
        pane: PaneId::Left,
        navigation: generation,
        scan: FolderSizeScanId(scan.0 + 1),
        entry: EntryId(7),
        size: size.clone(),
    });
    assert!(p.panes[0].folder_sizes.is_empty());
    p.apply(Event::FolderSizeUpdated {
        pane: PaneId::Left,
        navigation: generation,
        scan,
        entry: EntryId(7),
        size: size.clone(),
    });
    assert_eq!(
        p.panes[0].folder_sizes[&PathBuf::from("folder")].bytes,
        4096
    );
    assert!(!p.panes[0].folder_sizes[&PathBuf::from("folder")].finished);
    p.folder_sizes(PaneId::Left, &send);
    assert!(
        matches!(sent.borrow()[1], Command::CancelFolderSizes { scan: actual, .. } if actual == scan)
    );
    p.navigate(PaneId::Left, "other".into(), None, &send);
    assert!(p.panes[0].folder_sizes.is_empty());
    p.apply(Event::FolderSizeUpdated {
        pane: PaneId::Left,
        navigation: generation,
        scan,
        entry: EntryId(7),
        size: size.clone(),
    });
    assert!(p.panes[0].folder_sizes.is_empty());
}

#[test]
fn same_folder_refresh_keeps_committed_rows_cursor_and_selection_usable() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(7, "left/a"), entry(8, "left/b")],
    );
    select_row(&mut p, PaneId::Left, 1, true);
    let generation = begin(&mut p, PaneId::Left, "left");
    assert_eq!(
        p.panes[0].rows().len(),
        2,
        "refresh must keep the committed listing visible"
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(8)));
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: generation,
        entries: vec![entry(70, "left/a")],
    });
    assert_eq!(p.panes[0].rows()[1].id, EntryId(8));
    select_row(&mut p, PaneId::Left, 0, true);
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(80, "left/b"), entry(70, "left/a")],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(70)));
    assert_eq!(
        selected_ids(&p, PaneId::Left),
        HashSet::from([EntryId(70), EntryId(80)])
    );
}

#[test]
fn preference_loading_is_global_and_does_not_disturb_navigation() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
        theme: PreferenceTheme::Light,
        density: UiDensity::Touch,
        ..Default::default()
    }))));
    assert!(p.panes[0].loading);
    finish(&mut p, PaneId::Left, generation, vec![entry(1, "left/a")]);
    assert_eq!(p.panes[0].rows().len(), 1);
    assert!(!p.has_errors());
}

#[test]
fn loaded_windows_shell_is_frozen_in_the_explicit_terminal_request() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.apply(Event::DesktopContext {
        network_position: 0,
        shell: ShellFlavor::PowerShell,
        locations: vec![],
    });
    p.apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
        windows_shell: WindowsShell::Cmd,
        ..Default::default()
    }))));
    assert!(p.submit_command("echo 文档 👩🏽‍💻", &|command| {
        match command {
            Command::LaunchTerminal { terminal, .. } => {
                assert_eq!(terminal.shell, ShellFlavor::Cmd);
                assert_eq!(terminal.mode, TerminalMode::Standard);
                assert_eq!(terminal.command, "echo 文档 👩🏽‍💻");
            }
            _ => panic!(),
        }
        true
    }));
}
