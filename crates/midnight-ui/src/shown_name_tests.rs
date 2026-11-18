//! Shown names and paths escape explicit bidirectional controls, so a name cannot reorder
//! its displayed extension. Operations, the path editor's Enter and the clipboard keep the
//! raw names.
use crate::{OperationPresentation, test_harness::Harness};
use midnight_core::*;
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, path::PathBuf, sync::Arc};

/// Shown raw, the override control makes this name read "photoexe.jpg".
const SPOOF: &str = "photo\u{202e}gpj.exe";
const SHOWN: &str = "photo\\u202Egpj.exe";

fn folder() -> PathBuf {
    std::env::temp_dir().join("shown names")
}

fn escaped(text: &str) -> bool {
    !text.contains(is_explicit_bidi_control) && text.contains(SHOWN)
}

#[test]
fn tabs_and_the_path_bar_show_escapes_and_enter_reopens_the_raw_folder() {
    let location = folder().join(SPOOF);
    let h = Harness::with_location(UserPreferences::default(), location.clone());
    let tab = h.window.get_left_tabs().row_data(0).unwrap();
    assert_eq!(tab.label, SHOWN);
    assert!(escaped(&tab.path), "{}", tab.path);
    let path = h.window.get_left_path();
    assert!(escaped(&path), "{path}");
    let parts = h.window.get_left_path_parts();
    let last = parts.row_data(parts.row_count() - 1).unwrap();
    assert!(
        escaped(&last.label) && last.label.ends_with(SHOWN),
        "{}",
        last.label
    );
    assert!(escaped(&last.prefix), "{}", last.prefix);

    // The editor starts from the shown path; Enter on it reopens the folder it names.
    h.sent.borrow_mut().clear();
    h.window.invoke_navigate(0, path);
    assert!(
        h.sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Navigate { path, .. } if *path == location)),
        "the raw folder is reopened"
    );
}

fn preview(id: u64, kind: MutationKind) -> Arc<OperationPreview> {
    Arc::new(OperationPreview {
        id: OperationId(id),
        kind,
        files: 1,
        folders: 0,
        file_bytes: Some(3),
        sample: vec![MutationItem {
            path: folder().join(SPOOF),
            kind: EntryKind::File,
            metadata: EntryMetadata::default(),
        }],
    })
}

#[test]
fn a_delete_confirmation_shows_the_escaped_name() {
    let mut h = Harness::with_location(UserPreferences::default(), folder());
    {
        let mut p = h.presenter.borrow_mut();
        p.operations.pending = Some(OperationId(7));
        p.operations.preview = Some(preview(7, MutationKind::Delete(DeleteMode::Trash)));
    }
    h.refresh();
    let dialog = h.window.global::<OperationPresentation>();
    assert!(dialog.get_active());
    let body = dialog.get_body();
    assert!(body.contains(&format!("Name: {SHOWN}")), "{body}");
    assert!(escaped(&body), "{body}");
}

#[test]
fn a_rename_shows_the_escaped_path_and_edits_the_raw_name() {
    let mut h = Harness::with_location(UserPreferences::default(), folder());
    {
        let mut p = h.presenter.borrow_mut();
        p.operations.pending = Some(OperationId(8));
        p.operations.preview = Some(preview(8, MutationKind::Rename));
    }
    h.refresh();
    let body = h.window.get_operation_body();
    assert!(
        body.starts_with("Current item\n") && escaped(&body),
        "{body}"
    );
    // The name field holds what the rename receives.
    assert_eq!(h.window.get_operation_name(), SPOOF);
}

#[test]
fn notices_show_escapes_and_copy_details_keeps_the_raw_text() {
    let mut h = Harness::new(UserPreferences::default());
    let path = folder().join(SPOOF);
    let raw = format!("Could not open {}", path.display());
    h.presenter
        .borrow_mut()
        .desktop
        .report("Open failed", raw.clone());
    h.refresh();
    let body = h.window.get_message_body();
    assert!(escaped(&body), "{body}");
    h.sent.borrow_mut().clear();
    h.window.invoke_copy_message();
    assert!(
        h.sent.borrow().iter().any(
            |command| matches!(command, Command::CopyText { text, .. } if text.contains(&raw))
        ),
        "Copy details copies the raw text"
    );
}

#[test]
fn a_shown_panel_error_is_copied_as_the_raw_error() {
    let mut p = crate::Presenter::new(folder(), folder());
    let raw = format!("Access is denied. ({})", folder().join(SPOOF).display());
    p.panes[0].error = raw.clone();
    let shown = crate::shown_text(&raw);
    assert!(escaped(&shown), "{shown}");
    let sent = RefCell::new(Vec::new());
    p.copy_panel_error(PaneId::Left, &shown, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        matches!(sent.borrow().as_slice(), [Command::CopyText { text, .. }] if *text == raw),
        "the error itself is copied"
    );
}
