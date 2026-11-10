use super::*;
use slint::Model;

#[test]
fn ordered_prefix_and_streamed_tail_preserve_ids_with_constant_time_inverse_lookup() {
    let state = Rc::new(RefCell::new(Presenter::new(
        std::env::temp_dir(),
        std::env::temp_dir(),
    )));
    let id = state.borrow_mut().create_find(PaneId::Left).unwrap();
    state
        .borrow_mut()
        .start_find(id, FindForm::default(), &|_| true);
    let entries: Vec<_> = (1..=3)
        .map(|id| {
            Arc::new(FindResult {
                id: FindResultId(id),
                duplicate_group: None,
                duplicate_group_size: 0,
                entry: FindEntry {
                    path: std::env::temp_dir().join(format!("result-{id}")),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                    identity: TransferIdentity {
                        device: 1,
                        object: [id as u8; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
            })
        })
        .collect();
    state
        .borrow_mut()
        .finds
        .sessions
        .get_mut(&id)
        .unwrap()
        .results = entries;
    let rows = Rows {
        state: Rc::downgrade(&state),
        session: id,
        notify: ModelNotify::default(),
        structure: Cell::new((1, 2)),
        view: RefCell::new(Some(Arc::new(FindOrder {
            revision: 0,
            sort: FindSortSpec::default(),
            ids: vec![FindResultId(2), FindResultId(1)].into(),
            positions: vec![1, 0].into(),
        }))),
    };
    let prefix = rows.view.borrow().clone();
    let _ = rows.update(1, 3, prefix);
    let selected = rows.id_for_row(0).unwrap();
    let tail = rows.id_for_row(2).unwrap();
    assert_eq!(selected, FindResultId(2));
    assert_eq!(tail, FindResultId(3));
    let next = Arc::new(FindOrder {
        revision: 1,
        sort: FindSortSpec::default(),
        ids: vec![FindResultId(3), FindResultId(1), FindResultId(2)].into(),
        positions: vec![1, 2, 0].into(),
    });
    let _ = rows.update(1, 3, Some(next));
    assert_eq!(rows.row_for_id(selected), Some(2));
    assert_eq!(rows.row_for_id(tail), Some(0));
    assert_eq!(rows.row_data(0).unwrap().token.as_str(), "1:3");
    state
        .borrow_mut()
        .finds
        .sessions
        .get_mut(&id)
        .unwrap()
        .results
        .clear();
    let _ = rows.update(2, 0, None);
    assert_eq!(rows.id_for_row(0), None);
    assert_eq!(rows.row_for_id(selected), None);
}

#[test]
fn advanced_draft_maps_to_typed_query_without_enabling_hidden_content() {
    let draft = FindDraft {
        masks: "*.txt;*.log".into(),
        mode: 1,
        recursive: true,
        content: "Příliš 日本語".into(),
        content_enabled: true,
        content_case: true,
        whole_words: true,
        modified_enabled: true,
        modified_seconds: 5 * 60,
        duplicates: true,
        ..Default::default()
    };
    let converted = form(draft.clone());
    assert_eq!(converted.name_mode, FilterMode::Regex);
    assert_eq!(
        converted.modified_within,
        Some(std::time::Duration::from_secs(300))
    );
    assert!(converted.duplicates);
    let content = converted.content.unwrap();
    assert_eq!(content.text, "Příliš 日本語");
    assert!(content.whole_words && content.case_sensitive);

    let disabled = form(FindDraft {
        content_enabled: false,
        modified_enabled: false,
        ..draft
    });
    assert!(disabled.content.is_none());
    assert!(disabled.modified_within.is_none());
}

#[test]
fn large_duplicate_group_rows_keep_identity_and_group_boundaries_after_sort() {
    let state = Rc::new(RefCell::new(Presenter::new(
        std::env::temp_dir(),
        std::env::temp_dir(),
    )));
    let session = state.borrow_mut().create_find(PaneId::Left).unwrap();
    state
        .borrow_mut()
        .start_find(session, FindForm::default(), &|_| true);
    state
        .borrow_mut()
        .finds
        .sessions
        .get_mut(&session)
        .unwrap()
        .results = (1..=100)
        .map(|id| {
            Arc::new(FindResult {
                id: FindResultId(id),
                duplicate_group: Some(4),
                duplicate_group_size: 100,
                entry: FindEntry {
                    path: std::env::temp_dir().join(format!("result-{id}")),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                    identity: TransferIdentity {
                        device: 1,
                        object: [id as u8; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
            })
        })
        .collect();
    let rows = Rows {
        state: Rc::downgrade(&state),
        session,
        notify: ModelNotify::default(),
        structure: Cell::new((1, 100)),
        view: RefCell::new(None),
    };
    assert_eq!(
        rows.row_data(50).unwrap().duplicate_group.as_str(),
        "Group 4 · 100 files"
    );
    assert!(rows.row_data(0).unwrap().group_start);
    assert!(!rows.row_data(50).unwrap().group_start);
    let selected = rows.id_for_row(50).unwrap();
    let _ = rows.update(
        1,
        100,
        Some(Arc::new(FindOrder {
            revision: 1,
            sort: FindSortSpec::default(),
            ids: (1..=100).rev().map(FindResultId).collect::<Vec<_>>().into(),
            positions: (0..100).rev().collect::<Vec<_>>().into(),
        })),
    );
    assert_eq!(rows.row_for_id(selected), Some(49));
    assert_eq!(rows.row_data(49).unwrap().token.as_str(), "1:51");
    assert!(rows.row_data(0).unwrap().group_start);
    assert!(!rows.row_data(1).unwrap().group_start);
}

#[test]
fn duplicate_phases_and_counts_are_visible_and_cancellation_retains_diagnostics() {
    let mut presenter = Presenter::new(std::env::temp_dir(), std::env::temp_dir());
    let id = presenter.create_find(PaneId::Left).unwrap();
    presenter.start_find(
        id,
        FindForm {
            duplicates: true,
            ..Default::default()
        },
        &|_| true,
    );
    let session = presenter.finds.sessions.get_mut(&id).unwrap();
    session.progress.duplicate_candidates = 120;
    session.progress.duplicate_groups = 8;
    session.progress.duplicate_bytes = 4096;
    for (phase, label) in [
        (FindPhase::Scanning, "Searching…"),
        (FindPhase::HashingDuplicates, "Hashing candidates…"),
        (FindPhase::ComparingDuplicates, "Comparing contents…"),
    ] {
        session.progress.phase = phase;
        let caption = progress(session);
        assert!(caption.starts_with(label));
        assert!(caption.contains("8 groups · 120 candidates"));
    }
    session.stopping = true;
    assert!(progress(session).starts_with("Stopping…"));
    assert!(details(session).contains("8 verified groups · 120 candidates · 4096 bytes"));
}
