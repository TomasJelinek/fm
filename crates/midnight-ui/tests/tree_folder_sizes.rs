//! Directory-size commands through real Engine workers and installed Slint callbacks.
#[path = "support/native_panel_input.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use slint::{Model, platform::Key};
use std::sync::Arc;
use support::Harness;

fn focus(h: &Harness) {
    h.ui.window.set_active_pane(h.pane.index() as i32);
    h.ui.window.invoke_focus_files();
}

fn row_id(h: &Harness, name: &str) -> TreeRowId {
    h.view
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.entry.display_name == name)
        .unwrap()
        .id
}

fn mark(h: &mut Harness, name: &str) {
    let row = row_id(h, name);
    let index = h.rows().iter().position(|row| row.name == name).unwrap();
    h.ui.window
        .invoke_row_click(h.pane.index() as i32, index as i32, true, false);
    h.until(|h| h.selection.as_ref().unwrap().contains_row(row));
}

fn compute(h: &Harness) {
    focus(h);
    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "s");
}

fn wait_size(h: &mut Harness, name: &str, bytes: u64) {
    let row = row_id(h, name);
    h.until(|h| {
        h.sizes
            .as_ref()
            .and_then(|sizes| sizes.get(row))
            .is_some_and(|size| size.finished && size.errors == 0 && size.bytes == bytes)
    });
    let displayed = h.rows().into_iter().find(|row| row.name == name).unwrap();
    assert_eq!(
        displayed.size_state, 0,
        "completed values must lose their progress style"
    );
    assert!(!displayed.size_running, "this occurrence has finished");
    let digits: String = displayed
        .size
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    assert_eq!(digits, bytes.to_string());
}

fn folder_total(h: &Harness) -> Option<String> {
    let details = if h.pane == PaneId::Left {
        h.ui.window.get_left_details()
    } else {
        h.ui.window.get_right_details()
    };
    details
        .iter()
        .find(|row| row.label == "Selected folder bytes")
        .map(|row| row.value.to_string())
}

fn bulk_without_selection(archive: bool, pane: PaneId) {
    let mut h = Harness::with_sizes(pane, archive);
    h.ui.window.invoke_sizes_format_changed(true);
    let original = h.view.as_ref().unwrap().clone();
    compute(&h);
    wait_size(&mut h, "folder", 1_012);
    wait_size(&mut h, "large", 1_000);
    wait_size(&mut h, "small", 12);
    wait_size(&mut h, "empty", 0);
    assert!(
        Arc::ptr_eq(&original, h.view.as_ref().unwrap()),
        "progress must not reproject rows"
    );
    assert_eq!(
        h.sizes.as_ref().unwrap().len(),
        4,
        "each visible folder receives its own total"
    );
    for mode in [1, 2, 0] {
        h.ui.window.invoke_view_mode(pane.index() as i32, mode);
        assert!(
            h.rows()
                .iter()
                .any(|row| row.name == "folder" && row.size_state == 0)
        );
    }
}

#[test]
fn native_bulk_measures_visible_folders_without_reordering_the_tree() {
    bulk_without_selection(false, PaneId::Left);
}

#[test]
fn archive_bulk_reads_metadata_and_keeps_all_three_panel_layouts() {
    bulk_without_selection(true, PaneId::Right);
}

#[test]
fn selected_descendants_report_exact_bytes_without_measuring_unselected_parent() {
    let mut h = Harness::with_sizes(PaneId::Left, false);
    h.ui.window.invoke_sizes_format_changed(true);
    mark(&mut h, "small");
    compute(&h);
    wait_size(&mut h, "small", 12);
    h.until(|h| folder_total(h).is_some_and(|value| value.contains("12 bytes")));
    assert_eq!(h.sizes.as_ref().unwrap().get(row_id(&h, "folder")), None);
    assert_eq!(h.sizes.as_ref().unwrap().get(row_id(&h, "large")), None);
    assert_eq!(*h.calls.size_reads.lock().unwrap(), ["small"]);
}

