//! Native child input helpers; no desktop input or application callbacks are synthesized.
#![allow(dead_code)]
use i_slint_core::item_tree::ItemRc;
use slint::{
    SharedString,
    platform::{
        Key, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::MinimalSoftwareWindow,
    },
};

pub fn key(window: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

pub fn chord(window: &MinimalSoftwareWindow, modifiers: &[Key], text: impl Into<SharedString>) {
    for modifier in modifiers {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: (*modifier).into(),
        });
    }
    key(window, text);
    for modifier in modifiers.iter().rev() {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: (*modifier).into(),
        });
    }
}

pub fn click(window: &MinimalSoftwareWindow, item: &ItemRc) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    assert!(item.is_visible() && position.x >= 0.0 && position.y >= 0.0);
    let size = window.window().size();
    assert!(position.x < size.width as f32 && position.y < size.height as f32);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}
