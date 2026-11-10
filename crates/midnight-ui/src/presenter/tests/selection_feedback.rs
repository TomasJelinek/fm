use super::*;

fn file_entry(id: u64, path: &str) -> Entry {
    Entry::new(EntryId(id), path.into(), EntryKind::File)
}

fn loaded() -> Presenter {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        (0..6)
            .map(|row| {
                Entry::new(
                    EntryId(row),
                    format!("left/{row}").into(),
                    if row % 2 == 0 {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                )
            })
            .collect(),
    );
    p
}
fn visible(p: &Presenter) -> Vec<usize> {
    (0..p.panes[0].display_len())
        .filter(|row| p.panes[0].selection_visible(p.panes[0].display_entry(*row).unwrap()))
        .collect()
}
fn request(commands: &RefCell<Vec<Command>>) -> SelectionRequest {
    let Command::Selection(request) = commands.borrow_mut().pop().unwrap() else {
        panic!("selection")
    };
    request
}
fn ack(p: &mut Presenter, request: &SelectionRequest, bits: u64) {
    p.apply(Event::SelectionChanged {
        pane: request.pane,
        navigation: request.navigation,
        revision: request.revision,
        selection: Arc::new(
            SelectionSnapshot::from_bits(request.view.clone().unwrap(), vec![bits]).unwrap(),
        ),
        deferred: Default::default(),
    });
}

#[test]
fn invert_files_previews_preserve_selected_and_unselected_folders() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset, true, false, &send);
    let selected = request(&commands);
    ack(&mut p, &selected, 1);
    p.bulk_selection(PaneId::Left, SelectionOperation::Invert, &send);
    assert_eq!(visible(&p), [0, 1, 3, 5]);
    p.bulk_selection(PaneId::Left, SelectionOperation::Invert, &send);
    assert_eq!(visible(&p), [0]);
}

#[test]
fn select_files_only_keeps_streaming_feedback_after_an_older_select_all_ack() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![
            file_entry(1, "left/a"),
            Entry::new(EntryId(2), "left/junction".into(), EntryKind::DirectoryLink),
        ],
    });
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    assert_eq!(visible(&p), [0, 1]);
    let all = request(&commands);
    p.bulk_selection(PaneId::Left, SelectionOperation::FilesOnly, &send);
    let files = request(&commands);
    assert_eq!(visible(&p), [0]);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: all.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults::ALL,
    });
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![
            file_entry(3, "left/b"),
            Entry::new(EntryId(4), "left/folder".into(), EntryKind::Directory),
        ],
    });
    assert_eq!(visible(&p), [0, 2]);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: files.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults {
            files: true,
            directories: false,
        },
    });
    assert_eq!(visible(&p), [0, 2]);
}

#[test]
fn invert_files_streaming_defaults_keep_arriving_folders_unchanged() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    let folder = |id, path: &str| Entry::new(EntryId(id), path.into(), EntryKind::Directory);
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![
            file_entry(1, "left/a"),
            folder(2, "left/folder"),
            Entry::new(EntryId(3), "left/junction".into(), EntryKind::DirectoryLink),
        ],
    });
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::Invert, &send);
    assert_eq!(visible(&p), [0]);
    let inverted = request(&commands);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: inverted.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults {
            files: true,
            directories: false,
        },
    });
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![file_entry(4, "left/b"), folder(5, "left/late folder")],
    });
    assert_eq!(visible(&p), [0, 3]);
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let all = request(&commands);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: all.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults::ALL,
    });
    p.bulk_selection(PaneId::Left, SelectionOperation::Invert, &send);
    assert_eq!(visible(&p), [1, 2, 4]);
    let inverted = request(&commands);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: inverted.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults {
            files: false,
            directories: true,
        },
    });
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![folder(6, "left/last folder"), file_entry(7, "left/c")],
    });
    assert_eq!(visible(&p), [1, 2, 4, 5]);
}

#[test]
fn range_anchor_replaces_adds_and_survives_old_acknowledgements() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |cmd| {
        commands.borrow_mut().push(cmd);
        true
    };
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset + 1, false, false, &send);
    p.select_row_modified(PaneId::Left, offset + 3, false, true, &send);
    let first = request(&commands);
    assert_eq!(visible(&p), [1, 2, 3]);
    assert!(p.panes[0].selected.is_empty());
    p.select_row_modified(PaneId::Left, offset + 5, true, true, &send);
    let second = request(&commands);
    assert_eq!(visible(&p), [1, 2, 3, 4, 5]);
    ack(&mut p, &first, 0b001110);
    assert_eq!(visible(&p), [1, 2, 3, 4, 5]);
    ack(&mut p, &second, 0b111110);
    ack(&mut p, &first, 0b001110);
    assert_eq!(visible(&p), [1, 2, 3, 4, 5]);
    p.select_row_modified(PaneId::Left, offset, false, true, &send);
    assert_eq!(
        visible(&p),
        [0, 1],
        "Shift reuses the original plain-click anchor"
    );
}

