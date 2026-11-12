//! F5 through real Slint callbacks, tree actor and verified copy queue.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/tree_copy.rs"]
mod transfer;
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use slint::platform::Key;
use std::sync::Arc;
use support::Harness;

fn exercise(pane: PaneId, archive: bool) {
    let destination = Arc::new(transfer::Destination::default());
    let mut h = Harness::with_transfer(
        pane,
        archive,
        Arc::new(transfer::Transfer(destination.clone())),
    );
    h.ui.window.invoke_activate(pane.index() as i32);
    for name in ["large", "small"] {
        let row = h.rows().iter().position(|row| row.name == name).unwrap();
        h.ui.window
            .invoke_row_click(pane.index() as i32, row as i32, true, false);
    }
    h.until(|h| {
        h.selection
            .as_ref()
            .is_some_and(|selection| selection.len() == 2)
    });
    h.ui.window.invoke_focus_files();
    ui::key(&h.ui.window, Key::F5);
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    assert!(!h.ui.window.get_copy_draft_metadata_enabled());
    assert!(
        h.ui.window
            .get_copy_draft_body()
            .contains("2 selected items")
    );
    input::key(&h.dialog(), Key::Return);
    h.until(|h| {
        h.copy_progress
            .last()
            .is_some_and(|progress| progress.state.is_terminal())
    });
    let progress = h.copy_progress.last().unwrap();
    assert_eq!(
        progress.state,
        CopyOperationState::Completed,
        "{progress:?}"
    );
    assert_eq!(progress.files_completed, 2);
    h.until(|h| {
        h.selection
            .as_ref()
            .is_some_and(|selection| selection.is_empty())
    });
    let files = destination.files.lock().unwrap();
    assert_eq!(files.len(), 2);
    for (name, size) in [("large", 1000), ("small", 12)] {
        let bytes = files
            .iter()
            .find(|(path, _)| path.parent().unwrap().file_name().unwrap() == name)
            .unwrap()
            .1;
        assert_eq!(bytes, &vec![42; size]);
    }
    assert_eq!(
        *destination.readback.lock().unwrap(),
        1012,
        "Every saved byte must be read back for verification"
    );
    assert!(!h.rows().iter().any(|row| row.selected));
}

#[test]
fn native_nested_folders_copy_and_deselect_through_f5() {
    exercise(PaneId::Left, false);
}

#[test]
fn archive_nested_folders_copy_and_deselect_through_f5() {
    exercise(PaneId::Right, true);
}
