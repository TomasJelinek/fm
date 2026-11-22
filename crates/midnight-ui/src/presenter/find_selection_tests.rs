//! The Find window's multi-selection without a window: identity across streamed and
//! sorted results, clicks, keys, the menu, copies and Feed to panel.
use super::*;
use crate::presenter::find::FindForm;
use std::cell::RefCell;

fn root() -> PathBuf {
    std::env::temp_dir().join("find-selection")
}

/// Result `id`, a file `name` of `size` bytes in `root()`, or a folder without a size.
fn result(id: u64, name: &str, size: Option<u64>) -> Arc<FindResult> {
    Arc::new(FindResult {
        id: FindResultId(id),
        entry: FindEntry {
            path: root().join(name),
            kind: if size.is_some() {
                EntryKind::File
            } else {
                EntryKind::Directory
            },
            metadata: EntryMetadata {
                size,
                ..EntryMetadata::default()
            },
            identity: TransferIdentity {
                device: 1,
                object: [id as u8; 16],
            },
            revision: FindRevision([0; 6]),
        },
        duplicate_group: None,
        duplicate_group_size: 0,
        archive: None,
    })
}

/// Results `first..first + count`, `file-N.txt` of N bytes.
fn batch(first: u64, count: u64) -> Arc<[Arc<FindResult>]> {
    (first..first + count)
        .map(|id| result(id, &format!("file-{id}.txt"), Some(id)))
        .collect()
}

fn update(p: &mut Presenter, id: FindSessionId, update: FindUpdate) {
    p.apply(Event::Find(Box::new(FindEvent {
        session: id,
        generation: FindGeneration(1),
        update,
    })));
}

/// A running Find of the left panel that streamed results 1–`count`.
fn streaming(count: u64) -> (Presenter, FindSessionId) {
    let mut p = Presenter::new(root(), std::env::temp_dir().join("find-selection-target"));
    let id = p.create_find(PaneId::Left).unwrap();
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    update(&mut p, id, FindUpdate::Batch(batch(1, count)));
    (p, id)
}

/// A sort of the first results, shown in the order of `ids`.
fn sorted(ids: &[u64]) -> FindUpdate {
    let mut positions = vec![0; ids.len()];
    for (index, id) in ids.iter().enumerate() {
        positions[*id as usize - 1] = index;
    }
    FindUpdate::Sorted(Arc::new(FindOrder {
        revision: 0,
        sort: FindSortSpec::default(),
        ids: ids.iter().map(|id| FindResultId(*id)).collect(),
        positions: positions.into(),
    }))
}

/// The selected result IDs in the order shown.
fn selected(p: &Presenter, id: FindSessionId) -> Vec<u64> {
    let s = &p.finds.sessions[&id];
    let shown = Shown::of(s);
    (0..shown.len())
        .filter_map(|index| shown.slot_at(index))
        .filter(|slot| s.selection.is_selected(*slot))
        .map(|slot| slot as u64 + 1)
        .collect()
}

fn cursor(p: &Presenter, id: FindSessionId) -> Option<u64> {
    p.finds.sessions[&id]
        .selection
        .cursor()
        .map(|slot| slot as u64 + 1)
}

fn click(p: &mut Presenter, id: FindSessionId, result: u64, toggle: bool, range: bool) {
    assert!(p.find_select(
        id,
        FindSelect::Click {
            result: FindResultId(result),
            toggle,
            range,
        }
    ));
}

fn key(p: &mut Presenter, id: FindSessionId, result: u64, extend: bool, keep: bool) {
    assert!(p.find_select(
        id,
        FindSelect::Move {
            result: FindResultId(result),
            extend,
            keep,
        }
    ));
}

/// The text a copy action queued for the clipboard.
fn copied(p: &mut Presenter, id: FindSessionId, row: &str, copy: FindCopy) -> String {
    let commands = RefCell::new(Vec::new());
    assert!(p.find_copy(id, row, copy, &|command| {
        commands.borrow_mut().push(command);
        true
    }));
    match commands.into_inner().pop() {
        Some(Command::CopyText { text, .. }) => text,
        other => panic!("a copy queues clipboard text, not {other:?}"),
    }
}

