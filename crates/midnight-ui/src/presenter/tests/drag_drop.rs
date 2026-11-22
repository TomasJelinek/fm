use super::*;

#[test]
fn dragging_a_native_nested_tree_occurrence_requests_exact_backend_capture() {
    let (mut p, root) = super::tree::fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    let row = p.panes[0].tree_row(1).unwrap();
    let identity = p.panes[0].row_identity(&row.entry);
    let sent = RefCell::new(Vec::new());
    assert!(p.begin_file_drag(PaneId::Left, 2, &identity, &|command| {
        sent.borrow_mut().push(command);
        true
    }));
    assert!(sent.borrow().iter().any(|command| matches!(
        command,
        Command::Tree(request) if matches!(request.as_ref(), TreeCommand::Clipboard(_))
    )));
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::PrepareDrag { .. }))
    );
}

fn loaded() -> (Presenter, PathBuf) {
    let base = test_root().join("drag 文档 & $(literal)");
    let mut presenter = Presenter::new(base.clone(), test_root().join("target"));
    let navigation = begin(&mut presenter, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut presenter,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(1), base.join("one"), EntryKind::File),
            Entry::new(EntryId(2), base.join("two"), EntryKind::File),
            Entry::new(EntryId(3), base.join("three"), EntryKind::File),
        ],
    );
    (presenter, base)
}

fn identity(presenter: &Presenter, entry: u64) -> String {
    let p = &presenter.panes[0];
    p.row_identity(p.entry_by_id(EntryId(entry)).expect("fixture row"))
}

#[test]
fn dragging_selected_group_preserves_selection_and_unselected_row_targets_itself() {
    let (mut p, base) = loaded();
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    let selected = p.panes[0].selected.clone();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    assert!(p.begin_file_drag(PaneId::Left, 2, &identity(&p, 2), &send));
    let Command::PrepareDrag { id, files, .. } = commands.borrow_mut().remove(0) else {
        panic!("selected group must prepare on the worker");
    };
    assert!(Arc::ptr_eq(&files.selection, &selected));
    assert_eq!(files.fallback.as_ref(), Some(&base.join("two")));
    p.cancel_file_drag(&send);
    commands.borrow_mut().clear();
    assert!(p.begin_file_drag(PaneId::Left, 3, &identity(&p, 3), &send));
    let Command::PrepareDrag {
        id: next, files, ..
    } = commands.borrow_mut().remove(0)
    else {
        panic!("unselected row must prepare itself");
    };
    assert_ne!(next, id);
    assert!(files.selection.is_empty());
    assert_eq!(files.fallback.as_ref(), Some(&base.join("three")));
    assert!(Arc::ptr_eq(&p.panes[0].selected, &selected));
}

#[test]
fn drag_waits_for_exact_pending_selection_then_rejects_late_preparation_after_release() {
    let (mut p, _) = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let Command::Selection(all) = commands.borrow_mut().remove(0) else {
        panic!()
    };
    assert!(p.begin_file_drag(PaneId::Left, 1, &identity(&p, 1), &send));
    assert!(commands.borrow().is_empty());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: all.navigation,
            revision: all.revision,
            selection: Arc::new(SelectionSnapshot::from_bits(all.view.unwrap(), vec![7]).unwrap()),
            deferred: Default::default(),
        },
        &send,
    );
    let id = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PrepareDrag { id, files, .. } => {
                assert_eq!(files.selection.len(), 3);
                Some(*id)
            }
            _ => None,
        })
        .unwrap();
    p.release_file_drag(None, InternalDropIntent::Copy, &send);
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::Prepared { id, item_count: 3 }),
        &send,
    );
    assert!(
        p.drag_native_ready().is_none(),
        "release must disarm delayed preparation"
    );
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::StartDrag(_)))
    );
}

