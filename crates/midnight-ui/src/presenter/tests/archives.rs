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
        modified: None,
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
pub(super) fn archive_pane() -> Presenter {
    let mut p = Presenter::new(test_root(), test_root());
    let path = p.panes[0].path.to_string_lossy().into_owned();
    let nav = begin(&mut p, PaneId::Left, &path);
    finish(&mut p, PaneId::Left, nav, vec![]);
    let location = ArchiveLocation {
        mount: Some(Arc::new(FixtureMount)),
        address: ArchiveAddress {
            format: None,
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
    p.panes[0].folder_column_samples.observe(u64::MAX);
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
fn archive_listing_clears_native_folder_column_samples() {
    let p = archive_pane();
    assert_eq!(p.panes[0].folder_column_samples.maximum(), None);
}

#[test]
fn archive_bulk_selection_previews_and_acknowledges_opaque_file_and_folder_ids() {
    let mut p = archive_pane();
    for (operation, expected) in [
        (SelectionOperation::All, [true, true, true]),
        (SelectionOperation::FilesOnly, [false, true, true]),
        (SelectionOperation::Clear, [false, false, false]),
    ] {
        let command = RefCell::new(None);
        assert!(p.bulk_selection(PaneId::Left, operation, &|value| {
            *command.borrow_mut() = Some(value);
            true
        }));
        let Command::ArchiveSelect {
            navigation,
            revision,
            view,
            action: ArchiveSelectionAction::Bulk(sent),
            ..
        } = command.into_inner().unwrap()
        else {
            panic!("archive bulk selection command")
        };
        assert_eq!(sent, operation);
        let mut selected = HashSet::new();
        let mut totals = ArchiveSelectionTotals::default();
        for (entry, expected) in p.panes[0].entries.iter().zip(expected) {
            assert_eq!(p.panes[0].selection_visible(entry), expected);
            if expected {
                selected.insert(entry.id);
                totals.change(entry.kind, entry.metadata.size, true);
            }
        }
        p.apply(Event::ArchiveSelection {
            pane: PaneId::Left,
            navigation,
            revision,
            view,
            result: Ok(ArchiveSelectionResult {
                selected: Arc::new(selected),
                totals,
            }),
        });
        assert!(!p.panes[0].archive.as_ref().unwrap().selection_pending);
        assert!(p.panes[0].selection_preview.is_empty());
        for (entry, expected) in p.panes[0].entries.iter().zip(expected) {
            assert_eq!(p.panes[0].selection_visible(entry), expected);
        }
    }
}

#[test]
fn archive_selection_rejects_obsolete_view_and_independent_revision() {
    let mut p = archive_pane();
    let commands = RefCell::new(Vec::new());
    assert!(
        p.bulk_selection(PaneId::Left, SelectionOperation::All, &|command| {
            commands.borrow_mut().push(command);
            true
        })
    );
    let Command::ArchiveSelect {
        navigation,
        revision,
        view,
        ..
    } = commands.borrow_mut().pop().unwrap()
    else {
        panic!()
    };
    let result = ArchiveSelectionResult {
        selected: Arc::new(HashSet::from([EntryId(3)])),
        totals: ArchiveSelectionTotals::default(),
    };
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation,
        revision: SelectionRevision(0),
        view: view.clone(),
        result: Ok(result.clone()),
    });
    assert!(p.panes[0].archive.as_ref().unwrap().selection_pending);
    p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation,
        revision,
        view,
        result: Ok(result),
    });
    let archive = p.panes[0].archive.as_ref().unwrap();
    assert!(archive.selected.is_empty());
    assert!(!archive.selection_pending);
    assert!(p.panes[0].error.contains("view changed"));
}

#[test]
fn refused_archive_selection_retains_original_revision_and_marks() {
    let mut p = archive_pane();
    assert!(!p.bulk_selection(PaneId::Left, SelectionOperation::All, &|_| false));
    let archive = p.panes[0].archive.as_ref().unwrap();
    assert!(archive.selected.is_empty());
    assert!(!archive.selection_pending);
    assert_eq!(archive.selection_revision, SelectionRevision(0));
    assert!(p.panes[0].error.contains("queue is busy"));
}

#[test]
fn f5_captures_archive_identity_and_native_destination_without_io() {
    let mut p = archive_pane();
    p.panes[0].parent_cursor = false;
    p.panes[0].cursor = Some(EntryId(4));
    let original = p.panes[0].entries.clone();
    p.request_copy_dialog(PaneId::Left, &|_| {
        panic!("opening draft is presentation only")
    });
    let draft = p.copies.draft.as_ref().unwrap();
    let token = draft.token.clone();
    let destination = draft.destination.clone();
    p.panes[0].cursor = Some(EntryId(3));
    p.panes[1].path = test_root().join("changed");
    let queued = RefCell::new(Vec::new());
    p.confirm_copy_dialog(&token, &destination.to_string_lossy(), &|command| {
        queued.borrow_mut().push(command);
        true
    });
    let queued = queued.borrow();
    let [Command::EnqueueArchiveCopy(request)] = queued.as_slice() else {
        panic!("one archive copy")
    };
    assert_eq!(request.fallback, Some(EntryId(4)));
    assert_eq!(request.location.mount_id(), Some(42));
    assert_eq!(request.destination, destination);
    assert!(Arc::ptr_eq(&request.entries, &original));
    assert!(request.id.0 > 0);
}