#[test]
fn selection_is_kept_by_identity_while_results_stream_and_are_sorted() {
    let (mut p, id) = streaming(4);
    click(&mut p, id, 2, false, false);
    click(&mut p, id, 4, true, false);
    assert_eq!(selected(&p, id), [2, 4]);
    let revision = p.finds.sessions[&id].selection_revision;
    // More results stream in: they arrive unselected, the selection and cursor stay.
    update(&mut p, id, FindUpdate::Batch(batch(5, 3)));
    assert_eq!(selected(&p, id), [2, 4]);
    assert_eq!(cursor(&p, id), Some(4));
    assert_eq!(p.finds.sessions[&id].selection_revision, revision);
    // A sort moves the rows; the same results stay selected wherever they are shown.
    update(&mut p, id, sorted(&[7, 6, 5, 4, 3, 2, 1]));
    assert_eq!(selected(&p, id), [4, 2]);
    // Shift+click marks the shown range from the anchor (result 4) to result 1.
    click(&mut p, id, 1, false, true);
    assert_eq!(selected(&p, id), [4, 3, 2, 1]);
    assert_eq!(cursor(&p, id), Some(1));
    // Results streamed after the sort are shown after it and keep their identity too.
    update(&mut p, id, FindUpdate::Batch(batch(8, 2)));
    click(&mut p, id, 9, true, false);
    assert_eq!(selected(&p, id), [4, 3, 2, 1, 9]);
    // Unknown results change nothing.
    assert!(!p.find_select(
        id,
        FindSelect::Click {
            result: FindResultId(99),
            toggle: false,
            range: false,
        }
    ));
    assert_eq!(selected(&p, id), [4, 3, 2, 1, 9]);
}

#[test]
fn keys_move_extend_and_toggle_and_ctrl_a_and_ctrl_d_select_all_and_none() {
    let (mut p, id) = streaming(6);
    // Down to result 2 selects it alone; Shift+Down extends the range to result 4.
    key(&mut p, id, 2, false, false);
    assert_eq!(selected(&p, id), [2]);
    key(&mut p, id, 3, true, false);
    key(&mut p, id, 4, true, false);
    assert_eq!(selected(&p, id), [2, 3, 4]);
    // Shift+Up shrinks it again towards the anchor.
    key(&mut p, id, 3, true, false);
    assert_eq!(selected(&p, id), [2, 3]);
    // Ctrl moves the cursor alone; Space toggles the cursor's result in place.
    key(&mut p, id, 6, false, true);
    assert_eq!((selected(&p, id), cursor(&p, id)), (vec![2, 3], Some(6)));
    assert!(p.find_select(id, FindSelect::Toggle { advance: false }));
    assert_eq!((selected(&p, id), cursor(&p, id)), (vec![2, 3, 6], Some(6)));
    // Insert toggles and moves down, not past the last result.
    assert!(p.find_select(id, FindSelect::Toggle { advance: true }));
    assert_eq!((selected(&p, id), cursor(&p, id)), (vec![2, 3], Some(6)));
    key(&mut p, id, 1, false, true);
    assert!(p.find_select(id, FindSelect::Toggle { advance: true }));
    assert_eq!((selected(&p, id), cursor(&p, id)), (vec![1, 2, 3], Some(2)));
    // Ctrl+A selects every result shown, Ctrl+D none; the cursor stays.
    assert!(p.find_select(id, FindSelect::All));
    assert_eq!(selected(&p, id), [1, 2, 3, 4, 5, 6]);
    assert_eq!(p.finds.sessions[&id].selection_bytes, 21);
    assert!(p.find_select(id, FindSelect::Clear));
    assert!(selected(&p, id).is_empty());
    assert_eq!(p.finds.sessions[&id].selection_bytes, 0);
    assert_eq!(cursor(&p, id), Some(2));
}

