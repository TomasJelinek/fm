//! Keys in search results fed to a panel take that panel's file shortcuts, the
//! native hook's keypad keys among them: each acts on the list or says why it
//! cannot, and none reaches the folder under the list.
use crate::support::ui::{self, TestUi};
use i_slint_core::items::AccessibleRole;
use midnight_ui::{FeedPresentation, FeedView};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

/// What the window asked: `(pane, code)` of the list, its refusals, and the
/// folder's own bulk selection.
#[derive(Default)]
struct Routed {
    actions: Vec<(i32, i32)>,
    refused: Vec<(i32, String)>,
    folder_selection: Vec<(i32, i32)>,
}

/// The left panel shows search results, and the list has the keyboard.
fn fed() -> (TestUi, Rc<RefCell<Routed>>) {
    let mut ui = TestUi::new();
    let routed = Rc::new(RefCell::new(Routed::default()));
    let feeds = ui.window.global::<FeedPresentation>();
    let sink = routed.clone();
    feeds.on_action(move |pane, code| sink.borrow_mut().actions.push((pane, code)));
    let sink = routed.clone();
    feeds.on_refuse(move |pane, action| {
        sink.borrow_mut().refused.push((pane, action.into()));
    });
    let sink = routed.clone();
    ui.window.on_bulk_selection(move |pane, operation| {
        sink.borrow_mut().folder_selection.push((pane, operation));
    });
    feeds.set_left(FeedView {
        open: true,
        title: "Search results (2)".into(),
        ..Default::default()
    });
    ui.window.set_active_pane(0);
    ui.window.invoke_focus_files();
    ui.draw();
    assert_eq!(ui.window.get_feed_focused_pane(), 0);
    assert_eq!(ui.window.get_focused_pane(), -1, "the folder's list waits");
    (ui, routed)
}

/// The list's code for Ctrl+Shift+I (`find_feed::BULK` + invert): the files only.
const INVERT: i32 = 22;
/// The list's code for Ctrl+I (`find_feed::BULK` + invert all): files and folders.
const INVERT_ALL: i32 = 24;
/// The list's code for keypad * (`find_feed::MASK_INVERT`): the presenter inverts
/// as a panel does, the folders too when Select by mask includes them.
const MASK_INVERT: i32 = 12;

#[test]
fn keypad_and_mask_keys_in_search_results_route_to_the_fed_panel() {
    let (ui, routed) = fed();
    // The native window hook takes keypad + − * before Slint sees them.
    for key in ["NumpadAdd", "NumpadSubtract", "NumpadMultiply"] {
        assert!(
            ui.window
                .invoke_physical_shortcut(key.into(), false, false, false, false),
            "{key} is taken, not typed"
        );
    }
    // Ctrl+= and Ctrl+− select by mask without a keypad; Ctrl+H shows hidden files.
    ui::chord(&ui.window, &[Key::Control], "=");
    ui::chord(&ui.window, &[Key::Control], "-");
    ui::chord(&ui.window, &[Key::Control], "h");
    // Ctrl+I inverts files and folders, Ctrl+Shift+I the files only, in the list as in
    // a panel.
    ui::chord(&ui.window, &[Key::Control], "i");
    ui::chord(&ui.window, &[Key::Control, Key::Shift], "i");
    let routed = routed.borrow();
    let refused: Vec<_> = routed
        .refused
        .iter()
        .map(|(pane, action)| (*pane, action.as_str()))
        .collect();
    assert!(
        refused.is_empty(),
        "result-row mask and hidden actions are supported"
    );
    assert_eq!(
        routed.actions,
        [(0, MASK_INVERT), (0, INVERT_ALL), (0, INVERT)],
        "keypad * inverts as in a panel, with Select by mask's folder choice; Ctrl+I \
         inverts files and folders, Ctrl+Shift+I the files"
    );
    assert_eq!(
        routed.folder_selection,
        [(0, 6), (0, 7), (0, 6), (0, 7)],
        "mask dialogs receive their normal callback; their presenter captures the feed"
    );
}

#[test]
fn visible_go_to_file_and_enter_route_to_the_fed_panel() {
    let (mut ui, routed) = fed();
    ui.window.global::<FeedPresentation>().set_left_rows(
        Rc::new(slint::VecModel::from(vec![midnight_ui::FeedRow {
            name: "result.txt".into(),
            ..Default::default()
        }]))
        .into(),
    );
    ui.draw();
    ui::click_root(&ui.window, "Go to file", AccessibleRole::Button);
    ui.window.invoke_focus_files();
    ui::key(&ui.window, Key::Return);
    assert_eq!(routed.borrow().actions, [(0, 1), (0, 1)]);
    assert!(routed.borrow().folder_selection.is_empty());
}

#[test]
fn ctrl_m_opens_make_file_list_for_the_feed_without_refusal() {
    let (ui, routed) = fed();
    ui::chord(&ui.window, &[Key::Control], "m");
    assert_eq!(routed.borrow().actions, [(0, 40)]);
    assert!(routed.borrow().refused.is_empty());
}

