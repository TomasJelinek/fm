use super::*;

fn ready() -> Presenter {
    let root = test_root().join("metadata-editor");
    let mut p = Presenter::new(root.clone(), root.clone());
    let nav = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        nav,
        vec![
            Entry::new(EntryId(1), root.join("Mixed.TXT"), EntryKind::File),
            Entry::new(EntryId(2), root.join("Other.txt"), EntryKind::File),
        ],
    );
    p.panes[0].cursor = Some(EntryId(1));
    p.panes[0].parent_cursor = false;
    p
}

fn preview(revision: u64, patch: MetadataPatch) -> Event {
    Event::Operation(OperationEvent::MetadataPreview(Arc::new(MetadataPreview {
        id: OperationId(1),
        revision,
        patch,
        capabilities: MetadataCapabilities {
            rename: true,
            ..Default::default()
        },
        total: 1,
        conflicts: 0,
        unsupported: 0,
        hard_linked: 0,
        sample: vec![],
        errors: vec![],
    })))
}

#[test]
fn attributes_and_case_capture_targets_but_do_not_apply_changes_when_opened() {
    for enabled in [false, true] {
        let mut p = ready();
        let commands = RefCell::new(Vec::new());
        p.open_metadata(PaneId::Left, enabled, &|c| {
            commands.borrow_mut().push(c);
            true
        });
        let commands = commands.borrow();
        assert_eq!(commands.len(), 1);
        let Command::PrepareOperation(request) = &commands[0] else {
            panic!("prepare only")
        };
        assert_eq!(request.kind, MutationKind::EditMetadata);
        assert_eq!(request.focused, Some(EntryId(1)));
        assert_eq!(p.metadata.start_case, enabled);
        p.apply(preview(0, MetadataPatch::default()));
        assert!(
            p.operations.preview.is_none(),
            "metadata must not open the generic file dialog"
        );
        assert!(!p.metadata.can_apply());
    }
}

#[test]
fn dirty_draft_and_stale_preview_cannot_authorize_an_edit() {
    let mut p = ready();
    p.open_metadata(PaneId::Left, true, &|_| true);
    p.apply(preview(0, MetadataPatch::default()));
    let edit = MetadataPatch {
        case: Some(CaseChange::default()),
        ..Default::default()
    };
    p.preview_metadata("1", Ok(edit.clone()), &|_| true);
    let old_revision = p.metadata.revision;
    p.dirty_metadata("1");
    p.apply(preview(old_revision, edit.clone()));
    assert!(!p.metadata.can_apply());
    p.confirm_metadata("1", &old_revision.to_string(), 0, &|_| {
        panic!("stale apply")
    });
    p.preview_metadata("1", Ok(edit.clone()), &|_| true);
    let revision = p.metadata.revision;
    p.apply(preview(revision, edit));
    assert!(p.metadata.can_apply());
    p.confirm_metadata("other", &revision.to_string(), 0, &|_| {
        panic!("wrong operation")
    });
    p.confirm_metadata("1", &revision.to_string(), 0, &|_| false);
    assert!(
        p.metadata.can_apply(),
        "queue refusal must retain the reviewed draft"
    );
    let commands = RefCell::new(Vec::new());
    p.confirm_metadata("1", &revision.to_string(), 0, &|c| {
        commands.borrow_mut().push(c);
        true
    });
    assert!(
        matches!(commands.borrow()[0], Command::ConfirmMetadata { revision: r, .. } if r == revision)
    );
    assert!(p.metadata.id.is_none());
}

#[test]
fn refresh_can_inspect_an_unchanged_draft_without_authorizing_apply() {
    let mut p = ready();
    p.open_metadata(PaneId::Left, false, &|_| true);
    p.apply(preview(0, MetadataPatch::default()));
    let commands = RefCell::new(Vec::new());
    p.preview_metadata("1", Ok(MetadataPatch::default()), &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(
        matches!(commands.borrow().first(), Some(Command::PreviewMetadata { patch, .. }) if patch.is_empty())
    );
    let revision = p.metadata.revision;
    p.apply(preview(revision, MetadataPatch::default()));
    assert!(p.metadata.error.is_empty());
    assert!(!p.metadata.can_apply());
    p.confirm_metadata("1", &revision.to_string(), 0, &|_| {
        panic!("empty refresh must not mutate")
    });
}

#[test]
fn replacing_and_cancelling_metadata_defers_snapshot_cleanup() {
    let mut p = ready();
    p.open_metadata(PaneId::Left, false, &|_| true);
    p.apply(preview(0, MetadataPatch::default()));
    let old = Arc::downgrade(p.metadata.preview.as_ref().unwrap());
    p.apply(preview(0, MetadataPatch::default()));
    assert!(
        old.upgrade().is_some(),
        "old snapshot belongs to deferred cleanup"
    );
    drop(p.take_retired());
    assert!(old.upgrade().is_none());
    let current = Arc::downgrade(p.metadata.preview.as_ref().unwrap());
    p.cancel_metadata("1", &|_| true);
    assert!(
        current.upgrade().is_some(),
        "cancel hands the snapshot to backend reclamation"
    );
    drop(p.take_retired());
    assert!(current.upgrade().is_none());
}

#[test]
fn invalid_input_and_cancel_preserve_other_prepared_jobs() {
    let mut p = ready();
    p.open_metadata(PaneId::Left, false, &|_| true);
    p.apply(preview(0, MetadataPatch::default()));
    p.preview_metadata("1", Err("Enter an explicit timezone".into()), &|_| {
        panic!("invalid input")
    });
    assert!(p.metadata.error.contains("timezone"));
    assert!(!p.metadata.can_apply());
    p.cancel_metadata("999", &|_| panic!("wrong cancel"));
    assert_eq!(p.metadata.id, Some(OperationId(1)));
    p.cancel_metadata("1", &|_| false);
    assert_eq!(p.metadata.id, Some(OperationId(1)));
    p.cancel_metadata("1", &|_| true);
    p.apply(preview(0, MetadataPatch::default()));
    assert!(p.metadata.id.is_none());
}

#[test]
fn metadata_action_waits_for_captured_selection_and_rejects_navigation() {
    let mut p = ready();
    p.panes[0].selection_revision = SelectionRevision(8);
    p.panes[0].selection_resolved = SelectionRevision(7);
    p.open_metadata(PaneId::Left, false, &|_| panic!("must await selection"));
    assert!(p.metadata.id.is_none());
    let nav = p.panes[0].generation;
    let basis = p.panes[0].index.clone();
    let selection = Arc::new(SelectionSnapshot::from_bits(basis, vec![2]).unwrap());
    let commands = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: NavigationId(nav),
            revision: SelectionRevision(8),
            selection,
            deferred: false,
        },
        &|c| {
            commands.borrow_mut().push(c);
            true
        },
    );
    let commands = commands.borrow();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::PrepareOperation(request) => Some(request),
            _ => None,
        })
        .expect("prepared captured selection");
    assert!(
        request
            .selection
            .contains_path(&p.panes[0].path.join("Other.txt"))
    );
}

