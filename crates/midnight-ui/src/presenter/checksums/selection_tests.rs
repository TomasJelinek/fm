use super::*;
use crate::presenter::checksums_tests::{chunk, finished_run, listed, result_row};

/// A session over the temporary folder whose calculation is running.
fn running() -> (Presenter, ChecksumJobId, PathBuf) {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let files = SelectedFiles {
        selection: Arc::new(SelectionSnapshot::default()),
        view: Arc::new(DirectoryIndex::default()),
        fallback: Some(root.clone()),
    };
    let id = p
        .open_captured_checksums(PaneId::Left, root.clone(), files, &|_| true)
        .unwrap();
    assert!(p.start_checksums(id, ChecksumAlgorithms::default(), false, &|_| true));
    (p, id, root)
}

fn apply(p: &mut Presenter, id: ChecksumJobId, update: ChecksumUpdate) {
    let generation = p.checksums.sessions[&id].generation;
    p.apply(Event::Checksum(Box::new(ChecksumEvent {
        job: id,
        generation,
        update,
    })));
}

fn session(p: &mut Presenter, id: ChecksumJobId) -> &mut ChecksumSession {
    p.checksums.sessions.get_mut(&id).unwrap()
}

/// The names of the selected rows, in the order shown.
fn marked(p: &Presenter, id: ChecksumJobId) -> Vec<String> {
    let s = &p.checksums.sessions[&id];
    let shown = s.shown();
    let finished = s.rows().len();
    (0..crate::list_selection::ListOrder::len(&shown))
        .filter(|index| {
            crate::list_selection::ListOrder::slot_at(&shown, *index)
                .is_some_and(|slot| s.selection.is_selected(slot))
        })
        .map(|index| match s.rows().get(index) {
            Some(row) => row.name.to_string_lossy().into_owned(),
            None => s
                .pending_at(index - finished)
                .unwrap()
                .1
                .name
                .to_string_lossy()
                .into_owned(),
        })
        .collect()
}

fn calculated(id: u64, name: &str, digest: u8) -> ChecksumRow {
    result_row(
        id,
        Path::new(name),
        ChecksumRowState::Calculated,
        &[(ChecksumAlgorithm::Sha256, &[digest; 32])],
    )
}

