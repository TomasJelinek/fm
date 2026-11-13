use super::*;
use std::ffi::OsStr;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

fn focused(kind: EntryKind, name: &str) -> (Presenter, PathBuf) {
    let base = test_root().join("folder");
    let path = base.join(name);
    let mut p = Presenter::new(base.clone(), test_root());
    let navigation = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(8), path.clone(), kind)],
    );
    select_row(&mut p, PaneId::Left, 1, false);
    (p, path)
}

fn tree_publication_with_rows(
    source: &TreePublication,
    rows: Vec<TreeDisplayRow>,
    marks: u64,
) -> TreePublication {
    let stamp = TreeViewStamp {
        revision: TreeRevision(source.view.stamp.revision.0 + 1),
        ..source.view.stamp
    };
    let view = Arc::new(
        TreeProjection::new(
            source.view.root.clone(),
            stamp,
            TreeLayoutRevision(source.view.layout_revision.0 + 1),
            rows,
        )
        .unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![marks]).unwrap());
    TreePublication::new(view, source.selection_revision, selection).unwrap()
}

fn pending_native_tree(
    pane: PaneId,
    kind: EntryKind,
    name: &OsStr,
) -> (Presenter, TreePublication, PathBuf) {
    let base = test_root().join("command-tree");
    let folder = base.join("marked folder");
    let path = folder.join(name);
    let mut p = Presenter::new(base.clone(), base.clone());
    let navigation = begin(&mut p, pane, base.to_str().unwrap());
    finish(
        &mut p,
        pane,
        navigation,
        vec![Entry::new(EntryId(42), folder, EntryKind::Directory)],
    );
    let request = RefCell::new(None);
    p.select_row(pane, 1, true, &|command| {
        if let Command::Selection(actual) = command {
            *request.borrow_mut() = Some(actual);
        }
        true
    });
    let request = request
        .into_inner()
        .expect("the parent mark must be requested");
    let selection = Arc::new(
        SelectionSnapshot::from_bits(p.panes[pane.index()].index.clone(), vec![1]).unwrap(),
    );
    p.apply(Event::SelectionChanged {
        pane,
        navigation: request.navigation,
        revision: request.revision,
        selection,
        deferred: Default::default(),
    });
    let panel = &p.panes[pane.index()];
    assert!(Arc::ptr_eq(panel.selected.basis(), &panel.index));
    let root = TreeRootSnapshot {
        pane,
        navigation,
        entries: panel.entries.clone(),
        index: panel.index.clone(),
    };
    let identity = panel.row_identity(&root.entries[0]);
    let selection_revision = panel.selection_revision;
    p.disclose_tree(pane, &identity, &|_| true);
    let mut source = super::tree::publication(root, 1, true, true);
    source.selection_revision = selection_revision;
    let mut rows = source.view.rows.to_vec();
    rows[1].entry = Entry::new(EntryId(11), path.clone(), kind);
    rows[1].entry.display_name = "painted caption, never an insertion identity".into();
    rows[1].disclosure = if kind == EntryKind::Directory {
        TreeDisclosure::Unknown
    } else {
        TreeDisclosure::None
    };
    rows[1].navigable = kind == EntryKind::Directory;
    let publication = tree_publication_with_rows(&source, rows, 1);
    (p, publication, path)
}

fn acknowledged_native_tree(pane: PaneId, kind: EntryKind, name: &OsStr) -> (Presenter, PathBuf) {
    let (mut p, publication, path) = pending_native_tree(pane, kind, name);
    let view = publication.view.clone();
    let commands = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(actual)
            if Arc::ptr_eq(actual, &view)))));
    p.focus_display_row(pane, 2);
    assert!(p.panes[pane.index()].search_allows_activation());
    (p, path)
}

fn assert_no_command_insertion(p: &Presenter, pane: PaneId) {
    assert_eq!(p.focused_name(pane), None);
    assert_eq!(p.focused_full_path(pane), None);
}

