//! Owned native files through installed UI events, presenter and production Engine.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/operations_native.rs"]
mod native;
use crate::support::ui;
#[cfg(windows)]
#[path = "operations_native_ui/windows_delete.rs"]
mod windows_delete;
#[cfg(windows)]
#[path = "../../midnight-engine/tests/support/checksum_verify_links_windows.rs"]
mod windows_links;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::fs;

fn confirm_copy(h: &mut native::Harness, after: usize) -> std::sync::Arc<CopyProgress> {
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    input::key(&h.dialog(), Key::Return);
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
    input::key(&h.dialog(), Key::Escape);
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

#[cfg(windows)]
#[test]
fn real_copy_follows_a_junction_by_default() {
    copy_junction(true);
}

#[cfg(windows)]
#[test]
fn real_copy_without_following_creates_an_empty_directory() {
    copy_junction(false);
}

#[cfg(windows)]
fn copy_junction(follow: bool) {
    use i_slint_core::accessibility::AccessibleStringProperty;
    use std::{os::windows::fs::MetadataExt, path::Path};

    const PAYLOAD: &[u8] = b"Owned junction payload";
    const REPARSE_POINT: u32 = 0x400;
    let mut h = native::Harness::new();
    let target = h.root.join("referent");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("payload.bin"), PAYLOAD).unwrap();
    let junction = h.left.join("inside/junction");
    // The shared cmd.exe junction fixture takes ordinary local drive paths.
    let ordinary = |path: &Path| {
        let path = path
            .to_str()
            .expect("the authored fixture paths are Unicode");
        std::path::PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(path))
    };
    windows_links::create(&ordinary(&junction), &ordinary(&target));

    h.focus(PaneId::Left, "inside", false);
    let start = h.copies.len();
    ui::key(&h.ui.window, Key::F5);
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    h.dialog();
    let links = ui::popup_item(
        &h.ui.window,
        "Follow directory links",
        AccessibleRole::Checkbox,
    );
    assert_eq!(
        links
            .accessible_string_property(AccessibleStringProperty::Checked)
            .as_deref(),
        Some("true")
    );
    if !follow {
        ui::click_popup(
            &h.ui.window,
            "Follow directory links",
            AccessibleRole::Checkbox,
        );
        assert_eq!(
            links
                .accessible_string_property(AccessibleStringProperty::Checked)
                .as_deref(),
            Some("false")
        );
        ui::click_popup(
            &h.ui.window,
            "Destination folder",
            AccessibleRole::TextInput,
        );
    }

    let result = confirm_copy(&mut h, start);
    let destination = h.right.join("inside/junction");
    let metadata = fs::symlink_metadata(&destination).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.file_attributes() & REPARSE_POINT, 0);
    assert_eq!(result.completed_roots, 1);
    assert_eq!(result.files_completed, u64::from(follow));
    assert_eq!(
        result.verified_bytes,
        if follow { PAYLOAD.len() as u64 } else { 0 }
    );
    assert_eq!(
        fs::read_dir(&destination).unwrap().count(),
        usize::from(follow)
    );
    if follow {
        assert_eq!(fs::read(destination.join("payload.bin")).unwrap(), PAYLOAD);
    }
    assert_eq!(fs::read(junction.join("payload.bin")).unwrap(), PAYLOAD);
    fs::remove_dir(&junction).unwrap();
    assert_eq!(fs::read(target.join("payload.bin")).unwrap(), PAYLOAD);
    h.assert_sentinel();
}

