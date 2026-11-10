//! Hooked after the initial routing gate; the existing fixture owns all callbacks.
use super::*;
use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::Flickable,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.size.width,
        origin.y + rect.size.height,
    )
}
fn outer_viewport(row: &ItemRc) -> ItemRc {
    let mut parent = row.parent_item(ParentItemTraversalMode::StopAtPopups);
    let mut flickables = 0;
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            flickables += 1;
            if flickables == 2 {
                return item;
            }
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("settings list must be inside the Options content scroll viewport");
}
impl Harness {
    pub(super) fn save_count(&self) -> usize {
        self.commands
            .borrow()
            .iter()
            .filter(|c| matches!(c, Command::SavePreferences { .. }))
            .count()
    }
    fn row_point(&self, name: &str, fraction: f32) -> slint::LogicalPosition {
        let label = format!("{name} · Program · program.exe · Single");
        let row = ui::popup_item(&self.ui.window, &label, AccessibleRole::ListItem);
        let (left, top, _, bottom) = bounds(&row);
        slint::LogicalPosition::new(left + 36.0, top + (bottom - top) * fraction)
    }
    fn list_bounds(&self) -> (f32, f32, f32, f32) {
        bounds(&ui::popup_item(
            &self.ui.window,
            "Configured Custom Commands",
            AccessibleRole::List,
        ))
    }
    fn press_drag(&self, point: slint::LogicalPosition) {
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: point });
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position: point,
                button: PointerEventButton::Left,
            });
        slint::platform::update_timers_and_animations();
    }
    fn move_drag(&self, point: slint::LogicalPosition) {
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: point });
        slint::platform::update_timers_and_animations();
    }
    fn release_drag(&mut self, point: slint::LogicalPosition) {
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position: point,
                button: PointerEventButton::Left,
            });
        slint::platform::update_timers_and_animations();
        self.ui.draw();
    }
}

#[test]
fn reverse_drag_of_nonselected_command_keeps_its_identity_selected_and_buttons_accessible() {
    let mut h = Harness::new();
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    assert_eq!(view.get_draft().id, "one");
    let start = h.row_point("Second command", 0.5);
    let end = h.row_point("First command", 0.05);
    h.press_drag(start);
    assert!(view.get_drag_active());
    h.move_drag(end);
    h.ui.draw();
    bitmap::snapshot(
        &h.ui.pixels,
        h.ui.width,
        h.ui.height,
        "command-settings-drag-insertion",
    );
    assert_eq!(h.save_count(), 0, "preview must not persist an order");
    h.release_drag(end);
    assert_eq!(h.save_count(), 0);
    assert_eq!(
        h.rows().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["two", "one"]
    );
    assert_eq!(view.get_draft().id, "two");
    h.action("Move Custom Command down");
    h.ui.draw();
    assert_eq!(h.save_count(), 0);
    assert_eq!(
        h.rows().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert_eq!(view.get_draft().id, "two");
    let start = h.row_point("First command", 0.2);
    let end = h.row_point("First command", 0.4);
    h.press_drag(start);
    h.move_drag(end);
    h.release_drag(end);
    assert_eq!(h.save_count(), 0, "all drops remain staged until OK");
    assert_eq!(
        view.get_draft().id,
        "one",
        "valid no-op drop still selects its captured command"
    );
}

#[test]
fn outside_drop_escape_right_button_and_dirty_draft_cancel_without_reordering() {
    let mut h = Harness::new();
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    let (left, top, right, bottom) = h.list_bounds();
    for end in [
        slint::LogicalPosition::new(left - 20.0, top + 20.0),
        slint::LogicalPosition::new(right + 20.0, top + 20.0),
        slint::LogicalPosition::new(left + 36.0, top - 20.0),
        slint::LogicalPosition::new(left + 36.0, bottom + 20.0),
    ] {
        let start = h.row_point("Second command", 0.5);
        h.press_drag(start);
        assert!(view.get_drag_active());
        h.move_drag(end);
        h.release_drag(end);
        assert!(!view.get_drag_active());
        assert_eq!(h.save_count(), 0);
        assert_eq!(
            view.get_draft().id,
            "one",
            "cancelled drag must not select another command"
        );
    }
    let start = h.row_point("First command", 0.3);
    let end = h.row_point("Second command", 0.95);
    h.press_drag(start);
    h.move_drag(end);
    ui::key(&h.ui.window, Key::Escape);
    slint::platform::update_timers_and_animations();
    assert!(
        ui::has_popup(&h.ui.window),
        "first Escape only cancels the drag"
    );
    assert!(!view.get_drag_active());
    h.release_drag(end);
    h.press_drag(start);
    h.move_drag(end);
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position: end,
            button: PointerEventButton::Right,
        });
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Right,
        });
    slint::platform::update_timers_and_animations();
    assert!(!view.get_drag_active());
    h.release_drag(end);
    h.text("Command name", "Unsaved draft 文档");
    h.press_drag(start);
    h.move_drag(end);
    h.release_drag(end);
    assert!(!view.get_drag_active());
    assert_eq!(view.get_draft().name, "Unsaved draft 文档");
    assert!(view.get_dirty());
    assert!(view.get_error().contains("draft"));
    assert_eq!(h.save_count(), 0);
}