#[test]
fn root_shortcut_uses_the_native_anchor_of_the_pending_location() {
    let mut p = Presenter::new(test_root().join("committed"), test_root());
    let provisional = test_root().join("pending").join("nested");
    begin(&mut p, PaneId::Left, provisional.to_str().unwrap());
    assert!(p.go_root(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate {
            pane: PaneId::Left, path, ..
        } if path == test_root()));
        true
    }));
    assert_eq!(p.panes[0].display_path(), test_root());
}

#[test]
fn root_shortcut_rejects_relative_paths_without_guessing_a_drive() {
    let mut p = Presenter::new("relative/nested".into(), test_root());
    assert!(!p.go_root(PaneId::Left, &|_| panic!(
        "relative roots must not navigate"
    )));
    assert_eq!(p.panes[0].display_path(), Path::new("relative/nested"));
    assert!(!p.panes[0].error.is_empty());
}

#[test]
fn root_queue_failure_preserves_the_visible_location() {
    let initial = test_root().join("unchanged");
    let mut p = Presenter::new(initial.clone(), test_root());
    assert!(!p.go_root(PaneId::Left, &|_| false));
    assert_eq!(p.panes[0].display_path(), initial);
    assert!(!p.panes[0].loading);
    assert!(p.panes[0].error.contains("queue"));
}

#[test]
fn folder_shortcut_navigates_the_focused_known_directory() {
    let (mut p, path) = focused(EntryKind::Directory, "文档-🦀");
    assert!(p.open_folder(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate { path: actual, .. } if actual == path));
        true
    }));
    assert_eq!(p.panes[0].display_path(), path);
}

#[test]
fn folder_shortcut_never_launches_regular_files_links_or_special_files() {
    for kind in [EntryKind::File, EntryKind::Symlink, EntryKind::Other] {
        let (mut p, _) = focused(kind, "target");
        let before = p.panes[0].display_path().to_owned();
        assert!(!p.open_folder(PaneId::Left, &|_| panic!(
            "folder-only action must not send"
        )));
        assert_eq!(p.panes[0].display_path(), before);
        assert!(!p.panes[0].loading);
    }
}

#[test]
fn folder_shortcut_opens_the_synthetic_parent() {
    let (mut p, _) = focused(EntryKind::Directory, "child");
    select_row(&mut p, PaneId::Left, 0, false);
    assert!(p.open_folder(PaneId::Left, &|command| {
        assert!(matches!(command, Command::Navigate { path, .. } if path == test_root()));
        true
    }));
}

#[test]
fn folder_queue_failure_preserves_cursor_path_and_selection() {
    let (mut p, _) = focused(EntryKind::Directory, "child");
    select_row(&mut p, PaneId::Left, 1, true);
    let before = p.panes[0].display_path().to_owned();
    assert!(!p.open_folder(PaneId::Left, &|_| false));
    assert_eq!(p.panes[0].display_path(), before);
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(8)));
    assert!(p.panes[0].error.contains("queue"));
}

#[test]
fn focused_name_uses_the_raw_basename_without_interpreting_shell_text() {
    let name = "文档 ' & $() 👩🏽‍💻.txt";
    let (p, _) = focused(EntryKind::File, name);
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from(name)));
}

#[test]
fn focused_name_has_no_artificial_length_limit() {
    let name = "文档".repeat(9000);
    let (p, _) = focused(EntryKind::File, &name);
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from(name)));
}

#[test]
fn focused_name_represents_parent_but_not_an_empty_root() {
    let (mut p, _) = focused(EntryKind::Directory, "child");
    select_row(&mut p, PaneId::Left, 0, false);
    assert_eq!(p.focused_name(PaneId::Left), Some(PathBuf::from("..")));
    assert_eq!(p.focused_name(PaneId::Right), None);
}

