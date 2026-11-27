//! UI retries and stale replies retain the original focused occurrence.
use super::*;
#[derive(Debug)]
struct Source {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    row: TreeRowId,
}
impl CapturedRenameTarget for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn row(&self) -> TreeRowId {
        self.row
    }
    fn prepare(
        &self,
        _: &dyn MutationService,
        _: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        panic!("no UI native preparation")
    }
}
fn setup() -> (Presenter, TreeRootSnapshot) {
    let (mut p, root) = fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication(
            root.clone(),
            1,
            true,
            true,
        )))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    (p, root)
}
fn capture(p: &mut Presenter) -> TreeRenameRequest {
    let sent = RefCell::new(None);
    p.start_operation(PaneId::Left, MutationKind::Rename, &|command| {
        if let Command::Tree(command) = command
            && let TreeCommand::Rename(request) = *command
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner().expect("focused capture")
}
fn reply(request: &TreeRenameRequest) -> Event {
    Event::Tree(Box::new(TreeEvent::RenameCaptured {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(Arc::new(Source {
            stamp: request.view.stamp,
            request: request.request,
            row: request.row,
        })),
    }))
}
#[test]
fn rename_capture_backpressure_retries_same_focus_and_admits_only_once() {
    let (mut p, _) = setup();
    p.start_operation(PaneId::Left, MutationKind::Rename, &|_| false);
    p.focus_display_row(PaneId::Left, 1);
    let sent = RefCell::new(Vec::new());
    p.flush_tree_rename(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let Command::Tree(command) = sent.borrow_mut().pop().unwrap() else {
        panic!("capture")
    };
    let TreeCommand::Rename(request) = *command else {
        panic!("Rename")
    };
    assert_eq!(request.row, TreeRowId(11));
    p.apply_with_sender(reply(&request), &|_| false);
    assert!(p.operations.preview.is_none());
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.flush_tree_rename(&send);
    p.flush_tree_rename(&send);
    let commands = sent.borrow();
    let [Command::PrepareCapturedRename(prepared)] = commands.as_slice() else {
        panic!("one admission")
    };
    assert_eq!(prepared.source.row(), request.row);
    assert_eq!(prepared.source.stamp(), request.view.stamp);
    assert_eq!(prepared.source_directory, p.panes[0].path);
}
#[test]
fn rename_capture_reprojection_and_shutdown_cancel_pending_and_ignore_late_reply() {
    for closing in [false, true] {
        let (mut p, root) = setup();
        let request = capture(&mut p);
        if closing {
            p.retire_all();
        } else {
            p.apply_with_sender(
                Event::Tree(Box::new(TreeEvent::Published(publication(
                    root, 2, true, false,
                )))),
                &|_| true,
            );
            p.flush_tree_rename(&|_| panic!("stale capture must not prepare"));
        }
        assert!(request.cancel.is_cancelled());
        p.apply_with_sender(reply(&request), &|_| panic!("late reply must not prepare"));
        assert!(p.operations.preview.is_none());
        assert!(p.operations.pending.is_none());
    }
}
#[test]
fn rename_parent_focus_does_not_create_typed_or_flat_authority() {
    let (mut p, _) = setup();
    p.panes[0].parent_cursor = true;
    p.start_operation(PaneId::Left, MutationKind::Rename, &|_| {
        panic!("parent is navigation")
    });
    assert!(p.operations.tree_rename.is_none());
}

#[test]
fn rename_completion_reconciles_expanded_parent_without_flat_navigation() {
    let (mut p, _) = setup();
    let view = p.panes[0].tree_view().unwrap().clone();
    let request = capture(&mut p);
    p.apply_with_sender(reply(&request), &|_| true);
    let mut progress = (*p.operations.progress[0]).clone();
    progress.state = OperationState::Completed;
    progress.completed = 1;
    progress.renamed_to = Some(p.panes[0].path.join("folder/renamed"));
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Operation(OperationEvent::Progress(Arc::new(progress))),
        &|command| {
            assert!(!matches!(
                command,
                Command::Navigate {
                    pane: PaneId::Left,
                    ..
                } | Command::Refresh {
                    pane: PaneId::Left,
                    ..
                }
            ));
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(Arc::ptr_eq(p.panes[0].tree_view().unwrap(), &view));
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::RefreshLoaded(_))))
    );
}