#[test]
fn refused_failed_and_stale_selection_intents_do_not_leave_predicted_marks() {
    let mut p = loaded();
    let commands = RefCell::new(Vec::new());
    let send = |cmd| {
        commands.borrow_mut().push(cmd);
        true
    };
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset, true, false, &send);
    let first = request(&commands);
    p.select_row_modified(PaneId::Left, offset + 1, true, false, &|_| false);
    assert_eq!(visible(&p), [0]);
    p.apply(Event::SelectionFailed {
        pane: PaneId::Left,
        navigation: first.navigation,
        revision: first.revision,
        error: FsError::from_io("select", Path::new("left"), io::Error::other("refused")),
    });
    assert!(visible(&p).is_empty());
    p.flush_selection(&send);
    let rebase = request(&commands);
    ack(&mut p, &rebase, 0);
    ack(&mut p, &first, 1);
    assert!(visible(&p).is_empty());
    p.select_row_modified(PaneId::Left, offset + 2, true, false, &send);
    let newer = request(&commands);
    ack(&mut p, &newer, 4);
    p.apply(Event::SelectionFailed {
        pane: PaneId::Left,
        navigation: first.navigation,
        revision: first.revision,
        error: FsError::from_io(
            "select",
            Path::new("left"),
            io::Error::other("stale failure"),
        ),
    });
    assert_eq!(visible(&p), [2]);
    assert!(
        !p.panes[0].selection_rebase_pending,
        "stale failure cannot undo a newer acknowledgement"
    );
}

#[test]
fn range_anchor_tracks_paths_after_reorder_and_does_not_select_recycled_ids() {
    let mut p = loaded();
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset + 1, false, false, &|_| true);
    let navigation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            entry(1, "left/replacement"),
            entry(8, "left/1"),
            entry(9, "left/2"),
        ],
    );
    p.select_row_modified(PaneId::Left, offset + 2, false, true, &|_| true);
    assert_eq!(visible(&p), [1, 2]);
    let navigation = begin(&mut p, PaneId::Left, "other");
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![entry(8, "other/recycled"), entry(9, "other/next")],
    );
    assert!(visible(&p).is_empty());
    p.select_row_modified(
        PaneId::Left,
        p.panes[0].parent_offset() + 1,
        false,
        true,
        &|_| true,
    );
    assert_eq!(visible(&p), [1]);
}

#[test]
fn selection_preview_is_bounded_and_bulk_operations_preserve_order() {
    let mut p = loaded();
    let offset = p.panes[0].parent_offset();
    let accepted = std::cell::Cell::new(0);
    let send = |_| {
        accepted.set(accepted.get() + 1);
        true
    };
    for _ in 0..1000 {
        p.select_row_modified(PaneId::Left, offset, true, false, &send);
    }
    assert_eq!(
        accepted.get(),
        32,
        "bounded overlay refuses excess pending gestures before dispatch"
    );
    assert!(visible(&p).is_empty());
    let mut p = loaded();
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &|_| true);
    p.select_row_modified(PaneId::Left, offset + 2, true, false, &|_| true);
    assert_eq!(visible(&p), [0, 1, 3, 4, 5]);
    p.bulk_selection(PaneId::Left, SelectionOperation::Clear, &|_| false);
    assert_eq!(visible(&p), [0, 1, 3, 4, 5]);
}

#[test]
fn panel_details_report_selected_files_and_folders_even_on_parent_cursor() {
    let mut p = loaded();
    set_selected(&mut p, PaneId::Left, &[EntryId(0), EntryId(1), EntryId(2)]);
    p.panes[0].parent_cursor = true;
    let details = p.panel_details(PaneId::Left);
    assert!(
        details
            .iter()
            .any(|field| field.label == "Selected" && field.value == "1 file, 2 folders")
    );
}

#[test]
fn replacement_view_discards_range_preview_and_a_missing_anchor_starts_a_new_range() {
    let mut p = loaded();
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset + 1, false, false, &|_| true);
    p.select_row_modified(PaneId::Left, offset + 4, false, true, &|_| true);
    assert_eq!(visible(&p), [1, 2, 3, 4]);
    // A filtered projection can change within the same navigation generation.
    p.panes[0].entries = vec![
        entry(1, "left/new-a"),
        entry(2, "left/new-b"),
        entry(3, "left/new-c"),
    ]
    .into();
    p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
    p.panes[0].search.view_revision += 1;
    assert!(
        visible(&p).is_empty(),
        "range positions must not migrate to replacement rows"
    );
    p.select_row_modified(PaneId::Left, offset, false, true, &|_| true);
    assert_eq!(visible(&p), [0]);
    p.select_row_modified(PaneId::Left, offset + 2, false, true, &|_| true);
    assert_eq!(
        visible(&p),
        [0, 1, 2],
        "fallback anchor persists for subsequent Shift clicks"
    );
}

