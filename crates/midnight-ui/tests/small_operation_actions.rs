//! Native operation confirmations remain usable in a 320×200 host.
#[path = "support/operation_form.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable, FocusScope},
    window::WindowInner,
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

fn primary_caption(test: &ui::TestUi, title: &str) -> ItemRc {
    let action = ui::root_item(&test.window, title, AccessibleRole::Button);
    if action.downcast::<FocusScope>().is_some() {
        // The stationary proxy owns accessibility. Pointer/scroll regressions
        // still exercise the underlying native caption itself.
        ui::root_item(&test.window, title, AccessibleRole::None)
    } else {
        action
    }
}

fn named_buttons(item: ItemRc, title: &str) -> usize {
    let mut count = usize::from(
        item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|label| label == title),
    );
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        count += named_buttons(item, title);
    }
    count
}

fn assert_focused_primary(test: &ui::TestUi, title: &str) {
    let inner = WindowInner::from_pub(test.window.window());
    let focus = inner
        .focus_item
        .borrow()
        .upgrade()
        .expect("primary action needs focus");
    assert_eq!(focus.accessible_role(), AccessibleRole::Button);
    assert_eq!(
        focus
            .accessible_string_property(AccessibleStringProperty::Label)
            .unwrap(),
        title
    );
    assert_eq!(
        focus
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .unwrap(),
        "true"
    );
    assert_eq!(
        named_buttons(ItemRc::new_root(inner.component()), title),
        1,
        "one accessible primary action"
    );
    let (left, top, right, bottom) = bounds(&focus);
    inside(test, &focus, "focused primary viewport");
    let expected = test
        .window
        .global::<midnight_ui::Theme>()
        .get_active_frame();
    for (x, y) in [
        (left.floor() as usize + 1, ((top + bottom) / 2.0) as usize),
        (right.ceil() as usize - 2, ((top + bottom) / 2.0) as usize),
        (((left + right) / 2.0) as usize, top.floor() as usize + 1),
        (((left + right) / 2.0) as usize, bottom.ceil() as usize - 2),
    ] {
        let pixel = test.pixels[y * test.width + x];
        for (got, want) in [
            (pixel.r, expected.red()),
            (pixel.g, expected.green()),
            (pixel.b, expected.blue()),
        ] {
            assert!(
                got.abs_diff(want) <= 2,
                "visible focus boundary at {x},{y}: {pixel:?}, expected {expected:?}"
            );
        }
    }
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
    test.window
        .invoke_apply_preferences(0, 1, "".into(), font, font);
    test.resize(320, 200);
    settle(test);
}

#[test]
fn short_rename_summary_uses_a_compact_dialog_and_long_summaries_can_scroll() {
    let mut test = ui::TestUi::new();
    test.resize(1340, 900);
    test.window.set_kind(0);
    test.window.set_title_text("Rename".into());
    test.window.set_confirm_text("Rename".into());
    test.window.set_has_name(true);
    test.window.set_initial_name("example.txt".into());
    test.window.set_body("example.txt\n1 024 bytes".into());
    test.window.invoke_begin();
    settle(&mut test);
    let cancel = ui::root_item(&test.window, "Cancel", AccessibleRole::Button);
    let short_height = test.preferred_height();
    assert!(
        short_height < 400.0,
        "short form wastes vertical space: {short_height}"
    );
    ui::snapshot(&test, "compact-rename-content");
    test.window.set_body(
        "A longer description with a Unicode path C:\\文档\\example.txt\n"
            .repeat(30)
            .into(),
    );
    settle(&mut test);
    let long_height = test.preferred_height();
    assert!(long_height > short_height + 100.0);
    inside(&test, &cancel, "Cancel");
    small(&mut test, 32);
    inside(&test, &cancel, "Cancel after resize");
    ui::snapshot(&test, "compact-rename-long-small");
}

