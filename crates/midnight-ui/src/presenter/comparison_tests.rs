use super::*;
use std::{cell::RefCell, ffi::OsStr};

fn loaded() -> Presenter {
    let mut p = Presenter::new("left".into(), "right".into());
    for (slot, pane) in p.panes.iter_mut().enumerate() {
        pane.generation = 1;
        pane.successful_generation = Some(NavigationId(1));
        pane.selection_context = NavigationId(1);
        pane.selection_committed_context = NavigationId(1);
        pane.entries = vec![Entry::new(
            EntryId(1),
            pane.path.join("one"),
            EntryKind::File,
        )]
        .into();
        pane.index = Arc::new(DirectoryIndex::from_entries(&pane.entries));
        pane.cursor = Some(EntryId(1));
        assert_eq!(slot, if pane.path == Path::new("left") { 0 } else { 1 });
    }
    p
}

fn start_with(p: &mut Presenter, options: CompareOptions) -> Arc<CompareRequest> {
    let commands = RefCell::new(Vec::new());
    p.open_comparison();
    p.submit_comparison(options, IgnoreChoices::default(), &|command| {
        commands.borrow_mut().push(command);
        true
    });
    commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::CompareDirectories(request) => Some(Arc::new(request)),
            _ => None,
        })
        .unwrap()
}

fn start(p: &mut Presenter) -> Arc<CompareRequest> {
    start_with(p, CompareOptions::default())
}

fn long() -> CompareOptions {
    CompareOptions {
        by_content: true,
        recursive: true,
        ..CompareOptions::default()
    }
}

fn result(request: Arc<CompareRequest>) -> Event {
    result_with(request, 1)
}

fn outcome(request: Arc<CompareRequest>, bits: u64) -> CompareResult {
    let selections = request
        .sides
        .each_ref()
        .map(|side| Arc::new(SelectionSnapshot::from_bits(side.view.clone(), vec![bits]).unwrap()));
    CompareResult {
        request,
        selections,
        tree_replacements: [None, None],
        marks: None,
        unknown: 0,
        ambiguous: 0,
        differing_folders: 0,
        compared_folders: 0,
        content_differences: 0,
        content_files: 0,
        content_bytes: 0,
        walked_folders: 0,
        walked_files: 0,
        dst_shifts: 0,
        skipped: 0,
        errors: 0,
        limited: false,
        diagnostics: Vec::new(),
    }
}

/// The result of `request`, with `bits` as both panels' selection of their one row.
fn result_with(request: Arc<CompareRequest>, bits: u64) -> Event {
    Event::DirectoriesCompared {
        id: request.id,
        result: Ok(Arc::new(outcome(request, bits))),
    }
}

#[test]
fn a_quick_comparison_closes_the_dialog_at_once_and_a_long_one_shows_its_progress() {
    let mut p = loaded();
    let request = start(&mut p);
    assert!(
        !p.comparison.open,
        "metadata only: the selection is the result"
    );
    assert!(p.comparison.busy, "closing is not cancelling");
    assert!(!p.comparison.running());
    p.apply_with_sender(result(request), &|_| true);
    assert!(!p.comparison.open && p.comparison.error.is_empty());

    let mut p = loaded();
    let request = start_with(&mut p, long());
    assert!(
        p.comparison.open,
        "the dialog stays with its progress and Cancel"
    );
    assert!(p.comparison.running());
    p.apply_with_sender(result(request), &|_| true);
    assert!(!p.comparison.open, "it closes when the comparison ends");
    assert!(!p.comparison.running());
}

#[test]
fn cancel_closes_the_dialog_stops_the_backend_and_leaves_the_panels_unchanged() {
    let mut p = loaded();
    let request = start_with(&mut p, long());
    let sent = RefCell::new(Vec::new());
    p.cancel_comparison(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(!p.comparison.open && !p.comparison.running() && !p.comparison.busy);
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelComparison(id) if *id == request.id)),
        "the engine is told at once"
    );
    p.apply(result(request));
    assert!(p.panes.iter().all(|pane| pane.selected.is_empty()));
    assert!(p.comparison_note(PaneId::Left).is_none());
}