#[test]
fn menu_rename_uses_frozen_clicked_root_without_changing_child_marks_or_caret() {
    let (mut p, root) = fixture();
    let mut publication = publication(root, 1, true, false);
    publication.selection =
        Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![2]).unwrap());
    let view = publication.view.clone();
    let selection = publication.selection.clone();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p.focus_display_row(PaneId::Left, 2);
    let caret = p.panes[0].cursor;
    let sent = RefCell::new(Vec::new());
    p.request_tree_rename_target(PaneId::Left, view.clone(), TreeRowId(7), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Tree(command)
        if matches!(command.as_ref(), TreeCommand::Rename(request) if request.row == TreeRowId(7) && Arc::ptr_eq(&request.view, &view)))));
    assert_eq!(p.panes[0].cursor, caret);
    assert!(Arc::ptr_eq(
        &p.panes[0].tree.publication.as_ref().unwrap().selection,
        &selection
    ));
}

/// Revision `revision` of the tree in which `folder/child` is now `folder/renamed`.
fn renamed_publication(root: TreeRootSnapshot, revision: u64) -> TreePublication {
    let base = publication(root.clone(), revision, true, false);
    let mut rows = base.view.rows.to_vec();
    rows[1].id = TreeRowId(12);
    rows[1].entry = Entry::new(
        EntryId(12),
        root.entries[0].path.join("renamed"),
        EntryKind::Directory,
    );
    let view = Arc::new(
        TreeProjection::new(root, base.view.stamp, TreeLayoutRevision(revision), rows).unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    TreePublication::new(view, SelectionRevision(0), selection).unwrap()
}

/// F2 in a tree holds panel input from its confirmation until the refreshed
/// tree shows the new name, which then has the cursor; a failure never holds.
#[test]
fn tree_rename_holds_input_until_the_refreshed_tree_shows_and_focuses_the_new_name() {
    for succeeded in [true, false] {
        let (mut p, root) = setup();
        let request = capture(&mut p);
        p.apply_with_sender(reply(&request), &|_| true);
        let id = p.operations.pending.expect("prepared tree rename");
        let child = root.entries[0].path.join("child");
        p.apply(Event::Operation(OperationEvent::Preview(Arc::new(
            OperationPreview {
                id,
                kind: MutationKind::Rename,
                files: 0,
                folders: 1,
                file_bytes: None,
                sample: vec![MutationItem {
                    path: child.clone(),
                    kind: EntryKind::Directory,
                    metadata: Default::default(),
                }],
                trash_unavailable: None,
            },
        ))));
        p.confirm_operation(&id.0.to_string(), "renamed".into(), &|_| true);
        assert!(p.mutation_settling(), "input waits from the confirmation");

        let mut progress = (*p.operations.progress[0]).clone();
        let renamed = root.entries[0].path.join("renamed");
        if succeeded {
            progress.state = OperationState::Completed;
            progress.completed = 1;
            progress.renamed_to = Some(renamed.clone());
        } else {
            progress.state = OperationState::Failed;
            progress.failed = 1;
            progress.errors.push(FsError::from_io(
                "Rename",
                &child,
                io::Error::from(io::ErrorKind::PermissionDenied),
            ));
        }
        let sent = RefCell::new(Vec::new());
        p.apply_with_sender(
            Event::Operation(OperationEvent::Progress(Arc::new(progress))),
            &|command| {
                sent.borrow_mut().push(command);
                true
            },
        );
        let refresh = sent.borrow().iter().find_map(|command| match command {
            Command::Tree(command) => match command.as_ref() {
                TreeCommand::RefreshLoaded(request) => Some((request.view.stamp, request.request)),
                _ => None,
            },
            _ => None,
        });
        let (stamp, refresh) = refresh.expect("the completion refreshes the tree at once");
        if !succeeded {
            assert!(!p.mutation_settling(), "a failure never holds input");
            assert!(p.desktop.notice.is_some());
            continue;
        }
        assert!(p.mutation_settling(), "input waits for the tree refresh");
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(renamed_publication(
                root.clone(),
                2,
            )))),
            &|_| true,
        );
        assert_eq!(
            p.panes[0].cursor,
            Some(EntryId(12)),
            "the new name is focused"
        );
        assert!(!p.panes[0].parent_cursor);
        assert!(
            !p.panes[0].preserve_viewport,
            "the new name is scrolled into view"
        );
        assert!(p.mutation_settling(), "until the refresh completes");
        // The right panel lists the same folder flat; its re-read also shows the change.
        let flat: Vec<_> = sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Refresh {
                    pane: PaneId::Right,
                    navigation,
                    ..
                }
                | Command::Navigate {
                    pane: PaneId::Right,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .collect();
        let rows = p.panes[1].entries.to_vec();
        for navigation in flat {
            p.apply_with_sender(
                Event::ListingFinished {
                    pane: PaneId::Right,
                    navigation,
                    index: Arc::new(DirectoryIndex::from_entries(&rows)),
                    entries: rows.clone().into(),
                },
                &|_| true,
            );
        }
        assert!(p.mutation_settling(), "until the tree refresh completes");
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Refreshed {
                stamp,
                request: refresh,
                result: Ok(()),
            })),
            &|_| true,
        );
        assert!(!p.mutation_settling());
        assert!(
            p.panes[0]
                .tree_view()
                .unwrap()
                .rows
                .iter()
                .any(|row| row.entry.path == renamed)
        );
    }
}

