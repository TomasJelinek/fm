//! F5 through real Slint callbacks, tree actor and verified copy queue.
#[path = "support/window_input.rs"]
mod input;
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/tree_copy.rs"]
mod transfer;
use crate::support::ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::*;
use slint::platform::Key;
use std::sync::Arc;
use support::Harness;

fn exercise(pane: PaneId, archive: bool, follow_directory_links: bool) {
    let destination = Arc::new(transfer::Destination::default());
    let mut h = Harness::with_transfer(
        pane,
        archive,
        Arc::new(transfer::Transfer(destination.clone())),
    );
    // The copy reads its first member for longer than a progress heartbeat.
    h.calls
        .slow_read
        .store(archive, std::sync::atomic::Ordering::SeqCst);
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
    assert_eq!(h.ui.window.get_copy_draft_body(), "Copy 2 folders to:");
    let dialog = h.dialog();
    let links = modeless::accessible_popup_item(
        &dialog,
        AccessibleRole::Combobox,
        crate::support::link_option::LABEL,
    );
    if archive {
        assert!(links.is_none_or(|item| !item.is_visible()));
    } else {
        let links = links.expect("captured native tree dialog exposes the link choice");
        // No backend answers which default the destination has: links stay links.
        assert_eq!(
            links
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap(),
            crate::support::link_option::KEEP_LINKS
        );
        if follow_directory_links {
            // From the destination field past Browse to the link choice.
            input::key(&dialog, Key::Tab);
            input::key(&dialog, Key::Tab);
            input::key(&dialog, Key::UpArrow);
            assert_eq!(
                links
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap(),
                crate::support::link_option::COPY_FILES
            );
        }
    }
    input::key(&dialog, Key::Return);
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
    if archive {
        archive_progress(&h);
    }
    let observed = destination.link_policies.lock().unwrap();
    if archive {
        assert!(
            observed.is_empty(),
            "archive members keep their provider readers"
        );
    } else {
        let expected = crate::support::link_option::sent(false, follow_directory_links);
        assert!(
            !observed.is_empty(),
            "the captured native options must reach preparation"
        );
        assert!(observed.iter().all(|policy| *policy == expected));
    }
    drop(observed);
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

/// A copy out of an archive counts its members before it writes the first one and
/// shows the member it copies by its path inside the archive file, as Copy does for
/// files on disk.
fn archive_progress(h: &Harness) {
    let progress = h.copy_progress.last().unwrap();
    assert_eq!(
        progress.discovery.state,
        DiscoveryState::Complete,
        "{progress:?}"
    );
    assert_eq!(
        (
            progress.discovery.files,
            progress.discovery.directories,
            progress.discovery.bytes
        ),
        (2, 2, Some(1012))
    );
    let finished = h.copy_rows.last().unwrap();
    // On the base the total was unknown: "Files: 2 · Bytes: …" without "of".
    assert!(
        finished
            .transfer_totals
            .starts_with("Files: 2 of 2 · Bytes: ")
            && finished.transfer_totals.matches(" of ").count() == 2,
        "{finished:?}"
    );
    let reading = h
        .copy_rows
        .iter()
        .find(|row| row.active && row.transfer_name == "payload.bin")
        .unwrap_or_else(|| panic!("no row showed the member: {:?}", h.copy_rows));
    let folder = std::path::Path::new(reading.transfer_directory.as_str());
    assert!(
        folder.ends_with("fixture.zip/folder/large")
            || folder.ends_with("fixture.zip/folder/small"),
        "{reading:?}"
    );
    assert!(reading.determinate, "{reading:?}");
    assert!(reading.transfer_totals.contains(" of 2 · "), "{reading:?}");
}

#[test]
fn native_nested_folders_copy_and_deselect_through_f5() {
    exercise(PaneId::Left, false, true);
}

#[test]
fn captured_native_copy_passes_unchecked_directory_links_to_preparation() {
    exercise(PaneId::Left, false, false);
}

#[test]
fn archive_nested_folders_copy_and_deselect_through_f5() {
    exercise(PaneId::Right, true, true);
}
