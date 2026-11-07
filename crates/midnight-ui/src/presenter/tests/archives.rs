use super::*;

#[derive(Debug)]
struct FixtureMount;
impl ArchiveMount for FixtureMount {
    fn id(&self) -> u64 {
        42
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("UI performed listing")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("UI performed reading")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("UI performed validation")
    }
}
fn node(id: u64, name: &str, kind: EntryKind) -> Entry {
    Entry::from_archive(ArchiveNode {
        id: VirtualEntryId {
            mount: 42,
            entry: id,
        },
        parent: 1,
        raw_name: name.as_bytes().into(),
        display_name: name.into(),
        kind,
        size: Some(5),
        readable: kind == EntryKind::File,
    })
}
fn archive_pane() -> Presenter {
    let mut p = Presenter::new(test_root(), test_root());
    let path = p.panes[0].path.to_string_lossy().into_owned();
    let nav = begin(&mut p, PaneId::Left, &path);
    finish(&mut p, PaneId::Left, nav, vec![]);
    let location = ArchiveLocation {
        mount: Some(Arc::new(FixtureMount)),
        address: ArchiveAddress {
            provider: "midnight.archive".into(),
            container: test_root().join("sample.zip"),
            source_version: vec![1, 2, 3],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, "sample.zip".into())],
    };
    let entries: Arc<[Entry]> = vec![
        node(2, "folder", EntryKind::Directory),
        node(3, "same.txt", EntryKind::File),
        node(4, "same.txt", EntryKind::File),
    ]
    .into();
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: nav,
        result: Ok(ArchiveListing {
            location,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        }),
    });
    p
}

#[test]
fn archive_paths_and_breadcrumbs_use_native_separators_without_added_spaces() {
    let mut p = archive_pane();
    let expected_root = test_root().join("sample.zip");
    assert_eq!(
        p.panes[0].display_location_text(),
        expected_root.to_string_lossy()
    );
    let archive = p.panes[0].archive.as_mut().unwrap();
    archive.origin = test_root().join("different return folder");
    archive.location.trail.push((2, "inside 文档".into()));
    archive.location.trail.push((3, "deeper".into()));
    archive.location.address.components = vec![b"inside".to_vec(), b"deeper".to_vec()];
    archive.location.directory = 3;
    let expected = expected_root.join("inside 文档").join("deeper");
    assert_eq!(
        p.panes[0].display_location_text(),
        expected.to_string_lossy()
    );
    let parts = p.path_parts(PaneId::Left);
    assert_eq!(
        parts
            .iter()
            .map(|part| part.label.as_str())
            .collect::<String>(),
        expected.to_string_lossy()
    );
    assert_eq!(parts.last().unwrap().prefix, expected.to_string_lossy());
    let first = parts[0].identity.clone();
    let command = RefCell::new(None);
    assert!(p.navigate_path_part(PaneId::Left, &first, &|value| {
        *command.borrow_mut() = Some(value);
        true
    }));
    assert!(matches!(&*command.borrow(), Some(Command::ArchiveNavigate {
        action: ArchiveAction::List(location), ..
    }) if location.directory == 1 && location.address.components.is_empty()));
}

#[test]
fn archive_selection_and_viewer_use_opaque_identity_for_duplicate_names() {
    let mut p = archive_pane();
    assert!(p.panes[0].index.by_path.is_empty());
    assert!(p.visible_directory(PaneId::Left).is_none());
    p.select_row(PaneId::Left, 2, true, &|_| {
        panic!("single toggle must remain cached")
    });
    p.select_row(PaneId::Left, 3, true, &|_| {
        panic!("single toggle must remain cached")
    });
    assert_eq!(p.panes[0].selected_count(), 2);
    assert_eq!(
        p.panes[0].archive_selection_summary(),
        Some((2, 0, Some(10), false))
    );
    assert!(p.panes[0].selection_visible(&p.panes[0].entries[1]));
    assert!(p.panes[0].selection_visible(&p.panes[0].entries[2]));
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    let launch = p.viewer_launch(PaneId::Left, false).unwrap();
    assert_eq!(
        launch.archive.unwrap().focused,
        VirtualEntryId {
            mount: 42,
            entry: 4
        }
    );
    assert!(launch.focused.as_os_str().is_empty());
}

