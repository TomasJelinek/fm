//! Real F6 callbacks and queue against native files in fresh private fixtures.
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
#[path = "support/tree_watch.rs"]
mod watch;

use midnight_core::*;
use slint::{Model, platform::Key};
use std::{cell::RefCell, fs, rc::Rc, sync::Arc};

fn exercise(pane: PaneId) {
    let fixture = native::Fixture::new();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let mut h = support::Harness::with_native_transfer(
        pane,
        fixture.root.clone(),
        service.clone(),
        service,
    );
    h.ui.window.invoke_activate(pane.index() as i32);
    for name in ["large", "small", "empty"] {
        let row = h.rows().iter().position(|row| row.name == name).unwrap();
        h.ui.window
            .invoke_row_click(pane.index() as i32, row as i32, true, false);
    }
    h.until(|h| h.selection.as_ref().is_some_and(|s| s.len() == 3));
    h.ui.window.invoke_focus_files();
    ui::key(&h.ui.window, Key::F6);
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    assert!(!h.ui.window.get_copy_draft_metadata_enabled());
    ui::key(&h.ui.window, Key::Return);
    h.until(|h| {
        h.copy_progress
            .last()
            .is_some_and(|p| p.state.is_terminal())
    });
    let progress = h.copy_progress.last().unwrap();
    assert_eq!(progress.intent, TransferIntent::Move);
    assert_eq!(
        progress.state,
        CopyOperationState::Completed,
        "{progress:?}"
    );
    assert_eq!(progress.source_files_removed, 2);
    assert_eq!(progress.verified_bytes, 1012);
    for (name, byte, count) in [("large", 42, 1000), ("small", 43, 12)] {
        assert!(!fixture.root.join("folder").join(name).exists());
        assert_eq!(
            fs::read(
                fixture
                    .root
                    .join("destination")
                    .join(name)
                    .join("payload.bin")
            )
            .unwrap(),
            vec![byte; count]
        );
    }
    assert!(!fixture.root.join("folder/empty").exists());
    assert!(fixture.root.join("destination/empty").is_dir());
    assert!(fixture.root.join("destination/large/nested/empty").is_dir());
    assert_eq!(
        fs::read(fixture.root.join("root.txt")).unwrap(),
        b"unselected file"
    );
    assert_eq!(
        fs::read_dir(fixture.root.join("folder")).unwrap().count(),
        0,
        "no moved source or private staging sibling may remain"
    );
    h.until(|h| {
        !h.rows()
            .iter()
            .any(|row| ["large", "small", "empty"].contains(&row.name.as_str()))
    });
    assert!(!h.rows().iter().any(|row| row.selected));
}

#[test]
fn f6_moves_nested_native_directories_from_left_panel() {
    exercise(PaneId::Left);
}

#[test]
fn f6_moves_nested_native_directories_from_right_panel() {
    exercise(PaneId::Right);
}

#[test]
fn top_level_move_removes_the_row_and_preserves_the_expanded_tree_session() {
    let fixture = native::Fixture::new();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let mut h = support::Harness::with_native_transfer(
        PaneId::Left,
        fixture.root.clone(),
        service.clone(),
        service,
    );
    let session = h.view.as_ref().unwrap().stamp.session;
    let row = h
        .rows()
        .iter()
        .position(|row| row.name == "root.txt")
        .unwrap();
    h.ui.window.invoke_activate(0);
    h.ui.window.invoke_row_click(0, row as i32, true, false);
    h.until(|h| h.selection.as_ref().is_some_and(|s| s.len() == 1));
    h.ui.window.invoke_focus_files();
    ui::key(&h.ui.window, Key::F6);
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    ui::key(&h.ui.window, Key::Return);
    h.until(|h| {
        h.copy_progress
            .last()
            .is_some_and(|p| p.state.is_terminal())
    });
    assert_eq!(
        h.copy_progress.last().unwrap().state,
        CopyOperationState::Completed
    );
    assert!(!fixture.root.join("root.txt").exists());
    assert_eq!(
        fs::read(fixture.root.join("destination/root.txt")).unwrap(),
        b"unselected file"
    );
    h.until(|h| !h.rows().iter().any(|row| row.name == "root.txt"));
    assert_eq!(h.view.as_ref().unwrap().stamp.session, session);
    assert!(h.rows().iter().any(|row| row.name == "large"));
    assert!(h.rows().iter().any(|row| row.name == "small"));
}

#[test]
fn f6_refuses_an_archive_tree_without_starting_a_transfer() {
    let mut h = support::Harness::with_sizes(PaneId::Right, true);
    h.ui.window.invoke_activate(1);
    let row = h.rows().iter().position(|row| row.name == "small").unwrap();
    h.ui.window.invoke_row_click(1, row as i32, false, false);
    h.ui.window.invoke_focus_files();
    ui::key(&h.ui.window, Key::F6);
    h.until(|h| h.ui.window.get_message_title() == "Move unavailable");
    assert!(h.ui.window.get_copy_draft_token().is_empty());
    assert!(h.copy_progress.is_empty());
    assert!(h.rows().iter().any(|row| row.name == "small"));
}