#[test]
fn real_copy_conflict_shows_both_entries_in_the_native_locale() {
    let mut h = native::Harness::new();
    fs::write(h.right.join("file-03.bin"), "existing destination").unwrap();
    h.focus(PaneId::Left, "file-03.bin", false);
    let start = h.copies.len();
    ui::key(&h.ui.window, Key::F5);
    h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
    input::key(&h.dialog(), Key::Return);
    h.until(|h| h.ui.window.get_copy_token().starts_with("copy:"));
    // Production installs the audited locale, so Copy/Move conflicts format both
    // entries natively instead of the UI's labelled UTC fallback.
    for details in [
        h.ui.window.get_copy_source_details(),
        h.ui.window.get_copy_target_details(),
    ] {
        let modified = details
            .lines()
            .find(|line| line.starts_with("Modified: "))
            .unwrap_or_else(|| panic!("no modification time: {details}"));
        assert!(!modified.contains("UTC"), "{details}");
    }
    h.click_decision("Skip");
    let progress = h.copy_finished(start);
    assert!(progress.state.is_terminal(), "{progress:?}");
    assert_eq!(
        fs::read(h.right.join("file-03.bin")).unwrap(),
        b"existing destination"
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
    let cancelled = h.ui.window.get_operation_token().parse::<u64>().unwrap();
    input::key(&h.dialog(), Key::Escape);
    // Cancellation is asynchronous and its terminal reply refreshes the pane.
    // Start the second operation only after that exact cancellation and refresh.
    h.until(|h| {
        h.operations.iter().any(|progress| {
            progress.id == OperationId(cancelled) && progress.state == OperationState::Cancelled
        }) && h.ui.window.get_operation_token().is_empty()
            && !h.ui.window.get_left_loading()
    });
    assert!(!h.ui.window.get_dialog_input_blocked());
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.left.join("nested/child.txt").exists());
    h.focus(PaneId::Left, "nested", false);
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
    input::key(&h.dialog(), Key::Return);
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
    // The completion re-reads the panel and holds input only until it shows.
    h.until(|h| !h.rows(PaneId::Left).iter().any(|row| row.name == "nested"));
    assert!(!h.ui.window.get_mutation_input_blocked());
    assert!(h.left.join("file-00.bin").exists());
    assert!(h.left.join("inside").is_dir());
    h.assert_sentinel();
}

