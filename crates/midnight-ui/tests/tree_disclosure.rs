//! Real row widgets keep disclosure separate from navigation and selection.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibilityAction,
    input::TouchPhase,
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
    lengths::LogicalPoint,
    platform::InternalEvent,
};
use midnight_ui::{FileRow, MainWindow};
use slint::{
    ComponentHandle, Model, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn folders() -> Rc<VecModel<FileRow>> {
    Rc::new(VecModel::from(vec![FileRow {
        identity: "opaque occurrence:17".into(),
        name: "Folder 文档".into(),
        directory: true,
        tree_disclosure: 3,
        ..Default::default()
    }]))
}

fn pointer(window: &MainWindow, position: slint::LogicalPosition, pressed: bool) {
    let button = PointerEventButton::Left;
    window.window().dispatch_event(if pressed {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}

fn button(window: &MainWindow, label: &str) -> ItemRc {
    ui::root_item(window, label, AccessibleRole::Button)
}

fn visible_image(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(image) = visible_image(item) {
            return Some(image);
        }
    }
    None
}

fn assert_retry_svg(target: &ItemRc) {
    assert!(
        ui::visible_text_values(target.clone()).is_empty(),
        "retry must not depend on a font glyph"
    );
    let image = visible_image(target.clone()).expect("retry has visible image artwork");
    let source = image.downcast::<ImageItem>().unwrap();
    let actual = source
        .as_pin_ref()
        .source()
        .to_rgba8()
        .expect("retry image renders");
    let expected = slint::Image::load_from_svg_data(include_bytes!("../ui/icons/refresh.svg"))
        .unwrap()
        .to_rgba8()
        .unwrap();
    assert_eq!(actual.size(), expected.size());
    assert_eq!(
        actual.as_bytes(),
        expected.as_bytes(),
        "retry must use the refresh SVG"
    );
    assert!(actual.as_slice().iter().any(|pixel| pixel.a > 0));
    let rect = image.geometry();
    let origin = image.map_to_native_window(rect.origin);
    let target_origin = target.map_to_native_window(target.geometry().origin);
    assert!(rect.width() > 0.0 && rect.width() <= 16.0);
    assert!(rect.height() > 0.0 && rect.height() <= 16.0);
    assert!(origin.x >= target_origin.x && origin.y >= target_origin.y);
    assert!(origin.x + rect.width() <= target_origin.x + target.geometry().width());
    assert!(origin.y + rect.height() <= target_origin.y + target.geometry().height());
}

#[test]
fn disclosure_clicks_do_not_select_open_or_drag_in_any_layout_or_pane() {
    let mut ui = ui::TestUi::new();
    let rows = folders();
    let disclosed = Rc::new(RefCell::new(Vec::new()));
    let captured = disclosed.clone();
    ui.window.on_tree_disclosure(move |pane, identity| {
        captured.borrow_mut().push((pane, identity.to_string()));
    });
    let leaked = Rc::new(RefCell::new(Vec::new()));
    let captured = leaked.clone();
    ui.window
        .on_row_click(move |_, _, _, _| captured.borrow_mut().push("selection"));
    let captured = leaked.clone();
    ui.window
        .on_open(move |_, _| captured.borrow_mut().push("open"));
    let captured = leaked.clone();
    ui.window.on_row_drag(move |_, _, _, _, _| {
        captured.borrow_mut().push("drag");
        true
    });
    for pane in 0..2 {
        ui.window.set_left_rows(Default::default());
        ui.window.set_right_rows(Default::default());
        if pane == 0 {
            ui.window.set_left_rows(rows.clone().into());
        } else {
            ui.window.set_right_rows(rows.clone().into());
        }
        for mode in 0..3 {
            ui.window.set_left_view_mode(mode);
            ui.window.set_right_view_mode(mode);
            ui.draw();
            let point = ui::root_position(
                &ui.window,
                "Expand folders in Folder 文档",
                AccessibleRole::Button,
            );
            disclosed.borrow_mut().clear();
            for _ in 0..2 {
                ui::click(&ui.window, point.x, point.y);
            }
            assert_eq!(
                *disclosed.borrow(),
                vec![(pane, "opaque occurrence:17".to_owned()); 2]
            );
            assert!(leaked.borrow().is_empty(), "pane={pane}, mode={mode}");

            // A disclosure drag remains owned by the disclosure target.
            pointer(&ui.window, point, true);
            let outside = slint::LogicalPosition::new(point.x + 60.0, point.y + 2.0);
            ui.window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: outside });
            pointer(&ui.window, outside, false);
            assert_eq!(disclosed.borrow().len(), 2);
            assert!(leaked.borrow().is_empty());
        }
    }
}

