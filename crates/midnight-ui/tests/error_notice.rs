//! Dismissing presentation never clears the host's failure or grants capabilities.
#[path = "support/modeless.rs"]
mod ui;
use i_slint_core::{
    accessibility::AccessibilityAction,
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    import { ErrorNotice } from "../ui/error-notice.slint";
    export component NoticeFixture inherits Window {
        preferred-width: 320px; preferred-height: 200px;
        in-out property <string> error;
        in-out property <string> occurrence;
        in-out property <bool> compact;
        out property <bool> showing: notice.showing;
        out property <length> notice-height: notice.height;
        out property <int> dismiss-count;
        in-out property <string> copied;
        VerticalLayout {
            padding: 6px; spacing: 0px;
            Rectangle { vertical-stretch: 1; background: #252630; }
            notice := ErrorNotice {
                text: root.error; context-key: root.occurrence;
                copy-enabled: true;
                compact: root.compact;
                minimum-notice-height: root.compact ? 12px : 60px;
                preferred-notice-height: root.compact ? 12px : 100px;
                maximum-notice-height: root.compact ? 12px : 160px;
                copy-details(text) => { root.copied = text; }
                dismissed => { root.dismiss-count += 1; }
            }
        }
    }
}

fn image_item(item: ItemRc) -> Option<ItemRc> {
    if item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(image) = image_item(item) {
            return Some(image);
        }
    }
    None
}

#[test]
fn wide_notice_uses_its_width_and_keeps_actions_at_opposite_corners() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeFixture::new().unwrap();
    let message = "Cannot open folder. Access denied while reading the requested directory.\nPath\nD:\\資料\\protected\\folder";
    window.set_error(message.into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for (width, compact) in [(1550, false), (320, false), (320, true)] {
        window.set_compact(compact);
        ui::draw(
            &native,
            width,
            200,
            if compact {
                "error-notice-compact"
            } else {
                "error-notice-full-width"
            },
        );
        let text = ui::accessible_item(&native, AccessibleRole::Text, message).unwrap();
        assert!(
            text.geometry().width() >= width as f32 - 140.0,
            "message should fill the banner between its small action columns: {:?}",
            text.geometry()
        );
        let close =
            ui::accessible_item(&native, AccessibleRole::Button, "Dismiss message").unwrap();
        let copy =
            ui::accessible_item(&native, AccessibleRole::Button, "Copy error details").unwrap();
        let close_at = close.map_to_native_window(close.geometry().origin);
        let copy_at = copy.map_to_native_window(copy.geometry().origin);
        let glyph = image_item(copy.clone()).expect("Copy has an image");
        let glyph_at = glyph.map_to_native_window(glyph.geometry().origin);
        assert!(glyph_at.x >= copy_at.x && glyph_at.y >= copy_at.y);
        assert!(glyph_at.x + glyph.geometry().width() <= copy_at.x + copy.geometry().width());
        assert!(glyph_at.y + glyph.geometry().height() <= copy_at.y + copy.geometry().height());
        assert!(
            (glyph_at.x + glyph.geometry().width() / 2.0
                - copy_at.x
                - copy.geometry().width() / 2.0)
                .abs()
                < 0.1
        );
        assert!(
            (glyph_at.y + glyph.geometry().height() / 2.0
                - copy_at.y
                - copy.geometry().height() / 2.0)
                .abs()
                < 0.1
        );
        assert!(close_at.x + close.geometry().width() >= width as f32 - 22.0);
        assert!(
            copy_at.x < 45.0,
            "Copy belongs at the bottom-left: {copy_at:?}"
        );
        if compact {
            assert!(window.get_notice_height() <= 12.0);
        } else {
            assert!(
                window.get_notice_height() <= 100.1,
                "a short notice must not stretch beyond its preferred height: {}",
                window.get_notice_height()
            );
            assert!(copy_at.y > close_at.y + close.geometry().height());
        }
        ui::click(&native, "Copy error details");
        assert_eq!(window.get_copied(), message);
        for key in [Key::Return, Key::Space] {
            window.set_copied("".into());
            native
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            native
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            assert_eq!(window.get_copied(), message);
        }
        window.set_copied("".into());
        copy.accessible_action(&AccessibilityAction::Default);
        assert_eq!(window.get_copied(), message);
        assert!(window.get_showing());
    }
}

#[test]
fn long_notice_scrolls_while_copy_retains_the_complete_error() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeFixture::new().unwrap();
    let message = format!(
        "Access denied\n{}\nLast diagnostic line",
        "D:\\資料\\protected\\folder\n".repeat(30)
    );
    window.set_error(message.as_str().into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 320, 200, "error-notice-scroll-before");
    let text = ui::accessible_item(&native, AccessibleRole::Text, &message).unwrap();
    let before = text.map_to_native_window(text.geometry().origin).y;
    native
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(160.0, 160.0),
            delta_x: 0.0,
            delta_y: -10_000.0,
        });
    ui::draw(&native, 320, 200, "error-notice-scroll-after");
    assert!(text.map_to_native_window(text.geometry().origin).y < before - 20.0);
    ui::click(&native, "Copy error details");
    assert_eq!(window.get_copied(), message);
    assert!(window.get_showing());
}

#[test]
fn close_collapses_notice_but_new_occurrences_remain_visible() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeFixture::new().unwrap();
    let message = "Archive plugin unavailable\nD:\\資料\\example.zip";
    window.set_error(message.into());
    window.set_occurrence("request-1".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-before");
    assert!(window.get_showing());
    assert!(window.get_notice_height() > 0.0);
    ui::click(&native, "Copy error details");
    assert_eq!(window.get_copied(), message);
    assert!(window.get_showing(), "copying must not dismiss the error");
    ui::click(&native, "Dismiss message");
    ui::draw(&native, 320, 200, "error-close-after");
    assert!(!window.get_showing());
    assert_eq!(window.get_notice_height(), 0.0);
    assert_eq!(window.get_error(), message);
    assert_eq!(window.get_dismiss_count(), 1);

    window.set_occurrence("request-2".into());
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-new-occurrence");
    assert!(
        window.get_showing(),
        "identical diagnostics from a new request must reappear"
    );
    ui::click(&native, "Dismiss message");
    window.set_error("A different error".into());
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-new-text");
    assert!(window.get_showing());
    window.set_error("".into());
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-empty");
    assert!(!window.get_showing());
    assert_eq!(window.get_notice_height(), 0.0);
}