#[test]
fn archive_overlapping_selection_has_independent_ancestor_and_descendant_totals() {
    let mut h = Harness::with_sizes(PaneId::Right, true);
    h.ui.window.invoke_sizes_format_changed(true);
    mark(&mut h, "folder");
    mark(&mut h, "small");
    compute(&h);
    wait_size(&mut h, "folder", 1_012);
    wait_size(&mut h, "small", 12);
    h.until(|h| folder_total(h).is_some());
    let text = folder_total(&h).unwrap();
    assert!(
        text.chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .starts_with("1024"),
        "{text}"
    );
}

#[test]
fn space_measures_its_original_descendant_while_other_folders_are_marked() {
    let mut h = Harness::with_sizes(PaneId::Left, false);
    h.ui.window.invoke_sizes_format_changed(true);
    mark(&mut h, "large");
    focus(&h);
    ui::key(&h.ui.window, Key::DownArrow);
    assert!(h.rows().iter().any(|row| row.name == "small" && row.cursor));
    ui::key(&h.ui.window, " ");
    assert!(
        h.rows()
            .iter()
            .any(|row| row.name == "root.txt" && row.cursor)
    );
    wait_size(&mut h, "small", 12);
    assert_eq!(h.sizes.as_ref().unwrap().get(row_id(&h, "large")), None);
    h.until(|h| h.selection.as_ref().unwrap().len() == 2);
    assert_eq!(*h.calls.size_reads.lock().unwrap(), ["small"]);
}

#[test]
fn files_only_selection_never_falls_back_to_measuring_all_folders() {
    let mut h = Harness::with_sizes(PaneId::Left, false);
    mark(&mut h, "root.txt");
    compute(&h);
    h.until(|h| h.size_finished == 1);
    assert!(h.sizes.as_ref().is_none_or(|sizes| sizes.is_empty()));
    assert!(h.calls.size_reads.lock().unwrap().is_empty());
    assert!(folder_total(&h).is_none());
}

#[test]
fn explicit_size_sort_and_filter_preserve_calculated_descendant_values() {
    let mut h = Harness::with_sizes(PaneId::Right, true);
    h.ui.window.invoke_sizes_format_changed(true);
    for name in ["large", "small", "empty"] {
        mark(&mut h, name);
    }
    compute(&h);
    wait_size(&mut h, "large", 1_000);
    wait_size(&mut h, "small", 12);
    wait_size(&mut h, "empty", 0);
    let before = h.view.as_ref().unwrap().stamp.revision;
    h.ui.draw();
    // Both panels have the same accessible header; use the installed active-panel callback.
    h.ui.window.invoke_sort(1, 1);
    h.until(|h| h.view.as_ref().unwrap().stamp.revision != before);
    let names: Vec<_> = h
        .view
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .filter(|row| row.depth == 1)
        .map(|row| row.entry.display_name.as_str())
        .collect();
    assert_eq!(names, ["empty", "small", "large"]);
    let reads = h.calls.size_reads.lock().unwrap().len();

    h.ui.window.invoke_open_panel_filter(1);
    let child = native::filter(&h);
    // Masks retain directories by design; Expression can hide these directory rows.
    native::click(&child, "Filter mode", AccessibleRole::Combobox);
    native::key(&child, Key::DownArrow);
    native::key(&child, Key::DownArrow);
    native::key(&child, Key::Return);
    native::click(&child, "File filter expression", AccessibleRole::TextInput);
    native::chord(&child, &[Key::Control], "a");
    native::key(&child, "name = \"small\"");
    native::click(&child, "OK", AccessibleRole::Button);
    h.until(|h| h.filter_label().contains("small") && h.view.as_ref().unwrap().rows.len() == 2);
    wait_size(&mut h, "small", 12);
    h.ui.window.invoke_clear_panel_filter(1);
    h.until(|h| h.filter_label().is_empty() && h.view.as_ref().unwrap().rows.len() == 5);
    wait_size(&mut h, "large", 1_000);
    assert_eq!(
        h.calls.size_reads.lock().unwrap().len(),
        reads,
        "sort/filter must reuse accepted results"
    );
}
