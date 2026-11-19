//! Production watch routing keeps native tree sessions alive through external changes.
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
use crate::support::ui;
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
        false,
    );
    let branch = fixture.root.join("folder");
    // A live subscription must cover both directories (a historical batch that was
    // since replaced is not enough).
    h.until(|_| watch.live_covers(&fixture.root) && watch.live_covers(&branch));
    let session = h.view.as_ref().unwrap().stamp.session;
    let small = h.rows().iter().position(|row| row.name == "small").unwrap();
    h.ui.window
        .invoke_row_click(pane.index() as i32, small as i32, true, false);
    h.until(|h| {
        h.selection
            .as_ref()
            .is_some_and(|selection| selection.len() == 1)
    });
    // Routed hints are informational: the debounced hint of the fence's own
    // registration can arrive at any point, and hints legitimately coalesce. The
    // per-directory native evidence plus the UI row is what each change must show.
    for (parent, name) in [
        (&fixture.root, "new-root-文档"),
        (&branch, "new-child-žluťoučký"),
    ] {
        let mark = watch.mark();
        let hints = h.watch_hints[pane.index()];
        let directory = parent.join(name);
        fs::create_dir(&directory).unwrap();
        if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            h.until(|h| {
                watch.native_evidence_for(mark, parent)
                    && h.rows().iter().any(|row| row.name == name)
            });
        })) {
            let current_hints = h.watch_hints[pane.index()];
            let ui_row = h.rows().iter().any(|row| row.name == name);
            let projection_row = h.views[pane.index()]
                .as_ref()
                .is_some_and(|view| view.rows.iter().any(|row| row.entry.display_name == name));
            eprintln!(
                "tree watch creation predicates: pane={pane:?}, target={name:?}, parent={parent:?}, mark={mark:?}, native_evidence_for_parent_required={}, hints={current_hints}>{hints} (informational, coalescing allowed), ui_row_required={ui_row}, observed_projection_row={projection_row}, live(root,branch)={:?}, {}",
                watch.native_evidence_for(mark, parent),
                (watch.live_covers(&fixture.root), watch.live_covers(&branch)),
                watch.describe(),
            );
            std::panic::resume_unwind(panic);
        }
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