#[test]
fn readonly_archives_never_enqueue_native_operations_or_external_launch() {
    let mut p = archive_pane();
    let send = |command| panic!("archive routed a native command: {command:?}");
    p.select_row(PaneId::Left, 2, false, &send);
    p.start_operation(PaneId::Left, MutationKind::CreateDirectory, &send);
    p.start_operation(PaneId::Left, MutationKind::Rename, &send);
    p.start_operation(PaneId::Left, MutationKind::Delete(DeleteMode::Trash), &send);
    p.paste_files(PaneId::Left, &send);
    p.copy_files(PaneId::Left, &send);
    p.edit_file(PaneId::Left, &send);
    p.folder_sizes(PaneId::Left, &send);
    assert!(!p.open(PaneId::Left, 2, &send));
    assert!(!p.open_terminal(&send));
}

#[test]
fn directory_activation_parent_and_refresh_keep_the_provider_identity() {
    let mut p = archive_pane();
    let command = RefCell::new(None);
    assert!(p.open(PaneId::Left, 1, &|c| {
        *command.borrow_mut() = Some(c);
        true
    }));
    let Command::ArchiveNavigate {
        action: ArchiveAction::List(location),
        navigation,
        ..
    } = command.borrow_mut().take().unwrap()
    else {
        panic!("expected virtual directory navigation");
    };
    assert_eq!(location.mount_id(), Some(42));
    assert_eq!(location.directory, 2);
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation,
        result: Ok(ArchiveListing {
            location,
            entries: Arc::from([]),
            index: Arc::default(),
        }),
    });
    assert!(p.up(PaneId::Left, &|c| {
        *command.borrow_mut() = Some(c);
        true
    }));
    let Command::ArchiveNavigate {
        action: ArchiveAction::List(location),
        ..
    } = command.borrow_mut().take().unwrap()
    else {
        panic!("expected archive parent");
    };
    assert_eq!(location.directory, 1);
    assert!(p.refresh_location(PaneId::Left, &|c| matches!(
        c,
        Command::ArchiveNavigate { .. }
    )));
}

#[test]
fn native_navigation_failure_keeps_archive_readonly() {
    let mut p = archive_pane();
    let target = test_root().join("missing");
    let command = RefCell::new(None);
    assert!(p.navigate(PaneId::Left, target.clone(), None, &|c| {
        *command.borrow_mut() = Some(c);
        true
    }));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io("list", &target, io::ErrorKind::NotFound.into()),
    });
    assert!(p.panes[0].archive.is_some());
    assert!(!p.panes[0].native_capabilities());
    p.start_operation(PaneId::Left, MutationKind::CreateDirectory, &|_| {
        panic!("native mutation after failed exit")
    });
}

#[test]
fn missing_plugin_preserves_normal_native_activation() {
    let mut p = Presenter::new(test_root(), test_root());
    let path = p.panes[0].path.to_string_lossy().into_owned();
    let nav = begin(&mut p, PaneId::Left, &path);
    finish(
        &mut p,
        PaneId::Left,
        nav,
        vec![Entry::new(
            EntryId(7),
            test_root().join("sample.zip"),
            EntryKind::File,
        )],
    );
    p.apply(Event::ArchiveAvailability(vec![]));
    assert!(
        p.open(PaneId::Left, p.panes[0].parent_offset(), &|c| matches!(
            c,
            Command::OpenEntry { .. }
        ))
    );
}

