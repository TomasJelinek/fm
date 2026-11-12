//! Exercise the real color picker without a settings store or main app window.
#[path = "support/modeless.rs"]
mod ui;

use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    import { ColorPicker } from "../ui/color-picker.slint";
    import { Button } from "std-widgets.slint";
    export component ColorFixture inherits Window {
        preferred-width: 480px; preferred-height: 580px;
        in property <color> initial: #315c8e;
        in-out property <bool> open: true;
        out property <color> chosen: #315c8e;
        out property <int> accepted-count;
        out property <int> cancelled-count;
        VerticalLayout {
            padding: 12px;
            if root.open : ColorPicker {
                initial: root.initial;
                accepted(value) => { root.chosen = value; root.accepted-count += 1; root.open = false; }
                cancelled => { root.cancelled-count += 1; root.open = false; }
            }
            Button { text: "Reopen"; clicked => { root.open = true; } }
        }
    }
}

#[test]
fn choosing_is_explicit_cancel_preserves_the_value_and_reopen_resets_the_draft() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = ColorFixture::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let draw = || ui::draw(&native, 480, 580, "color-picker");
    draw();
    ui::click(&native, "RGB 239, 68, 68");
    draw();
    assert_eq!(window.get_accepted_count(), 0);
    ui::click(&native, "Cancel");
    assert_eq!(window.get_cancelled_count(), 1);
    assert_eq!(window.get_chosen(), slint::Color::from_rgb_u8(49, 92, 142));
    draw();
    ui::click(&native, "Reopen");
    draw();
    ui::click(&native, "Use color");
    assert_eq!(window.get_chosen(), slint::Color::from_rgb_u8(49, 92, 142));
    assert_eq!(window.get_accepted_count(), 1);
    draw();
    ui::click(&native, "Reopen");
    draw();
    ui::click(&native, "RGB 59, 130, 246");
    for event in [
        WindowEvent::KeyPressed {
            text: Key::Space.into(),
        },
        WindowEvent::KeyReleased {
            text: Key::Space.into(),
        },
    ] {
        window.window().dispatch_event(event);
    }
    draw();
    assert_eq!(window.get_accepted_count(), 1);
    ui::click(&native, "Use color");
    assert_eq!(window.get_chosen(), slint::Color::from_rgb_u8(59, 130, 246));
    assert_eq!(window.get_accepted_count(), 2);
}