#[test]
fn comparison_publishes_both_and_next_toggle_carries_authoritative_seed() {
    let mut p = loaded();
    let request = start(&mut p);
    p.apply(result(request));
    assert_eq!(p.panes.each_ref().map(|pane| pane.selected.len()), [1, 1]);
    let commands = RefCell::new(Vec::new());
    p.send_selection(
        PaneId::Left,
        SelectionAction::Toggle(Path::new("left/one").into()),
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    let Command::Selection(request) = commands.into_inner().pop().unwrap() else {
        panic!("selection");
    };
    assert_eq!(request.seed_revision, SelectionRevision(1));
    assert_eq!(request.revision, SelectionRevision(2));
    assert_eq!(request.seed.len(), 1);
    let authority = request.authoritative.as_ref().unwrap();
    assert_eq!(authority.id, 1);
    assert_eq!(authority.origin.navigation, NavigationId(1));
    assert_eq!(authority.snapshot.len(), 1);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        revision: request.revision,
        selection: Arc::new(SelectionSnapshot::from_bits(request.view.unwrap(), vec![0]).unwrap()),
        deferred: Default::default(),
    });
    assert!(
        p.comparison.seeds[0].is_none(),
        "acknowledged source index must not remain pinned by the seed"
    );
}

#[test]
fn comparison_stamp_is_carried_across_navigation_revision_reset() {
    let mut p = loaded();
    let request = start(&mut p);
    p.apply(result(request));
    p.selection_navigation_accepted(PaneId::Left, NavigationId(2), Path::new("left"));
    p.panes[0].generation = 2;
    let commands = RefCell::new(Vec::new());
    p.send_selection(PaneId::Left, SelectionAction::Rebase, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let Command::Selection(request) = commands.into_inner().pop().unwrap() else {
        panic!("selection");
    };
    assert_eq!(request.navigation, NavigationId(2));
    assert_eq!(request.seed_revision, SelectionRevision(0));
    let authority = request.authoritative.unwrap();
    assert_eq!(authority.origin.navigation, NavigationId(1));
    assert_eq!(authority.snapshot.len(), 1);
}

#[test]
fn selection_edit_navigation_or_cancel_rejects_both_comparison_results() {
    for change in 0..3 {
        let mut p = loaded();
        let request = start(&mut p);
        match change {
            0 => p.panes[1].selection_revision.0 += 1,
            1 => p.panes[0].successful_generation = Some(NavigationId(2)),
            _ => p.cancel_comparison(&|_| true),
        }
        p.apply(result(request));
        assert!(p.panes.iter().all(|pane| pane.selected.is_empty()));
    }
}

#[test]
fn watch_refresh_waits_for_a_running_comparison_then_rereads_the_folder() {
    let mut p = loaded();
    let request = start(&mut p);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(!p.refresh_if_changed(PaneId::Left, NavigationId(1), &send));
    p.flush_selection(&send);
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Refresh { .. })),
        "a watch hint must not replace rows the comparison captured"
    );
    p.apply_with_sender(result(request), &send);
    assert!(p.comparison.error.is_empty(), "the comparison completes");
    assert!(
        sent.borrow().iter().any(|command| matches!(
            command,
            Command::Refresh {
                pane: PaneId::Left,
                ..
            }
        )),
        "the retained hint re-reads the folder after the comparison"
    );
}

/// Compare contents can run for minutes while both panels keep the rows it
/// compares: a change of their folders is refused rather than shown that late.
#[test]
fn a_long_comparison_refuses_panel_changes_that_could_show_only_after_it() {
    let mut p = loaded();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    // A quick comparison ends in a moment: changes wait for it as before.
    let request = start(&mut p);
    assert!(!p.comparison_refuses("Rename"));
    p.apply_with_sender(result(request), &send);
    p.open_comparison();
    p.submit_comparison(
        CompareOptions {
            by_content: true,
            ..CompareOptions::default()
        },
        IgnoreChoices::default(),
        &send,
    );
    assert!(p.comparison.in_progress());
    sent.borrow_mut().clear();
    let hidden = p.preferences.values.show_hidden_files;
    p.start_operation(PaneId::Left, MutationKind::Rename, &send);
    let (title, body) = p.desktop.notice.clone().expect("F2 says why");
    assert_eq!(title, "Directory comparison running");
    assert!(body.starts_with("Rename is not available"), "{body}");
    assert!(body.contains("Cancel"), "{body}");
    p.toggle_hidden_files(&send);
    assert_eq!(p.preferences.values.show_hidden_files, hidden, "Ctrl+H");
    p.request_move_dialog(PaneId::Right, &send);
    assert!(p.copies.draft.is_none(), "F6");
    assert!(sent.borrow().is_empty(), "no change starts");
    // Cancelled, the panels take changes again.
    p.cancel_comparison(&send);
    assert!(!p.comparison_refuses("Rename"));
}