#[test]
fn archive_badges_and_activation_honor_format_availability_and_user_associations() {
    use slint::Model;
    use std::rc::Rc;

    let mut p = Presenter::new(test_root(), test_root());
    let directory = p.panes[0].path.to_string_lossy().into_owned();
    let navigation = begin(&mut p, PaneId::Left, &directory);
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), test_root().join("package.whl"), EntryKind::File),
            Entry::new(EntryId(2), test_root().join("report.docx"), EntryKind::File),
            Entry::new(
                EntryId(3),
                test_root().join("folder.zip"),
                EntryKind::Directory,
            ),
            Entry::new(EntryId(4), test_root().join("archive.7z"), EntryKind::File),
            node(5, "nested.zip", EntryKind::File),
        ],
    );
    let p = Rc::new(RefCell::new(p));
    let rows = crate::Rows::new(p.clone(), PaneId::Left);
    let badge = |name: &str| {
        (0..rows.row_count())
            .filter_map(|row| rows.row_data(row))
            .find(|row| row.name == name)
            .unwrap()
            .browsable_archive
    };
    assert!(!badge("package.whl"), "missing provider");
    p.borrow_mut()
        .apply(Event::ArchiveAvailability(vec![ArchiveFormat::Zip]));
    assert!(
        badge("package.whl"),
        "late discovery updates existing entries"
    );
    for name in ["report.docx", "folder.zip", "archive.7z", "nested.zip"] {
        assert!(!badge(name), "{name}");
    }
    let doc_row = {
        let p = p.borrow();
        (0..p.panes[0].display_len())
            .find(|row| {
                p.panes[0]
                    .display_entry(*row)
                    .is_some_and(|entry| entry.id == EntryId(2))
            })
            .unwrap()
            + p.panes[0].parent_offset()
    };
    assert!(p.borrow_mut().open(PaneId::Left, doc_row, &|command| {
        matches!(command, Command::OpenEntry { .. })
    }));
    p.borrow_mut()
        .preferences
        .values
        .archive_association_overrides
        .insert("docx".into(), true);
    assert!(badge("report.docx"));
    p.borrow_mut()
        .preferences
        .values
        .archive_association_overrides
        .insert("whl".into(), false);
    assert!(!badge("package.whl"));
    p.borrow_mut().apply(Event::ArchiveAvailability(vec![]));
    assert!(
        !badge("report.docx"),
        "enabling a checkbox cannot install a provider"
    );
}

#[test]
fn installed_provider_claims_native_archive_activation_and_disables_mutations_while_opening() {
    for name in [
        "sample.zip",
        "sample.7z",
        "sample.gz",
        "sample.tar",
        "sample.tar.gz",
        "sample.whl",
        "sample.NUPKG",
    ] {
        let mut p = Presenter::new(test_root(), test_root());
        let directory = p.panes[0].path.to_string_lossy().into_owned();
        let nav = begin(&mut p, PaneId::Left, &directory);
        let source = test_root().join(name);
        finish(
            &mut p,
            PaneId::Left,
            nav,
            vec![Entry::new(EntryId(7), source.clone(), EntryKind::File)],
        );
        p.apply(Event::ArchiveAvailability(vec![
            ArchiveFormat::Zip,
            ArchiveFormat::Tar,
            ArchiveFormat::Gzip,
            ArchiveFormat::TarGzip,
            ArchiveFormat::SevenZip,
        ]));
        assert!(
            p.open(PaneId::Left, p.panes[0].parent_offset(), &|command| {
                matches!(command, Command::ArchiveNavigate {
                action: ArchiveAction::Open { path }, ..
            } if path == source)
            })
        );
        assert!(!p.panes[0].native_capabilities());
        p.start_operation(PaneId::Left, MutationKind::CreateDirectory, &|_| {
            panic!("Archive activation must disable native mutation while the mount opens")
        });
    }
}

