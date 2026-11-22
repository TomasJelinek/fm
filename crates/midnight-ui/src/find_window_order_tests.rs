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
                archive: None,
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
        content_regex: true,
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
    assert!(content.whole_words && content.case_sensitive && content.regex);

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
                archive: None,
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
fn duplicate_phases_and_localized_counts_are_visible_and_cancellation_retains_diagnostics() {
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
    session.progress.duplicate_candidates = 1_200;
    session.progress.duplicate_groups = 8;
    session.progress.duplicate_bytes = 4_096;
    let locale = NumberLocale {
        decimal_separator: ",".into(),
        group_separator: ".".into(),
        grouping: vec![3],
        repeat_last_group: true,
    };
    for (phase, label) in [
        (FindPhase::Scanning, "Searching…"),
        (FindPhase::HashingDuplicates, "Hashing candidates…"),
        (FindPhase::ComparingDuplicates, "Comparing contents…"),
    ] {
        session.progress.phase = phase;
        let caption = progress(session, &locale);
        assert!(caption.starts_with(label));
        assert!(caption.contains("8 groups · 1.200 candidates"), "{caption}");
    }
    session.stopping = true;
    assert!(progress(session, &locale).starts_with("Stopping…"));
    assert!(
        details(session, &locale).contains("8 verified groups · 1.200 candidates · 4.096 bytes")
    );
    // A cancelled run keeps its diagnostics and names the limits in plain words.
    let unreadable = std::env::temp_dir().join("unreadable\nReached limits: forged");
    session.stopping = false;
    session.running = false;
    session.summary = Some(FindSummary {
        completion: FindCompletion::Cancelled,
        progress: session.progress.clone(),
        reached_budgets: vec![FindBudget::TotalContent, FindBudget::DuplicateCandidates],
        diagnostics: vec![FsError::from_io(
            "read content",
            &unreadable,
            std::io::Error::other("Access denied"),
        )],
    });
    assert!(progress(session, &locale).starts_with("Cancelled"));
    let text = details(session, &locale);
    assert!(
        text.contains("Reached limits: total content read, duplicate candidates"),
        "{text}"
    );
    assert!(text.contains("Access denied"), "{text}");
    assert_eq!(text.matches("\nReached limits:").count(), 1, "{text}");
}

/// Counts what a model tells its listeners.
#[derive(Default)]
struct Notifications {
    resets: Cell<usize>,
    rows: Cell<usize>,
}
impl i_slint_core::model::ModelChangeListener for Notifications {
    fn row_changed(self: std::pin::Pin<&Self>, _: usize) {
        self.rows.set(self.rows.get() + 1);
    }
    fn row_added(self: std::pin::Pin<&Self>, _: usize, _: usize) {
        self.rows.set(self.rows.get() + 1);
    }
    fn row_removed(self: std::pin::Pin<&Self>, _: usize, _: usize) {
        self.rows.set(self.rows.get() + 1);
    }
    fn reset(self: std::pin::Pin<&Self>) {
        self.resets.set(self.resets.get() + 1);
    }
}

/// A session with `count` results, shown in reverse (a backend sort), and the window's
/// row and mark models over it.
fn reversed_marks(count: u64) -> (Rc<RefCell<Presenter>>, FindSessionId, Rc<Rows>, Marks) {
    let state = Rc::new(RefCell::new(Presenter::new(
        std::env::temp_dir(),
        std::env::temp_dir(),
    )));
    let id = state.borrow_mut().create_find(PaneId::Left).unwrap();
    state
        .borrow_mut()
        .start_find(id, FindForm::default(), &|_| true);
    let order = Arc::new(FindOrder {
        revision: 0,
        sort: FindSortSpec::default(),
        ids: (1..=count)
            .rev()
            .map(FindResultId)
            .collect::<Vec<_>>()
            .into(),
        positions: (0..count as usize).rev().collect::<Vec<_>>().into(),
    });
    {
        let mut p = state.borrow_mut();
        let session = p.finds.sessions.get_mut(&id).unwrap();
        session.results = (1..=count)
            .map(|n| {
                Arc::new(FindResult {
                    id: FindResultId(n),
                    duplicate_group: None,
                    duplicate_group_size: 0,
                    archive: None,
                    entry: FindEntry {
                        path: std::env::temp_dir().join(format!("result-{n}.txt")),
                        kind: EntryKind::File,
                        metadata: EntryMetadata {
                            size: Some(n),
                            ..EntryMetadata::default()
                        },
                        identity: TransferIdentity {
                            device: 1,
                            object: [n as u8; 16],
                        },
                        revision: FindRevision([0; 6]),
                    },
                })
            })
            .collect();
        session.order = Some(order.clone());
    }
    let rows = Rc::new(Rows {
        state: Rc::downgrade(&state),
        session: id,
        notify: ModelNotify::default(),
        structure: Cell::new((1, count as usize)),
        view: RefCell::new(Some(order)),
    });
    let marks = Marks {
        rows: rows.clone(),
        notify: ModelNotify::default(),
        shown: Cell::new(None),
    };
    (state, id, rows, marks)
}

