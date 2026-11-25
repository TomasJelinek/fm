//! Primary and destructive actions answer hovering and pressing, and on every palette
//! their caption and glyph keep the palette's text contrast on each state's fill.
use crate::support::modeless as ui;
use crate::themes::{contrast, text_minimum};

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, ImageItem, SimpleText},
};
use slint::{
    Color, ComponentHandle,
    platform::{
        PointerEventButton, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow,
    },
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    // Exported so that the scheme is not a compile-time constant, which would copy the
    // whole palette into every color binding of the fixture.
    import { Theme } from "../ui/theme.slint";
    export { Theme }
    import { ActionButton, FallbackIcons } from "../ui/action-icons.slint";
    export component ActionStatesFixture inherits Window {
        preferred-width: 360px; preferred-height: 80px;
        background: Theme.surface;
        HorizontalLayout {
            padding: 16px; spacing: 16px; alignment: start;
            ActionButton { text: "Copy"; primary: true; fallback-icon: FallbackIcons.copy; }
            ActionButton { text: "Delete"; destructive: true; }
        }
    }
}

const WIDTH: u32 = 360;
const HEIGHT: u32 = 80;

/// What a button draws in one state: its fill beside the content, its caption's colour
/// and its glyph's tint.
#[derive(Debug, PartialEq)]
struct Paint {
    fill: Color,
    caption: Color,
    glyph: Color,
}

fn descendant<T>(item: &ItemRc, read: &dyn Fn(&ItemRc) -> Option<T>) -> Option<T> {
    if item.is_visible()
        && let Some(found) = read(item)
    {
        return Some(found);
    }
    let mut child = item.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        if let Some(found) = descendant(&next, read) {
            return Some(found);
        }
    }
    None
}

fn paint(native: &MinimalSoftwareWindow, label: &str, state: &str) -> Paint {
    let pixels = ui::render(native, WIDTH, HEIGHT, &format!("action-{label}-{state}"));
    let button = ui::accessible_item(native, AccessibleRole::Button, label).unwrap();
    let rect = button.geometry();
    let origin = button.map_to_native_window(rect.origin);
    // Inside the frame and left of the content's padding.
    let (x, y) = (origin.x + 5.0, origin.y + rect.height() / 2.0);
    let pixel = pixels[y as usize * WIDTH as usize + x as usize];
    let caption = descendant(&button, &|item| {
        item.downcast::<SimpleText>()
            .map(|text| text.as_pin_ref().color().color())
            .or_else(|| {
                item.downcast::<ComplexText>()
                    .map(|text| text.as_pin_ref().color().color())
            })
    })
    .expect("caption");
    let glyph = descendant(&button, &|item| {
        item.downcast::<ImageItem>()
            .map(|image| image.as_pin_ref().colorize().color())
    })
    .expect("glyph");
    Paint {
        fill: Color::from_rgb_u8(pixel.r, pixel.g, pixel.b),
        caption,
        glyph,
    }
}

#[test]
fn primary_and_destructive_actions_answer_hover_and_press_readably_in_every_palette() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let fixture = ActionStatesFixture::new().unwrap();
    fixture.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::render(&native, WIDTH, HEIGHT, "action-states");
    let outside = slint::LogicalPosition::new(WIDTH as f32 - 4.0, HEIGHT as f32 - 4.0);
    let window = native.window();
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        fixture.global::<Theme>().set_scheme(scheme);
        let minimum = text_minimum(scheme);
        for label in ["Copy", "Delete"] {
            let button = ui::accessible_item(&native, AccessibleRole::Button, label).unwrap();
            let rect = button.geometry();
            let origin = button.map_to_native_window(rect.origin);
            let center = slint::LogicalPosition::new(
                origin.x + rect.width() / 2.0,
                origin.y + rect.height() / 2.0,
            );
            window.dispatch_event(WindowEvent::PointerExited);
            let rest = paint(&native, label, "rest");
            window.dispatch_event(WindowEvent::PointerMoved { position: center });
            let hovered = paint(&native, label, "hovered");
            window.dispatch_event(WindowEvent::PointerPressed {
                position: center,
                button: PointerEventButton::Left,
            });
            let pressed = paint(&native, label, "pressed");
            // Released outside: no click, so no focus ring in the next palette.
            window.dispatch_event(WindowEvent::PointerMoved { position: outside });
            window.dispatch_event(WindowEvent::PointerReleased {
                position: outside,
                button: PointerEventButton::Left,
            });
            window.dispatch_event(WindowEvent::PointerExited);
            let case = format!("{label} in palette {scheme}");
            assert_ne!(hovered.fill, rest.fill, "{case}: hovering changes nothing");
            assert_ne!(pressed.fill, hovered.fill, "{case}: pressing looks hovered");
            assert_ne!(pressed.fill, rest.fill, "{case}: pressing changes nothing");
            for (state, drawn) in [
                ("rest", &rest),
                ("hovered", &hovered),
                ("pressed", &pressed),
            ] {
                for (part, ink) in [("caption", drawn.caption), ("glyph", drawn.glyph)] {
                    let ratio = contrast(ink, drawn.fill);
                    assert!(
                        ratio >= minimum,
                        "{case}, {state}: {part} {ink:?} on {:?} is {ratio:.2}:1, below {minimum}:1",
                        drawn.fill
                    );
                }
            }
        }
    }
}