#[test]
fn accepted_drop_retries_one_operation_identity_and_keeps_captured_destination() {
    let (mut p, _) = loaded();
    let destination = Arc::new(test_root().join("captured target"));
    let event = Event::DragDrop(DragDropEvent::DropAccepted {
        drag: None,
        intent: TransferIntent::Copy,
        verification: None,
        token: DropToken(7),
        pane: PaneId::Right,
        item_count: 2,
        destination: destination.clone(),
    });
    p.apply_with_sender(event.clone(), &|_| false);
    assert_eq!(p.copies.progress.len(), 1);
    let id = p.copies.progress[0].id;
    assert_eq!(p.copies.progress[0].destination_directory, *destination);
    p.apply_with_sender(event, &|_| false);
    assert_eq!(
        p.copies.progress.len(),
        1,
        "replayed token cannot create another UI job"
    );
    p.panes[1].path = test_root().join("new navigation");
    let sent = RefCell::new(Vec::new());
    p.flush_drag_commands(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let sent = sent.borrow();
    let [
        Command::AcceptDrop {
            token, id: actual, ..
        },
    ] = sent.as_slice()
    else {
        panic!("one admitted drop must enqueue exactly one handoff");
    };
    assert_eq!(*token, DropToken(7));
    assert_eq!(*actual, id);
    assert_eq!(p.copies.progress[0].destination_directory, *destination);
    p.flush_drag_commands(&|_| panic!("accepted handoff must not repeat"));
}

#[test]
fn accepted_internal_move_retains_its_frozen_verification_and_operation_label() {
    let (mut p, _) = loaded();
    assert_eq!(p.transfer_verification(), TransferVerification::FullSha256);
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::DropAccepted {
            drag: None,
            token: DropToken(8),
            pane: PaneId::Right,
            item_count: 1,
            destination: Arc::new(test_root().join("captured target")),
            intent: TransferIntent::Move,
            verification: Some(TransferVerification::SizeAndSamples),
        }),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert_eq!(p.copies.progress[0].intent, TransferIntent::Move);
    assert_eq!(
        p.copies.progress[0].verification,
        TransferVerification::SizeAndSamples
    );
    assert!(sent.borrow().iter().any(|command| matches!(
        command,
        Command::AcceptDrop {
            verification: TransferVerification::SizeAndSamples,
            ..
        }
    )));
}

#[test]
fn late_internal_acceptance_cannot_clear_marks_from_a_new_source_navigation() {
    let (mut p, base) = loaded();
    set_selected(&mut p, PaneId::Left, &[EntryId(1)]);
    assert!(p.begin_file_drag(PaneId::Left, 1, &identity(&p, 1), &|_| true));
    let drag = DragId(p.drag_drop.lifecycle.active().unwrap().id);
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::Prepared {
            id: drag,
            item_count: 1,
        }),
        &|_| true,
    );
    let target = p.preview_internal_drag_target(PaneId::Left, None).unwrap();
    p.release_file_drag(Some(target), InternalDropIntent::Copy, &|_| true);
    let away = test_root().join("another location");
    let navigation = begin(&mut p, PaneId::Left, away.to_str().unwrap());
    finish(&mut p, PaneId::Left, navigation, Vec::new());
    let navigation = begin(&mut p, PaneId::Left, base.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(EntryId(101), base.join("one"), EntryKind::File)],
    );
    set_selected(&mut p, PaneId::Left, &[EntryId(101)]);
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::DropAccepted {
            drag: Some(drag),
            token: DropToken(12),
            pane: PaneId::Right,
            item_count: 1,
            destination: Arc::new(test_root().join("accepted target")),
            intent: TransferIntent::Copy,
            verification: Some(TransferVerification::FullSha256),
        }),
        &|_| true,
    );
    let operation = p.copies.progress[0].id;
    p.apply(Event::Copy(CopyEvent::RootsCompleted {
        id: operation,
        sources: vec![CompletedCopySource::Native(base.join("one"))],
    }));
    let sent = RefCell::new(Vec::new());
    p.flush_copy_selection(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Selection(_))),
        "a late acceptance must retain the old gesture origin, not clear new marks"
    );
}