#[test]
fn real_inline_delete_uses_confirmation_and_reconciles_only_the_selected_branch() {
    let mut h = native::Harness::new();
    let deleted = h.left.join("nested/empty");
    fs::create_dir(h.left.join("nested/retained")).unwrap();
    fs::create_dir(deleted.join("část 文档")).unwrap();
    fs::write(
        deleted.join("část 文档/payload.txt"),
        b"Owned branch payload",
    )
    .unwrap();
    h.until(|h| {
        h.rows(PaneId::Left)
            .iter()
            .any(|row| row.name == "nested" && matches!(row.tree_disclosure, 1 | 3))
    });
    let parent = h
        .rows(PaneId::Left)
        .iter()
        .find(|row| row.name == "nested")
        .unwrap()
        .identity
        .clone();
    h.ui.window.invoke_tree_disclosure(0, parent);
    h.until(|h| {
        h.rows(PaneId::Left)
            .iter()
            .any(|row| row.name == "empty" && row.tree_depth == 1)
    });
    h.focus(PaneId::Left, "empty", false);
    // Exercise the actual key dispatcher and normal confirmation. The raw target
    // must be the nested occurrence, not source/empty or the expanded parent.
    ui::chord(&h.ui.window, &[Key::Shift], Key::F8);
    h.until(|h| !h.ui.window.get_operation_token().is_empty());
    assert!(h.ui.window.get_operation_dangerous());
    assert!(h.ui.window.get_operation_body().contains("empty"));
    assert!(deleted.join("část 文档/payload.txt").is_file());
    let token = OperationId(h.ui.window.get_operation_token().parse().unwrap());
    input::key(&h.dialog(), Key::Return);
    h.until(|h| {
        h.operations
            .iter()
            .any(|p| p.id == token && p.state.is_terminal())
    });
    let result = h
        .operations
        .iter()
        .find(|p| p.id == token && p.state.is_terminal())
        .unwrap();
    assert_eq!(result.state, OperationState::Completed, "{result:?}");
    assert_eq!(result.kind, MutationKind::Delete(DeleteMode::Permanent));
    assert!(!deleted.exists());
    assert_eq!(
        fs::read(h.left.join("nested/child.txt")).unwrap(),
        b"Owned nested source"
    );
    assert!(h.left.join("file-00.bin").is_file());
    assert!(h.left.join("inside").is_dir());
    h.until(|h| {
        let rows = h.rows(PaneId::Left);
        !rows.iter().any(|row| row.name == "empty")
            && rows
                .iter()
                .any(|row| row.name == "nested" && row.tree_expanded)
            && rows
                .iter()
                .any(|row| row.name == "retained" && row.tree_depth == 1)
    });
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

/// F2 on `name`, type `stem` over the selected basename and press Enter.
fn rename_with_f2(h: &mut native::Harness, name: &str, stem: &str) {
    h.focus(PaneId::Left, name, false);
    ui::key(&h.ui.window, Key::F2);
    h.until(|h| !h.ui.window.get_operation_token().is_empty());
    input::key(&h.dialog(), stem);
    input::key(&h.dialog(), Key::Return);
}

#[test]
fn real_f2_rename_shows_the_new_name_and_holds_panel_input_until_then() {
    let mut h = native::Harness::new();
    let operations = h.operations.len();
    rename_with_f2(&mut h, "file-05.bin", "renamed-05");
    // No Engine event has been applied yet: the panel still lists the old name.
    assert!(
        h.ui.window.get_operation_token().is_empty(),
        "the dialog closed"
    );
    assert!(h.ui.window.get_mutation_input_blocked());
    assert!(
        h.rows(PaneId::Left)
            .iter()
            .any(|row| row.name == "file-05.bin")
    );
    let cursor = h.cursor(PaneId::Left);
    ui::key(&h.ui.window, Key::DownArrow);
    assert_eq!(
        h.cursor(PaneId::Left),
        cursor,
        "panel keys wait for the new name"
    );

    // Completion alone re-reads the folder; no watch notification is needed.
    h.until(|h| {
        h.rows(PaneId::Left)
            .iter()
            .any(|row| row.name == "renamed-05.bin")
    });
    assert!(!h.ui.window.get_mutation_input_blocked());
    assert!(
        h.operations[operations..]
            .iter()
            .any(|progress| progress.state == OperationState::Completed)
    );
    let rows = h.rows(PaneId::Left);
    assert!(!rows.iter().any(|row| row.name == "file-05.bin"));
    let cursor = usize::try_from(h.cursor(PaneId::Left)).unwrap();
    assert_eq!(rows[cursor].name, "renamed-05.bin");
    assert!(h.left.join("renamed-05.bin").exists());
    ui::key(&h.ui.window, Key::DownArrow);
    assert_eq!(
        usize::try_from(h.cursor(PaneId::Left)).unwrap(),
        cursor + 1,
        "input is accepted again"
    );
    h.assert_sentinel();
}

#[test]
fn real_failed_f2_rename_reports_the_error_without_holding_input() {
    let mut h = native::Harness::new();
    let operations = h.operations.len();
    // A folder never replaces the existing folder `inside`.
    rename_with_f2(&mut h, "nested", "inside");
    h.until(|h| {
        h.operations[operations..]
            .iter()
            .any(|progress| progress.state.is_terminal())
    });
    h.settle();
    let done = h.operations[operations..]
        .iter()
        .find(|progress| progress.state.is_terminal())
        .unwrap();
    assert_eq!(done.state, OperationState::Failed, "{done:?}");
    assert!(!h.ui.window.get_mutation_input_blocked());
    assert!(
        !h.ui.window.get_message_title().is_empty(),
        "the error is shown"
    );
    assert!(h.left.join("nested/child.txt").exists());
    h.assert_sentinel();
}