#[test]
fn feed_columns_resize_independently_at_their_header_separators() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let (mut ui, _) = fed();
    ui.resize(2400, 820);
    for title in ["Name", "Size", "Modified", "Folder"] {
        let label = format!("{title} column");
        let header = ui::root_item(&ui.window, &label, AccessibleRole::Button);
        let before = header.geometry().width();
        let bounds = header.geometry();
        let origin = header.map_to_native_window(bounds.origin);
        let from =
            slint::LogicalPosition::new(origin.x + before + 5.0, origin.y + bounds.height() / 2.0);
        let to = slint::LogicalPosition::new(from.x + 40.0, from.y);
        let window = ui.window.window();
        window.dispatch_event(WindowEvent::PointerMoved { position: from });
        window.dispatch_event(WindowEvent::PointerPressed {
            position: from,
            button: PointerEventButton::Left,
        });
        window.dispatch_event(WindowEvent::PointerMoved { position: to });
        window.dispatch_event(WindowEvent::PointerReleased {
            position: to,
            button: PointerEventButton::Left,
        });
        ui.draw();
        let after = ui::root_item(&ui.window, &label, AccessibleRole::Button)
            .geometry()
            .width();
        assert!(
            (after - before - 40.0).abs() <= 1.0,
            "{title}: {before} -> {after}"
        );
    }
}

#[test]
fn feed_sort_headers_and_ctrl_f8_reach_supported_row_actions() {
    let (mut ui, routed) = fed();
    let headers = Rc::new(RefCell::new(Vec::new()));
    let sink = headers.clone();
    ui.window
        .global::<FeedPresentation>()
        .on_sort(move |pane, column| sink.borrow_mut().push((pane, column)));
    ui.resize(2400, 820);
    for title in ["Name", "Folder", "Size", "Modified"] {
        ui::click_root(
            &ui.window,
            &format!("{title} column"),
            AccessibleRole::Button,
        );
    }
    assert_eq!(*headers.borrow(), [(0, 0), (0, 1), (0, 2), (0, 3)]);
    let normal = Rc::new(RefCell::new(Vec::new()));
    let sink = normal.clone();
    ui.window
        .on_sort(move |pane, column| sink.borrow_mut().push((pane, column)));
    let sink = normal.clone();
    ui.window
        .on_calculate_checksums(move |pane| sink.borrow_mut().push((pane, 8)));
    ui.window.invoke_focus_files();
    ui::chord(&ui.window, &[Key::Control], Key::F4);
    ui::chord(&ui.window, &[Key::Control], Key::F8);
    assert_eq!(*normal.borrow(), [(0, 3), (0, 8)]);
    assert!(routed.borrow().refused.is_empty());
}

#[test]
fn feed_headings_fit_above_the_first_row_at_minimum_width_in_both_densities() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let (mut ui, _) = fed();
    ui.window.global::<FeedPresentation>().set_left_rows(
        Rc::new(slint::VecModel::from(vec![midnight_ui::FeedRow {
            name: "layout.txt".into(),
            folder: "/fixture".into(),
            size: "10 B".into(),
            modified: "2026-11-24".into(),
            ..Default::default()
        }]))
        .into(),
    );
    for (density, font) in [(0, 12), (0, 24), (1, 15), (1, 28)] {
        ui.window.invoke_apply_preferences(0, density);
        ui.window.invoke_apply_fonts("".into(), font, font);
        for width in [900, 2400] {
            ui.resize(width, 1000);
            let first = ui::root_item(&ui.window, "Name column", AccessibleRole::Button);
            let bounds = first.geometry();
            let origin = first.map_to_native_window(bounds.origin);
            let from = slint::LogicalPosition::new(
                origin.x + bounds.width() + 5.0,
                origin.y + bounds.height() / 2.0,
            );
            let to = slint::LogicalPosition::new(from.x - 5000.0, from.y);
            let window = ui.window.window();
            window.dispatch_event(WindowEvent::PointerMoved { position: from });
            window.dispatch_event(WindowEvent::PointerPressed {
                position: from,
                button: PointerEventButton::Left,
            });
            window.dispatch_event(WindowEvent::PointerMoved { position: to });
            window.dispatch_event(WindowEvent::PointerReleased {
                position: to,
                button: PointerEventButton::Left,
            });
            ui.draw();
            let first = ui::root_item(&ui.window, "Name column", AccessibleRole::Button);
            let minimum = 3.0 * (font as f32 - if density == 1 { 2.0 } else { 1.0 }).max(8.0);
            assert!(
                (first.geometry().width() - minimum).abs() <= 1.0,
                "minimum header width: density={density},font={font},width={width}"
            );
            let row = ui::root_item(
                &ui.window,
                "layout.txt · /fixture · 10 B · 2026-11-24",
                AccessibleRole::ListItem,
            );
            let top = row.map_to_native_window(row.geometry().origin).y;
            for title in ["Name", "Folder", "Size", "Modified"] {
                let header = ui::root_item(
                    &ui.window,
                    &format!("{title} column"),
                    AccessibleRole::Button,
                );
                let bounds = header.geometry();
                let bottom = header.map_to_native_window(bounds.origin).y + bounds.height();
                assert!(
                    bottom <= top + 0.5,
                    "{title} must not overlap first row: density={density},font={font},width={width}"
                );
            }
        }
    }
}