#[test]
fn navigation_cancels_a_metadata_intent_waiting_for_selection() {
    let mut p = ready();
    p.panes[0].selection_revision = SelectionRevision(8);
    p.panes[0].selection_resolved = SelectionRevision(7);
    let nav = p.panes[0].generation;
    let old = p.panes[0].selected.clone();
    p.open_metadata(PaneId::Left, true, &|_| panic!("must wait"));
    begin(
        &mut p,
        PaneId::Left,
        test_root().join("elsewhere").to_str().unwrap(),
    );
    p.apply_with_sender(
        Event::SelectionChanged {
            pane: PaneId::Left,
            navigation: NavigationId(nav),
            revision: SelectionRevision(8),
            selection: old,
            deferred: false,
        },
        &|command| {
            assert!(!matches!(command, Command::PrepareOperation(_)));
            true
        },
    );
    assert!(p.metadata.id.is_none());
}

#[test]
fn metadata_conflict_replies_require_the_exact_live_token_and_preserve_queue_refusal() {
    let mut p = ready();
    p.open_metadata(PaneId::Left, false, &|_| true);
    let conflict = |token| {
        Arc::new(MetadataConflict {
            id: OperationId(1),
            token,
            source: p.panes[0].path.join("Mixed.TXT"),
            destination: p.panes[0].path.join("mixed.txt"),
        })
    };
    let first = conflict(1);
    let second = conflict(2);
    p.apply(Event::Operation(OperationEvent::MetadataConflict(
        first.clone(),
    )));
    p.apply(Event::Operation(OperationEvent::MetadataConflict(second)));
    p.apply(Event::Operation(OperationEvent::MetadataConflict(first)));
    assert_eq!(p.operations.metadata_conflicts.len(), 1);
    assert_eq!(p.operations.metadata_conflicts[0].token, 2);
    p.decide_metadata_conflict("metadata:1:1", 1, &|_| panic!("stale decision"));
    p.decide_metadata_conflict("metadata:1:2", 2, &|_| panic!("replacement is unavailable"));
    p.decide_metadata_conflict("metadata:1:2", 1, &|_| false);
    assert_eq!(p.operations.metadata_conflicts.len(), 1);
    let commands = RefCell::new(Vec::new());
    p.decide_metadata_conflict("metadata:1:2", 1, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(p.operations.metadata_conflicts.is_empty());
    assert!(matches!(
        commands.borrow()[0],
        Command::ResolveMetadataConflict {
            id: OperationId(1),
            token: 2,
            choice: MetadataConflictPolicy::SkipAll,
        }
    ));
}

#[test]
fn conflicts_from_unknown_or_completed_jobs_do_not_open_dialogs() {
    let mut p = ready();
    let conflict = Arc::new(MetadataConflict {
        id: OperationId(1),
        token: 1,
        source: p.panes[0].path.join("Mixed.TXT"),
        destination: p.panes[0].path.join("mixed.txt"),
    });
    p.apply(Event::Operation(OperationEvent::MetadataConflict(
        conflict.clone(),
    )));
    assert!(p.operations.metadata_conflicts.is_empty());
    p.open_metadata(PaneId::Left, false, &|_| true);
    let mut done = (*p.operations.progress[0]).clone();
    done.state = OperationState::Completed;
    p.apply(Event::Operation(OperationEvent::Progress(Arc::new(done))));
    p.apply(Event::Operation(OperationEvent::MetadataConflict(conflict)));
    assert!(p.operations.metadata_conflicts.is_empty());
    assert!(p.metadata.id.is_none());
}