#[test]
fn streamed_ranges_preview_without_scanning_or_capping_the_listing() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: (0..10_000)
            .map(|row| entry(90_000 - row, &format!("left/{row}")))
            .collect(),
    });
    let offset = p.panes[0].parent_offset();
    p.select_row_modified(PaneId::Left, offset + 1, false, false, &|_| true);
    let commands = RefCell::new(Vec::new());
    p.select_row_modified(PaneId::Left, offset + 9_999, false, true, &|cmd| {
        commands.borrow_mut().push(cmd);
        true
    });
    let range = request(&commands);
    assert!(
        range.view.is_none(),
        "UI never constructs a complete streamed index"
    );
    assert!(matches!(range.action, SelectionAction::Range { .. }));
    assert!(!p.panes[0].selection_visible(p.panes[0].display_entry(0).unwrap()));
    for row in [1, 5000, 9999] {
        assert!(p.panes[0].selection_visible(p.panes[0].display_entry(row).unwrap()));
    }
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![entry(999_999, "left/later")],
    });
    assert!(!p.panes[0].selection_visible(p.panes[0].display_entry(10_000).unwrap()));
}

#[test]
fn deferred_all_acknowledgement_keeps_marks_for_existing_and_arriving_rows() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![entry(1, "left/a"), entry(2, "left/b")],
    });
    let commands = RefCell::new(Vec::new());
    let send = |cmd| {
        commands.borrow_mut().push(cmd);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &send);
    let all = request(&commands);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: all.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults::ALL,
    });
    assert_eq!(
        visible(&p),
        [0, 1],
        "deferred acknowledgement must retain selected appearance"
    );
    assert!(
        p.panel_details(PaneId::Left)
            .iter()
            .any(|field| field.label == "Selected" && field.value.contains("updating"))
    );
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![entry(3, "left/c")],
    });
    assert_eq!(visible(&p), [0, 1, 2]);
    let rows = p.panes[0].streaming.clone();
    finish(&mut p, PaneId::Left, navigation, rows);
    assert_eq!(
        visible(&p),
        [0, 1, 2],
        "listing commit retains the acknowledged default until final rebase"
    );
    p.panes[0].selection_rebase_pending = true;
    p.flush_selection(&send);
    let rebase = request(&commands);
    ack(&mut p, &rebase, 7);
    assert_eq!(visible(&p), [0, 1, 2]);
    assert!(!p.panes[0].selection_deferred.any());
}

#[test]
fn deferred_defaults_keep_explicit_exceptions_and_survive_later_queue_failure() {
    let mut p = Presenter::new("left".into(), "right".into());
    let navigation = begin(&mut p, PaneId::Left, "left");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![file_entry(1, "left/a"), file_entry(2, "left/b")],
    });
    let commands = RefCell::new(Vec::new());
    let send = |cmd| {
        commands.borrow_mut().push(cmd);
        true
    };
    p.bulk_selection(PaneId::Left, SelectionOperation::Invert, &send);
    let inverted = request(&commands);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: inverted.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults {
            files: true,
            directories: false,
        },
    });
    p.select_row_modified(PaneId::Left, 1, true, false, &send);
    let toggle = request(&commands);
    let exception = Arc::new(DirectoryIndex::from_entries(&[file_entry(2, "left/b")]));
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: toggle.revision,
        selection: Arc::new(SelectionSnapshot::from_bits(exception, vec![0]).unwrap()),
        deferred: SelectionDefaults {
            files: true,
            directories: false,
        },
    });
    assert_eq!(
        visible(&p),
        [0],
        "explicit unselected paths override the default"
    );
    p.select_row_modified(PaneId::Left, 0, true, false, &send);
    let failed = request(&commands);
    p.apply(Event::SelectionFailed {
        pane: PaneId::Left,
        navigation,
        revision: failed.revision,
        error: FsError::from_io("selection", Path::new("left"), io::Error::other("busy")),
    });
    assert_eq!(
        visible(&p),
        [0],
        "a failed new intent cannot erase an acknowledged default"
    );
    p.bulk_selection(PaneId::Left, SelectionOperation::Invert, &send);
    assert_eq!(visible(&p), [1], "new pending inversion applies only once");
}

#[test]
fn failed_destination_does_not_transfer_its_deferred_default_to_restored_rows() {
    let mut p = loaded();
    let navigation = begin(&mut p, PaneId::Left, "elsewhere");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![entry(90, "elsewhere/a")],
    });
    let commands = RefCell::new(Vec::new());
    p.bulk_selection(PaneId::Left, SelectionOperation::All, &|cmd| {
        commands.borrow_mut().push(cmd);
        true
    });
    let all = request(&commands);
    p.apply(Event::SelectionChanged {
        pane: PaneId::Left,
        navigation,
        revision: all.revision,
        selection: Arc::default(),
        deferred: SelectionDefaults::ALL,
    });
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io(
            "list",
            Path::new("elsewhere"),
            io::Error::other("unavailable"),
        ),
    });
    assert!(!p.panes[0].selection_deferred.any());
    assert!(visible(&p).is_empty());
}