/// A completed Copy from `source` into `destination`, which ran briefly.
fn copied(p: &mut Presenter, source: &Path, destination: &Path) -> Event {
    let mut progress = CopyProgress::queued(
        OperationId(5),
        destination.to_owned(),
        1,
        TransferIntent::Copy,
    );
    progress.source_directories = vec![source.to_owned()];
    progress.state = CopyOperationState::Running;
    p.copies.progress.push(Arc::new(progress.clone()));
    progress.state = CopyOperationState::Completed;
    Event::Copy(CopyEvent::Progress(Arc::new(progress)))
}

fn tree_refresh(sent: &RefCell<Vec<Command>>) -> Option<(TreeViewStamp, TreeRequestId)> {
    sent.borrow().iter().find_map(|command| match command {
        Command::Tree(command) => match command.as_ref() {
            TreeCommand::RefreshLoaded(request) => Some((request.view.stamp, request.request)),
            _ => None,
        },
        _ => None,
    })
}

/// A Copy into a folder a tree shows holds that tree until its refresh
/// completes; a Copy out of it changes nothing the tree shows and holds nothing.
#[test]
fn a_copy_holds_a_tree_only_when_it_shows_the_destination() {
    for into_tree in [true, false] {
        let (mut p, root) = setup();
        let inside = root.entries[0].path.clone();
        let elsewhere = test_root().join("elsewhere");
        let event = if into_tree {
            copied(&mut p, &elsewhere, &inside)
        } else {
            copied(&mut p, &inside, &elsewhere)
        };
        let sent = RefCell::new(Vec::new());
        p.apply_with_sender(event, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        let (stamp, request) =
            tree_refresh(&sent).expect("the tree refreshes its loaded branches either way");
        assert_eq!(p.mutation_settling_panes()[0], into_tree, "{into_tree}");
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Refreshed {
                stamp,
                request,
                result: Ok(()),
            })),
            &|_| true,
        );
        assert!(!p.mutation_settling_panes()[0]);
    }
}

/// A tree refresh that cannot start now (a full command queue) runs later
/// without a hold: the tree's input never waits for it.
#[test]
fn a_tree_hold_ends_when_its_refresh_cannot_start() {
    let (mut p, root) = setup();
    let inside = root.entries[0].path.clone();
    let event = copied(&mut p, &test_root().join("elsewhere"), &inside);
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(event, &|command| {
        let refused = matches!(&command, Command::Tree(command)
            if matches!(command.as_ref(), TreeCommand::RefreshLoaded(_)));
        sent.borrow_mut().push(command);
        !refused
    });
    assert!(tree_refresh(&sent).is_some(), "the refresh was attempted");
    assert!(!p.mutation_settling_panes()[0]);
    // It is sent once the queue has room, still without a hold.
    sent.borrow_mut().clear();
    p.flush_selection(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(tree_refresh(&sent).is_some());
    assert!(!p.mutation_settling_panes()[0]);
}

/// A Copy into the tree that completes while a Rename's refresh is pending
/// keeps the renamed entry as the cursor target.
#[test]
fn a_later_completion_keeps_an_earlier_tree_reveal() {
    let (mut p, root) = setup();
    let request = capture(&mut p);
    p.apply_with_sender(reply(&request), &|_| true);
    let id = p.operations.pending.expect("prepared tree rename");
    p.apply(Event::Operation(OperationEvent::Preview(Arc::new(
        OperationPreview {
            id,
            kind: MutationKind::Rename,
            files: 0,
            folders: 1,
            file_bytes: None,
            sample: vec![MutationItem {
                path: root.entries[0].path.join("child"),
                kind: EntryKind::Directory,
                metadata: Default::default(),
            }],
            trash_unavailable: None,
        },
    ))));
    p.confirm_operation(&id.0.to_string(), "renamed".into(), &|_| true);
    let mut progress = (*p.operations.progress[0]).clone();
    progress.state = OperationState::Completed;
    progress.completed = 1;
    progress.renamed_to = Some(root.entries[0].path.join("renamed"));
    p.apply_with_sender(
        Event::Operation(OperationEvent::Progress(Arc::new(progress))),
        &|_| true,
    );
    let event = copied(
        &mut p,
        &test_root().join("elsewhere"),
        &root.entries[0].path,
    );
    p.apply_with_sender(event, &|_| true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(renamed_publication(
            root.clone(),
            2,
        )))),
        &|_| true,
    );
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(12)),
        "the renamed entry still takes the cursor"
    );
}