#[test]
fn deleted_source_closed_options_and_reentrant_cancel_reject_late_drop_callbacks() {
    let mut h = Harness::new();
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    let start = h.row_point("First command", 0.3);
    let end = h.row_point("Second command", 0.95);
    h.press_drag(start);
    h.move_drag(end);
    let token = view.get_drag_token();
    {
        let _held = h.state.borrow_mut();
        view.invoke_drag_cancel();
    }
    assert!(!view.get_drag_active());
    assert!(!view.invoke_drag_end(token, 2, true));
    h.release_drag(end);
    assert_eq!(h.save_count(), 0);
    h.press_drag(start);
    h.move_drag(end);
    let token = view.get_drag_token();
    h.action("Cancel");
    assert!(!view.get_drag_active());
    assert!(!view.invoke_drag_end(token, 2, true));
    h.release_drag(end);
    h.open();
    h.press_drag(start);
    h.move_drag(end);
    let token = view.get_drag_token();
    h.action("Delete Custom Command");
    h.ui.draw();
    assert_eq!(
        h.save_count(),
        0,
        "deletion from the command list remains staged until OK"
    );
    assert_eq!(h.rows()[0].id.as_str(), "two");
    assert!(!view.invoke_drag_end(token, 1, true));
    h.release_drag(end);
    assert_eq!(h.save_count(), 0);
}

#[test]
fn held_drag_autoscrolls_virtualized_rows_and_keyboard_selection_reveals_its_row() {
    let mut h = Harness::with_count(64);
    h.open();
    let window = h.ui.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    let start = h.row_point("First command", 0.3);
    let (left, _, _, bottom) = h.list_bounds();
    let end = slint::LogicalPosition::new(left + 36.0, bottom - 4.0);
    h.press_drag(start);
    h.move_drag(end);
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(35));
        slint::platform::update_timers_and_animations();
        h.ui.draw();
        assert!(
            view.get_drag_active(),
            "viewport capture survives virtualizing the source row"
        );
    }
    assert_eq!(h.save_count(), 0);
    h.release_drag(end);
    assert_eq!(h.save_count(), 0);
    let snapshot = h.rows();
    let position = snapshot
        .iter()
        .position(|c| c.id.as_str() == "one")
        .unwrap();
    assert!(
        position >= 3,
        "edge scrolling must reach beyond the initial viewport"
    );
    assert_eq!(view.get_draft().id, "one");
    ui::key(&h.ui.window, Key::Home);
    for _ in 0..10 {
        ui::key(&h.ui.window, Key::DownArrow);
        slint::platform::update_timers_and_animations();
    }
    h.ui.draw();
    assert_eq!(view.get_selected_index(), 10);
    let row = view.get_rows().row_data(10).unwrap();
    let selected = ui::popup_item(
        &h.ui.window,
        &format!("{} · {}", row.name, row.summary),
        AccessibleRole::ListItem,
    );
    let selected = bounds(&selected);
    let list = h.list_bounds();
    assert!(selected.1 >= list.1 - 0.5 && selected.3 <= list.3 + 0.5);
}

