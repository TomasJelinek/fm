//! The quick search only moves the cursor: a command closes it and acts on the row the
//! panel shows, and Enter opens only a match (`presenter::search_commands`).
use super::search::{accept, query, result, snapshot};
use super::*;
use crate::presenter::copy_dialog::CopyDraftInput;

const NAMES: [&str; 3] = ["alpha.txt", "beta.txt", "readme.md"];

fn path(name: &str) -> PathBuf {
    test_root().join(name)
}

fn listed() -> Presenter {
    let mut p = Presenter::new(test_root(), test_root());
    p.preferences.loaded = true;
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    snapshot(&mut p, &NAMES);
    p
}

/// Text typed by accident with the cursor on readme.md: no name matches it.
fn unmatched() -> Presenter {
    let mut p = listed();
    p.focus_display_row(PaneId::Left, 2);
    let request = query(&mut p, "Paged vs continuous");
    accept(&mut p, &request, result(&request, &[], 0));
    assert_eq!(p.panes[0].search_match_count(), 0);
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)));
    p
}

/// The command closed the search, cancelled its backend work and kept the cursor.
fn assert_closed_on(p: &Presenter, sent: &[Command], cursor: u64) {
    assert_eq!(p.panes[0].search.query, "");
    assert_eq!(p.panes[0].cursor, Some(EntryId(cursor)));
    assert!(
        sent.iter()
            .any(|command| matches!(command, Command::CancelSearch { .. })),
        "the closed query's backend work is cancelled"
    );
}

