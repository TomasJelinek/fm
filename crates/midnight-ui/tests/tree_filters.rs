//! Production Engine replies through the installed Slint filter/sort controls.
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use slint::platform::Key;
use std::sync::{Arc, atomic::Ordering};
use support::Harness;

fn apply(h: &mut Harness, mode: i32, expression: &str) {
    let previous_label = h.filter_label();
    h.ui.draw();
    let button = if h.pane == PaneId::Left {
        "Left panel filter"
    } else {
        "Right panel filter"
    };
    ui::click_root(&h.ui.window, button, AccessibleRole::Button);
    assert!(h.ui.window.get_panel_filter_view().open);
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Filter mode", AccessibleRole::Combobox);
    for _ in 0..h.ui.window.get_panel_filter_view().mode {
        ui::key(&h.ui.window, Key::UpArrow);
    }
    for _ in 0..mode {
        ui::key(&h.ui.window, Key::DownArrow);
    }
    ui::key(&h.ui.window, Key::Return);
    ui::click_popup(
        &h.ui.window,
        "File filter expression",
        AccessibleRole::TextInput,
    );
    ui::chord(&h.ui.window, &[Key::Control], "a");
    ui::key(&h.ui.window, expression);
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    assert!(h.ui.window.get_panel_filter_view().busy);
    h.until(|h| !h.ui.window.get_panel_filter_view().open);
    assert_eq!(
        h.filter_label(),
        previous_label,
        "queueing OK does not display the new filter as applied"
    );
    let requested = h.requests.borrow().last().unwrap().clone();
    let expected = expression.to_owned();
    h.until(|h| h.filter_label().contains(&expected));
    assert!(Arc::ptr_eq(
        &requested.view.root.entries,
        &h.view.as_ref().unwrap().root.entries
    ));
    assert!(Arc::ptr_eq(
        &requested.view.root.index,
        &h.view.as_ref().unwrap().root.index
    ));
    assert_eq!(
        requested.view.stamp.session,
        h.view.as_ref().unwrap().stamp.session
    );
}