#[test]
fn ancestor_clipped_vertical_drop_cancels_over_footer() {
    let mut h = Harness::new();
    h.open();
    h.ui.resize(320, 200);
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    let row = ui::popup_item(
        &h.ui.window,
        "First command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let outer = outer_viewport(&row);
    let clip = bounds(&outer);
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(clip.0 + 5.0, clip.1 + 5.0),
            delta_x: 0.0,
            delta_y: -24.0,
        });
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    let row = bounds(&row);
    // Measure the exposed row after real scrolling; font metrics differ by OS.
    // The pinned Fluent horizontal scrollbar overlays 14 logical pixels.
    let visible_top = row.1.max(clip.1);
    let visible_bottom = row.3.min(clip.3 - 14.0);
    assert!(
        visible_top < visible_bottom,
        "a real visible row must receive the press"
    );
    let start = slint::LogicalPosition::new(row.0 + 20.0, (visible_top + visible_bottom) / 2.0);
    let second = ui::popup_item(
        &h.ui.window,
        "Second command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let second = bounds(&second);
    let end = slint::LogicalPosition::new(start.x, (second.1 + second.3) / 2.0 + 1.0);
    assert!(
        end.y > clip.3 && end.y < h.ui.height as f32,
        "release is over the clipped footer, not visible body content"
    );
    h.press_drag(start);
    h.ui.draw();
    bitmap::snapshot(
        &h.ui.pixels,
        h.ui.width,
        h.ui.height,
        "command-settings-vertical-press",
    );
    assert!(
        h.ui.window
            .global::<UserCommandSettingsView>()
            .get_drag_active(),
        "row={row:?} clip={clip:?} start={start:?}, error={:?}",
        h.ui.window.global::<UserCommandSettingsView>().get_error()
    );
    h.move_drag(end);
    h.release_drag(end);
    assert_eq!(h.save_count(), 0, "the footer is not a command drop target");
}

#[test]
fn overflowing_list_scrollbar_is_not_a_command_drop_target() {
    let mut h = Harness::with_count(64);
    h.open();
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    let row = ui::popup_item(
        &h.ui.window,
        "First command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let mut parent = row.parent_item(ParentItemTraversalMode::StopAtPopups);
    let mut overflowing = false;
    while let Some(item) = parent {
        if let Some(flick) = item.downcast::<Flickable>() {
            overflowing = flick.as_pin_ref().content_height().get() > item.geometry().size.height;
            break;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    assert!(
        overflowing,
        "the fixture must target an actually overflowing list"
    );
    let clip = h.list_bounds();
    let end_row = h.row_point("Second command", 0.95);
    let start = h.row_point("First command", 0.2);
    let end = slint::LogicalPosition::new(clip.2 - 2.0, end_row.y);
    h.press_drag(start);
    assert!(
        h.ui.window
            .global::<UserCommandSettingsView>()
            .get_drag_active()
    );
    h.move_drag(end);
    h.release_drag(end);
    assert_eq!(
        h.save_count(),
        0,
        "a scrollbar overlay is not visible list content"
    );
}

#[test]
fn long_form_wheel_scroll_reveals_apply_for_a_real_pointer_click() {
    let mut h = Harness::new();
    h.open();
    h.ui.resize(320, 500);
    h.text("Command name", "Saved through visible Apply");
    h.settle();
    let first = ui::popup_item(
        &h.ui.window,
        "First command · Program · program.exe · Single",
        AccessibleRole::ListItem,
    );
    let outer = outer_viewport(&first);
    let clip = bounds(&outer);
    let flick = outer.downcast::<Flickable>().unwrap();
    assert!(
        flick.as_pin_ref().content_height().get() > outer.geometry().size.height,
        "the long form must contribute real scrollable height"
    );
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(clip.0 + 5.0, clip.1 + 5.0),
            delta_x: 0.0,
            delta_y: -1000.0,
        });
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        h.ui.draw();
    }
    let apply = ui::popup_item(
        &h.ui.window,
        "Preview Custom Command",
        AccessibleRole::Button,
    );
    let apply = bounds(&apply);
    assert!(
        apply.1 >= clip.1 && apply.3 <= clip.3,
        "wheel scrolling must reveal the last form action"
    );
    // The pinned Fluent horizontal scrollbar overlays 14 logical pixels.
    assert!(
        apply.3 <= clip.3 - 14.0,
        "the form action must also clear the scrollbar overlay"
    );
    ui::click_popup(&h.ui.window, "OK", AccessibleRole::Button);
    slint::platform::update_timers_and_animations();
    h.ui.draw();
    assert_eq!(h.save_count(), 1);
    assert_eq!(
        h.latest().user_commands[0].name,
        "Saved through visible Apply"
    );
}
