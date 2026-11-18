//! Shown names and paths escape explicit bidirectional controls, so a name cannot reorder
//! its displayed extension. Operations, the path editor's Enter and the clipboard keep the
//! raw names.
use crate::{FileListPresentation, OperationPresentation, test_harness::Harness};
use midnight_core::*;
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, io, path::PathBuf, sync::Arc};

/// Shown raw, the override control makes this name read "photoexe.jpg".
const SPOOF: &str = "photo\u{202e}gpj.exe";
const SHOWN: &str = "photo\\u202Egpj.exe";

fn folder() -> PathBuf {
    std::env::temp_dir().join("shown names")
}

fn escaped(text: &str) -> bool {
    !text.contains(is_explicit_bidi_control) && text.contains(SHOWN)
}

/// The texts the backend was asked to put on the clipboard.
fn copied(h: &Harness) -> Vec<String> {
    h.sent
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::CopyText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// An error that embeds the path of the spoofing name.
fn spoof_error(operation: &'static str) -> FsError {
    FsError::from_io(
        operation,
        &folder().join(SPOOF),
        io::Error::from(io::ErrorKind::PermissionDenied),
    )
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
fn jobs_rows_show_escapes_and_copy_details_keeps_the_raw_paths() {
    let mut h = Harness::new(UserPreferences::default());
    let destination = folder().join(SPOOF);
    h.presenter
        .borrow_mut()
        .copies
        .progress
        .push(Arc::new(CopyProgress::queued(
            OperationId(3),
            destination.clone(),
            1,
            TransferIntent::Copy,
        )));
    h.refresh();
    let row = h.window.get_operations().row_data(0).unwrap();
    assert!(escaped(&row.detail), "{}", row.detail);
    h.sent.borrow_mut().clear();
    h.window.invoke_copy_job_details(row.identity.clone());
    let raw = destination.display().to_string();
    assert!(
        matches!(copied(&h).as_slice(), [text] if text.contains(&raw)),
        "Copy details copies the raw destination"
    );
}

#[test]
fn a_file_list_error_shows_escapes_and_copies_the_raw_error() {
    let mut h = Harness::with_location(UserPreferences::default(), folder());
    let navigation = h
        .sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .expect("the left panel opens its folder");
    let entries: Arc<[Entry]> = [Entry::new(
        EntryId(1),
        folder().join(SPOOF),
        EntryKind::File,
    )]
    .into();
    h.presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    h.refresh();
    h.window.invoke_make_file_list(0);
    h.settle();
    let (id, generation) = {
        let p = h.presenter.borrow();
        let s = p
            .file_list
            .session
            .as_ref()
            .expect("Make File List is open");
        (s.id, s.generation)
    };
    h.presenter
        .borrow_mut()
        .apply(Event::FileList(Box::new(FileListEvent {
            id,
            generation,
            result: Err(spoof_error("write_file_list")),
        })));
    h.refresh();
    let raw = h
        .presenter
        .borrow()
        .file_list
        .session
        .as_ref()
        .unwrap()
        .error
        .clone();
    assert!(raw.contains(SPOOF), "{raw}");
    let form = h.window.global::<FileListPresentation>();
    let shown = form.get_error_text();
    assert!(escaped(&shown), "{shown}");

    // The notice's Copy passes the shown text; the error itself is copied.
    h.sent.borrow_mut().clear();
    form.invoke_copy_error(form.get_token(), shown);
    assert_eq!(copied(&h), [raw]);
    // Text that is not the current error copies nothing.
    h.sent.borrow_mut().clear();
    form.invoke_copy_error(form.get_token(), "An earlier error".into());
    assert!(copied(&h).is_empty());
}

#[test]
fn a_shown_panel_error_is_copied_and_inspected_as_the_raw_error() {
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
    // Show error details reports the error itself; `presenter::panel_warnings_tests` covers
    // an error with full details.
    p.inspect_panel_error(PaneId::Left, &shown, &|_| {
        panic!("an error without details reads nothing")
    });
    assert_eq!(
        p.desktop.notice.as_ref().map(|(_, body)| body.as_str()),
        Some(raw.as_str())
    );
}