#[test]
fn a_drop_sooner_than_the_minimum_drag_duration_cancels_and_one_at_it_proceeds() {
    use crate::presenter::drag_drop::MIN_DRAG_DURATION;
    use std::time::Duration;
    let (mut p, _) = loaded();
    let other = test_root().join("target");
    let navigation = begin(&mut p, PaneId::Right, other.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Right,
        navigation,
        vec![Entry::new(
            EntryId(9),
            other.join("folder 資料"),
            EntryKind::Directory,
        )],
    );
    let folder = {
        let right = &p.panes[1];
        let entry = right.entry_by_id(EntryId(9)).unwrap();
        (right.parent_offset(), right.row_identity(entry))
    };
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(2)]);
    let selected = p.panes[0].selected.clone();
    let quick = MIN_DRAG_DURATION - Duration::from_millis(1);
    for (held, onto_folder, intent) in [
        (quick, false, InternalDropIntent::Auto),
        (quick, true, InternalDropIntent::Move),
        (MIN_DRAG_DURATION, false, InternalDropIntent::Move),
        (MIN_DRAG_DURATION, true, InternalDropIntent::Copy),
    ] {
        let sent = RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        assert!(p.begin_file_drag(PaneId::Left, 2, &identity(&p, 2), &send));
        assert!(
            p.quick_drag_note(PaneId::Left).is_none(),
            "a new drag outdates the note"
        );
        let id = DragId(p.drag_drop.lifecycle.active().unwrap().id);
        p.apply_with_sender(
            Event::DragDrop(DragDropEvent::Prepared { id, item_count: 2 }),
            &send,
        );
        let cursor = p.panes[0].cursor;
        let row = onto_folder.then_some((folder.0, folder.1.as_str()));
        let target = p.preview_internal_drag_target(PaneId::Right, row).unwrap();
        sent.borrow_mut().clear();
        let noted = p.release_file_drag_held(held, Some(target.clone()), intent, &send);
        let sent = sent.into_inner();
        let drops: Vec<_> = sent
            .iter()
            .filter_map(|command| match command {
                Command::InternalDrop(request) => Some(request),
                _ => None,
            })
            .collect();
        let note = p.panel_details(PaneId::Left).remove(0);
        if held < MIN_DRAG_DURATION {
            assert_eq!(noted, Some(PaneId::Left), "{held:?}");
            assert!(drops.is_empty(), "a quick release transfers nothing");
            assert!(
                !sent
                    .iter()
                    .any(|command| matches!(command, Command::StartDrag(_)))
            );
            assert!(
                sent.iter()
                    .any(|command| matches!(command, Command::CancelDrag(actual) if *actual == id)),
                "the prepared sources are released"
            );
            assert!(p.drag_drop.lifecycle.active().is_none());
            assert!(p.drag_drop.hover.is_none() && !p.drag_drop.pointer_down);
            assert_eq!(p.panes[0].cursor, cursor);
            assert!(Arc::ptr_eq(&p.panes[0].selected, &selected));
            assert_eq!(note.label, "Drag");
            assert_eq!(note.value, "Drag cancelled: released within 0.4 s");
            assert!(
                p.panel_details(PaneId::Right)
                    .iter()
                    .all(|field| field.label != "Drag"),
                "only the source panel notes it"
            );
        } else {
            assert_eq!(noted, None);
            assert_eq!(drops.len(), 1, "{held:?} drops as before");
            assert_eq!(drops[0].id, id);
            assert_eq!(drops[0].intent, intent, "Ctrl and Shift choices are kept");
            assert!(match (&drops[0].destination, &target) {
                (
                    InternalDropDestination::PanelEntry {
                        pane,
                        entry: dropped,
                        ..
                    },
                    InternalDropDestination::PanelEntry { entry, .. },
                ) => *pane == PaneId::Right && dropped == entry && *entry == EntryId(9),
                (
                    InternalDropDestination::PaneRoot { pane, navigation },
                    InternalDropDestination::PaneRoot {
                        navigation: wanted, ..
                    },
                ) => *pane == PaneId::Right && navigation == wanted,
                _ => false,
            });
            assert_ne!(note.label, "Drag");
        }
    }
    // A release without a gesture (a drag already ended) notes nothing.
    assert_eq!(
        p.release_file_drag_held(Duration::ZERO, None, InternalDropIntent::Auto, &|_| true),
        None
    );
    assert!(p.quick_drag_note(PaneId::Left).is_none());
}

#[test]
fn stale_rows_parent_and_changed_projection_never_prepare_a_drag() {
    let (mut p, _) = loaded();
    let token = identity(&p, 1);
    assert!(!p.begin_file_drag(PaneId::Left, 0, &token, &|_| panic!()));
    p.panes[0].search.view_revision += 1;
    assert!(!p.begin_file_drag(PaneId::Left, 1, &token, &|_| panic!()));
    assert!(!p.begin_file_drag(PaneId::Left, usize::MAX, &token, &|_| panic!()));
}

