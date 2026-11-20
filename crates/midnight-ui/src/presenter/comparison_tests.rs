use super::*;
use std::cell::RefCell;

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

fn start(p: &mut Presenter) -> Arc<CompareRequest> {
    let commands = RefCell::new(Vec::new());
    p.open_comparison();
    p.submit_comparison(CompareOptions::default(), &|command| {
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

fn result(request: Arc<CompareRequest>) -> Event {
    result_with(request, 1)
}

/// The result of `request`, with `bits` as both panels' selection of their one row.
fn result_with(request: Arc<CompareRequest>, bits: u64) -> Event {
    let selections = request
        .sides
        .each_ref()
        .map(|side| Arc::new(SelectionSnapshot::from_bits(side.view.clone(), vec![bits]).unwrap()));
    Event::DirectoriesCompared {
        id: request.id,
        result: Ok(Arc::new(CompareResult {
            request,
            selections,
            tree_replacements: [None, None],
            unknown: 0,
            ambiguous: 0,
            differing_folders: 0,
            content_differences: 0,
            content_files: 0,
            content_bytes: 0,
            skipped: 0,
            errors: 0,
            limited: false,
            diagnostics: Vec::new(),
        })),
    }
}

#[test]
fn the_dialog_closes_as_soon_as_compare_is_pressed_and_the_comparison_keeps_running() {
    let mut p = loaded();
    let request = start(&mut p);
    assert!(!p.comparison.open, "no dialog is left open over the panels");
    assert!(p.comparison.busy, "closing is not cancelling");
    p.apply_with_sender(result(request), &|_| true);
    assert!(!p.comparison.open && p.comparison.error.is_empty());
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
    assert!(p.comparison.summary.contains("Found 1 differences"));
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
        &send,
    );
    assert!(p.comparison.in_progress());
    sent.borrow_mut().clear();
    let hidden = p.preferences.values.show_hidden_files;
    p.start_operation(PaneId::Left, MutationKind::Rename, &send);
    let (title, body) = p.desktop.notice.clone().expect("F2 says why");
    assert_eq!(title, "Directory comparison running");
    assert!(body.starts_with("Rename is not available"), "{body}");
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
    assert!(p.comparison_status().is_none());
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
    // Reopened while it runs, the dialog shows the outcome instead of a notice.
    let mut p = loaded();
    let request = start(&mut p);
    p.open_comparison();
    p.apply(Event::DirectoriesCompared {
        id: request.id,
        result: failure(),
    });
    assert!(p.desktop.notice.is_none());
    assert!(!p.comparison.error.is_empty());
}

#[test]
fn the_result_is_one_status_line_while_the_panels_show_its_selection() {
    let mut p = loaded();
    let request = start(&mut p);
    p.apply(result(request));
    assert!(p.desktop.notice.is_none(), "no window reports the result");
    assert_eq!(
        p.comparison_status(),
        Some("Compared: 1 selected in the left panel, 1 in the right")
    );
    // Once a panel's selection changes, the line no longer describes it.
    p.panes[0].selection_revision.0 += 1;
    assert!(p.comparison_status().is_none());
    let mut p = loaded();
    let request = start(&mut p);
    p.apply(result_with(request, 0));
    assert_eq!(
        p.comparison_status(),
        Some("Compared: no differences for the chosen criteria")
    );
}

#[test]
fn a_content_and_subfolder_comparison_shows_progress_then_its_counts() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    p.open_comparison();
    p.submit_comparison(
        CompareOptions {
            by_content: true,
            recursive: true,
            ..CompareOptions::default()
        },
        &|command| {
            commands.borrow_mut().push(command);
            true
        },
    );
    let request = commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::CompareDirectories(request) => Some(Arc::new(request)),
            _ => None,
        })
        .unwrap();
    assert!(request.options.by_content && request.options.recursive);
    assert!(!p.comparison.open, "the dialog still closes at once");
    assert_eq!(p.comparison_status(), Some("Comparing directories…"));
    p.apply(Event::ComparisonProgress {
        id: request.id,
        progress: CompareProgress {
            phase: ComparePhase::Scanning,
            folders: 3,
            entries: 40,
            ..Default::default()
        },
    });
    assert_eq!(
        p.comparison_status(),
        Some("Comparing subfolders: 40 entries in 3 folders…")
    );
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
    let line = p.comparison_status().unwrap().to_owned();
    assert!(
        line.starts_with("Comparing contents: 1 of 4 files · 2"),
        "{line}"
    );
    // A sample of another comparison changes nothing.
    p.apply(Event::ComparisonProgress {
        id: CompareId(request.id.0 + 7),
        progress: CompareProgress::default(),
    });
    assert_eq!(p.comparison_status(), Some(line.as_str()));
    let Event::DirectoriesCompared { id, result } = result(request) else {
        unreachable!();
    };
    let mut counted = Arc::unwrap_or_clone(result.unwrap());
    counted.differing_folders = 2;
    counted.content_differences = 1;
    counted.content_files = 3;
    counted.content_bytes = 300;
    counted.errors = 1;
    counted.diagnostics = vec![FsError::from_io(
        "compare directories",
        Path::new("left/locked"),
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    )];
    p.apply(Event::DirectoriesCompared {
        id,
        result: Ok(Arc::new(counted)),
    });
    let status = p.comparison_status().unwrap();
    for part in [
        "Compared: 1 selected in the left panel, 1 in the right",
        "2 folders with differences",
        "1 with different contents",
        "3 file pairs read",
        "1 not readable",
    ] {
        assert!(status.contains(part), "{part:?} in {status:?}");
    }
    let summary = &p.comparison.summary;
    assert!(
        summary.contains("Folders with differences: 2."),
        "{summary}"
    );
    assert!(summary.contains("different contents: 1."), "{summary}");
    assert!(
        summary.contains("locked"),
        "the diagnostic names its path: {summary}"
    );
    assert!(p.desktop.notice.is_none(), "still no window");
}
