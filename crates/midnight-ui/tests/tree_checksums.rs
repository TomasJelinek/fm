//! Actual Ctrl+F8 and checksum Options callbacks read owned native files through tree capture.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use slint::{platform::Key, platform::WindowAdapter};
use std::{fs, sync::Arc};
use support::modeless;

fn finished(h: &mut support::Harness, pane: PaneId) -> Arc<ChecksumRun> {
    loop {
        let event = h.receive();
        if let Event::Tree(tree) = &event
            && let TreeEvent::ChecksumsCaptured { stamp, result, .. } = tree.as_ref()
        {
            assert_eq!(stamp.pane, pane);
            assert!(result.is_ok(), "{result:?}");
        }
        if let Event::ChecksumRejected(rejected) = &event {
            panic!("native checksum admission failed: {rejected:?}");
        }
        let run = if let Event::Checksum(event) = &event
            && let ChecksumUpdate::Finished(run) = &event.update
        {
            Some(run.clone())
        } else {
            None
        };
        h.apply_event(event);
        if let Some(run) = run {
            return run;
        }
    }
}

fn exercise(pane: PaneId, folders: bool) {
    let fixture = native::Fixture::new();
    fs::write(fixture.root.join("root.txt"), b"abc").unwrap();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let mut h = support::Harness::with_native_transfer(
        pane,
        fixture.root.clone(),
        service.clone(),
        service,
    );
    h.ui.window.invoke_activate(pane.index() as i32);
    for name in if folders {
        vec!["small", "root.txt"]
    } else {
        vec!["root.txt"]
    } {
        let position = h.rows().iter().position(|row| row.name == name).unwrap();
        h.ui.window
            .invoke_row_click(pane.index() as i32, position as i32, folders, false);
    }
    if folders {
        h.until(|h| {
            h.selection
                .as_ref()
                .is_some_and(|selection| selection.len() == 2)
        });
    }
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control], Key::F8);
    let first = finished(&mut h, pane);
    let root = first
        .rows
        .iter()
        .find(|row| row.path == fixture.root.join("root.txt"))
        .unwrap();
    assert_eq!(root.state, ChecksumRowState::Calculated);
    assert_eq!(
        root.digest(ChecksumAlgorithm::Sha256).unwrap().hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let child = h.tool_window();
    modeless::draw(&child, 1000, 720, "tree-checksums-result");
    if folders {
        assert!(
            first
                .rows
                .iter()
                .any(|row| row.path == fixture.root.join("folder/small")
                    && row.state == ChecksumRowState::Skipped)
        );
        modeless::click(&child, "Options");
        modeless::draw(&child, 1000, 720, "tree-checksums-options");
        let recursive = modeless::accessible_item(
            &child,
            i_slint_core::items::AccessibleRole::Checkbox,
            "Include subfolders",
        )
        .unwrap();
        input::click(&child, &recursive);
        modeless::click(&child, "Calculate");
        let next = finished(&mut h, pane);
        assert_eq!(next.generation.0, first.generation.0 + 1);
        assert_eq!(next.captured_roots, first.captured_roots);
        assert_eq!(next.summary.completion, ChecksumCompletion::Completed);
        assert_eq!(next.summary.progress.calculated, 2);
        assert!(next.rows.iter().any(|row| row.path
            == fixture.root.join("folder/small/payload.bin")
            && row.bytes_read == 12
            && row.state == ChecksumRowState::Calculated));
        modeless::draw(&child, 1000, 720, "tree-checksums-recursive");
    }
    input::key(&child, Key::Escape);
    assert!(!child.window().is_visible());
}

#[test]
fn tree_checksums_ctrl_f8_left_native_file_fallback() {
    exercise(PaneId::Left, false);
}
#[test]
fn tree_checksums_ctrl_f8_right_selected_nested_folder_and_root_file_recalculate() {
    exercise(PaneId::Right, true);
}