#[test]
fn tree_command_insertion_uses_focused_raw_native_descendants_in_both_panes() {
    let names = [
        "文档 ' & $() ; [🦀] 👩🏽‍💻.txt".to_owned(),
        format!(" long ' & $() ; {}.txt ", "文档-🦀".repeat(9000)),
    ];
    for pane in [PaneId::Left, PaneId::Right] {
        for kind in [EntryKind::Directory, EntryKind::File] {
            for name in &names {
                let (p, path) = acknowledged_native_tree(pane, kind, OsStr::new(name));
                let panel = &p.panes[pane.index()];
                let view = panel.tree_view().unwrap();
                assert!(panel.selection_visible(&view.rows[0].entry));
                assert!(!panel.selection_visible(&view.rows[1].entry));
                assert_eq!(panel.cursor, Some(EntryId(11)));
                assert_ne!(view.rows[1].entry.display_name, *name);
                assert_eq!(p.focused_name(pane), Some(PathBuf::from(name)));
                assert_eq!(p.focused_full_path(pane), Some(path.clone()));
                assert_eq!(
                    crate::command_line::command_path_text(&path).unwrap(),
                    path.to_str().unwrap()
                );
            }
        }
    }
}

#[test]
fn tree_command_insertion_preserves_parent_name_without_a_parent_full_path() {
    for pane in [PaneId::Left, PaneId::Right] {
        let (mut p, _) = acknowledged_native_tree(pane, EntryKind::File, OsStr::new("child"));
        p.focus_display_row(pane, 0);
        assert_eq!(p.focused_name(pane), Some(PathBuf::from("..")));
        assert_eq!(p.focused_full_path(pane), None);
    }
}

#[test]
fn tree_command_insertion_waits_for_exact_commit_after_queue_refusal() {
    for pane in [PaneId::Left, PaneId::Right] {
        let (mut p, publication, path) =
            pending_native_tree(pane, EntryKind::File, OsStr::new("raw ' & 文档.txt"));
        let view = publication.view.clone();
        let refused = RefCell::new(Vec::new());
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication))),
            &|command| {
                refused.borrow_mut().push(command);
                false
            },
        );
        p.focus_display_row(pane, 2);
        assert!(p.panes[pane.index()].tree.commit_pending);
        assert_no_command_insertion(&p, pane);
        assert!(refused.borrow().iter().any(|command| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(actual)
                if Arc::ptr_eq(actual, &view)))));
        let accepted = RefCell::new(Vec::new());
        p.flush_selection(&|command| {
            accepted.borrow_mut().push(command);
            true
        });
        assert!(accepted.borrow().iter().any(|command| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(actual)
                if Arc::ptr_eq(actual, &view)))));
        assert!(!p.panes[pane.index()].tree.commit_pending);
        assert_eq!(
            p.focused_name(pane),
            Some(PathBuf::from("raw ' & 文档.txt"))
        );
        assert_eq!(p.focused_full_path(pane), Some(path));
    }
}

#[test]
fn tree_command_insertion_refuses_selection_replay_until_it_can_be_sent() {
    let pane = PaneId::Right;
    let (mut p, path) = acknowledged_native_tree(pane, EntryKind::File, OsStr::new("child.txt"));
    assert!(p.send_tree_selection(pane, TreeSelectionAction::Toggle(TreeRowId(11)), &|_| true));
    let source = p.panes[pane.index()].tree.publication.as_ref().unwrap();
    let publication = tree_publication_with_rows(source, source.view.rows.to_vec(), 1);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|command| {
            !matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Select(_)))
        },
    );
    assert!(!p.panes[pane.index()].tree.commit_pending);
    assert_eq!(p.panes[pane.index()].tree.replay.len(), 1);
    assert_no_command_insertion(&p, pane);
    let replay = RefCell::new(None);
    p.flush_selection(&|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Select(request) = *command
        {
            *replay.borrow_mut() = Some(request);
        }
        true
    });
    let replay = replay
        .into_inner()
        .expect("the pending toggle must be replayed");
    let selection =
        Arc::new(TreeSelectionSnapshot::from_bits(replay.view.clone(), vec![3]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view: replay.view,
            revision: replay.revision,
            selection,
        })),
        &|_| true,
    );
    assert!(p.panes[pane.index()].tree.replay.is_empty());
    assert_eq!(p.focused_name(pane), Some(PathBuf::from("child.txt")));
    assert_eq!(p.focused_full_path(pane), Some(path));
}