fn copies(sent: &std::cell::RefCell<Vec<Command>>) -> Vec<String> {
    sent.borrow()
        .iter()
        .filter_map(|command| match command {
            Command::CopyText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Marks belong to files: a listed file's mark follows it to its row whenever the row
/// arrives, rows arriving out of listing order move no mark, and the finished run
/// keeps every mark.
#[test]
fn the_selection_follows_its_files_through_the_run() {
    let (mut p, id, root) = running();
    apply(
        &mut p,
        id,
        ChecksumUpdate::Planned(listed(&root, &["a", "b", "c", "d"])),
    );
    // While all four wait: select b and d (rows 1 and 3).
    session(&mut p, id).click_row(1, false, false).unwrap();
    session(&mut p, id).click_row(3, true, false).unwrap();
    assert_eq!(marked(&p, id), ["b", "d"]);
    // d's row arrives first, then a row the listing left out (an unreadable folder):
    // the waiting rows move, the marks stay with b and d.
    apply(
        &mut p,
        id,
        ChecksumUpdate::Batch(chunk(
            &root,
            vec![
                calculated(1, "d", 4),
                result_row(2, Path::new("locked"), ChecksumRowState::ReadError, &[]),
            ],
        )),
    );
    assert_eq!(
        marked(&p, id),
        ["d", "b"],
        "rows first, then the waiting files"
    );
    apply(
        &mut p,
        id,
        ChecksumUpdate::Batch(chunk(
            &root,
            vec![
                calculated(3, "a", 1),
                calculated(4, "b", 2),
                calculated(5, "c", 3),
            ],
        )),
    );
    assert_eq!(marked(&p, id), ["d", "b"]);
    // The cursor stays on its file too (d, now the first row).
    assert_eq!(p.checksums.sessions[&id].cursor_row(), Some(0));
    // The finished run holds the same rows: the marks stay.
    let rows: Vec<_> = p.checksums.sessions[&id].rows().chunks().to_vec();
    finished_run(
        &mut p,
        id,
        ChecksumAlgorithms::default(),
        rows,
        ChecksumSummary {
            completion: ChecksumCompletion::CompletedWithErrors,
            progress: ChecksumProgress::default(),
            diagnostics: vec![],
        },
    );
    assert_eq!(marked(&p, id), ["d", "b"]);
    assert_eq!(p.checksums.sessions[&id].selection_counts(), (2, 2));
}

/// Copy writes exactly the selected rows' checksum-file lines, in the order shown;
/// Ctrl+C their checksums one per line, Ctrl+Shift+C their full rows a blank line apart.
#[test]
fn copies_take_the_selected_rows_in_the_order_shown() {
    let (mut p, id, root) = running();
    let rows = (1..=5)
        .map(|index| calculated(index, &format!("f{index}.bin"), index as u8))
        .collect();
    finished_run(
        &mut p,
        id,
        ChecksumAlgorithms::default(),
        vec![chunk(&root, rows)],
        ChecksumSummary {
            completion: ChecksumCompletion::Completed,
            progress: ChecksumProgress {
                calculated: 5,
                ..Default::default()
            },
            diagnostics: vec![],
        },
    );
    // Selected in the order 4, 2: copied in the order shown, 2 then 4.
    session(&mut p, id).click_row(3, false, false).unwrap();
    session(&mut p, id).click_row(1, true, false).unwrap();
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let hash = |byte: u8| format!("{byte:02x}").repeat(32);
    p.copy_checksum_file(id, ChecksumAlgorithm::Sha256, &send);
    p.copy_checksums(id, ChecksumAlgorithm::Sha256, false, &send);
    p.copy_checksums(id, ChecksumAlgorithm::Sha256, true, &send);
    let path = |name: &str| root.join(name).display().to_string();
    assert_eq!(
        copies(&sent),
        [
            format!("{} *f2.bin\n{} *f4.bin\n", hash(2), hash(4)),
            format!("{}\n{}", hash(2), hash(4)),
            format!(
                "{}\nSHA-256: {}\n\n{}\nSHA-256: {}",
                path("f2.bin"),
                hash(2),
                path("f4.bin"),
                hash(4)
            ),
        ]
    );
    assert!(
        p.checksums.sessions[&id].copy_note.is_none(),
        "nothing left out"
    );
}

/// No selection, or a selection without a calculated row, gives Copy nothing to write;
/// Ctrl+D empties a selection again.
#[test]
fn copy_needs_a_selected_calculated_row_and_ctrl_d_empties_the_selection() {
    let (mut p, id, root) = running();
    let rows = vec![
        calculated(1, "ok.bin", 1),
        result_row(2, Path::new("locked.bin"), ChecksumRowState::ReadError, &[]),
    ];
    finished_run(
        &mut p,
        id,
        ChecksumAlgorithms::default(),
        vec![chunk(&root, rows)],
        ChecksumSummary {
            completion: ChecksumCompletion::CompletedWithErrors,
            progress: ChecksumProgress::default(),
            diagnostics: vec![],
        },
    );
    let s = session(&mut p, id);
    assert_eq!(s.selection_counts(), (0, 0), "nothing selected");
    s.click_row(1, false, false).unwrap();
    assert_eq!(s.selection_counts(), (0, 0), "only a failed row");
    s.click_row(0, true, false).unwrap();
    assert_eq!(s.selection_counts(), (1, 1));
    assert_eq!(s.deselect_all_rows(), crate::list_selection::one(0..2));
    assert_eq!(s.selection_counts(), (0, 0));
    assert!(s.selection.is_empty());
    let sent = std::cell::RefCell::new(Vec::new());
    p.copy_checksum_file(id, ChecksumAlgorithm::Sha256, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(copies(&sent).is_empty(), "nothing selected, nothing copied");
}

/// Selected rows that add nothing are named in the copy's note, by reason.
#[test]
fn the_note_names_the_selected_rows_a_copy_left_out() {
    let (mut p, id, root) = running();
    apply(
        &mut p,
        id,
        ChecksumUpdate::Planned(listed(&root, &["a.bin", "wait.bin"])),
    );
    apply(
        &mut p,
        id,
        ChecksumUpdate::Batch(chunk(
            &root,
            vec![
                calculated(1, "a.bin", 7),
                result_row(2, Path::new("locked"), ChecksumRowState::ReadError, &[]),
                result_row(3, Path::new("link"), ChecksumRowState::Skipped, &[]),
            ],
        )),
    );
    session(&mut p, id).select_all_rows();
    let sent = std::cell::RefCell::new(Vec::new());
    p.copy_checksum_file(id, ChecksumAlgorithm::Sha256, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert_eq!(copies(&sent), [format!("{} *a.bin\n", "07".repeat(32))]);
    let s = &p.checksums.sessions[&id];
    assert_eq!(
        s.save_status(&NumberLocale::default()),
        "Copied 1 checksum. Left out: 1 failed file; 1 skipped file; 1 file not calculated yet."
    );
}

/// A context menu acts on the whole selection when its row is selected, else on that
/// row alone.
#[test]
fn a_row_menu_copies_the_selection_it_belongs_to_or_its_row() {
    let (mut p, id, root) = running();
    let rows = (1..=3)
        .map(|index| calculated(index, &format!("m{index}"), index as u8))
        .collect();
    finished_run(
        &mut p,
        id,
        ChecksumAlgorithms::default(),
        vec![chunk(&root, rows)],
        ChecksumSummary {
            completion: ChecksumCompletion::Completed,
            progress: ChecksumProgress::default(),
            diagnostics: vec![],
        },
    );
    session(&mut p, id).click_row(0, false, false).unwrap();
    session(&mut p, id).click_row(1, true, false).unwrap();
    let generation = p.checksums.sessions[&id].generation.0;
    let sent = std::cell::RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let hash = |byte: u8| format!("{byte:02x}").repeat(32);
    p.copy_checksum(
        id,
        &format!("{generation}:2"),
        ChecksumAlgorithm::Sha256,
        false,
        true,
        &send,
    );
    p.copy_checksum(
        id,
        &format!("{generation}:3"),
        ChecksumAlgorithm::Sha256,
        false,
        true,
        &send,
    );
    assert_eq!(
        copies(&sent),
        [format!("{}\n{}", hash(1), hash(2)), hash(3)]
    );
    // A token of another run copies nothing.
    p.copy_checksum(id, "999:1", ChecksumAlgorithm::Sha256, false, true, &send);
    assert_eq!(copies(&sent).len(), 2);
}

/// A new calculation of the same files selects the same files again as it lists them;
/// changing Include subfolders, or a verification, starts with no selection.
#[test]
fn a_new_run_of_the_same_files_keeps_their_selection() {
    let (mut p, id, root) = running();
    let finish = |p: &mut Presenter, rows: Vec<ChecksumRow>| {
        finished_run(
            p,
            id,
            ChecksumAlgorithms::default(),
            vec![chunk(&root, rows)],
            ChecksumSummary {
                completion: ChecksumCompletion::Completed,
                progress: ChecksumProgress::default(),
                diagnostics: vec![],
            },
        );
    };
    finish(&mut p, vec![calculated(1, "x", 1), calculated(2, "y", 2)]);
    session(&mut p, id).click_row(1, false, false).unwrap();
    assert!(p.start_checksums(id, ChecksumAlgorithms::all(), false, &|_| true));
    assert!(
        p.checksums.sessions[&id].selection.is_empty(),
        "nothing listed yet"
    );
    apply(
        &mut p,
        id,
        ChecksumUpdate::Planned(listed(&root, &["x", "y"])),
    );
    assert_eq!(
        marked(&p, id),
        ["y"],
        "selected again as soon as it is listed"
    );
    apply(
        &mut p,
        id,
        ChecksumUpdate::Batch(chunk(
            &root,
            vec![calculated(1, "x", 1), calculated(2, "y", 2)],
        )),
    );
    assert_eq!(marked(&p, id), ["y"]);
    assert_eq!(p.checksums.sessions[&id].cursor_row(), Some(1));
    // Include subfolders changes which files are read: no selection.
    let rows = p.checksums.sessions[&id].rows().chunks().to_vec();
    finished_run(
        &mut p,
        id,
        ChecksumAlgorithms::default(),
        rows,
        ChecksumSummary {
            completion: ChecksumCompletion::Completed,
            progress: ChecksumProgress::default(),
            diagnostics: vec![],
        },
    );
    assert!(p.start_checksums(id, ChecksumAlgorithms::default(), true, &|_| true));
    apply(
        &mut p,
        id,
        ChecksumUpdate::Planned(listed(&root, &["x", "y"])),
    );
    assert!(marked(&p, id).is_empty());
    // A verification reads another file's entries: no selection either.
    finish(&mut p, vec![calculated(1, "x", 1)]);
    session(&mut p, id).click_row(0, false, false).unwrap();
    assert!(p.start_checksum_verification(id, "sums.sha256", &|_| true));
    assert!(p.checksums.sessions[&id].selection.is_empty());
}

/// Select all marks every shown row, listed files included, and reports one change for
/// the model; a range is marked from the anchor.
#[test]
fn select_all_and_ranges_report_the_rows_to_notify() {
    let (mut p, id, root) = running();
    apply(
        &mut p,
        id,
        ChecksumUpdate::Planned(listed(&root, &["a", "b", "c", "d", "e"])),
    );
    apply(
        &mut p,
        id,
        ChecksumUpdate::Batch(chunk(
            &root,
            vec![calculated(1, "a", 1), calculated(2, "b", 2)],
        )),
    );
    let s = session(&mut p, id);
    assert_eq!(s.select_all_rows(), crate::list_selection::one(0..5));
    assert_eq!(s.selection.len(), 5);
    assert_eq!(s.click_row(1, false, false).unwrap(), vec![0..5, 1..2]);
    assert_eq!(s.click_row(3, false, true).unwrap(), vec![1..2, 1..4]);
    assert_eq!(marked(&p, id), ["b", "c", "d"]);
}
