//! Real engine publications through Slint's panel text input and Options controls.
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::refresh;
use slint::platform::Key;
use std::sync::{Arc, atomic::Ordering};
use support::Harness;

fn query(h: &Harness) -> String {
    if h.pane == PaneId::Left {
        h.ui.window.get_left_search_query().to_string()
    } else {
        h.ui.window.get_right_search_query().to_string()
    }
}

fn pending(h: &Harness) -> bool {
    if h.pane == PaneId::Left {
        h.ui.window.get_left_search_pending()
    } else {
        h.ui.window.get_right_search_pending()
    }
}

fn count(h: &Harness) -> i32 {
    if h.pane == PaneId::Left {
        h.ui.window.get_left_search_count()
    } else {
        h.ui.window.get_right_search_count()
    }
}

fn focused_name(h: &Harness) -> String {
    h.rows()
        .iter()
        .find(|row| row.cursor)
        .map(|row| row.name.to_string())
        .unwrap_or_default()
}

fn focus(h: &Harness) {
    h.ui.window.set_active_pane(h.pane.index() as i32);
    h.ui.window.invoke_focus_files();
}

fn settle(h: &mut Harness, expected: &str, matches: i32) {
    h.until(|h| query(h) == expected && !pending(h) && count(h) == matches);
}

#[test]
fn unicode_typing_and_match_navigation_use_real_native_tree_publications() {
    let mut h = Harness::with_children(
        PaneId::Left,
        false,
        &["Straße 文档", "Strasse other", "unrelated"],
    );
    let lists = h.calls.lists.load(Ordering::SeqCst);
    let original = h.view.as_ref().unwrap().clone();
    let selected_row = h
        .rows()
        .iter()
        .position(|row| row.name == "unrelated")
        .unwrap();
    h.ui.window
        .invoke_row_click(0, selected_row as i32, true, false);
    h.until(|h| h.selection.as_ref().unwrap().len() == 1);
    focus(&h);
    ui::key(&h.ui.window, "strass");
    settle(&mut h, "strass", 2);
    assert_eq!(h.view.as_ref().unwrap().rows.len(), original.rows.len());
    assert!(Arc::ptr_eq(
        &h.view.as_ref().unwrap().root.entries,
        &original.root.entries
    ));
    assert_eq!(
        h.selection.as_ref().unwrap().len(),
        1,
        "jump search keeps marks"
    );
    let names: Vec<_> = h
        .rows()
        .iter()
        .filter(|row| row.prefix_end > 0)
        .map(|row| row.name.to_string())
        .collect();
    assert_eq!(names.len(), 2);
    assert_eq!(focused_name(&h), names[0]);
    ui::key(&h.ui.window, Key::DownArrow);
    assert_eq!(focused_name(&h), names[1]);
    ui::key(&h.ui.window, Key::DownArrow);
    assert_eq!(focused_name(&h), names[1], "match movement does not wrap");
    ui::key(&h.ui.window, Key::Home);
    assert_eq!(focused_name(&h), names[0]);
    ui::key(&h.ui.window, Key::End);
    assert_eq!(focused_name(&h), names[1]);
    for layout in [1, 2, 0] {
        h.ui.window.invoke_view_mode(0, layout);
        assert_eq!(query(&h), "strass");
        assert_eq!(count(&h), 2);
    }
    let cursor = focused_name(&h);
    ui::key(&h.ui.window, Key::Escape);
    h.until(|h| query(h).is_empty() && !pending(h));
    assert_eq!(focused_name(&h), cursor);
    assert!(h.rows().iter().all(|row| row.prefix_end == 0));
    assert_eq!(h.calls.lists.load(Ordering::SeqCst), lists);
}

#[test]
fn archive_quick_filter_preview_cancel_and_clear_keep_ancestor_authority() {
    let mut h = Harness::with_children(PaneId::Right, true, &["alpha", "alphabet", "beta"]);
    focus(&h);
    ui::key(&h.ui.window, "al");
    settle(&mut h, "al", 2);
    let source = h.view.as_ref().unwrap().root.clone();
    let lists = h.calls.lists.load(Ordering::SeqCst);
    h.ui.draw();
    ui::click_root(&h.ui.window, "Options", AccessibleRole::Button);
    h.ui.draw();
    ui::click_popup(
        &h.ui.window,
        "Show only filename search matches",
        AccessibleRole::Checkbox,
    );
    h.until(|h| !pending(h) && h.view.as_ref().unwrap().rows.len() == 3);
    let filtered = h.view.as_ref().unwrap();
    assert!(filtered.rows[0].context_only && !filtered.rows[0].selectable);
    assert!(
        filtered
            .rows
            .iter()
            .all(|row| row.entry.virtual_node.is_some())
    );
    assert!(filtered.root.same_snapshot(&source));
    assert_eq!(count(&h), 2);
    assert!(
        h.filter_label().is_empty(),
        "quick search is not a persistent filter"
    );
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    h.until(|h| !pending(h) && h.view.as_ref().unwrap().rows.len() == 5);
    assert!(!h.ui.window.get_quick_search_filter());
    assert_eq!(query(&h), "al");
    assert_eq!(count(&h), 2);
    focus(&h);
    ui::key(&h.ui.window, Key::Escape);
    h.until(|h| query(h).is_empty() && !pending(h));
    assert!(
        h.view
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .all(|row| !row.context_only)
    );
    assert_eq!(h.calls.lists.load(Ordering::SeqCst), lists);
}