#[test]
fn saved_archive_tabs_restore_lazily_and_keep_addresses_when_provider_is_missing() {
    let original = archive_pane();
    let address = original.panes[0]
        .archive
        .as_ref()
        .unwrap()
        .location
        .address
        .clone();
    let saved = PaneTabPreferences {
        paths: vec![test_root(), test_root()],
        active: 1,
        archives: [(1, address.clone())].into(),
    };
    let mut p = Presenter::new(test_root(), test_root());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(UserPreferences {
            tabs: [saved.clone(), PaneTabPreferences::default()],
            ..Default::default()
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::ArchiveNavigate { action: ArchiveAction::List(location), .. }
        if location.mount.is_none() && location.address == address)));
    assert!(!p.panes[0].native_capabilities());
    let navigation = NavigationId(p.panes[0].generation);
    p.apply_with_sender(
        Event::ArchiveListed {
            pane: PaneId::Left,
            navigation,
            result: Err(FsError::from_io(
                "restore archive",
                &address.container,
                io::Error::new(io::ErrorKind::NotFound, "provider missing"),
            )),
        },
        &|_| true,
    );
    p.flush_tab_sessions(&|_| true);
    assert_eq!(p.preferences.values.tabs[0], saved);
    assert!(p.panes[0].error.contains("provider missing"));
    assert!(p.visible_directory(PaneId::Left).is_none());
    p.paste_files(PaneId::Left, &|_| {
        panic!("unavailable provider cannot paste natively")
    });
}

#[test]
fn archive_tabs_and_history_keep_provider_addresses_and_filters() {
    let mut p = archive_pane();
    let address = p.panes[0]
        .archive
        .as_ref()
        .unwrap()
        .location
        .address
        .clone();
    p.preferences.values.panel_filters[0].enabled = true;
    p.preferences.values.panel_filters[0].expression = "*.txt".into();
    let sent = RefCell::new(Vec::new());
    assert!(p.new_tab(PaneId::Left, &|c| {
        sent.borrow_mut().push(c);
        true
    }));
    assert_eq!(p.preferences.values.tabs[0].archives.len(), 2);
    assert!(
        p.preferences.values.tabs[0]
            .archives
            .values()
            .all(|a| a == &address)
    );
    assert!(
        sent.borrow()
            .iter()
            .any(|c| matches!(c, Command::ArchiveNavigate { .. }))
    );
    assert!(p.panes[0].search.panel_filter.enabled);
    assert_eq!(p.panes[0].search.panel_filter.expression, "*.txt");
}

#[test]
fn rejected_or_failed_archive_sort_keeps_the_committed_order_indicator() {
    let mut p = archive_pane();
    let before = p.panes[0].sort;
    assert!(!p.sort(PaneId::Left, SortColumn::Size, &|_| false));
    assert_eq!(p.panes[0].sort, before);
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|_| true));
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        result: Err(FsError::from_io(
            "archive sort",
            Path::new(""),
            io::Error::other("source changed"),
        )),
    });
    assert_eq!(p.panes[0].sort, before);
}

#[test]
fn unavailable_nested_archive_can_navigate_out_without_restoring_the_provider() {
    let mut p = archive_pane();
    let a = p.panes[0].archive.as_mut().unwrap();
    let mut address = a.location.address.clone();
    address.components.push(b"folder".to_vec());
    let container = address.container.clone();
    a.location = ArchiveLocation::unavailable(address);
    assert!(p.up(PaneId::Left, &|c| matches!(
        c,
        Command::ArchiveNavigate { .. }
    )));
    assert!(
        p.panes[0]
            .archive
            .as_ref()
            .unwrap()
            .location
            .address
            .components
            .is_empty()
    );
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
        result: Err(FsError::from_io(
            "restore",
            &container,
            io::Error::other("missing plugin"),
        )),
    });
    assert!(p.up(PaneId::Left, &|c| matches!(c, Command::Navigate { .. })));
    assert_eq!(
        p.panes[0].pending.as_ref().unwrap().wanted_cursor.as_ref(),
        Some(&container)
    );
}
