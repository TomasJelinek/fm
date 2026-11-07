use super::*;

fn stable_panel() -> Presenter {
    let root = test_root().join("rectangle 文档");
    let mut p = Presenter::new(root.clone(), root.clone());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        (0..6)
            .map(|row| {
                Entry::new(
                    EntryId(100 + row * 17),
                    root.join(format!("é{row}")),
                    EntryKind::File,
                )
            })
            .collect(),
    );
    p.panes[0].selection_rebase_pending = false;
    p
}
fn ranges() -> SelectionRanges {
    SelectionRanges::new(
        6,
        vec![
            SelectionSpan { first: 1, last: 2 },
            SelectionSpan { first: 5, last: 5 },
        ],
    )
    .unwrap()
}

#[test]
fn rectangle_preview_is_immediate_and_refused_commit_restores_original_selection() {
    let mut p = stable_panel();
    let original = p.panes[0].selected.clone();
    let capture = p.capture_rectangle(PaneId::Left).unwrap();
    assert!(p.preview_rectangle(PaneId::Left, &capture, ranges(), SelectionMode::Replace));
    let marks: Vec<_> = p.panes[0]
        .entries
        .iter()
        .map(|entry| p.panes[0].selection_visible(entry))
        .collect();
    assert_eq!(marks, vec![false, true, true, false, false, true]);
    assert!(Arc::ptr_eq(&original, &p.panes[0].selected));
    assert!(!p.commit_rectangle(
        PaneId::Left,
        &capture,
        ranges(),
        SelectionMode::Replace,
        &|_| false
    ));
    assert!(p.panes[0].rectangle_preview.is_none());
    assert!(
        p.panes[0]
            .entries
            .iter()
            .all(|entry| !p.panes[0].selection_visible(entry))
    );
    assert!(Arc::ptr_eq(&original, &p.panes[0].selected));
    assert!(p.panes[0].error.contains("queue is busy"));
}

#[test]
fn changed_view_and_selection_revision_cancel_without_retargeting() {
    for change_revision in [false, true] {
        let mut p = stable_panel();
        let capture = p.capture_rectangle(PaneId::Left).unwrap();
        assert!(p.preview_rectangle(PaneId::Left, &capture, ranges(), SelectionMode::Add));
        if change_revision {
            p.panes[0].selection_revision = SelectionRevision(1);
        } else {
            p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
        }
        assert!(!p.rectangle_current(PaneId::Left, &capture));
        assert!(
            p.panes[0]
                .entries
                .iter()
                .all(|entry| !p.panes[0].selection_visible(entry))
        );
        assert!(!p.commit_rectangle(
            PaneId::Left,
            &capture,
            ranges(),
            SelectionMode::Add,
            &|_| panic!("stale gesture queued")
        ));
    }
}

#[test]
fn successful_release_sends_one_frozen_view_action_and_keeps_preview_until_backend_ack() {
    let mut p = stable_panel();
    let capture = p.capture_rectangle(PaneId::Left).unwrap();
    let commands = RefCell::new(Vec::new());
    assert!(p.commit_rectangle(
        PaneId::Left,
        &capture,
        ranges(),
        SelectionMode::Toggle,
        &|command| {
            commands.borrow_mut().push(command);
            true
        }
    ));
    let Command::Selection(request) = commands.borrow_mut().pop().unwrap() else {
        panic!()
    };
    assert!(Arc::ptr_eq(
        request.view.as_ref().unwrap(),
        &p.panes[0].index
    ));
    assert_eq!(request.revision, SelectionRevision(1));
    let SelectionAction::Ranges { ranges, mode } = request.action else {
        panic!()
    };
    assert_eq!(mode, SelectionMode::Toggle);
    assert_eq!(ranges.spans().len(), 2);
    assert!(p.panes[0].rectangle_preview.is_none());
    assert!(p.panes[0].selection_visible(&p.panes[0].entries[1]));
    assert!(commands.borrow().is_empty());
}

#[test]
fn obsolete_index_is_not_kept_alive_by_gesture_and_stale_archive_payload_is_retired() {
    let mut p = stable_panel();
    let capture = p.capture_rectangle(PaneId::Left).unwrap();
    let old = Arc::downgrade(&p.panes[0].index);
    p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
    assert!(
        old.upgrade().is_none(),
        "capture must not keep a large old index alive on UI"
    );
    assert!(!p.rectangle_current(PaneId::Left, &capture));
    let view = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
    let weak = Arc::downgrade(&view);
    p.apply(Event::ArchiveSelection {
        pane: PaneId::Left,
        navigation: NavigationId(0),
        revision: SelectionRevision(1),
        view,
        result: Ok(ArchiveSelectionResult::default()),
    });
    assert!(
        weak.upgrade().is_some(),
        "last obsolete response index must be retired"
    );
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak.upgrade().is_none());
}