#[test]
fn marks_read_the_selection_by_identity_and_notify_once_per_change() {
    use i_slint_core::model::ModelChangeListenerContainer;
    let (state, id, _rows, marks) = reversed_marks(1_000);
    let listener = Box::pin(ModelChangeListenerContainer::new(Notifications::default()));
    marks
        .model_tracker()
        .attach_peer(listener.as_ref().model_peer());
    let select = |select| {
        state.borrow_mut().find_select(id, select);
        marks.show(state.borrow().finds.sessions[&id].selection_revision)
    };
    // Ctrl+A over a thousand results: one notification, every row marked.
    assert!(select(FindSelect::All));
    assert_eq!((listener.resets.get(), listener.rows.get()), (1, 0));
    assert!((0..1_000).all(|row| marks.row_data(row) == Some(true)));
    // The same selection shown again sends nothing.
    assert!(!marks.show(state.borrow().finds.sessions[&id].selection_revision));
    assert_eq!(listener.resets.get(), 1);
    // A click on result 1, shown last: one notification, only its row is marked.
    assert!(select(FindSelect::Click {
        result: FindResultId(1),
        toggle: false,
        range: false,
    }));
    assert_eq!(listener.resets.get(), 2);
    assert_eq!(marks.row_data(999), Some(true));
    assert_eq!(
        (0..1_000)
            .filter(|row| marks.row_data(*row) == Some(true))
            .count(),
        1
    );
    // Shift+click on result 1000, shown first: a range over every row, one notification.
    assert!(select(FindSelect::Click {
        result: FindResultId(1_000),
        toggle: false,
        range: true,
    }));
    assert_eq!((listener.resets.get(), listener.rows.get()), (2 + 1, 0));
    assert!((0..1_000).all(|row| marks.row_data(row) == Some(true)));
    assert_eq!(marks.row_data(1_000), None, "no row past the shown ones");
}

/// Opt-in release measurement of the window's share at 100,000 results:
/// `cargo test --release -p midnight-ui --lib find_window_marks_measure_100k --
/// --ignored --nocapture`.
#[test]
#[ignore = "measurement; run in release with --ignored --nocapture"]
fn find_window_marks_measure_100k_results() {
    const COUNT: u64 = 100_000;
    let (state, id, rows, marks) = reversed_marks(COUNT);
    let best = |label: &str, action: &mut dyn FnMut()| {
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            action();
            best = best.min(start.elapsed());
        }
        println!("{label}: {best:?} (best of 5)");
    };
    let revision = || state.borrow().finds.sessions[&id].selection_revision;
    best("Ctrl+A, then one notification", &mut || {
        state.borrow_mut().find_select(id, FindSelect::All);
        assert!(marks.show(revision()));
    });
    best("a visible page of 40 marks read", &mut || {
        for row in 50_000..50_040 {
            assert_eq!(marks.row_data(row), Some(true));
        }
    });
    best("a visible page of 40 rows formatted", &mut || {
        for row in 50_000..50_040 {
            assert!(rows.row_data(row).is_some());
        }
    });
    best(
        "Shift+click range of 100,000, then one notification",
        &mut || {
            state.borrow_mut().find_select(
                id,
                FindSelect::Click {
                    result: FindResultId(COUNT),
                    toggle: false,
                    range: false,
                },
            );
            state.borrow_mut().find_select(
                id,
                FindSelect::Click {
                    result: FindResultId(1),
                    toggle: false,
                    range: true,
                },
            );
            assert!(marks.show(revision()));
        },
    );
}