#[test]
fn f6_and_cut_reject_archive_members_without_native_admission() {
    let mut p = archive_pane();
    p.request_move_dialog(PaneId::Left, &|_| {
        panic!("Archive Move cannot enter native admission")
    });
    assert!(p.copies.draft.is_none());
    p.cut_files(PaneId::Left, &|_| {
        panic!("Archive Cut cannot publish native paths")
    });
    assert!(!p.has_live_operations());
    assert!(p.desktop.notice.is_some());
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
    p.start_operation(
        PaneId::Left,
        MutationKind::Delete(DeleteMode::Permanent),
        &send,
    );
    p.start_operation(PaneId::Left, MutationKind::CreateFile, &send);
    p.start_operation(PaneId::Left, MutationKind::EditMetadata, &send);
    p.paste_files(PaneId::Left, &send);
    p.copy_files(PaneId::Left, &send);
    p.edit_file(PaneId::Left, &send);
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
        navigation,
        ..
    } = command.borrow_mut().take().unwrap()
    else {
        panic!("expected archive parent");
    };
    assert_eq!(location.directory, 1);
    let entries: Arc<[Entry]> = vec![
        node(7, "folder", EntryKind::Directory),
        node(2, "folder", EntryKind::Directory),
    ]
    .into();
    p.apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation,
        result: Ok(ArchiveListing {
            location,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        }),
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert_eq!(p.panes[0].cursor_index(), Some(2));
    assert!(p.panes[0].archive.as_ref().unwrap().selected.is_empty());
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

fn native_archive_fixture(name: &str, kind: EntryKind) -> (Presenter, PathBuf) {
    let mut p = Presenter::new(test_root(), test_root());
    let directory = p.panes[0].path.to_string_lossy().into_owned();
    let navigation = begin(&mut p, PaneId::Left, &directory);
    let source = test_root().join(name);
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(7), source.clone(), kind)],
    );
    (p, source)
}

#[test]
fn configured_archive_activation_never_falls_back_to_desktop_before_or_after_discovery() {
    for discovered in [false, true] {
        for (name, expected) in [
            ("sample.zip", ArchiveFormat::Zip),
            ("sample.7z", ArchiveFormat::SevenZip),
            ("ovclash-0.2.2-linux-x86_64.tar.gz", ArchiveFormat::TarGzip),
        ] {
            let (mut p, source) = native_archive_fixture(name, EntryKind::File);
            if discovered {
                p.apply(Event::ArchiveAvailability(Vec::new()));
            }
            let entries = p.panes[0].entries.clone();
            let directory = p.panes[0].path.clone();
            assert!(
                !p.can_browse_archive(&entries[0]),
                "missing provider has no badge"
            );
            let commands = RefCell::new(Vec::new());
            let send = |command| {
                commands.borrow_mut().push(command);
                true
            };
            assert!(p.open(PaneId::Left, p.panes[0].parent_offset(), &send));
            let navigation = commands
                .borrow()
                .iter()
                .find_map(|command| match command {
                    Command::ArchiveNavigate {
                        pane: PaneId::Left,
                        navigation,
                        action: ArchiveAction::Open { path, format },
                        ..
                    } if path == &source && *format == expected => Some(*navigation),
                    _ => None,
                })
                .expect("an enabled association must ask the internal archive service");
            assert!(
                !commands
                    .borrow()
                    .iter()
                    .any(|command| matches!(command, Command::OpenEntry { .. }))
            );
            p.apply_with_sender(
                Event::ArchiveListed {
                    pane: PaneId::Left,
                    navigation,
                    result: Err(FsError::from_io(
                        "open archive",
                        &source,
                        io::Error::new(
                            io::ErrorKind::Unsupported,
                            "Compatible archive filesystem plugin is unavailable",
                        ),
                    )),
                },
                &send,
            );
            assert!(!p.panes[0].loading);
            assert!(p.panes[0].archive.is_none());
            assert!(p.panes[0].native_capabilities());
            assert_eq!(p.panes[0].path, directory);
            assert!(Arc::ptr_eq(&p.panes[0].entries, &entries));
            assert!(p.panes[0].error.contains("plugin is unavailable"));
            assert!(
                !commands
                    .borrow()
                    .iter()
                    .any(|command| matches!(command, Command::OpenEntry { .. })),
                "provider failure cannot launch the desktop archive application"
            );
        }
    }
}