#[test]
fn tree_command_insertion_requires_the_current_matching_search_publication() {
    let pane = PaneId::Left;
    let (mut p, path) = acknowledged_native_tree(pane, EntryKind::File, OsStr::new("native.txt"));
    p.preferences.values.quick_search_filter = false;
    let request = RefCell::new(None);
    p.change_search_query(pane, "painted", &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Reproject(actual) = *command
        {
            *request.borrow_mut() = Some(actual);
        }
        true
    });
    assert!(p.panes[pane.index()].search.pending);
    assert_no_command_insertion(&p, pane);
    let request = request
        .into_inner()
        .expect("tree search must request a publication");
    let source = p.panes[pane.index()].tree.publication.as_ref().unwrap();
    let mut rows = source.view.rows.to_vec();
    rows[1].search_match = true;
    rows[1].prefix_end = "painted".len();
    let publication = tree_publication_with_rows(source, rows, 1);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Reprojected {
            previous: request.view.stamp,
            request: request.request,
            options: request.options,
            publication,
        })),
        &|_| true,
    );
    assert!(!p.panes[pane.index()].search.pending);
    assert_eq!(p.focused_name(pane), Some(PathBuf::from("native.txt")));
    assert_eq!(p.focused_full_path(pane), Some(path));
    p.focus_display_row(pane, 1);
    assert!(
        !p.panes[pane.index()]
            .tree_cursor_row()
            .unwrap()
            .search_match
    );
    assert_no_command_insertion(&p, pane);
}

#[test]
fn tree_command_insertion_waits_for_options_and_navigation() {
    for pane in [PaneId::Left, PaneId::Right] {
        let (mut p, path) =
            acknowledged_native_tree(pane, EntryKind::Directory, OsStr::new("child folder"));
        let request = RefCell::new(None);
        assert!(p.sort(pane, SortColumn::Size, &|command| {
            if let Command::Tree(command) = command
                && let TreeCommand::Reproject(actual) = *command
            {
                *request.borrow_mut() = Some(actual);
            }
            true
        }));
        assert_no_command_insertion(&p, pane);
        let request = request
            .into_inner()
            .expect("sort must request tree options");
        let source = p.panes[pane.index()].tree.publication.as_ref().unwrap();
        let publication = tree_publication_with_rows(source, source.view.rows.to_vec(), 1);
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Reprojected {
                previous: request.view.stamp,
                request: request.request,
                options: request.options,
                publication,
            })),
            &|_| true,
        );
        assert_eq!(p.focused_full_path(pane), Some(path));
        assert!(p.open(pane, 2, &|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Navigate {
            row: TreeRowId(11), ..
        }))));
        assert!(!p.panes[pane.index()].loading);
        assert_no_command_insertion(&p, pane);
    }
}

#[test]
fn tree_command_insertion_rejects_unselectable_and_context_only_rows() {
    for context_only in [false, true] {
        let pane = PaneId::Left;
        let (mut p, _) = acknowledged_native_tree(pane, EntryKind::File, OsStr::new("child.txt"));
        let source = p.panes[pane.index()].tree.publication.as_ref().unwrap();
        let mut rows = source.view.rows.to_vec();
        let position = if context_only { 0 } else { 1 };
        rows[position].selectable = false;
        rows[position].context_only = context_only;
        let publication = tree_publication_with_rows(source, rows, 0);
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication))),
            &|_| true,
        );
        p.focus_display_row(pane, position + 1);
        assert!(p.panes[pane.index()].native_capabilities());
        assert_no_command_insertion(&p, pane);
    }
}