#[test]
fn exhausted_operation_ids_reject_an_admitted_drop_without_creating_a_job() {
    let (mut p, _) = loaded();
    p.operations.next_id = u64::MAX;
    let event = Event::DragDrop(DragDropEvent::DropAccepted {
        drag: None,
        intent: TransferIntent::Copy,
        verification: None,
        token: DropToken(7),
        pane: PaneId::Right,
        item_count: 1,
        destination: Arc::new(test_root().join("captured target")),
    });
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(event.clone(), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(p.copies.progress.is_empty());
    assert_eq!(p.operations.next_id, u64::MAX);
    assert!(matches!(
        sent.borrow().as_slice(),
        [Command::RejectDrop(DropToken(7))]
    ));
    p.apply_with_sender(event, &|_| panic!("a replayed token must stay retired"));
}

#[test]
fn refused_external_drop_reports_its_reason_and_keeps_native_dragging() {
    let (mut p, _) = loaded();
    let send = |_: Command| true;
    p.attach_drag_host(NativeDragWindow::Windows { hwnd: 1 }, &send);
    let host_generation = p.drag_drop.host.as_ref().unwrap().generation;
    let ready = |supported| {
        Event::DragDrop(DragDropEvent::Ready {
            host_generation,
            supported,
        })
    };
    p.apply_with_sender(ready(true), &send);
    assert_eq!(p.drag_drop.supported, Some(true));
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::Failed {
            id: None,
            error: FsError::from_io(
                "Windows file drag/drop",
                Path::new(""),
                io::Error::other("The drop destination changed."),
            ),
        }),
        &send,
    );
    // The reason alone: no repeated operation name and no empty path suffix.
    assert_eq!(p.desktop.status, "Drag/drop: The drop destination changed.");
    let path = test_root().join("拖放 & $(literal)").join("e\u{301}.txt");
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::Failed {
            id: None,
            error: FsError::from_io(
                "Windows file drag/drop",
                &path,
                io::Error::other("The drag source did not provide its file list.\nDetails"),
            ),
        }),
        &send,
    );
    assert_eq!(
        p.desktop.status,
        format!(
            "Drag/drop: The drag source did not provide its file list. ({})",
            path.display()
        )
    );
    assert_eq!(
        p.drag_drop.supported,
        Some(true),
        "a refused drop must not disable native dragging"
    );
    p.apply_with_sender(ready(false), &send);
    assert_eq!(
        p.drag_drop.supported,
        Some(false),
        "a lost session still revokes native dragging"
    );
}

#[test]
fn unavailable_native_gesture_keeps_the_next_gesture_eligible_for_native_dragging() {
    let (mut p, _) = loaded();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.drag_drop.supported = Some(true);
    let mut previous = None;
    for attempt in 0..2 {
        assert!(p.begin_file_drag(PaneId::Left, 1, &identity(&p, 1), &send));
        let id = sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PrepareDrag { id, .. } => Some(*id),
                _ => None,
            })
            .unwrap();
        assert_ne!(Some(id), previous);
        p.apply_with_sender(
            Event::DragDrop(DragDropEvent::Prepared { id, item_count: 1 }),
            &send,
        );
        assert_eq!(p.drag_native_ready(), Some(id), "native attempt {attempt}");
        assert!(p.arm_native_drag(id));
        p.apply_with_sender(
            Event::DragDrop(DragDropEvent::Finished {
                id,
                outcome: DragFinish::Unavailable,
            }),
            &send,
        );
        assert_eq!(p.drag_native_ready(), None, "do not retry the same gesture");
        p.release_file_drag(None, InternalDropIntent::Copy, &send);
        previous = Some(id);
    }
}