fn opened(sent: &[Command]) -> Vec<PathBuf> {
    sent.iter()
        .filter_map(|command| match command {
            Command::OpenEntry { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn f3_closes_a_nonmatching_search_and_views_the_focused_file() {
    for basic in [false, true] {
        let mut p = unmatched();
        let sent = RefCell::new(Vec::new());
        let launch = p
            .view_command(PaneId::Left, basic, &|command| {
                sent.borrow_mut().push(command);
                true
            })
            .expect("the quick search never stops the viewer");
        assert_eq!(launch.focused, path("readme.md"));
        assert_eq!(launch.basic, basic);
        assert_closed_on(&p, &sent.borrow(), 3);
    }
}

#[test]
fn f4_closes_a_nonmatching_search_and_edits_the_focused_file() {
    let mut p = unmatched();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.panel_command(PaneId::Left, &send, |p| p.edit_file(PaneId::Left, &send));
    assert!(
        sent.borrow().iter().any(|command| matches!(command,
            Command::EditFile { path: edited, .. } if *edited == path("readme.md"))),
        "F4 edits the focused file"
    );
    assert_eq!(p.desktop.notice, None);
    assert_closed_on(&p, &sent.borrow(), 3);
}

#[test]
fn f5_and_f6_close_a_nonmatching_search_and_capture_the_focused_file() {
    for moving in [false, true] {
        let mut p = unmatched();
        let sent = RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        p.panel_command(PaneId::Left, &send, |p| {
            if moving {
                p.request_move_dialog(PaneId::Left, &send);
            } else {
                p.request_copy_dialog(PaneId::Left, &send);
            }
        });
        let draft = p
            .copies
            .draft
            .as_ref()
            .expect("the quick search never stops F5 or F6");
        let CopyDraftInput::Native(files) = &draft.input else {
            panic!("a native draft");
        };
        assert!(files.selection.is_empty());
        assert_eq!(files.fallback, Some(path("readme.md")));
        assert_eq!(
            draft.intent,
            if moving {
                TransferIntent::Move
            } else {
                TransferIntent::Copy
            }
        );
        assert_closed_on(&p, &sent.borrow(), 3);
    }
}

#[test]
fn f8_closes_a_nonmatching_search_and_deletes_the_focused_row() {
    for mode in [DeleteMode::Trash, DeleteMode::Permanent] {
        let mut p = unmatched();
        let sent = RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        p.panel_command(PaneId::Left, &send, |p| {
            p.start_operation(PaneId::Left, MutationKind::Delete(mode), &send);
        });
        let sent = sent.into_inner();
        let prepare = sent
            .iter()
            .find_map(|command| match command {
                Command::PrepareOperation(request) => Some(request),
                _ => None,
            })
            .expect("F8 prepares the deletion");
        assert_eq!(prepare.kind, MutationKind::Delete(mode));
        assert!(prepare.selection.is_empty());
        assert_eq!(
            prepare.focused,
            Some(EntryId(3)),
            "the focused row is the target"
        );
        assert_closed_on(&p, &sent, 3);
    }
}

#[test]
fn ctrl_f8_closes_a_nonmatching_search_and_checksums_the_focused_file() {
    let mut p = unmatched();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.panel_command(PaneId::Left, &send, |p| {
        p.request_checksums(PaneId::Left, &send)
    });
    let (_, session) = p
        .checksums
        .sessions
        .iter()
        .next()
        .expect("the quick search never stops Ctrl+F8");
    assert_eq!(session.files.fallback, Some(path("readme.md")));
    assert_closed_on(&p, &sent.borrow(), 3);
}

#[test]
fn enter_opens_only_a_match_and_a_missing_name_closes_the_search_with_a_note() {
    let mut p = unmatched();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(!p.activate_focused(PaneId::Left, &send));
    assert!(opened(&sent.borrow()).is_empty(), "Enter starts nothing");
    assert_closed_on(&p, &sent.borrow(), 3);
    assert_eq!(
        p.search_note(PaneId::Left),
        Some("No name matches “Paged vs continuous”")
    );
    assert_eq!(p.desktop.notice, None, "a note, not a dialog");
    // The focused row is what the panel now shows; Enter opens it.
    assert!(p.activate_focused(PaneId::Left, &send));
    assert_eq!(opened(&sent.borrow()), [path("readme.md")]);
    assert_eq!(p.search_note(PaneId::Left), None);
    // A matching row opens as before, and its search stays.
    sent.borrow_mut().clear();
    p.focus_display_row(PaneId::Left, 0);
    let request = query(&mut p, "bet");
    accept(&mut p, &request, result(&request, &[1], 3));
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert!(p.activate_focused(PaneId::Left, &send));
    assert_eq!(opened(&sent.borrow()), [path("beta.txt")]);
    assert_eq!(p.panes[0].search.query, "bet");
}

#[test]
fn enter_while_the_search_computes_opens_nothing_until_a_match_is_focused() {
    let mut p = listed();
    p.focus_display_row(PaneId::Left, 0);
    let request = query(&mut p, "read");
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    assert!(!p.activate_focused(PaneId::Left, &send));
    assert!(
        opened(&sent.borrow()).is_empty(),
        "the old row does not open"
    );
    assert_eq!(p.panes[0].search.query, "read", "the search goes on");
    assert_eq!(
        p.search_note(PaneId::Left),
        Some("Still searching for “read”…")
    );
    accept(&mut p, &request, result(&request, &[2], 4));
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)), "its first match");
    assert_eq!(p.search_note(PaneId::Left), None);
    assert!(p.activate_focused(PaneId::Left, &send));
    assert_eq!(opened(&sent.borrow()), [path("readme.md")]);
}

#[test]
fn a_command_cancels_a_computing_search_whose_late_result_cannot_move_the_cursor() {
    for filtering in [false, true] {
        let mut p = listed();
        // With "Show only filename search matches" the list waits for the result.
        p.preferences.values.quick_search_filter = filtering;
        p.focus_display_row(PaneId::Left, 1);
        let request = query(&mut p, "read");
        assert_eq!(p.panes[0].search_view_waiting(), filtering);
        let sent = RefCell::new(Vec::new());
        let launch = p
            .view_command(PaneId::Left, false, &|command| {
                sent.borrow_mut().push(command);
                true
            })
            .expect("a computing search never stops the viewer");
        assert_eq!(launch.focused, path("beta.txt"), "the row focused at F3");
        assert_closed_on(&p, &sent.borrow(), 2);
        assert!(!p.panes[0].search_view_waiting());
        accept(&mut p, &request, result(&request, &[2], 4));
        assert_eq!(
            p.panes[0].cursor,
            Some(EntryId(2)),
            "the late result does not move the cursor"
        );
        assert_eq!(p.panes[0].display_len(), NAMES.len());
    }
}

#[test]
fn a_quick_filtered_command_acts_on_the_shown_rows_before_they_all_return() {
    let mut p = listed();
    p.preferences.values.quick_search_filter = true;
    p.focus_display_row(PaneId::Left, 0);
    let request = query(&mut p, "zz");
    accept(&mut p, &request, result(&request, &[], 0));
    assert_eq!(p.panes[0].display_len(), 0, "no row is shown");
    assert_eq!(p.panes[0].cursor, None);
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.panel_command(PaneId::Left, &send, |p| {
        p.request_copy_dialog(PaneId::Left, &send)
    });
    assert!(p.copies.draft.is_none(), "nothing shown, nothing copied");
    assert_eq!(p.panes[0].search.query, "", "the search still closes");
    assert_eq!(p.panes[0].display_len(), NAMES.len());
}

/// Accept the persistent filter's projection of `request`: `shown` rows pass the filter.
fn accept_filtered(p: &mut Presenter, request: &SearchRequest, shown: &[usize], matches: &[usize]) {
    let rows = |rows: &[usize], end| -> (Arc<[SearchMatch]>, Arc<DirectoryIndex>) {
        let entries: Vec<_> = rows
            .iter()
            .map(|row| request.source[*row].clone())
            .collect();
        (
            rows.iter()
                .map(|row| SearchMatch {
                    source_row: *row,
                    display_prefix_end: end,
                })
                .collect::<Vec<_>>()
                .into(),
            Arc::new(DirectoryIndex::from_entries(&entries)),
        )
    };
    let (filtered, filtered_index) = rows(shown, 0);
    let (matches, index) = rows(matches, request.query.len());
    accept(
        p,
        request,
        Arc::new(SearchResult {
            source: request.source.clone(),
            matches,
            index,
            filtered,
            filtered_index,
        }),
    );
}

/// A panel filter that shows beta.txt and readme.md, with the cursor on readme.md.
fn panel_filtered() -> Presenter {
    let mut p = listed();
    p.preferences.values.panel_filters[0] = PanelFilter {
        enabled: true,
        expression: "b*;*.md".into(),
        ..Default::default()
    };
    p.search_filter_changed();
    let sent = RefCell::new(Vec::new());
    p.flush_search(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let request = sent
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Search(request) => Some(request),
            _ => None,
        })
        .expect("the panel filter is computed on the backend");
    accept_filtered(&mut p, &request, &[1, 2], &[1, 2]);
    p.focus_display_row(PaneId::Left, 1);
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)));
    settle_selection(&mut p);
    p
}