#[test]
fn tree_command_insertion_does_not_retain_a_removed_descendants_identity() {
    let pane = PaneId::Right;
    let (mut p, removed) =
        acknowledged_native_tree(pane, EntryKind::File, OsStr::new("removed.txt"));
    let source = p.panes[pane.index()].tree.publication.as_ref().unwrap();
    let mut rows = source.view.rows[..1].to_vec();
    let fallback = rows[0].entry.path.clone();
    rows[0].expanded = false;
    rows[0].disclosure = TreeDisclosure::Collapsed;
    let publication = tree_publication_with_rows(source, rows, 1);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    assert!(p.panes[pane.index()].entry_by_id(EntryId(11)).is_none());
    assert_eq!(p.focused_name(pane), Some(PathBuf::from("marked folder")));
    assert_eq!(p.focused_full_path(pane), Some(fallback));
    assert_ne!(p.focused_full_path(pane), Some(removed));
}

#[test]
fn flat_command_insertion_refuses_loading_and_pending_search() {
    let (mut p, path) = focused(EntryKind::File, "文档 ' & $().txt");
    p.change_search_query(PaneId::Left, "文", &|_| false);
    assert!(p.panes[0].search.pending);
    assert_no_command_insertion(&p, PaneId::Left);
    p.change_search_query(PaneId::Left, "", &|_| true);
    assert_eq!(
        p.focused_name(PaneId::Left),
        Some(PathBuf::from("文档 ' & $().txt"))
    );
    assert_eq!(p.focused_full_path(PaneId::Left), Some(path));
    let pending = test_root().join("replacement");
    begin(&mut p, PaneId::Left, pending.to_str().unwrap());
    assert!(p.panes[0].loading);
    assert_no_command_insertion(&p, PaneId::Left);
}

#[test]
fn drive_information_requests_fresh_backend_details_even_with_cached_capacity() {
    let base = test_root().join("nested");
    let mut p = Presenter::new(base.clone(), test_root());
    p.drive_info(PaneId::Left, &|command| {
        matches!(command, Command::QueryVolumeInformation(_))
    });
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(body.contains(base.to_string_lossy().as_ref()));
    assert!(body.contains("Reading native"));
    assert!(!body.contains("0 bytes"));
    p.panes[0].space = Some(SpaceInfo {
        available: 0,
        total: 1_234,
    });
    p.drive_info(PaneId::Left, &|command| {
        matches!(command, Command::QueryVolumeInformation(_))
    });
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(!body.contains("0 bytes"));
    assert!(!body.contains("1234 bytes"));
    assert!(body.contains("Reading native"));
}

#[test]
fn drive_information_uses_pending_path_and_space_without_stale_capacity() {
    let mut p = Presenter::new(test_root().join("old"), test_root());
    p.panes[0].space = Some(SpaceInfo {
        available: 99,
        total: 123,
    });
    let pending = test_root().join("loading");
    let navigation = begin(&mut p, PaneId::Left, pending.to_str().unwrap());
    p.drive_info(PaneId::Left, &|command| matches!(command, Command::QueryVolumeInformation(request) if request.path == pending && request.navigation == navigation));
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(body.contains(pending.to_string_lossy().as_ref()));
    assert!(body.contains("Reading native"));
    assert!(!body.contains("99 bytes"));
    p.apply(Event::SpaceUpdated {
        pane: PaneId::Left,
        navigation,
        space: SpaceInfo {
            available: 17,
            total: 42,
        },
    });
    p.drive_info(PaneId::Left, &|command| matches!(command, Command::QueryVolumeInformation(request) if request.path == pending && request.navigation == navigation));
    let (_, body) = p.desktop.notice.as_ref().unwrap();
    assert!(!body.contains("17 bytes"));
    assert!(!body.contains("42 bytes"));
    assert_eq!(p.panes[0].display_path(), pending);
}

