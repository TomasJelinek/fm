//! Keys in search results fed to a panel take that panel's file shortcuts, the
//! native hook's keypad keys among them: each acts on the list or says why it
//! cannot, and none reaches the folder under the list.
use crate::support::ui::{self, TestUi};
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

/// The list's code for Ctrl+I (`find_feed::BULK` + invert).
const INVERT: i32 = 22;

#[test]
fn keypad_and_mask_keys_in_search_results_invert_the_marks_or_say_why_not() {
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
    let routed = routed.borrow();
    let refused: Vec<_> = routed
        .refused
        .iter()
        .map(|(pane, action)| (*pane, action.as_str()))
        .collect();
    assert_eq!(
        refused,
        [
            (0, "Select by mask"),
            (0, "Deselect by mask"),
            (0, "Select by mask"),
            (0, "Deselect by mask"),
            (0, "Show hidden files"),
        ]
    );
    assert_eq!(
        routed.actions,
        [(0, INVERT)],
        "keypad * inverts like Ctrl+I"
    );
    assert!(
        routed.folder_selection.is_empty(),
        "the folder under the list is never selected"
    );
}