#[test]
fn long_operation_confirmations_scroll_and_keep_cancel_visible() {
    let mut test = ui::TestUi::new();
    small(&mut test, 32);
    let confirmed = Rc::new(RefCell::new(Vec::new()));
    let captured = confirmed.clone();
    test.window
        .on_confirm(move |name| captured.borrow_mut().push(name));
    let cancelled = Rc::new(Cell::new(0));
    let captured = cancelled.clone();
    test.window
        .on_request_dismiss(move || captured.set(captured.get() + 1));
    for (kind, title, has_name) in [
        (1, "Move to Trash", false),
        (2, "Permanently delete", false),
        (3, "Create directory", true),
    ] {
        test.window.set_kind(kind);
        test.window.set_title_text(title.into());
        test.window.set_confirm_text(title.into());
        test.window.set_has_name(has_name);
        test.window.set_initial_name("new folder".into());
        test.window.invoke_begin();
        settle(&mut test);
        let cancel = ui::root_item(&test.window, "Cancel", AccessibleRole::Button);
        inside(&test, &cancel, "Cancel");
        let confirm = primary_caption(&test, title);
        let viewport = scroll_parent(&confirm);
        inside(&test, &viewport, title);
        // A form instance retains its scroll offset across presentation updates.
        scroll(&mut test, &viewport, 1000.0, 0.0);
        let before = bounds(&confirm).0;
        let count = confirmed.borrow().len();
        ui::snapshot(&test, &format!("operation-{kind}-before-320"));
        scroll(&mut test, &viewport, -1000.0, 0.0);
        assert!(
            bounds(&confirm).0 < before,
            "{title} must be horizontally scrollable: action {:?}, viewport {:?}, previous x {before}",
            bounds(&confirm),
            bounds(&viewport)
        );
        assert_eq!(confirmed.borrow().len(), count, "scrolling cannot confirm");
        inside(&test, &cancel, "Cancel after scrolling");
        ui::snapshot(&test, &format!("operation-{kind}-320"));
        click_visible(&mut test, &confirm, &viewport);
        assert_eq!(confirmed.borrow().len(), count + 1);
        assert_focused_primary(&test, title);
        assert_eq!(confirmed.borrow().last().unwrap().as_str(), "new folder");
        ui::key(&test.window, Key::Escape);
        settle(&mut test);
        assert_eq!(
            cancelled.get(),
            kind,
            "Escape must cancel operation {kind} after scrolling and clicking its primary action"
        );
        settle(&mut test);
    }
    assert_eq!(cancelled.get(), 3);
}

#[test]
fn fitting_primary_focus_uses_the_native_caption_width() {
    let mut test = ui::TestUi::new();
    test.window.set_kind(0);
    test.window.set_title_text("Rename".into());
    test.window.set_confirm_text("Rename".into());
    test.window.set_has_name(false);
    test.window.invoke_begin();
    settle(&mut test);
    let caption = primary_caption(&test, "Rename");
    let viewport = scroll_parent(&caption);
    assert!(caption.geometry().width() < viewport.geometry().width());
    let semantic = ui::root_item(&test.window, "Rename", AccessibleRole::Button);
    assert!(
        (semantic.geometry().width() - caption.geometry().width()).abs() < 1.0,
        "fitting focus must not outline unused footer width"
    );
    assert!(
        (bounds(&semantic).0 - bounds(&caption).0).abs() < 1.0,
        "fitting focus must follow the right-aligned caption"
    );
    assert_focused_primary(&test, "Rename");
}

#[test]
fn scrolling_primary_confirmation_keeps_default_enter_and_escape() {
    let mut test = ui::TestUi::new();
    small(&mut test, 32);
    let confirmed = Rc::new(Cell::new(0));
    let captured = confirmed.clone();
    test.window
        .on_confirm(move |_| captured.set(captured.get() + 1));
    let cancelled = Rc::new(Cell::new(0));
    let captured = cancelled.clone();
    test.window
        .on_request_dismiss(move || captured.set(captured.get() + 1));
    for (kind, title) in [(1, "Move to Trash"), (2, "Permanently delete")] {
        test.window.set_kind(kind);
        test.window.set_title_text(title.into());
        test.window.set_confirm_text(title.into());
        test.window.set_has_name(false);
        test.window.invoke_begin();
        settle(&mut test);
        let confirm = primary_caption(&test, title);
        let viewport = scroll_parent(&confirm);
        scroll(&mut test, &viewport, 1000.0, 0.0);
        scroll(&mut test, &viewport, -1000.0, 0.0);
        assert!(
            bounds(&confirm).0 < bounds(&viewport).0,
            "the primary button's focus origin must be scrolled outside its viewport"
        );
        assert_eq!(
            confirmed.get(),
            (kind - 1) * 2,
            "wheel input never confirms"
        );
        assert_focused_primary(&test, title);
        ui::key(&test.window, Key::Tab);
        settle(&mut test);
        assert_eq!(
            named_buttons(
                ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
                title
            ),
            1,
            "unfocused proxy must not duplicate the native action"
        );
        ui::key(&test.window, Key::Tab);
        settle(&mut test);
        assert_focused_primary(&test, title);
        // No pointer click can restore focus before these keys.
        ui::key(&test.window, Key::Return);
        assert_eq!(
            confirmed.get(),
            (kind - 1) * 2 + 1,
            "Return keeps the primary action after scrolling"
        );
        assert_eq!(cancelled.get(), kind - 1, "Return must not select Cancel");
        ui::key(&test.window, Key::Space);
        assert_eq!(
            confirmed.get(),
            kind * 2,
            "Space activates the focused primary action"
        );
        ui::key(&test.window, Key::Escape);
        settle(&mut test);
        assert_eq!(
            cancelled.get(),
            kind,
            "Escape remains available after scrolling"
        );
        settle(&mut test);
    }
}