#[test]
fn disabled_unknown_and_symlink_associations_preserve_native_activation() {
    for (name, kind, disabled) in [
        ("disabled.zip", EntryKind::File, true),
        ("unknown.bin", EntryKind::File, false),
        ("link.zip", EntryKind::Symlink, false),
    ] {
        let (mut p, source) = native_archive_fixture(name, kind);
        p.apply(Event::ArchiveAvailability(Vec::new()));
        if disabled {
            p.preferences
                .values
                .archive_association_overrides
                .insert("zip".into(), false);
        }
        let commands = RefCell::new(Vec::new());
        assert!(
            p.open(PaneId::Left, p.panes[0].parent_offset(), &|command| {
                commands.borrow_mut().push(command);
                true
            })
        );
        assert!(
            commands.borrow().iter().any(
                |command| matches!(command, Command::OpenEntry { path, .. } if path == &source)
            )
        );
        assert!(
            !commands
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::ArchiveNavigate { .. }))
        );
    }
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
                action: ArchiveAction::Open { path, .. }, ..
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
        networks: Default::default(),
    };
    let mut p = Presenter::new(test_root(), test_root());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            tabs: [saved.clone(), PaneTabPreferences::default()],
            ..Default::default()
        }))),
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

#[test]
fn custom_compound_suffix_dispatches_the_chosen_decoder_with_the_original_native_path() {
    let mut presenter = Presenter::new(test_root(), test_root());
    let mut preferences = UserPreferences::default();
    preferences
        .archive_custom_extensions
        .insert("backup.gz".into(), ArchiveFormat::SevenZip);
    presenter.apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
    let directory = presenter.panes[0].path.to_string_lossy().into_owned();
    let navigation = begin(&mut presenter, PaneId::Left, &directory);
    let source = test_root().join("文档.BACKUP.GZ");
    finish(
        &mut presenter,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(7), source.clone(), EntryKind::File)],
    );
    presenter.apply(Event::ArchiveAvailability(vec![
        ArchiveFormat::Gzip,
        ArchiveFormat::SevenZip,
    ]));
    assert!(presenter.open(
        PaneId::Left,
        presenter.panes[0].parent_offset(),
        &|command| {
            matches!(command, Command::ArchiveNavigate {
            action: ArchiveAction::Open { path, format: ArchiveFormat::SevenZip }, ..
        } if path == source)
        }
    ));
}

#[test]
fn retained_tree_root_archive_activation_uses_original_authority_without_provider_fallback() {
    let directory = test_root().join("archive tree activation");
    let mut p = Presenter::new(directory.clone(), directory.clone());
    let navigation = begin(&mut p, PaneId::Left, directory.to_str().unwrap());
    let source = directory.join("ovclash-0.2.2-linux-x86_64.tar.gz");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(42), directory.join("folder"), EntryKind::Directory),
            Entry::new(EntryId(11), source.clone(), EntryKind::File),
        ],
    );
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation,
        entries: p.panes[0].entries.clone(),
        index: p.panes[0].index.clone(),
    };
    let identity = p.panes[0].row_identity(&root.entries[0]);
    p.disclose_tree(PaneId::Left, &identity, &|_| true);
    let publication = super::tree::publication(root.clone(), 1, true, false);
    let mut rows = publication.view.rows.to_vec();
    rows[1].entry = Entry::new(
        EntryId(11),
        directory.join("folder/nested.tar.gz"),
        EntryKind::File,
    );
    rows[1].disclosure = TreeDisclosure::None;
    rows[1].navigable = false;
    let mut file = root.entries[1].clone();
    file.id = EntryId(77);
    rows.push(TreeDisplayRow {
        id: TreeRowId(77),
        entry: file,
        parent: None,
        depth: 0,
        disclosure: TreeDisclosure::None,
        expanded: false,
        selectable: true,
        navigable: false,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: Some(EntryId(11)),
        prefix_end: 0,
        search_match: false,
    });
    let view = Arc::new(
        TreeProjection::new(
            root,
            publication.view.stamp,
            publication.view.layout_revision,
            rows,
        )
        .unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    let publication = TreePublication::new(view, SelectionRevision(0), selection).unwrap();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p.apply(Event::ArchiveAvailability(Vec::new()));
    assert!(!p.open(PaneId::Left, 2, &|_| panic!(
        "a descendant path cannot substitute for retained root authority"
    )));
    let commands = RefCell::new(Vec::new());
    assert!(p.open(PaneId::Left, 3, &|command| {
        commands.borrow_mut().push(command);
        true
    }));
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ArchiveNavigate { action: ArchiveAction::Open { path, format: ArchiveFormat::TarGzip }, .. }
            if path == &source
    )));
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::OpenEntry { .. }))
    );
}
