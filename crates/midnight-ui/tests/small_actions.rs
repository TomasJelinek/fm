//! Fallback action menus remain usable and scrollable in a 320×200 host.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    window::{WindowInner, WindowKind},
};
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn settle(test: &mut ui::TestUi) {
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let origin = item.map_to_native_window(item.geometry().origin);
    (
        origin.x,
        origin.y,
        origin.x + item.geometry().width(),
        origin.y + item.geometry().height(),
    )
}

fn inside(test: &ui::TestUi, item: &ItemRc, label: &str) {
    let (left, top, right, bottom) = bounds(item);
    assert!(
        left >= 0.0
            && top >= 0.0
            && right <= test.width as f32 + 0.5
            && bottom <= test.height as f32 + 0.5,
        "{label} outside viewport: {:?}",
        bounds(item)
    );
}

fn scroll_parent(item: &ItemRc) -> ItemRc {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            return item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("overflowing action needs a scrollable viewport");
}

fn scroll(test: &mut ui::TestUi, viewport: &ItemRc, x: f32, y: f32) {
    let (left, top, right, bottom) = bounds(viewport);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new((left + right) / 2.0, (top + bottom) / 2.0),
            delta_x: x,
            delta_y: y,
        });
    settle(test);
}

fn click_visible(test: &mut ui::TestUi, item: &ItemRc, viewport: &ItemRc) {
    let (left, top, right, bottom) = bounds(item);
    let (clip_left, clip_top, clip_right, clip_bottom) = bounds(viewport);
    let visible = (
        left.max(clip_left),
        top.max(clip_top),
        right.min(clip_right),
        bottom.min(clip_bottom),
    );
    assert!(
        visible.2 - visible.0 >= 24.0 && visible.3 - visible.1 >= item.geometry().height() - 0.5,
        "action has no complete visible row: {:?} in {:?}",
        bounds(item),
        bounds(viewport)
    );
    ui::click(
        &test.window,
        (visible.0 + visible.2) / 2.0,
        (visible.1 + visible.3) / 2.0,
    );
    settle(test);
}

fn small(test: &mut ui::TestUi, font: i32) {
    test.window.invoke_apply_fonts("".into(), font, font);
    test.window.invoke_apply_preferences(0, 1);
    test.resize(320, 200);
    settle(test);
}

#[test]
fn f9_and_f10_menus_scroll_to_the_final_action_without_dismissing() {
    let mut test = ui::TestUi::new();
    let exits = Rc::new(Cell::new(0));
    let captured = exits.clone();
    test.window
        .on_exit(move || captured.set(captured.get() + 1));
    for font in [15, 32] {
        small(&mut test, font);
        for (menu, label) in [(1, "Close"), (2, "Exit")] {
            test.window.invoke_show_app_menu(menu);
            settle(&mut test);
            let action = ui::popup_item(&test.window, label, AccessibleRole::Button);
            let viewport = scroll_parent(&action);
            inside(&test, &viewport, "menu scroll viewport");
            let (_, top, right, bottom) = bounds(&viewport);
            ui::click(&test.window, right - 5.0, (top + bottom) / 2.0);
            settle(&mut test);
            assert!(
                ui::has_popup(&test.window),
                "scrollbar clicks keep the menu open"
            );
            let before = bounds(&action).1;
            scroll(&mut test, &viewport, 0.0, -10000.0);
            assert!(ui::has_popup(&test.window), "scrolling keeps the menu open");
            assert!(
                bounds(&action).1 < before,
                "final menu row must scroll into view"
            );
            render::snapshot(&test, &format!("command-menu-{menu}-{font}-320"));
            click_visible(&mut test, &action, &viewport);
            assert!(
                !ui::has_popup(&test.window),
                "selected menu action closes its popup"
            );
            test.window.invoke_show_app_menu(menu);
            settle(&mut test);
            ui::key(&test.window, Key::Escape);
            settle(&mut test);
            assert!(!ui::has_popup(&test.window), "Escape closes the menu");
        }
    }
    assert_eq!(exits.get(), 2);
}

#[test]
fn fallback_file_menu_is_capped_scrollable_and_keyboard_reveals_selection() {
    let mut test = ui::TestUi::new();
    small(&mut test, 32);
    let choices = Rc::new(RefCell::new(Vec::new()));
    let captured = choices.clone();
    test.window
        .on_file_menu_action(move |token, action| captured.borrow_mut().push((token, action)));
    test.window.set_file_menu_mutable(true);
    test.window.set_file_menu_token("menu:1".into());
    settle(&mut test);
    let popups = WindowInner::from_pub(test.window.window()).active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .unwrap();
    inside(
        &test,
        &ItemRc::new_root(popup.component.clone()),
        "file menu popup",
    );
    drop(popups);
    let properties = ui::popup_item(&test.window, "Properties", AccessibleRole::Button);
    let viewport = scroll_parent(&properties);
    // Open -> Copy -> Cut -> Paste -> Rename -> Trash -> Properties.
    for _ in 0..6 {
        ui::key(&test.window, Key::DownArrow);
        settle(&mut test);
    }
    render::snapshot(&test, "fallback-menu-320");
    assert!(
        bounds(&properties).1 >= bounds(&viewport).1
            && bounds(&properties).3 <= bounds(&viewport).3 + 0.5,
        "keyboard selection must reveal the complete final row"
    );
    ui::key(&test.window, Key::Return);
    settle(&mut test);
    assert_eq!(choices.borrow().last().unwrap().1, 5);
    assert!(!ui::has_popup(&test.window));
    test.window.set_file_menu_token("menu:2".into());
    settle(&mut test);
    let properties = ui::popup_item(&test.window, "Properties", AccessibleRole::Button);
    let viewport = scroll_parent(&properties);
    scroll(&mut test, &viewport, 0.0, -1000.0);
    click_visible(&mut test, &properties, &viewport);
    assert_eq!(choices.borrow().len(), 2);
}