#[test]
fn status_bytes_count_selected_files_only_and_a_new_search_clears_the_selection() {
    let (mut p, id) = streaming(2);
    update(
        &mut p,
        id,
        FindUpdate::Batch(
            vec![
                result(3, "folder", None),
                result(4, "big.bin", Some(1 << 40)),
            ]
            .into(),
        ),
    );
    assert!(p.find_select(id, FindSelect::All));
    let s = &p.finds.sessions[&id];
    assert_eq!(s.selection.len(), 4);
    assert_eq!(s.selection_bytes, 1 + 2 + (1 << 40));
    let revision = s.selection_revision;
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    let s = &p.finds.sessions[&id];
    assert!(s.selection.is_empty());
    assert_eq!((s.selection.cursor(), s.selection.anchor()), (None, None));
    assert_eq!(s.selection_bytes, 0);
    assert_ne!(
        s.selection_revision, revision,
        "the window must drop its marks"
    );
}

#[test]
fn the_menu_keeps_a_selection_that_holds_its_row_and_otherwise_selects_the_row() {
    let (mut p, id) = streaming(5);
    click(&mut p, id, 2, false, false);
    click(&mut p, id, 4, false, true);
    assert!(p.find_select(
        id,
        FindSelect::Menu {
            result: FindResultId(3)
        }
    ));
    assert_eq!((selected(&p, id), cursor(&p, id)), (vec![2, 3, 4], Some(3)));
    assert!(p.find_select(
        id,
        FindSelect::Menu {
            result: FindResultId(5)
        }
    ));
    assert_eq!((selected(&p, id), cursor(&p, id)), (vec![5], Some(5)));
}

#[test]
fn copies_act_on_the_selection_holding_the_row_and_otherwise_on_the_row_alone() {
    let (mut p, id) = streaming(4);
    update(&mut p, id, sorted(&[4, 3, 2, 1]));
    click(&mut p, id, 1, false, false);
    click(&mut p, id, 3, true, false);
    let raw = |name: &str| root().join(name).to_string_lossy().into_owned();
    // A selected row: every selected result, in the order shown.
    assert_eq!(
        copied(&mut p, id, "1:1", FindCopy::Paths),
        format!("{}\n{}", raw("file-3.txt"), raw("file-1.txt"))
    );
    assert_eq!(
        copied(&mut p, id, "1:3", FindCopy::Names),
        "file-3.txt\nfile-1.txt"
    );
    // A row outside the selection: that row alone.
    assert_eq!(copied(&mut p, id, "1:2", FindCopy::Names), "file-2.txt");
    assert_eq!(
        copied(&mut p, id, "1:2", FindCopy::Details),
        format!("file-2.txt\ttxt\t{}\t2 B\t", root().to_string_lossy())
    );
    // After Ctrl+D the cursor's row is acted on alone.
    assert!(p.find_select(id, FindSelect::Clear));
    assert_eq!(copied(&mut p, id, "1:3", FindCopy::Names), "file-3.txt");
    // A row of another search, or none, copies nothing.
    assert!(!p.find_copy(id, "2:1", FindCopy::Names, &|_| panic!("stale row")));
    assert!(!p.find_copy(id, "1:9", FindCopy::Names, &|_| panic!("no such row")));
}