#[test]
fn native_tree_controls_apply_clear_sort_and_cancel_across_layouts() {
    let mut h = Harness::new(PaneId::Left, false);
    h.ui.window.set_active_pane(0);
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control], Key::F12);
    assert!(
        h.ui.window.get_panel_filter_view().open,
        "Ctrl+F12 reaches the adopted tree dialog"
    );
    ui::key(&h.ui.window, Key::Escape);
    assert!(!h.ui.window.get_panel_filter_view().open);
    let hidden = h
        .view
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.entry.display_name == "a.bin")
        .unwrap()
        .id;
    let position = h.rows().iter().position(|row| row.name == "a.bin").unwrap();
    h.ui.window
        .invoke_row_click(0, position as i32, true, false);
    h.until(|h| h.selection.as_ref().unwrap().contains_row(hidden));
    for (layout, mode, expression) in [
        (0, 0, "*.txt"),
        (1, 1, "\\.txt$"),
        (2, 2, "name = \"z.txt\""),
    ] {
        h.ui.window.invoke_view_mode(0, layout);
        apply(&mut h, mode, expression);
        let view = h.view.as_ref().unwrap();
        assert!(
            view.rows
                .iter()
                .any(|row| row.depth == 1 && row.entry.display_name == "z.txt")
        );
        assert_eq!(
            view.rows
                .iter()
                .any(|row| row.entry.display_name == "a.bin"),
            mode != 2,
            "Masks and Regex keep folders; Expression can filter them"
        );
        assert!(view.rows.iter().any(|row| row.expanded));
        if mode == 2 {
            let ancestor = view.rows.iter().find(|row| row.depth == 0).unwrap();
            assert!(ancestor.context_only && !ancestor.selectable);
            let position = h
                .rows()
                .iter()
                .position(|row| row.name == "folder")
                .unwrap();
            h.ui.window
                .invoke_row_click(0, position as i32, true, false);
            assert!(!h.rows()[position].selected);
            let child = h
                .view
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .find(|row| row.depth == 1)
                .unwrap()
                .id;
            let child_position = h.rows().iter().position(|row| row.name == "z.txt").unwrap();
            h.ui.window
                .invoke_row_click(0, child_position as i32, true, false);
            h.until(|h| h.selection.as_ref().unwrap().contains_row(child));
            h.ui.window.invoke_focus_files();
            ui::chord(&h.ui.window, &[Key::Control], "a");
            h.until(|h| !h.selection.as_ref().unwrap().contains_row(child));
            assert!(
                h.rows().iter().all(|row| !row.selected),
                "Ctrl+A excludes context-only ancestors and clears nested marks"
            );
        }
        h.ui.draw();
        ui::click_root(&h.ui.window, "Clear panel filter", AccessibleRole::Button);
        h.until(|h| h.filter_label().is_empty());
        assert_eq!(h.view.as_ref().unwrap().rows.len(), 4);
        assert!(
            mode != 2 || !h.selection.as_ref().unwrap().contains_row(hidden),
            "clearing must not restore marks hidden by a filter"
        );
        h.ui.window.invoke_open_panel_filter(0);
        assert_eq!(h.ui.window.get_panel_filter_view().expression, expression);
        h.ui.window.invoke_cancel_panel_filter();
    }
    let revision = h.view.as_ref().unwrap().stamp.revision;
    h.ui.window.invoke_view_mode(0, 0);
    h.ui.draw();
    ui::click_root(&h.ui.window, "Sort by Name", AccessibleRole::Button);
    h.until(|h| h.view.as_ref().unwrap().stamp.revision.0 > revision.0);
    let view = h.view.as_ref().unwrap();
    assert!(view.rows.iter().any(|row| row.expanded));
    let names: Vec<_> = view
        .rows
        .iter()
        .filter(|row| row.depth == 1)
        .map(|row| row.entry.display_name.as_str())
        .collect();
    assert_eq!(names, ["z.txt", "a.bin"]);
    let parent = h
        .rows()
        .iter()
        .find(|row| row.name == "folder")
        .unwrap()
        .identity
        .clone();
    h.ui.window.invoke_tree_disclosure(0, parent);
    h.until(|h| h.view.as_ref().unwrap().rows.len() == 2);
    apply(&mut h, 0, "*.txt");
    h.ui.window.invoke_clear_panel_filter(0);
    h.until(|h| h.filter_label().is_empty());
    assert_eq!(
        h.view.as_ref().unwrap().rows.len(),
        2,
        "clearing keeps collapsed branches collapsed"
    );
    h.ui.resize(320, 200);
    h.ui.window.invoke_open_panel_filter(0);
    h.ui.draw();
    ui::click_popup(&h.ui.window, "Cancel", AccessibleRole::Button);
    assert!(!h.ui.window.get_panel_filter_view().open);
}

#[test]
fn archive_tree_inactive_panel_filter_preserves_authority_and_last_valid_result() {
    let mut h = Harness::new(PaneId::Right, true);
    h.ui.window.set_active_pane(0);
    apply(&mut h, 2, "name = \"z.txt\"");
    assert_eq!(
        h.requests.borrow().last().unwrap().view.stamp.pane,
        PaneId::Right
    );
    assert!(
        h.view
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .all(|row| row.entry.virtual_node.is_some())
    );
    let prior = h.view.as_ref().unwrap().clone();
    let label = h.filter_label();
    h.ui.window.invoke_open_panel_filter(1);
    h.ui.window
        .invoke_apply_panel_filter(1, true, 2, "kind =".into(), false);
    h.until(|h| !h.ui.window.get_panel_filter_view().busy);
    assert!(!h.ui.window.get_panel_filter_view().error.is_empty());
    assert!(Arc::ptr_eq(&prior, h.view.as_ref().unwrap()));
    assert_eq!(h.filter_label(), label);
    h.ui.window.invoke_cancel_panel_filter();

    *h.reject_reproject.borrow_mut() = true;
    h.ui.window.invoke_open_panel_filter(1);
    h.ui.window
        .invoke_apply_panel_filter(1, true, 0, "*.txt".into(), false);
    h.until(|h| !h.ui.window.get_panel_filter_view().busy);
    assert!(h.ui.window.get_panel_filter_view().open);
    assert!(!h.ui.window.get_panel_filter_view().error.is_empty());
    assert_eq!(h.filter_label(), label);
    *h.reject_reproject.borrow_mut() = false;
    h.ui.window.invoke_cancel_panel_filter();
    h.calls.fail_save.store(true, Ordering::SeqCst);
    apply(&mut h, 0, "*.txt");
    h.until(|h| h.ui.window.get_preferences_save_status().contains("denied"));
    assert!(h.filter_label().contains("*.txt"));
    assert_eq!(h.view.as_ref().unwrap().rows.len(), 4);
}