#[test]
fn a_comparison_the_panels_cancel_is_reported_although_its_dialog_closed() {
    let mut p = loaded();
    let request = start(&mut p);
    assert!(p.desktop.notice.is_none());
    // The panels stay live while the comparison runs; a selection change cancels it.
    p.panes[1].selection_revision.0 += 1;
    p.flush_comparison(&|_| true);
    let (title, body) = p
        .desktop
        .notice
        .clone()
        .expect("the cancellation is reported");
    assert_eq!(title, "Directory comparison cancelled");
    assert!(body.contains("a panel or selection changed"), "{body}");
    // Its late result changes nothing and reports nothing more.
    let generation = p.desktop.notice_generation;
    p.apply(result(request));
    assert_eq!(p.desktop.notice_generation, generation);
    assert!(p.panes.iter().all(|pane| pane.selected.is_empty()));
    assert!(p.comparison_note(PaneId::Left).is_none());
}

#[test]
fn a_failed_comparison_is_reported_and_an_open_dialog_shows_it_itself() {
    let failure = || {
        Err(FsError::from_io(
            "compare directories",
            Path::new("left"),
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        ))
    };
    let mut p = loaded();
    let request = start(&mut p);
    p.apply(Event::DirectoriesCompared {
        id: request.id,
        result: failure(),
    });
    let (title, _) = p.desktop.notice.clone().expect("the failure is reported");
    assert_eq!(title, "Directory comparison failed");
    // A long comparison's dialog is still open: it shows the outcome instead.
    let mut p = loaded();
    let request = start_with(&mut p, long());
    p.apply(Event::DirectoriesCompared {
        id: request.id,
        result: failure(),
    });
    assert!(p.desktop.notice.is_none());
    assert!(!p.comparison.error.is_empty());
    assert!(p.comparison.open && !p.comparison.running());
}

#[test]
fn the_result_is_a_line_below_the_active_panel_while_the_panels_show_its_selection() {
    let mut p = loaded();
    p.active = PaneId::Right;
    let request = start(&mut p);
    p.apply(result(request));
    assert!(p.desktop.notice.is_none(), "no window reports the result");
    assert!(
        p.comparison_note(PaneId::Left).is_none(),
        "only below the active panel"
    );
    let (line, details) = p.comparison_note(PaneId::Right).unwrap();
    assert_eq!(line, "Compared: 1 selected here, 1 in the left panel");
    assert!(details.contains("Selected: 1 in the left panel, 1 in the right panel."));
    // Once a panel's selection changes, the line no longer describes it.
    p.panes[0].selection_revision.0 += 1;
    assert!(p.comparison_note(PaneId::Right).is_none());

    let mut p = loaded();
    let request = start(&mut p);
    p.apply(result_with(request, 0));
    assert_eq!(
        p.comparison_note(PaneId::Left).unwrap().0,
        "Compared: The files in the two folders appear to be identical."
    );
    let mut p = loaded();
    let request = start_with(
        &mut p,
        CompareOptions {
            by_content: true,
            ..CompareOptions::default()
        },
    );
    p.apply(result_with(request, 0));
    assert_eq!(
        p.comparison_note(PaneId::Left).unwrap().0,
        "Compared: The files in the two folders are identical."
    );
}

