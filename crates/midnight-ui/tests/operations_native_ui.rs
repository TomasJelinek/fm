//! Owned native files through installed UI events, presenter and production Engine.
#[path = "support/operations_native.rs"]
mod native;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::fs;

fn confirm_copy(h: &mut native::Harness, after: usize) -> std::sync::Arc<CopyProgress> {
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    ui::key(&h.ui.window, Key::Return);
    let progress = h.copy_finished(after);
    assert_eq!(
        progress.state,
        CopyOperationState::Completed,
        "{progress:?}"
    );
    h.until(|h| h.ui.window.get_copy_draft_token().is_empty());
    progress
}

#[test]
fn real_f5_multiselection_toolbar_copy_and_f6_move_preserve_bytes_and_clear_selection() {
    let mut h = native::Harness::new();
    for name in ["file-00.bin", "file-01.bin"] {
        h.focus(PaneId::Left, name, true);
    }
    h.until(|h| h.rows(PaneId::Left).iter().filter(|r| r.selected).count() == 2);
    let start = h.copies.len();
    ui::key(&h.ui.window, Key::F5);
    let result = confirm_copy(&mut h, start);
    assert_eq!(result.files_completed, 2);
    assert_eq!(result.verified_bytes, 8193);
    for name in ["file-00.bin", "file-01.bin"] {
        assert_eq!(
            fs::read(h.left.join(name)).unwrap(),
            fs::read(h.right.join(name)).unwrap()
        );
    }
    h.until(|h| h.rows(PaneId::Left).iter().all(|r| !r.selected));

    h.focus(PaneId::Left, "file-02.bin", false);
    let start = h.copies.len();
    ui::click_root(&h.ui.window, "F5 Copy", AccessibleRole::Button);
    confirm_copy(&mut h, start);
    assert_eq!(
        fs::read(h.right.join("file-02.bin")).unwrap(),
        vec![2; 4098]
    );

    h.focus(PaneId::Left, "část 文档 🦀.txt", false);
    ui::key(&h.ui.window, Key::F6);
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    assert!(h.ui.window.get_copy_draft_move());
    ui::key(&h.ui.window, Key::Escape);
    h.until(|h| h.ui.window.get_copy_draft_token().is_empty());
    assert!(h.left.join("část 文档 🦀.txt").exists());
    let start = h.copies.len();
    h.ui.window.invoke_focus_files();
    ui::key(&h.ui.window, Key::F6);
    let result = confirm_copy(&mut h, start);
    assert_eq!(result.intent, TransferIntent::Move);
    assert_eq!(result.native_entries_moved, 1);
    assert_eq!((result.copied_bytes, result.verified_bytes), (0, 0));
    assert!(!h.left.join("část 文档 🦀.txt").exists());
    assert_eq!(
        fs::read(h.right.join("část 文档 🦀.txt")).unwrap(),
        b"Owned Unicode source"
    );
    h.assert_sentinel();
}

#[test]
fn real_delete_confirmation_cancel_and_shift_f8_permanently_delete_only_the_selected_tree() {
    let mut h = native::Harness::new();
    h.focus(PaneId::Left, "nested", false);
    ui::key(&h.ui.window, Key::Delete);
    h.until(|h| !h.ui.window.get_operation_token().is_empty());
    assert!(!h.ui.window.get_operation_dangerous());
    assert!(h.ui.window.get_operation_body().contains("nested"));
    ui::key(&h.ui.window, Key::Escape);
    h.until(|h| h.ui.window.get_operation_token().is_empty());
    assert!(h.left.join("nested/child.txt").exists());
    h.ui.window.invoke_focus_files();
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
    ui::key(&h.ui.window, Key::F8);
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
    h.until(|h| !h.ui.window.get_operation_token().is_empty());
    assert!(h.ui.window.get_operation_dangerous());
    let start = h.operations.len();
    ui::key(&h.ui.window, Key::Return);
    h.until(|h| {
        h.operations
            .iter()
            .skip(start)
            .any(|p| p.state.is_terminal())
    });
    let result = h
        .operations
        .iter()
        .skip(start)
        .find(|p| p.state.is_terminal())
        .unwrap();
    assert_eq!(result.state, OperationState::Completed, "{result:?}");
    assert_eq!(result.kind, MutationKind::Delete(DeleteMode::Permanent));
    assert!(!h.left.join("nested").exists());
    assert!(h.left.join("file-00.bin").exists());
    assert!(h.left.join("inside").is_dir());
    h.assert_sentinel();
}

fn drag(
    h: &mut native::Harness,
    source: &str,
    target: &str,
    control: bool,
    cancel: bool,
) -> DragId {
    h.focus(PaneId::Left, source, false);
    let from = ui::root_position(&h.ui.window, source, AccessibleRole::ListItem);
    let to = ui::root_position(&h.ui.window, target, AccessibleRole::ListItem);
    let prepared = h.prepared_drags;
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position: from,
            button: PointerEventButton::Left,
        });
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(from.x + 16., from.y),
        });
    h.until(|h| h.prepared_drags > prepared);
    let id = h.last_prepared_drag.unwrap();
    if control {
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
    }
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: to });
    h.settle();
    if cancel {
        ui::key(&h.ui.window, Key::Escape);
    }
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: to,
            button: PointerEventButton::Left,
        });
    if control {
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
    }
    id
}

#[test]
fn real_folder_drag_uses_same_volume_move_ctrl_copy_and_escape_cancellation() {
    let mut h = native::Harness::new();
    let start = h.copies.len();
    drag(&mut h, "file-03.bin", "inside", false, false);
    let moved = h.copy_finished(start);
    assert_eq!(moved.state, CopyOperationState::Completed, "{moved:?}");
    assert_eq!(moved.intent, TransferIntent::Move);
    assert!(!h.left.join("file-03.bin").exists());
    assert_eq!(
        fs::read(h.left.join("inside/file-03.bin")).unwrap(),
        vec![3; 4099]
    );

    let start = h.copies.len();
    drag(&mut h, "file-04.bin", "target folder", true, false);
    let copied = h.copy_finished(start);
    assert_eq!(copied.state, CopyOperationState::Completed, "{copied:?}");
    assert_eq!(copied.intent, TransferIntent::Copy);
    assert_eq!(copied.verified_bytes, 4100);
    assert!(h.left.join("file-04.bin").exists());
    assert_eq!(
        fs::read(h.right.join("target folder/file-04.bin")).unwrap(),
        vec![4; 4100]
    );

    let start = h.copies.len();
    let drops = h.internal_drops.get();
    let id = drag(&mut h, "file-05.bin", "inside", false, true);
    h.until(|h| h.finished_drags.contains(&(id, DragFinish::Cancelled)));
    assert_eq!(h.internal_drops.get(), drops);
    h.shutdown();
    assert!(h.left.join("file-05.bin").exists());
    assert!(!h.left.join("inside/file-05.bin").exists());
    assert_eq!(h.copies.len(), start);
    h.assert_sentinel();
}
