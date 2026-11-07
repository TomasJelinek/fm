//! Dialog headers move real open popups without replacing their focused controls.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{ComplexText, SimpleText},
    window::{WindowInner, WindowKind},
};
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{Key, PointerEventButton, WindowEvent},
};

fn find_text(item: ItemRc, value: &str) -> Option<ItemRc> {
    let text = item
        .downcast::<SimpleText>()
        .map(|text| text.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|text| text.as_pin_ref().text())
        });
    if text.is_some_and(|text| text == value) {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_text(item, value) {
            return Some(found);
        }
    }
    None
}

fn popup_title(test: &ui::TestUi, title: &str) -> ItemRc {
    let popups = WindowInner::from_pub(test.window.window()).active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .unwrap();
    find_text(ItemRc::new_root(popup.component.clone()), title).unwrap()
}

fn origin(item: &ItemRc) -> LogicalPosition {
    let point = item.map_to_native_window(item.geometry().origin);
    LogicalPosition::new(point.x, point.y)
}

fn drag(test: &mut ui::TestUi, title: &ItemRc, dx: f32, dy: f32) {
    let start = origin(title);
    let start = LogicalPosition::new(start.x + 20.0, start.y + title.geometry().height() / 2.0);
    let end = LogicalPosition::new(start.x + dx, start.y + dy);
    for event in [
        WindowEvent::PointerMoved { position: start },
        WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerMoved { position: end },
        WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        },
    ] {
        test.window.window().dispatch_event(event);
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn moves(test: &mut ui::TestUi, title: ItemRc) {
    let before = origin(&title);
    drag(test, &title, 32.0, 20.0);
    let after = origin(&title);
    assert!(
        (after.x - before.x - 32.0).abs() < 1.0,
        "horizontal: {before:?} -> {after:?}"
    );
    assert!(
        (after.y - before.y - 20.0).abs() < 1.0,
        "vertical: {before:?} -> {after:?}"
    );
    // A second drag must start from the current position, without snapping back.
    drag(test, &title, -12.0, -8.0);
    let after = origin(&title);
    assert!(
        (after.x - before.x - 20.0).abs() < 1.0,
        "second horizontal: {before:?} -> {after:?}"
    );
    assert!(
        (after.y - before.y - 12.0).abs() < 1.0,
        "second vertical: {before:?} -> {after:?}"
    );
}

fn draw(test: &mut ui::TestUi) {
    // Model-bound popup visibility and the popup layout tracker settle separately.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn root_title(test: &ui::TestUi, title: &str) -> ItemRc {
    find_text(
        ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        title,
    )
    .unwrap()
}

fn inside(test: &ui::TestUi, item: &ItemRc, label: &str) {
    if let Some(error) = bounds_error(test, item, label) {
        panic!("{error}");
    }
}

fn bounds_error(test: &ui::TestUi, item: &ItemRc, label: &str) -> Option<String> {
    let at = origin(item);
    let size = item.geometry().size;
    let inside = at.x >= 0.0
        && at.y >= 0.0
        && at.x + size.width <= test.width as f32 + 0.5
        && at.y + size.height <= test.height as f32 + 0.5;
    (!inside).then(|| {
        format!(
            "{label} is outside {}×{}: {at:?}, {size:?}",
            test.width, test.height
        )
    })
}

#[test]
fn open_options_follows_header_drag_and_remains_reachable_after_resize() {
    let mut test = ui::TestUi::new();
    test.window.invoke_show_options();
    test.draw();
    let title = popup_title(&test, "Options");
    let before = origin(&title);
    drag(&mut test, &title, 32.0, 20.0);
    let after = origin(&title);
    assert!(
        (after.x - before.x - 32.0).abs() < 1.0,
        "header did not move horizontally: {before:?} -> {after:?}"
    );
    assert!(
        (after.y - before.y - 20.0).abs() < 1.0,
        "header did not move vertically: {before:?} -> {after:?}"
    );
    drag(&mut test, &title, 10_000.0, 10_000.0);
    inside(&test, &title, "Options header at drag boundary");
    test.resize(320, 200);
    draw(&mut test);
    let title = popup_title(&test, "Options");
    let position = origin(&title);
    assert!(position.x >= 0.0 && position.y >= 0.0);
    inside(
        &test,
        &ui::popup_item(
            &test.window,
            "Done",
            i_slint_core::items::AccessibleRole::Button,
        ),
        "Done after shrink",
    );
    ui::click_popup(
        &test.window,
        "Done",
        i_slint_core::items::AccessibleRole::Button,
    );
    test.draw();
    assert!(
        WindowInner::from_pub(test.window.window())
            .active_popups()
            .is_empty()
    );
}

#[test]
fn dialog_headers_move_and_escape_keeps_existing_close_behavior() {
    use i_slint_core::items::AccessibleRole;
    let mut test = ui::TestUi::new();
    let mut outside = Vec::new();
    test.window
        .set_hot_path_choices(slint::ModelRc::new(slint::VecModel::from(vec![
            midnight_ui::NavigationChoiceRow {
                identity: "manage-hot-paths".into(),
                label: "Manage Hot Paths…".into(),
                ..Default::default()
            },
        ])));
    let window = test.window.as_weak();
    test.window
        .on_cancel_metadata(move |_| window.upgrade().unwrap().set_metadata_token("".into()));
    let window = test.window.as_weak();
    test.window.on_cancel_comparison(move || {
        let window = window.upgrade().unwrap();
        let mut model = window.get_comparison_view();
        model.open = false;
        window.set_comparison_view(model);
    });
    let window = test.window.as_weak();
    test.window.on_cancel_panel_filter(move || {
        let window = window.upgrade().unwrap();
        let mut model = window.get_panel_filter_view();
        model.open = false;
        window.set_panel_filter_view(model);
    });
    for kind in 0..11 {
        test.window.invoke_apply_fonts("".into(), 11, 15);
        test.window.invoke_apply_preferences(0, 0);
        test.resize(1340, 820);
        let title = match kind {
            0 => {
                test.window.invoke_show_options();
                "Options"
            }
            1 => {
                test.window.invoke_show_operations();
                "Jobs"
            }
            2 => {
                test.window.invoke_show_app_menu(0);
                "Midnight Explorer keyboard help"
            }
            3 => {
                test.window.set_copy_token("copy:1:1".into());
                test.window.invoke_show_copy_conflict();
                "Item already exists"
            }
            4 => {
                test.window
                    .set_metadata_conflict_token("metadata:1:1".into());
                test.window.invoke_show_metadata_conflict();
                "Item already exists"
            }
            5 => {
                test.window.set_metadata_token("metadata:1".into());
                "Attributes, times and case"
            }
            6 => {
                test.window
                    .set_comparison_view(midnight_ui::ComparisonView {
                        open: true,
                        ..Default::default()
                    });
                "Compare displayed directories"
            }
            7 => {
                test.window
                    .set_panel_filter_view(midnight_ui::PanelFilterView {
                        open: true,
                        ..Default::default()
                    });
                "Panel file filter"
            }
            8 => {
                test.window.set_message_title("Error details".into());
                test.window.set_message_generation(1);
                "Error details"
            }
            9 => {
                ui::click_root(&test.window, "Hot Paths", AccessibleRole::Combobox);
                draw(&mut test);
                ui::click_popup(&test.window, "Manage Hot Paths…", AccessibleRole::Button);
                "Manage Hot Paths"
            }
            _ => {
                test.window.invoke_show_app_menu(2);
                draw(&mut test);
                ui::click_popup(
                    &test.window,
                    "About Midnight Explorer",
                    AccessibleRole::Button,
                );
                "Midnight Explorer · File Manager"
            }
        };
        draw(&mut test);
        let item = popup_title(&test, title);
        moves(&mut test, item);
        test.resize(320, 200);
        draw(&mut test);
        outside.extend(bounds_error(&test, &popup_title(&test, title), title));
        let close = match kind {
            0 => "Done",
            3 | 4 => "Decide later",
            5 | 7 => "Cancel",
            _ => "Close",
        };
        outside.extend(bounds_error(
            &test,
            &ui::popup_item(&test.window, close, AccessibleRole::Button),
            close,
        ));
        test.window.invoke_apply_fonts("".into(), 32, 32);
        test.window.invoke_apply_preferences(0, 1);
        draw(&mut test);
        outside.extend(bounds_error(&test, &popup_title(&test, title), title));
        outside.extend(bounds_error(
            &test,
            &ui::popup_item(&test.window, close, AccessibleRole::Button),
            close,
        ));
        ui::key(&test.window, Key::Escape);
        draw(&mut test);
        assert!(!ui::has_popup(&test.window), "Escape did not close {title}");
    }
    assert!(outside.is_empty(), "{}", outside.join("\n"));
}

#[test]
fn outside_click_preserves_dialog_policies_and_reopening_centers_again() {
    let mut test = ui::TestUi::new();
    test.window.invoke_show_options();
    draw(&mut test);
    let title = popup_title(&test, "Options");
    let center = origin(&title);
    moves(&mut test, title);
    ui::click(&test.window, 2.0, 2.0);
    draw(&mut test);
    assert!(
        !ui::has_popup(&test.window),
        "Options keeps outside-click dismissal"
    );
    test.window.invoke_show_options();
    draw(&mut test);
    assert_eq!(origin(&popup_title(&test, "Options")), center);
    ui::click(&test.window, -2.0, -2.0);
    draw(&mut test);
    assert!(
        !ui::has_popup(&test.window),
        "Options also dismisses outside the host viewport"
    );
    test.window.invoke_show_operations();
    draw(&mut test);
    ui::click(&test.window, 2.0, 2.0);
    draw(&mut test);
    assert!(
        ui::has_popup(&test.window),
        "Jobs keeps no-auto-close policy"
    );
    ui::key(&test.window, Key::Escape);
    draw(&mut test);
    assert!(!ui::has_popup(&test.window));
}

#[test]
fn scrolling_dialogs_open_at_minimum_size_with_keyboard_close_available() {
    let mut test = ui::TestUi::new();
    test.window.invoke_apply_fonts("".into(), 32, 32);
    test.window.invoke_apply_preferences(0, 1);
    test.resize(320, 200);
    let window = test.window.as_weak();
    test.window.on_cancel_comparison(move || {
        let window = window.upgrade().unwrap();
        let mut model = window.get_comparison_view();
        model.open = false;
        window.set_comparison_view(model);
    });
    let window = test.window.as_weak();
    test.window.on_cancel_panel_filter(move || {
        let window = window.upgrade().unwrap();
        let mut model = window.get_panel_filter_view();
        model.open = false;
        window.set_panel_filter_view(model);
    });
    test.window
        .set_comparison_view(midnight_ui::ComparisonView {
            open: true,
            ..Default::default()
        });
    draw(&mut test);
    ui::key(&test.window, Key::Escape);
    draw(&mut test);
    assert!(
        !ui::has_popup(&test.window),
        "comparison Escape at initial minimum size"
    );
    test.window
        .set_panel_filter_view(midnight_ui::PanelFilterView {
            open: true,
            ..Default::default()
        });
    draw(&mut test);
    ui::key(&test.window, Key::Escape);
    draw(&mut test);
    assert!(
        !ui::has_popup(&test.window),
        "filter Escape at initial minimum size"
    );
}

#[test]
fn operation_and_copy_prompts_move_without_losing_the_name_draft() {
    use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
    let mut test = ui::TestUi::new();
    for (kind, title, has_name) in [
        (0, "Rename", true),
        (1, "Move to Trash", false),
        (2, "Permanently delete", false),
        (3, "Create directory", true),
        (4, "New empty file", true),
    ] {
        test.window.set_operation_token("operation:1".into());
        test.window.set_operation_title(title.into());
        test.window.set_operation_kind(kind);
        test.window.set_operation_has_name(has_name);
        test.window.set_operation_name("draft.txt".into());
        test.window.invoke_begin_operation();
        draw(&mut test);
        let item = root_title(&test, title);
        moves(&mut test, item);
        if has_name {
            let field = ui::root_item(&test.window, title, AccessibleRole::TextInput);
            assert_eq!(
                field
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap()
                    .as_str(),
                "draft.txt"
            );
            ui::key(&test.window, "Ω");
            assert!(
                field
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap()
                    .contains('Ω'),
                "dragging the header must keep keyboard focus in {title}"
            );
        }
        test.window.set_operation_token("".into());
        draw(&mut test);
    }
    test.window.set_copy_draft_token("copy-draft:1".into());
    test.window.set_copy_draft_destination("destination".into());
    test.window.invoke_show_copy_draft();
    draw(&mut test);
    let item = root_title(&test, "Copy");
    moves(&mut test, item);
    test.window.set_copy_draft_token("".into());
}