/// The event pump's selection rebase onto the filtered rows, acknowledged.
fn settle_selection(p: &mut Presenter) {
    let sent = RefCell::new(Vec::new());
    p.flush_selection(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    for command in sent.into_inner() {
        if let Command::Selection(request) = command {
            let view = request.view.clone().expect("a complete view");
            let bits = vec![0; view.by_id.len().div_ceil(64)];
            p.apply(Event::SelectionChanged {
                pane: request.pane,
                navigation: request.navigation,
                revision: request.revision,
                selection: Arc::new(SelectionSnapshot::from_bits(view, bits).unwrap()),
                deferred: Default::default(),
            });
        }
    }
    assert!(!p.panes[0].selection_rebase_pending);
}

#[test]
fn closing_a_search_keeps_the_panel_filter_rows_without_computing_them_again() {
    let mut p = panel_filtered();
    let request = query(&mut p, "zz");
    accept_filtered(&mut p, &request, &[1, 2], &[]);
    let sent = RefCell::new(Vec::new());
    let launch = p
        .view_command(PaneId::Left, false, &|command| {
            sent.borrow_mut().push(command);
            true
        })
        .expect("the quick search never stops the viewer");
    assert_eq!(launch.focused, path("readme.md"));
    assert_closed_on(&p, &sent.borrow(), 3);
    assert!(
        !p.panes[0].search_view_waiting(),
        "the filter's rows stay shown"
    );
    assert_eq!(p.panes[0].display_len(), 2);
    assert_eq!(p.panes[0].search_prefix_end(1), 0, "no stale highlight");
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Search(_))),
        "the filter is not computed again"
    );
    // Escape keeps them too, so Insert's toggle finds the focused row shown.
    let request = query(&mut p, "zz");
    accept_filtered(&mut p, &request, &[1, 2], &[]);
    p.change_search_query(PaneId::Left, "", &|_| true);
    assert!(!p.panes[0].search_view_waiting());
    let sent = RefCell::new(Vec::new());
    p.move_cursor(PaneId::Left, 9, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Selection(request) if matches!(&request.action,
            SelectionAction::Toggle(toggled) if *toggled == path("readme.md")))));
}

#[test]
fn a_panel_filter_that_hides_the_focused_row_is_explained_never_as_the_quick_search() {
    let mut p = panel_filtered();
    // alpha.txt is not shown; a cursor left on it is not a target.
    p.panes[0].cursor = Some(EntryId(1));
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let reason = p
        .view_command(PaneId::Left, false, &send)
        .expect_err("nothing shown is focused")
        .expect("F3 explains why");
    assert!(reason.contains("panel filter"), "{reason}");
    assert!(!reason.contains("quick search"), "{reason}");
    assert!(!p.activate_focused(PaneId::Left, &send));
    let note = p.search_note(PaneId::Left).expect("Enter explains why");
    assert!(note.contains("panel filter"), "{note}");
    p.panel_command(PaneId::Left, &send, |p| {
        p.request_copy_dialog(PaneId::Left, &send)
    });
    assert!(p.copies.draft.is_none());
    assert!(
        p.search_note(PaneId::Left)
            .is_some_and(|note| note.contains("panel filter"))
    );
    assert!(opened(&sent.borrow()).is_empty());
}
