//! Small tool windows preserve editable queries, navigation and job diagnostics.
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/progress_form.rs"]
mod ui;

use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
};
use midnight_ui::{ChecksumWindow, FindWindow, OperationRow};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

fn scroll_parent(item: &ItemRc) -> ItemRc {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    loop {
        let item = parent.expect("diagnostics must have a scrolling viewport");
        if item.downcast::<Flickable>().is_some() {
            return item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

fn inside(item: &ItemRc, width: f32, height: f32) {
    let area = bounds(item);
    assert!(
        area.0 >= 0.0 && area.1 >= 0.0 && area.2 <= width + 1.0 && area.3 <= height + 1.0,
        "control is outside the window: {area:?}"
    );
}

#[test]
fn minimum_find_window_exposes_an_editable_mask_before_query_options() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    let searches = Rc::new(RefCell::new(Vec::new()));
    let captured = searches.clone();
    window.on_search(move |draft| captured.borrow_mut().push(draft.masks.to_string()));
    window.invoke_apply_preferences(0, 1, "".into(), 32, 32);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 320, 200, "find-mask-minimum-font32");
    let mask =
        modeless::accessible_item(&native, AccessibleRole::TextInput, "File name masks").unwrap();
    let viewport = scroll_parent(&mask);
    let area = bounds(&mask);
    let clip = bounds(&viewport);
    inside(&mask, 320.0, 200.0);
    assert!(
        area.1 >= clip.1 && area.3 <= clip.3 + 1.0,
        "the whole initial mask editor must be visible through its query clip: {area:?}, {clip:?}"
    );
    for label in ["Find", "Close"] {
        inside(
            &modeless::accessible_item(&native, AccessibleRole::Button, label).unwrap(),
            320.0,
            200.0,
        );
    }
    let empty = modeless::accessible_item(&native, AccessibleRole::Text, "No results").unwrap();
    assert!(
        empty.is_visible() && empty.geometry().height() >= 36.0,
        "the empty message needs its own complete line at font32"
    );
    inside(&empty, 320.0, 200.0);
    if let Some(header) = modeless::accessible_item(&native, AccessibleRole::Button, "Sort by Name")
    {
        assert!(
            bounds(&empty).1 >= bounds(&header).3,
            "the empty hint must sit below any visible column heading"
        );
    }
    let position = slint::LogicalPosition::new((area.0 + area.2) / 2.0, (area.1 + area.3) / 2.0);
    let button = slint::platform::PointerEventButton::Left;
    native
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    let keys: [slint::SharedString; 3] = [Key::End.into(), "X".into(), Key::Return.into()];
    for text in keys {
        native
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        native
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }
    assert_eq!(searches.borrow().last().map(String::as_str), Some("*.*X"));
    modeless::draw(&native, 860, 620, "find-empty-wide-font32");
    let header =
        modeless::accessible_item(&native, AccessibleRole::Button, "Sort by Name").unwrap();
    let empty = modeless::accessible_item(&native, AccessibleRole::Text, "No results").unwrap();
    assert!(
        bounds(&empty).1 >= bounds(&header).3,
        "normal-height empty results retain a separate header"
    );
    window.set_running(true);
    window.set_current_directory("D:\\文档\\current-folder".into());
    window.set_progress("42 files examined".into());
    modeless::draw(&native, 320, 200, "find-running-minimum-font32");
    for label in ["Stop", "Close"] {
        inside(
            &modeless::accessible_item(&native, AccessibleRole::Button, label).unwrap(),
            320.0,
            200.0,
        );
    }
    window.hide().unwrap();
}

#[test]
fn minimum_checksum_window_keeps_both_page_and_algorithm_pickers_usable_at_font32() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(0, 1, "".into(), 32, 32);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 320, 200, "checksum-navigation-minimum-font32");
    for label in ["Checksum page", "Output algorithm"] {
        let item = modeless::accessible_item(&native, AccessibleRole::Combobox, label).unwrap();
        inside(&item, 320.0, 200.0);
        assert!(
            item.geometry().width() >= 100.0,
            "{label} needs a readable trigger"
        );
    }
    window.set_page(4);
    modeless::draw(&native, 320, 200, "checksum-verify-minimum-font32");
    inside(
        &modeless::accessible_item(&native, AccessibleRole::Button, "Close").unwrap(),
        320.0,
        200.0,
    );
    window.hide().unwrap();
}

#[test]
fn small_progress_scrolls_chart_and_details_without_moving_close() {
    let mut test = ui::TestUi::new();
    test.window
        .invoke_apply_preferences(0, 1, "".into(), 32, 32);
    test.resize(320, 200);
    test.window.set_job(OperationRow {
        identity: "copy".into(), title: "Copy files".into(),
        detail: "Source\nD:\\文档\\source.txt\nDestination\nE:\\target.txt\nComplete diagnostics remain scrollable.".into(),
        cancellable: true, ..Default::default()
    });
    test.window.show().unwrap();
    test.draw();
    ui::click_popup(&test.window, "Details", AccessibleRole::Button);
    test.draw();
    let close = ui::popup_item(&test.window, "Close", AccessibleRole::Button);
    inside(&close, 320.0, 200.0);
    let before = bounds(&close);
    let chart = ui::popup_item(&test.window, "Transfer speed", AccessibleRole::Groupbox);
    let viewport = scroll_parent(&chart);
    inside(&viewport, 320.0, 200.0);
    let clip = bounds(&viewport);
    assert!(
        clip.3 - clip.1 >= 32.0,
        "compact progress must retain a readable diagnostics viewport: {clip:?}"
    );
    let chart_top = bounds(&chart).1;
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(clip.0 + 8.0, clip.1 + 8.0),
            delta_x: 0.0,
            delta_y: -10000.0,
        });
    for _ in 0..3 {
        test.draw();
    }
    assert!(
        bounds(&chart).1 < chart_top,
        "job content must actually scroll"
    );
    assert_eq!(
        bounds(&close),
        before,
        "Close stays outside scrolling diagnostics"
    );
    test.snapshot("jobs-scroll-minimum-font32");
    ui::key(&test.window, Key::Escape);
    assert!(
        !ui::has_popup(&test.window),
        "Escape must work after the focused list scrolls away"
    );
}
