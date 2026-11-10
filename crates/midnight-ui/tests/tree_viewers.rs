//! Real tree publications and F3/Alt+F3 callbacks retain viewer source identities.
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use slint::platform::Key;
use std::{cell::RefCell, rc::Rc, sync::Arc};
use support::Harness;

fn focus(h: &Harness, name: &str) {
    let row = h.rows().iter().position(|row| row.name == name).unwrap();
    // A real file click first activates the panel through the application callback.
    h.ui.window.invoke_activate(h.pane.index() as i32);
    h.ui.window
        .invoke_row_click(h.pane.index() as i32, row as i32, false, false);
    assert_eq!(h.ui.window.get_active_pane(), h.pane.index() as i32);
    h.ui.window.invoke_focus_files();
}

fn exercise(pane: PaneId, archive: bool) {
    let mut h = Harness::new(pane, archive);
    let captured = Rc::new(RefCell::new(Vec::<(i32, bool, Option<ViewerLaunch>)>::new()));
    let output = captured.clone();
    let presenter = h.presenter.clone();
    // The application callback hands this same immutable launch to ViewerWindows.
    h.ui.window.on_view_file(move |pane, basic| {
        let id = if pane == 0 {
            PaneId::Left
        } else {
            PaneId::Right
        };
        output
            .borrow_mut()
            .push((pane, basic, presenter.borrow().viewer_launch(id, basic)));
    });

    for layout in [0, 1, 2] {
        h.ui.window.invoke_view_mode(pane.index() as i32, layout);
        focus(&h, "root.txt");
        h.until(|h| h.presenter.borrow().viewer_launch(pane, false).is_some());
        for basic in [false, true] {
            let before = captured.borrow().len();
            if basic {
                ui::chord(&h.ui.window, &[Key::Alt], Key::F3);
            } else {
                ui::key(&h.ui.window, Key::F3);
            }
            assert_eq!(captured.borrow().len(), before + 1);
            let launches = captured.borrow();
            let (actual_pane, actual_basic, launch) = launches.last().unwrap();
            assert_eq!(*actual_pane, pane.index() as i32);
            assert_eq!(*actual_basic, basic);
            let launch = launch.as_ref().expect("F3 should work in an adopted tree");
            assert_eq!(launch.basic, basic);
            let tree = launch.tree.as_ref().unwrap();
            let view = h.view.as_ref().unwrap();
            assert!(Arc::ptr_eq(&tree.view, view));
            assert!(Arc::ptr_eq(&launch.entries, &view.root.entries));
            assert!(Arc::ptr_eq(&launch.view, &view.root.index));
            let row = &view.rows[view.index.row_positions[&tree.focused]];
            assert_eq!(row.entry.display_name, "root.txt");
            assert_eq!(row.depth, 0);
            assert!(row.original_root_entry.is_some());
            if archive {
                let scope = launch.archive.as_ref().unwrap();
                assert_eq!(scope.focused, row.entry.virtual_node.as_ref().unwrap().id);
                assert_eq!(scope.mount.id(), 77);
                assert_eq!(scope.source_version, [1]);
                assert!(launch.focused.as_os_str().is_empty());
            } else {
                assert!(launch.archive.is_none());
                assert_eq!(launch.focused, row.entry.path);
            }
        }
    }

    // F3 must never turn a nested folder or the parent row into a content path.
    focus(&h, "a.bin");
    assert!(h.presenter.borrow().viewer_launch(pane, false).is_none());
    focus(&h, "..");
    assert!(h.presenter.borrow().viewer_launch(pane, false).is_none());

    let retained = captured.borrow().last().unwrap().2.clone().unwrap();
    h.calls
        .frozen
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let next = retained.entries[0]
        .path
        .parent()
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
        .join("new-location");
    h.ui.window.invoke_navigate(
        pane.index() as i32,
        next.to_string_lossy().into_owned().into(),
    );
    assert!(Arc::ptr_eq(
        &retained.entries,
        &retained.tree.as_ref().unwrap().view.root.entries
    ));
    assert_eq!(retained.archive.is_some(), archive);
}

#[test]
fn native_tree_f3_and_basic_viewer_keep_exact_root_identity_in_all_layouts() {
    exercise(PaneId::Left, false);
}

#[test]
fn archive_tree_f3_and_basic_viewer_keep_mount_identity_in_all_layouts() {
    exercise(PaneId::Right, true);
}