#[test]
fn drag_hover_is_latest_only_and_escape_cancels_under_queue_backpressure() {
    use crate::presenter::drag_drop::preview::Status;
    let (mut p, _) = loaded();
    assert!(p.begin_file_drag(PaneId::Left, 1, &identity(&p, 1), &|_| true));
    let id = DragId(p.drag_drop.lifecycle.active().unwrap().id);
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::Prepared { id, item_count: 1 }),
        &|_| true,
    );
    let destination = InternalDropDestination::PaneRoot {
        pane: PaneId::Left,
        navigation: NavigationId(p.panes[0].generation),
    };
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.update_drag_preview(Some(destination.clone()), InternalDropIntent::Auto, &send);
    let first = match sent.borrow_mut().remove(0) {
        Command::PreviewInternalDrop(request) => *request,
        _ => panic!("hover preview"),
    };
    assert_eq!(p.drag_preview_status(), Status::Pending);
    p.update_drag_preview(Some(destination.clone()), InternalDropIntent::Copy, &send);
    let second = match sent.borrow_mut().remove(0) {
        Command::PreviewInternalDrop(request) => *request,
        _ => panic!("changed modifier preview"),
    };
    assert!(first.cancel.is_cancelled());
    assert!(second.revision.0 > first.revision.0);
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::InternalPreview {
            id,
            revision: first.revision,
            intent: Some(TransferIntent::Move),
        }),
        &send,
    );
    assert_eq!(
        p.drag_preview_status(),
        Status::Pending,
        "late reply cannot claim Move for the current Copy gesture"
    );
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::InternalPreview {
            id,
            revision: second.revision,
            intent: Some(TransferIntent::Copy),
        }),
        &send,
    );
    assert_eq!(
        p.drag_preview_status(),
        Status::Allowed(TransferIntent::Copy)
    );
    assert!(sent.borrow().is_empty(), "hover never admits a transfer");
    p.cancel_file_drag(&|_| false);
    assert!(
        second.cancel.is_cancelled(),
        "Escape revokes worker checks even with a full command queue"
    );
    assert!(p.drag_drop.lifecycle.active().is_none());
    p.apply_with_sender(
        Event::DragDrop(DragDropEvent::InternalPreview {
            id,
            revision: second.revision,
            intent: Some(TransferIntent::Move),
        }),
        &|_| false,
    );
    assert_eq!(p.drag_preview_status(), Status::Rejected);
    p.release_file_drag(Some(destination), InternalDropIntent::Move, &send);
    assert!(
        sent.borrow()
            .iter()
            .all(|command| !matches!(command, Command::InternalDrop(_)))
    );
}

#[test]
fn flat_file_drop_targets_the_root_only_with_current_native_identity() {
    for kind in [
        EntryKind::File,
        EntryKind::Directory,
        EntryKind::DirectoryLink,
        EntryKind::Symlink,
        EntryKind::Other,
    ] {
        let base = test_root().join("drop target kinds");
        let mut p = Presenter::new(base.clone(), base.clone());
        let navigation = begin(&mut p, PaneId::Left, base.to_str().unwrap());
        finish(
            &mut p,
            PaneId::Left,
            navigation,
            vec![Entry::new(EntryId(1), base.join("target"), kind)],
        );
        let current = identity(&p, 1);
        let destination = p.preview_internal_drag_target(PaneId::Left, Some((1, &current)));
        match kind {
            EntryKind::File => assert!(matches!(destination,
                Some(InternalDropDestination::PaneRoot { pane: PaneId::Left, navigation: captured })
                    if captured == navigation
            )),
            EntryKind::Directory => assert!(matches!(destination,
                Some(InternalDropDestination::PanelEntry { pane: PaneId::Left, navigation: captured, entry: EntryId(1) })
                    if captured == navigation
            )),
            _ => assert!(destination.is_none()),
        }
        assert!(
            p.preview_internal_drag_target(PaneId::Left, Some((1, "stale-identity")))
                .is_none()
        );
        assert!(
            p.preview_internal_drag_target(PaneId::Left, Some((0, &current)))
                .is_none()
        );
        assert!(
            p.preview_internal_drag_target(PaneId::Left, Some((2, &current)))
                .is_none()
        );
    }
}

#[test]
fn watch_refresh_waits_for_a_drag_from_its_pane_and_refused_navigation_keeps_it() {
    let (mut p, _) = loaded();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(p.begin_file_drag(PaneId::Left, 1, &identity(&p, 1), &send));
    let committed = p.panes[0].successful_generation.unwrap();
    sent.borrow_mut().clear();
    assert!(!p.refresh_if_changed(PaneId::Left, committed, &send));
    p.flush_selection(&send);
    assert!(
        p.drag_drop.lifecycle.active().is_some(),
        "the gesture survives"
    );
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Refresh { .. } | Command::CancelDrag(_))),
        "a watch hint neither re-reads the source nor cancels the drag"
    );
    assert!(
        !p.navigate(PaneId::Left, test_root().join("elsewhere"), None, &|_| {
            false
        })
    );
    assert!(
        p.drag_drop.lifecycle.active().is_some(),
        "a refused navigation leaves the drag alive"
    );
    p.cancel_file_drag(&send);
    p.flush_selection(&send);
    assert!(
        sent.borrow().iter().any(|command| matches!(
            command,
            Command::Refresh {
                pane: PaneId::Left,
                ..
            }
        )),
        "the retained hint re-reads the folder after the gesture"
    );
}