#[test]
fn disclosure_rejects_replaced_rows_changed_states_and_cancelled_presses() {
    let mut ui = ui::TestUi::new();
    let rows = folders();
    ui.window.set_left_rows(rows.clone().into());
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    ui.window
        .on_tree_disclosure(move |_, identity| captured.borrow_mut().push(identity));
    for mode in 0..3 {
        ui.window.set_left_view_mode(mode);
        for change in 0..3 {
            rows.set_row_data(0, folders().row_data(0).unwrap());
            ui.draw();
            let point = ui::root_position(
                &ui.window,
                "Expand folders in Folder 文档",
                AccessibleRole::Button,
            );
            pointer(&ui.window, point, true);
            if change < 2 {
                let mut row = rows.row_data(0).unwrap();
                if change == 0 {
                    row.identity = "replacement occurrence".into();
                } else {
                    row.tree_disclosure = 2;
                }
                rows.set_row_data(0, row);
                ui.draw();
            } else {
                ui.window
                    .window()
                    .dispatch_event(WindowEvent::PointerExited);
            }
            pointer(&ui.window, point, false);
            assert!(calls.borrow().is_empty(), "mode={mode}, change={change}");
        }
    }
}

#[test]
fn disclosure_states_and_deep_indent_preserve_row_geometry() {
    let mut ui = ui::TestUi::new();
    let rows = folders();
    ui.window.set_left_rows(rows.clone().into());
    let retries = Rc::new(RefCell::new(Vec::new()));
    let captured = retries.clone();
    ui.window.on_tree_disclosure(move |pane, identity| {
        captured.borrow_mut().push((pane, identity.to_string()));
    });
    for mode in 0..3 {
        ui.window.set_left_view_mode(mode);
        ui.draw();
        let baseline =
            ui::root_item(&ui.window, "Folder 文档", AccessibleRole::ListItem).geometry();
        for (state, expanded, label, glyph) in [
            (1, false, "Discover folders in Folder 文档", "›"),
            (2, false, "Loading folders in Folder 文档", "…"),
            (3, false, "Expand folders in Folder 文档", "+"),
            (4, true, "Collapse folders in Folder 文档", "−"),
            (6, false, "Retry folders in Folder 文档", ""),
            (6, true, "Collapse folders in Folder 文档", "−"),
        ] {
            let mut row = rows.row_data(0).unwrap();
            row.tree_disclosure = state;
            row.tree_expanded = expanded;
            row.tree_depth = 64;
            rows.set_row_data(0, row);
            ui.draw();
            let target = button(&ui.window, label);
            if state == 6 && !expanded {
                assert_retry_svg(&target);
                let before = retries.borrow().len();
                target.accessible_action(&AccessibilityAction::Default);
                assert_eq!(retries.borrow().len(), before + 1);
                assert_eq!(
                    retries.borrow().last().unwrap(),
                    &(0, "opaque occurrence:17".into())
                );
            } else {
                assert!(
                    ui::visible_text_values(target.clone())
                        .iter()
                        .any(|text| text == glyph)
                );
            }
            let row = ui::root_item(&ui.window, "Folder 文档", AccessibleRole::ListItem);
            assert_eq!(row.geometry(), baseline, "mode={mode}, state={state}");
            let origin = target.map_to_native_window(target.geometry().origin);
            let row_origin = row.map_to_native_window(row.geometry().origin);
            assert!(
                origin.x >= row_origin.x
                    && origin.x + target.geometry().width()
                        <= row_origin.x + row.geometry().width()
            );
        }
    }
    // Modified disclosure clicks still only disclose; modifiers cannot leak to selection.
    ui.window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    ui::click_root(
        &ui.window,
        "Collapse folders in Folder 文档",
        AccessibleRole::Button,
    );
    ui.window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

type Disclosures = Rc<RefCell<Vec<(i32, String, bool)>>>;
type InputLeaks = Rc<RefCell<Vec<&'static str>>>;

fn record_disclosures(window: &MainWindow) -> (Disclosures, InputLeaks) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let output = calls.clone();
    window.on_tree_disclosure(move |pane, identity| {
        output
            .borrow_mut()
            .push((pane, identity.to_string(), false));
    });
    let output = calls.clone();
    window.on_tree_disclosure_recursive(move |pane, identity| {
        output.borrow_mut().push((pane, identity.to_string(), true));
    });
    let leaks = Rc::new(RefCell::new(Vec::new()));
    let output = leaks.clone();
    window.on_row_click(move |_, _, _, _| output.borrow_mut().push("selection"));
    let output = leaks.clone();
    window.on_open(move |_, _| output.borrow_mut().push("open"));
    let output = leaks.clone();
    window.on_row_drag(move |_, _, _, _, _| {
        output.borrow_mut().push("drag");
        true
    });
    (calls, leaks)
}

fn control(window: &MainWindow, pressed: bool) {
    let text = Key::Control.into();
    window.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    });
    assert_eq!(window.get_function_modifiers() & 1 != 0, pressed);
}