#[test]
fn feed_to_panel_marks_an_explicit_selection_and_starts_at_the_cursor() {
    let finished = |p: &mut Presenter, id| {
        update(
            p,
            id,
            FindUpdate::Finished(FindSummary {
                completion: FindCompletion::Completed,
                progress: FindProgress::default(),
                reached_budgets: vec![],
                diagnostics: vec![],
            }),
        )
    };
    let (mut p, id) = streaming(5);
    finished(&mut p, id);
    update(&mut p, id, sorted(&[5, 4, 3, 2, 1]));
    click(&mut p, id, 4, false, false);
    click(&mut p, id, 2, false, true);
    p.feed_find_results(id).unwrap();
    let feed = p.feed(PaneId::Left).unwrap();
    assert_eq!(feed.selected, [false, true, true, true, false]);
    assert_eq!((feed.selected_count, feed.selected_bytes), (3, 4 + 3 + 2));
    assert_eq!(
        feed.cursor, 3,
        "the list starts on the Find's cursor (result 2)"
    );
    // F8 there asks to delete exactly the three results the Find had selected.
    let commands = RefCell::new(Vec::new());
    p.feed_action(
        PaneId::Left,
        feed::FeedAction::Delete(DeleteMode::Trash),
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    let prepared = commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::PrepareCapturedDelete(request) => Some(request),
            _ => None,
        })
        .expect("the ordinary captured delete");
    assert_eq!(prepared.sources.root_count(), 3);
    assert!(
        p.operations.unconfirmed.contains(&prepared.id),
        "it waits for the usual confirmation"
    );

    // A selection that only follows the cursor marks nothing; the cursor still carries.
    let (mut p, id) = streaming(5);
    finished(&mut p, id);
    key(&mut p, id, 3, false, false);
    p.feed_find_results(id).unwrap();
    let feed = p.feed(PaneId::Left).unwrap();
    assert_eq!(feed.selected_count, 0);
    assert_eq!(feed.cursor, 2);
}

/// Opt-in release measurement: `cargo test --release -p midnight-ui --lib
/// find_selection_measures_100k -- --ignored --nocapture`. The retained-result limit is
/// 10,000; this is ten times that.
#[test]
#[ignore = "measurement; run in release with --ignored --nocapture"]
fn find_selection_measures_100k_results() {
    const COUNT: u64 = 100_000;
    let mut p = Presenter::new(root(), std::env::temp_dir().join("find-selection-target"));
    let id = p.create_find(PaneId::Left).unwrap();
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    {
        let s = p.finds.sessions.get_mut(&id).unwrap();
        s.results = batch(1, COUNT).to_vec();
        let ids: Vec<u64> = (1..=COUNT).rev().collect();
        let FindUpdate::Sorted(order) = sorted(&ids) else {
            unreachable!()
        };
        s.order = Some(order);
    }
    let time = |label: &str, p: &mut Presenter, action: &mut dyn FnMut(&mut Presenter)| {
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            action(p);
            best = best.min(start.elapsed());
        }
        println!("{label}: {best:?} (best of 5)");
    };
    time("click", &mut p, &mut |p| click(p, id, 50_000, false, false));
    time("select all (Ctrl+A)", &mut p, &mut |p| {
        p.find_select(id, FindSelect::All);
    });
    time("deselect all (Ctrl+D)", &mut p, &mut |p| {
        p.find_select(id, FindSelect::All);
        p.find_select(id, FindSelect::Clear);
    });
    time("Shift+click range of 100,000", &mut p, &mut |p| {
        click(p, id, COUNT, false, false);
        click(p, id, 1, false, true);
    });
    time("Ctrl+click toggle in a full selection", &mut p, &mut |p| {
        click(p, id, 7, true, false)
    });
    time("Shift+Down from a full range", &mut p, &mut |p| {
        key(p, id, 2, true, false)
    });
    p.find_select(id, FindSelect::All);
    time("copy 100,000 full paths", &mut p, &mut |p| {
        copied(p, id, "1:1", FindCopy::Paths);
    });
    time("copy 100,000 row details", &mut p, &mut |p| {
        copied(p, id, "1:1", FindCopy::Details);
    });
    let s = &p.finds.sessions[&id];
    let start = std::time::Instant::now();
    let mut marked = 0;
    for slot in 0..COUNT as usize {
        marked += usize::from(s.selection.is_selected(slot));
    }
    println!(
        "100,000 mark lookups: {:?} ({marked} marked)",
        start.elapsed()
    );
}