fn root_transfer_updates_both_trees(intent: TransferIntent) {
    let fixture = native::Fixture::new();
    fs::create_dir_all(fixture.root.join("destination/existing/retained")).unwrap();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let watch = Arc::new(watch::ObservedWatch::new(service.clone()));
    let mut h = support::Harness::with_native_watch_transfer(
        PaneId::Left,
        fixture.root.clone(),
        service.clone(),
        service,
        watch.clone(),
    );
    let source_session = h.view.as_ref().unwrap().stamp.session;
    let existing =
        h.ui.window
            .get_right_rows()
            .iter()
            .find(|row| row.name == "existing")
            .unwrap()
            .identity;
    h.ui.window.invoke_tree_disclosure(1, existing);
    h.until(|h| {
        h.views[1].as_ref().is_some_and(|view| {
            view.rows
                .iter()
                .any(|row| row.entry.display_name == "retained")
        })
    });
    h.until(|_| watch.observes(&fixture.root.join("destination/existing")));
    let destination = h.views[1].as_ref().unwrap().clone();
    let retained =
        h.ui.window
            .get_right_rows()
            .iter()
            .position(|row| row.name == "retained")
            .unwrap();
    h.ui.window
        .invoke_row_click(1, retained as i32, true, false);
    h.until(|h| {
        h.ui.window
            .get_right_rows()
            .iter()
            .any(|row| row.name == "retained" && row.selected)
    });

    let source = h
        .rows()
        .iter()
        .position(|row| row.name == "root.txt")
        .unwrap();
    h.ui.window.invoke_activate(0);
    h.ui.window.invoke_row_click(0, source as i32, true, false);
    h.until(|h| {
        h.selection
            .as_ref()
            .is_some_and(|selection| selection.len() == 1)
    });
    h.ui.window.invoke_focus_files();
    let callbacks = watch.callbacks();
    ui::key(
        &h.ui.window,
        if intent == TransferIntent::Move {
            Key::F6
        } else {
            Key::F5
        },
    );
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    ui::key(&h.ui.window, Key::Return);
    h.until(|h| {
        h.copy_progress
            .last()
            .is_some_and(|progress| progress.state.is_terminal())
    });
    let progress = h.copy_progress.last().unwrap();
    assert_eq!(progress.intent, intent);
    assert_eq!(
        progress.state,
        CopyOperationState::Completed,
        "{progress:?}"
    );
    assert_eq!(
        fs::read(fixture.root.join("destination/root.txt")).unwrap(),
        b"unselected file"
    );
    assert_eq!(
        fixture.root.join("root.txt").exists(),
        intent == TransferIntent::Copy
    );
    h.until(|h| {
        let source = h.rows();
        source.iter().any(|row| row.name == "root.txt") == (intent == TransferIntent::Copy)
            && !source.iter().any(|row| row.selected)
            && watch.callbacks() > callbacks
            && h.ui
                .window
                .get_right_rows()
                .iter()
                .any(|row| row.name == "root.txt")
    });
    // Terminal maintenance may acknowledge own-write events before debounce,
    // correctly suppressing a redundant hint. A later external change proves
    // observation still works after those callbacks and preserves both trees.
    fs::create_dir(fixture.root.join("destination/after-transfer")).unwrap();
    h.until(|h| {
        h.ui.window
            .get_right_rows()
            .iter()
            .any(|row| row.name == "after-transfer")
    });
    assert_eq!(h.view.as_ref().unwrap().stamp.session, source_session);
    assert!(h.rows().iter().any(|row| row.name == "large"));
    let updated = h.views[1].as_ref().unwrap();
    assert_eq!(updated.stamp.session, destination.stamp.session);
    assert!(Arc::ptr_eq(
        &updated.root.entries,
        &destination.root.entries
    ));
    assert!(!Arc::ptr_eq(
        &updated.root_actions.entries,
        &destination.root.entries
    ));
    assert!(
        h.ui.window
            .get_right_rows()
            .iter()
            .any(|row| row.name == "retained" && row.selected)
    );

    let captured = Rc::new(RefCell::new(None));
    let output = captured.clone();
    let presenter = h.presenter.clone();
    h.ui.window.on_view_file(move |pane, basic| {
        assert_eq!(pane, 1);
        *output.borrow_mut() = presenter.borrow().viewer_launch(PaneId::Right, basic);
    });
    let new_file =
        h.ui.window
            .get_right_rows()
            .iter()
            .position(|row| row.name == "root.txt")
            .unwrap();
    h.ui.window.invoke_activate(1);
    h.ui.window
        .invoke_row_click(1, new_file as i32, false, false);
    h.ui.window.invoke_focus_files();
    h.until(|h| {
        h.presenter
            .borrow()
            .viewer_launch(PaneId::Right, false)
            .is_some()
    });
    ui::key(&h.ui.window, Key::F3);
    let launch = captured.borrow();
    let launch = launch
        .as_ref()
        .expect("F3 must open a newly transferred root file");
    assert_eq!(launch.focused, fixture.root.join("destination/root.txt"));
    assert!(Arc::ptr_eq(
        &launch.entries,
        &h.views[1].as_ref().unwrap().root_actions.entries
    ));
}

#[test]
fn native_copy_updates_destination_tree_and_new_file_viewer_without_losing_marks() {
    root_transfer_updates_both_trees(TransferIntent::Copy);
}

#[test]
fn native_move_updates_both_trees_and_new_file_viewer_without_losing_marks() {
    root_transfer_updates_both_trees(TransferIntent::Move);
}