#[test]
fn progress_names_the_pair_counts_and_fraction_and_ignores_other_comparisons() {
    let mut p = loaded();
    let request = start_with(&mut p, long());
    assert!(request.options.by_content && request.options.recursive);
    let locale = p.number_locale.clone();
    let (counts, _, fraction) = progress_texts(p.comparison.progress.as_ref(), &locale);
    assert_eq!((counts.as_str(), fraction), ("Starting…", -1.0));
    let pair = Arc::new([PathBuf::from("left/src"), PathBuf::from("right/src")]);
    p.apply(Event::ComparisonProgress {
        id: request.id,
        progress: CompareProgress {
            current: Some(pair.clone()),
            top_done: 1,
            top_total: 4,
            folders: 30,
            files: 999,
            differences: 1,
            ..Default::default()
        },
    });
    let (counts, detail, fraction) = progress_texts(p.comparison.progress.as_ref(), &locale);
    assert_eq!(
        counts,
        "Folder 2 of 4 · 30 folders and 999 files compared · 1 difference found"
    );
    assert!(detail.is_empty());
    assert!((fraction - 0.25).abs() < 1e-6);
    assert_eq!(p.comparison.progress.as_ref().unwrap().current, Some(pair));
    p.apply(Event::ComparisonProgress {
        id: request.id,
        progress: CompareProgress {
            phase: ComparePhase::Contents,
            content_files: 1,
            content_files_total: 4,
            content_bytes: 2048,
            content_bytes_total: 8192,
            ..Default::default()
        },
    });
    let (_, detail, fraction) = progress_texts(p.comparison.progress.as_ref(), &locale);
    assert!(
        detail.starts_with("Contents: 1 of 4 file pairs read · 2"),
        "{detail}"
    );
    assert!((fraction - 0.25).abs() < 1e-6);
    // A sample of another comparison changes nothing.
    let before = p.comparison.progress.clone();
    p.apply(Event::ComparisonProgress {
        id: CompareId(request.id.0 + 7),
        progress: CompareProgress::default(),
    });
    assert_eq!(p.comparison.progress, before);
}

#[test]
fn problems_keep_the_dialog_open_with_the_summary_and_marks_follow_the_listings() {
    let mut p = loaded();
    let request = start_with(&mut p, long());
    let mut counted = outcome(request.clone(), 1);
    counted.differing_folders = 2;
    counted.compared_folders = 3;
    counted.content_differences = 1;
    counted.content_files = 3;
    counted.content_bytes = 300;
    counted.dst_shifts = 2;
    counted.errors = 1;
    counted.diagnostics = vec![FsError::from_io(
        "compare directories",
        Path::new("left/locked"),
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    )];
    let mut marks = CompareMarks::new([PathBuf::from("left"), PathBuf::from("right")], false);
    marks.set(
        &marks.child_key(OsStr::new(""), OsStr::new("one")),
        0,
        CompareMark::Differs,
    );
    counted.marks = Some(Arc::new(marks));
    p.apply(Event::DirectoriesCompared {
        id: request.id,
        result: Ok(Arc::new(counted)),
    });
    assert!(
        p.comparison.open,
        "an unreadable folder keeps the dialog open"
    );
    let summary = &p.comparison.summary;
    for part in [
        "with differences: 2.",
        "different contents: 1.",
        "2 file pairs differ by exactly one or two hours",
        "locked",
    ] {
        assert!(summary.contains(part), "{part:?} in {summary}");
    }
    let (line, _) = p.comparison_note(PaneId::Left).unwrap();
    for part in [
        "Compared: 1 selected here, 1 in the right panel",
        "2 of 3 folders differ",
        "1 with different contents",
        "2 one- or two-hour time differences",
        "1 not readable",
    ] {
        assert!(line.contains(part), "{part:?} in {line:?}");
    }
    let one = p.panes[0].entries[0].clone();
    assert_eq!(p.comparison_mark(PaneId::Left, &one), 2);
    assert_eq!(
        p.comparison_mark(PaneId::Right, &p.panes[1].entries[0].clone()),
        0
    );
    // Another listing of either panel clears the marks; so does a new comparison.
    p.panes[1].generation = 5;
    p.flush_comparison(&|_| true);
    assert_eq!(p.comparison_mark(PaneId::Left, &one), 0);
    assert!(p.comparison.marks.is_none());
}