#[test]
fn full_path_insertion_uses_captured_identity_and_never_parent_or_loading_rows() {
    let (mut p, _) = focused(EntryKind::File, "文档 ' $().txt");
    let expected = p.panes[0].entry_by_id(EntryId(8)).unwrap().path.clone();
    assert_eq!(p.focused_full_path(PaneId::Left), Some(expected));
    p.panes[0].loading = true;
    assert_eq!(p.focused_full_path(PaneId::Left), None);
    p.panes[0].loading = false;
    select_row(&mut p, PaneId::Left, 0, false);
    assert_eq!(p.focused_full_path(PaneId::Left), None);
}

#[test]
fn full_path_insertion_rejects_virtual_descendants_even_under_a_native_root() {
    let (mut p, root) = super::tree::fixture();
    let publication = super::tree::publication(root, 1, true, false);
    let mut rows = publication.view.rows.to_vec();
    let backing = test_root().join("private-provider-backing/member.txt");
    rows[1].entry = Entry::from_archive(ArchiveNode {
        id: VirtualEntryId {
            mount: 1,
            entry: 11,
        },
        parent: 0,
        raw_name: b"member.txt".to_vec().into(),
        display_name: backing.display().to_string(),
        kind: EntryKind::File,
        size: Some(1),
        readable: true,
        modified: None,
    });
    rows[1].disclosure = TreeDisclosure::None;
    rows[1].navigable = false;
    // Valid virtual rows have no native path, even when provider text resembles one.
    // A backend publication carrying a backing path is rejected before UI adoption.
    let mut malformed = rows.clone();
    malformed[1].entry.path = backing;
    assert!(
        TreeProjection::new(
            publication.view.root.clone(),
            publication.view.stamp,
            publication.view.layout_revision,
            malformed,
        )
        .is_err()
    );
    let view = Arc::new(
        TreeProjection::new(
            publication.view.root.clone(),
            publication.view.stamp,
            publication.view.layout_revision,
            rows,
        )
        .unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
        ))),
        &|_| true,
    );
    p.panes[0].cursor = Some(EntryId(11));
    p.panes[0].parent_cursor = false;
    assert!(p.panes[0].native_capabilities());
    let entry = p.panes[0].entry_by_id(EntryId(11)).unwrap();
    assert!(entry.virtual_node.is_some());
    assert!(entry.native_path().is_none());
    assert!(Path::new(&entry.display_name).is_absolute());
    assert_eq!(p.focused_name(PaneId::Left), None);
    assert_eq!(p.focused_full_path(PaneId::Left), None);
}

#[test]
fn full_path_insertion_rejects_relative_native_row_text() {
    let (mut p, _) = focused(EntryKind::File, "file.txt");
    let mut entries = p.panes[0].entries.to_vec();
    entries[0].path = "relative/file.txt".into();
    p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&entries));
    p.panes[0].entries = entries.into();
    assert_eq!(
        p.focused_name(PaneId::Left),
        Some(PathBuf::from("file.txt"))
    );
    assert_eq!(p.focused_full_path(PaneId::Left), None);
}

#[test]
fn viewer_and_comparison_resolvers_keep_file_and_difference_navigation_separate() {
    let presenter = Presenter::new(test_root(), test_root());
    let previous = ShortcutChord::parse("Alt+Left").unwrap();
    assert_eq!(
        presenter.viewer_key(&previous, false).as_deref(),
        Some("viewer.previous-file")
    );
    assert_eq!(
        presenter.comparison_key(&previous, false).as_deref(),
        Some("compare.previous-difference")
    );
    assert_eq!(presenter.viewer_key(&previous, true), None);
    assert_eq!(presenter.comparison_key(&previous, true), None);
}