#[test]
fn actual_engine_receipt_after_navigation_cannot_apply_to_replacement_pane() {
    let mut h = Harness::new(PaneId::Left, false);
    h.ui.window.invoke_open_panel_filter(0);
    h.ui.window
        .invoke_apply_panel_filter(0, true, 0, "*.txt".into(), false);
    h.until(|h| !h.ui.window.get_panel_filter_view().open);
    let delayed = loop {
        let event = h.receive();
        if matches!(&event, Event::Tree(tree) if matches!(tree.as_ref(), TreeEvent::Reprojected { .. }))
        {
            break event;
        }
        h.apply_event(event);
    };
    let path = h.view.as_ref().unwrap().root.entries[0]
        .path
        .parent()
        .unwrap()
        .join("replacement");
    h.calls.frozen.store(false, Ordering::SeqCst);
    h.ui.window
        .invoke_navigate(0, path.to_str().unwrap().into());
    h.apply_event(delayed);
    assert!(h.filter_label().is_empty());
    assert!(!h.ui.window.get_panel_filter_view().open);
    assert!(h.rows().iter().all(|row| !row.identity.contains(":tree:")));
}

#[test]
fn real_branch_overtaking_filter_reports_failure_and_keeps_draft_retryable() {
    let mut h = Harness::new(PaneId::Left, false);
    let identity = h
        .rows()
        .iter()
        .find(|row| row.name == "folder")
        .unwrap()
        .identity
        .clone();
    h.ui.window.invoke_tree_disclosure(0, identity);
    h.ui.window.invoke_open_panel_filter(0);
    h.ui.window
        .invoke_apply_panel_filter(0, true, 0, "*.txt".into(), false);
    let mut delayed = Vec::new();
    loop {
        let event = h.receive();
        match &event {
            Event::Tree(tree) if matches!(tree.as_ref(), TreeEvent::Published(_)) => {
                delayed.push(event)
            }
            Event::PanelFilterValidated { .. } => {
                h.apply_event(event);
                break;
            }
            _ => h.apply_event(event),
        }
    }
    assert!(!h.ui.window.get_panel_filter_view().open);
    let failure = loop {
        let event = h.receive();
        match &event {
            Event::Tree(tree) if matches!(tree.as_ref(), TreeEvent::ReprojectFailed { .. }) => {
                break event;
            }
            Event::Tree(tree) if matches!(tree.as_ref(), TreeEvent::Published(_)) => {
                delayed.push(event)
            }
            Event::Tree(tree) if matches!(tree.as_ref(), TreeEvent::Reprojected { .. }) => {
                panic!("old view cannot reproject")
            }
            _ => h.apply_event(event),
        }
    };
    assert!(
        !delayed.is_empty(),
        "the real collapse produced a newer publication"
    );
    for event in delayed {
        h.apply_event(event);
    }
    h.apply_event(failure);
    assert!(h.filter_label().is_empty());
    assert_eq!(h.view.as_ref().unwrap().rows.len(), 2);
    assert!(!h.ui.window.get_left_error().is_empty());
    h.ui.window.invoke_open_panel_filter(0);
    assert!(h.ui.window.get_panel_filter_view().open);
    assert_eq!(h.ui.window.get_panel_filter_view().expression, "*.txt");
}