#[test]
fn rapid_typing_escape_and_full_queue_converge_without_reopening_folders() {
    let mut h = Harness::with_children(PaneId::Left, false, &["alpha", "alphabet", "beta"]);
    focus(&h);
    let source = h.view.as_ref().unwrap().root.clone();
    ui::key(&h.ui.window, "a");
    ui::key(&h.ui.window, "l");
    ui::key(&h.ui.window, "p");
    assert_eq!(query(&h), "alp");
    assert!(
        h.requests.borrow().len() <= 1,
        "only one options request is in flight"
    );
    // A queued backend completion cannot replace the latest committed input.
    let event = h.receive();
    h.apply_event(event);
    assert_eq!(query(&h), "alp");
    ui::key(&h.ui.window, Key::Escape);
    h.until(|h| query(h).is_empty() && !pending(h));
    assert!(h.rows().iter().all(|row| row.prefix_end == 0));
    assert!(h.view.as_ref().unwrap().root.same_snapshot(&source));

    *h.reject_reproject.borrow_mut() = true;
    ui::key(&h.ui.window, "be");
    assert_eq!(
        query(&h),
        "be",
        "queue pressure must not discard committed text"
    );
    assert!(pending(&h));
    *h.reject_reproject.borrow_mut() = false;
    h.presenter.borrow_mut().flush_search(h.send.as_ref());
    refresh(&h.ui.window, &h.presenter.borrow());
    settle(&mut h, "be", 1);
    assert_eq!(focused_name(&h), "beta");
    ui::key(&h.ui.window, "none");
    settle(&mut h, "benone", 0);
    let navigations = h.calls.navigations.load(Ordering::SeqCst);
    ui::key(&h.ui.window, Key::Return);
    assert_eq!(
        h.calls.navigations.load(Ordering::SeqCst),
        navigations,
        "zero matches cannot activate a stale directory caret"
    );
}

#[test]
fn insert_clears_search_then_marks_the_captured_match_and_advances() {
    let mut h = Harness::with_children(PaneId::Left, false, &["alpha", "alphabet", "beta"]);
    let lists = h.calls.lists.load(Ordering::SeqCst);
    focus(&h);
    ui::key(&h.ui.window, "al");
    settle(&mut h, "al", 2);
    assert_eq!(focused_name(&h), "alpha");
    ui::key(&h.ui.window, Key::Insert);
    h.until(|h| query(h).is_empty() && !pending(h) && h.selection.as_ref().unwrap().len() == 1);
    let marked: Vec<_> = h
        .rows()
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect();
    assert_eq!(marked, ["alpha"]);
    assert_eq!(focused_name(&h), "alphabet");
    assert_eq!(h.calls.lists.load(Ordering::SeqCst), lists);
}

#[test]
fn branch_collapse_rebuilds_matches_and_escape_preserves_collapsed_state() {
    let mut h = Harness::new(PaneId::Left, false);
    focus(&h);
    ui::key(&h.ui.window, "z");
    settle(&mut h, "z", 1);
    let parent = h
        .rows()
        .iter()
        .find(|row| row.name == "folder")
        .unwrap()
        .identity
        .clone();
    h.ui.window.invoke_tree_disclosure(0, parent);
    h.until(|h| h.view.as_ref().unwrap().rows.len() == 2 && !pending(h) && count(h) == 0);
    assert_eq!(query(&h), "z");
    let navigations = h.calls.navigations.load(Ordering::SeqCst);
    ui::key(&h.ui.window, Key::Return);
    assert_eq!(h.calls.navigations.load(Ordering::SeqCst), navigations);
    ui::key(&h.ui.window, Key::Escape);
    h.until(|h| query(h).is_empty() && !pending(h));
    assert_eq!(h.view.as_ref().unwrap().rows.len(), 2);
    assert!(
        h.view
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .all(|row| !row.expanded)
    );
}
