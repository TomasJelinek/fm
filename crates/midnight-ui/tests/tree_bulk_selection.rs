//! Real tree selection publications and immediate Slint feedback for both providers.
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use slint::platform::Key;
use support::Harness;

fn selected(h: &Harness, names: &[&str]) {
    for row in h.rows() {
        assert_eq!(
            row.selected,
            names.contains(&row.name.as_str()),
            "{}",
            row.name
        );
    }
}

fn settle(h: &mut Harness, names: &[&str]) {
    h.until(|h| {
        let selection = h.selection.as_ref().unwrap();
        h.view.as_ref().unwrap().rows.iter().all(|row| {
            selection.contains_row(row.id) == names.contains(&row.entry.display_name.as_str())
        })
    });
    selected(h, names);
}

fn exercise(pane: PaneId, archive: bool) {
    let mut h = Harness::new(pane, archive);
    // A real FilePane click activates the presenter before dispatching row-click.
    // Setting only the rendered property is undone by the next presenter refresh.
    h.ui.window.invoke_activate(pane.index() as i32);
    let nested = h.rows().iter().position(|row| row.name == "z.txt").unwrap();
    h.ui.window
        .invoke_row_click(pane.index() as i32, nested as i32, true, false);
    settle(&mut h, &["z.txt"]);
    h.ui.window.invoke_focus_files();
    // A full refresh republishes the presenter's active pane, so this also
    // verifies backend-facing activation rather than only a local UI property.
    midnight_ui::refresh(&h.ui.window, &h.presenter.borrow());
    assert_eq!(h.ui.window.get_active_pane(), pane.index() as i32);
    assert_eq!(h.ui.window.get_focused_pane(), pane.index() as i32);

    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "i");
    selected(&h, &["folder", "root.txt", "z.txt"]);
    settle(&mut h, &["folder", "root.txt", "z.txt"]);
    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "i");
    selected(&h, &["z.txt"]);
    settle(&mut h, &["z.txt"]);

    ui::chord(&h.ui.window, &[Key::Control], "a");
    selected(&h, &["root.txt"]);
    settle(&mut h, &["root.txt"]);
    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "a");
    selected(&h, &["folder", "root.txt"]);
    settle(&mut h, &["folder", "root.txt"]);
    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "d");
    selected(&h, &["folder", "root.txt"]);
    settle(&mut h, &["folder", "root.txt"]);
    ui::chord(&h.ui.window, &[Key::Control], "d");
    selected(&h, &[]);
    settle(&mut h, &[]);
}

#[test]
fn native_left_tree_shortcuts_distinguish_files_and_all_entries() {
    exercise(PaneId::Left, false);
}

#[test]
fn native_right_tree_shortcuts_distinguish_files_and_all_entries() {
    exercise(PaneId::Right, false);
}

#[test]
fn archive_left_tree_shortcuts_distinguish_files_and_all_entries() {
    exercise(PaneId::Left, true);
}

#[test]
fn archive_right_tree_shortcuts_distinguish_files_and_all_entries() {
    exercise(PaneId::Right, true);
}

fn clear_nested_marks(pane: PaneId, archive: bool) {
    let mut h = Harness::with_sizes(pane, archive);
    h.ui.window.invoke_activate(pane.index() as i32);
    // Inline branches display folders only; a file-only child has no expansion arrow.
    assert!(h.view.as_ref().unwrap().rows.iter().any(|row| {
        row.entry.display_name == "large" && row.depth > 0 && row.entry.kind == EntryKind::Directory
    }));
    for name in ["folder", "large", "root.txt"] {
        let index = h.rows().iter().position(|row| row.name == name).unwrap();
        h.ui.window
            .invoke_row_click(pane.index() as i32, index as i32, true, false);
    }
    settle(&mut h, &["folder", "large", "root.txt"]);
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "d");
    selected(&h, &["folder", "large", "root.txt"]);
    settle(&mut h, &["folder", "large", "root.txt"]);
    ui::chord(&h.ui.window, &[Key::Control], "d");
    selected(&h, &[]);
    settle(&mut h, &[]);
    ui::chord(&h.ui.window, &[Key::Control], "a");
    selected(&h, &["root.txt"]);
    settle(&mut h, &["root.txt"]);
    ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "a");
    selected(&h, &["folder", "root.txt"]);
    settle(&mut h, &["folder", "root.txt"]);
}

#[test]
fn native_left_clear_removes_nested_folder_marks() {
    clear_nested_marks(PaneId::Left, false);
}

#[test]
fn native_right_clear_removes_nested_folder_marks() {
    clear_nested_marks(PaneId::Right, false);
}

#[test]
fn archive_left_clear_removes_nested_folder_marks() {
    clear_nested_marks(PaneId::Left, true);
}

#[test]
fn archive_right_clear_removes_nested_folder_marks() {
    clear_nested_marks(PaneId::Right, true);
}