fn native_tree_touch(window: &MainWindow, point: slint::LogicalPosition, phase: TouchPhase) {
    // Same public capture/translation ordering as the existing native observer.
    // Replacing this with PointerPressed would recapture the later modifiers.
    match phase {
        TouchPhase::Started => window.invoke_capture_function_press(point.x, point.y),
        TouchPhase::Cancelled => window.invoke_cancel_function_press(),
        _ => {}
    }
    window
        .window()
        .dispatch_event(WindowEvent::internal(InternalEvent::Touch {
            id: 1,
            position: LogicalPoint::new(point.x, point.y),
            phase,
        }));
    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
        window.invoke_release_function_press();
    }
}

#[test]
fn recursive_disclosure_uses_pressed_control_in_both_panes_and_all_layouts() {
    let mut test = ui::TestUi::new();
    let rows = folders();
    let (calls, leaks) = record_disclosures(&test.window);
    for pane in 0..2 {
        test.window.set_left_rows(Default::default());
        test.window.set_right_rows(Default::default());
        if pane == 0 {
            test.window.set_left_rows(rows.clone().into());
        } else {
            test.window.set_right_rows(rows.clone().into());
        }
        for mode in 0..3 {
            test.window.set_left_view_mode(mode);
            test.window.set_right_view_mode(mode);
            for link in [false, true] {
                for expanded in [false, true] {
                    let mut row = folders().row_data(0).unwrap();
                    row.kind = if link { "Folder link" } else { "Folder" }.into();
                    row.tree_expanded = expanded;
                    row.tree_disclosure = if expanded { 4 } else { 3 };
                    rows.set_row_data(0, row);
                    test.window.set_active_pane(pane);
                    test.window.invoke_focus_files();
                    test.draw();
                    let label = if expanded {
                        "Collapse folders in Folder 文档"
                    } else {
                        "Expand folders in Folder 文档"
                    };
                    let point = ui::root_position(&test.window, label, AccessibleRole::Button);
                    for (pressed_control, released_control) in
                        [(false, false), (true, true), (true, false), (false, true)]
                    {
                        calls.borrow_mut().clear();
                        control(&test.window, pressed_control);
                        pointer(&test.window, point, true);
                        control(&test.window, released_control);
                        pointer(&test.window, point, false);
                        control(&test.window, false);
                        assert_eq!(
                            *calls.borrow(),
                            [(pane, "opaque occurrence:17".to_owned(), pressed_control)],
                            "pane={pane}, mode={mode}, link={link}, expanded={expanded}, down={pressed_control}, up={released_control}"
                        );
                        assert!(leaks.borrow().is_empty(), "expander leaked row input");
                    }
                }
            }
        }
    }
}

#[test]
fn recursive_disclosure_keeps_native_press_control_and_rejects_stale_deferred_touches() {
    let clock = Rc::new(Cell::new(Duration::ZERO));
    let mut test = ui::TestUi::with_clock(Some(clock.clone()));
    let rows = folders();
    test.window.set_left_rows(rows.clone().into());
    let (calls, leaks) = record_disclosures(&test.window);
    for delayed in [false, true] {
        for pressed_control in [false, true] {
            for change in 0..7 {
                rows.set_row_data(0, folders().row_data(0).unwrap());
                test.window
                    .set_left_search_context("same navigation".into());
                test.window.set_left_view_mode(0);
                test.window.invoke_focus_files();
                test.draw();
                let point = ui::root_position(
                    &test.window,
                    "Expand folders in Folder 文档",
                    AccessibleRole::Button,
                );
                calls.borrow_mut().clear();
                control(&test.window, pressed_control);
                native_tree_touch(&test.window, point, TouchPhase::Started);
                control(&test.window, !pressed_control);
                if change == 1 {
                    let mut row = rows.row_data(0).unwrap();
                    row.identity = "replacement occurrence".into();
                    rows.set_row_data(0, row);
                } else if change == 2 {
                    let mut row = rows.row_data(0).unwrap();
                    row.tree_disclosure = 6;
                    rows.set_row_data(0, row);
                } else if change == 3 {
                    let mut row = rows.row_data(0).unwrap();
                    row.tree_expanded = true;
                    row.tree_disclosure = 4;
                    rows.set_row_data(0, row);
                } else if change == 4 {
                    test.window.set_left_search_context("new navigation".into());
                } else if change == 5 {
                    test.window.set_left_view_mode(1);
                }
                if delayed {
                    clock.set(clock.get() + Duration::from_millis(120));
                    test.draw();
                }
                native_tree_touch(
                    &test.window,
                    point,
                    if change == 6 {
                        TouchPhase::Cancelled
                    } else {
                        TouchPhase::Ended
                    },
                );
                test.draw();
                control(&test.window, false);
                if change == 0 {
                    assert_eq!(
                        *calls.borrow(),
                        [(0, "opaque occurrence:17".to_owned(), pressed_control)],
                        "original press modifiers were lost: delay={delayed}, Ctrl={pressed_control}"
                    );
                } else {
                    assert!(
                        calls.borrow().is_empty(),
                        "stale/cancelled press dispatched: delay={delayed}, Ctrl={pressed_control}, change={change}"
                    );
                }
                assert!(leaks.borrow().is_empty(), "touch expander leaked row input");
            }
        }
    }
}
