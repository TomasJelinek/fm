//! Production watch routing keeps native tree sessions alive through external changes.
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
#[path = "support/tree_watch.rs"]
mod watch;

use midnight_core::*;
use std::{fs, sync::Arc};
use watch::ObservedWatch;

fn exercise_external_changes(pane: PaneId) {
    let fixture = native::Fixture::new();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let watch = Arc::new(ObservedWatch::new(service.clone()));
    let mut h = support::Harness::with_native_watch_transfer(
        pane,
        fixture.root.clone(),
        service.clone(),
        service,
        watch.clone(),
    );
    let branch = fixture.root.join("folder");
    h.until(|_| watch.observes(&branch));
    let session = h.view.as_ref().unwrap().stamp.session;
    let small = h.rows().iter().position(|row| row.name == "small").unwrap();
    h.ui.window
        .invoke_row_click(pane.index() as i32, small as i32, true, false);
    h.until(|h| {
        h.selection
            .as_ref()
            .is_some_and(|selection| selection.len() == 1)
    });
    for (parent, name) in [
        (&fixture.root, "new-root-文档"),
        (&branch, "new-child-žluťoučký"),
    ] {
        let before = watch.callbacks();
        let hints = h.watch_hints[pane.index()];
        let directory = parent.join(name);
        fs::create_dir(&directory).unwrap();
        h.until(|h| {
            watch.callbacks() > before
                && h.watch_hints[pane.index()] > hints
                && h.rows().iter().any(|row| row.name == name)
        });
        assert_eq!(h.view.as_ref().unwrap().stamp.session, session);
        assert!(
            h.rows()
                .iter()
                .any(|row| row.name == "small" && row.selected && row.cursor)
        );
        fs::remove_dir(&directory).unwrap();
        h.until(|h| !h.rows().iter().any(|row| row.name == name));
        assert_eq!(h.view.as_ref().unwrap().stamp.session, session);
        assert!(
            h.rows()
                .iter()
                .any(|row| row.name == "small" && row.selected)
        );
    }
    assert!(h.ui.window.get_left_error().is_empty());
    assert!(h.ui.window.get_right_error().is_empty());
}

#[test]
fn external_root_and_expanded_branch_changes_preserve_left_tree() {
    exercise_external_changes(PaneId::Left);
}

#[test]
fn external_root_and_expanded_branch_changes_preserve_right_tree() {
    exercise_external_changes(PaneId::Right);
}
