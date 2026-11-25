//! Polish batch 3, the U-D05 leftover (docs/developer/acceptance/ui-polish.md, U-K30):
//! a WrappingCheckBox answers hovering, checked or not, and so does the Options › Colors
//! swatch. The hover look adds the theme's active frame around the box or swatch, which
//! in every theme stands out from the surface at least as much as the resting frame,
//! and changes no colour that a caption, check mark or colour sample is drawn on.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    window::{WindowInner, WindowKind},
};
use midnight_ui::{ColorOptionsView, Theme};
use slint::{
    Color, ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{Key, WindowEvent},
};
use std::{cell::Cell, time::Duration};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

/// Themes for the behaviour checks: Blue, Light and High contrast dark. Themes select
/// colours only (ui/theme.slint), so what answers the pointer is the same in each;
/// `hover_looks_keep_each_themes_contrast` shows the hover look in every theme.
const THEMES: [i32; 3] = [0, 2, 21];

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(found) = find(current, role, label) {
            return Some(found);
        }
    }
    None
}

fn dialog_item(test: &ui::TestUi, role: AccessibleRole, label: &str) -> ItemRc {
    let inner = WindowInner::from_pub(test.window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("a dialog is open");
    find(ItemRc::new_root(popup.component.clone()), role, label)
        .unwrap_or_else(|| panic!("no {label}"))
}

/// Scroll the scroll views around the dialog control `label` until it is inside them.
fn reveal(test: &mut ui::TestUi, role: AccessibleRole, label: &str) -> ItemRc {
    for _ in 0..40 {
        let item = dialog_item(test, role, label);
        let area = bounds(&item);
        let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        let mut scroll = None;
        while let Some(ancestor) = parent {
            if ancestor.downcast::<Flickable>().is_some() {
                let view = bounds(&ancestor);
                if area.1 < view.1 - 0.5 || area.1 + area.3 > view.1 + view.3 + 0.5 {
                    scroll = Some((view, area.1 < view.1));
                    break;
                }
            }
            parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        let Some((view, up)) = scroll else {
            return item;
        };
        // Over the view's scroll bar strip, which no page control covers.
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(view.0 + view.2 - 6.0, view.1 + view.3 / 2.0),
                delta_x: 0.0,
                delta_y: if up { 40.0 } else { -40.0 },
            });
        test.draw();
    }
    panic!("{label} never scrolls into view");
}

fn checked(item: &ItemRc) -> bool {
    item.accessible_string_property(AccessibleStringProperty::Checked)
        .is_some_and(|value| value == "true")
}

/// The rendered pixels of `area` and a two-pixel margin around it, as the gate's U-K30
/// compares them.
fn region(test: &ui::TestUi, (x, y, w, h): Rect) -> Vec<slint::Rgb8Pixel> {
    let left = (x - 2.0).floor().max(0.0) as usize;
    let top = (y - 2.0).floor().max(0.0) as usize;
    let right = ((x + w + 2.0).ceil() as usize).min(test.width);
    let bottom = ((y + h + 2.0).ceil() as usize).min(test.height);
    (top..bottom)
        .flat_map(|row| (left..right).map(move |column| row * test.width + column))
        .map(|index| test.pixels[index])
        .collect()
}

fn pointer_out(test: &mut ui::TestUi, clock: &Cell<Duration>) {
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerExited);
    ui::advance(clock, Duration::from_millis(200));
    test.draw();
}

/// The pixels of `item` before and while the pointer rests on its centre.
fn hovered(
    test: &mut ui::TestUi,
    clock: &Cell<Duration>,
    item: &ItemRc,
) -> (Vec<slint::Rgb8Pixel>, Vec<slint::Rgb8Pixel>) {
    let area = bounds(item);
    pointer_out(test, clock);
    let before = region(test, area);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(area.0 + area.2 / 2.0, area.1 + area.3 / 2.0),
        });
    ui::advance(clock, Duration::from_millis(200));
    test.draw();
    let after = region(test, area);
    pointer_out(test, clock);
    (before, after)
}

fn luminance(color: Color) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

fn contrast(first: Color, second: Color) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn pixel_color(pixel: slint::Rgb8Pixel) -> Color {
    Color::from_rgb_u8(pixel.r, pixel.g, pixel.b)
}

/// Options › General's check boxes, unchecked and checked, answer hovering.
#[test]
fn wrapping_check_boxes_answer_hovering_checked_or_not() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    test.resize(1000, 720);
    for scheme in THEMES {
        test.window.invoke_apply_preferences(scheme, 0);
        test.window.invoke_show_options();
        test.window.set_options_topic_index(0);
        test.draw();
        let mut seen = (false, false);
        for label in [
            "Show only filename search matches",
            "Show hidden files",
            "Always show file sizes in bytes",
            "Verify entire copied file with SHA-256 (recommended)",
        ] {
            let item = reveal(&mut test, AccessibleRole::Checkbox, label);
            let is_checked = checked(&item);
            if is_checked {
                seen.1 = true;
            } else {
                seen.0 = true;
            }
            let (before, after) = hovered(&mut test, &clock, &item);
            assert_ne!(
                before, after,
                "theme {scheme}: {label:?} (checked: {is_checked}) does not change when hovered"
            );
            // The caption answers too: the box lights while the pointer is on the text.
            let caption = bounds(&item);
            let (x, y, _, h) = caption;
            let on_text = slint::LogicalPosition::new(x + caption.2 + 24.0, y + h / 2.0);
            pointer_out(&mut test, &clock);
            let rest = region(&test, caption);
            test.window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: on_text });
            ui::advance(&clock, Duration::from_millis(200));
            test.draw();
            assert_ne!(
                rest,
                region(&test, caption),
                "theme {scheme}: {label:?}'s box does not answer the pointer on its caption"
            );
            pointer_out(&mut test, &clock);
        }
        assert_eq!(
            seen,
            (true, true),
            "checked and unchecked boxes were hovered"
        );
        ui::key(&test.window, Key::Escape);
        test.draw();
    }
}

/// The Colors page's swatches answer hovering.
#[test]
fn colour_swatches_answer_hovering() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    test.resize(1000, 720);
    let colors = test.window.global::<ColorOptionsView>();
    colors.set_labels(ModelRc::new(VecModel::from(vec![SharedString::from(
        "*.rs;*.toml · 文档",
    )])));
    colors.set_selected(0);
    for scheme in THEMES {
        test.window.invoke_apply_preferences(scheme, 0);
        test.window.invoke_show_options();
        test.window.set_options_topic_index(8);
        test.draw();
        for label in ["Choose Foreground", "Choose Background"] {
            let item = reveal(&mut test, AccessibleRole::Button, label);
            let (before, after) = hovered(&mut test, &clock, &item);
            assert_ne!(
                before, after,
                "theme {scheme}: {label:?} does not change when hovered"
            );
        }
        ui::key(&test.window, Key::Escape);
        test.draw();
    }
}

/// In every theme, a check box and a swatch answer hovering, the hover look's frame
/// stands out from the dialog surface at least as much as the resting frame, and the
/// pixels it changes are only that frame: the box's or swatch's inside keeps the
/// colour that its mark or sample is drawn on.
#[test]
fn hover_looks_keep_each_themes_contrast() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    let colors = test.window.global::<ColorOptionsView>();
    colors.set_labels(ModelRc::new(VecModel::from(vec![SharedString::from(
        "*.rs",
    )])));
    colors.set_selected(0);
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        test.window.invoke_apply_preferences(scheme, 0);
        let theme = test.window.global::<Theme>();
        let (surface, frame, active) = (
            theme.get_surface(),
            theme.get_frame(),
            theme.get_active_frame(),
        );
        assert!(
            contrast(active, surface) >= contrast(frame, surface) - 0.01,
            "theme {scheme}: the hover frame {active:?} stands out less than the frame {frame:?} on {surface:?}"
        );
        test.window.invoke_show_options();
        for (topic, role, label) in [
            (
                0,
                AccessibleRole::Checkbox,
                "Verify entire copied file with SHA-256 (recommended)",
            ),
            (8, AccessibleRole::Button, "Choose Foreground"),
        ] {
            test.window.set_options_topic_index(topic);
            test.draw();
            let item = reveal(&mut test, role, label);
            let (x, y, w, h) = bounds(&item);
            // The box is the 18px square centred in the check box's input column; the
            // swatch fills its button.
            let inner = if role == AccessibleRole::Checkbox {
                let side = 18.0;
                (
                    x + (w - side) / 2.0 + 3.0,
                    y + (h - side) / 2.0 + 3.0,
                    side - 6.0,
                    side - 6.0,
                )
            } else {
                (x + 3.0, y + 3.0, w - 6.0, h - 6.0)
            };
            pointer_out(&mut test, &clock);
            let (before, fill_before) = (region(&test, (x, y, w, h)), inside(&test, inner));
            test.window
                .window()
                .dispatch_event(WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(x + w / 2.0, y + h / 2.0),
                });
            ui::advance(&clock, Duration::from_millis(200));
            test.draw();
            assert_ne!(
                before,
                region(&test, (x, y, w, h)),
                "theme {scheme}: {label:?} does not change when hovered"
            );
            let fill_after = inside(&test, inner);
            assert_eq!(
                fill_before, fill_after,
                "theme {scheme}: hovering {label:?} changes what its mark or sample is drawn on"
            );
            pointer_out(&mut test, &clock);
        }
        ui::key(&test.window, Key::Escape);
        test.draw();
    }
}

/// The colours inside `area`, which lies 3px inside a frame of at most 2px.
fn inside(test: &ui::TestUi, (x, y, w, h): Rect) -> Vec<Color> {
    let (left, top) = (x.ceil() as usize, y.ceil() as usize);
    let (right, bottom) = ((x + w).floor() as usize, (y + h).floor() as usize);
    (top..bottom)
        .flat_map(|row| (left..right).map(move |column| (row, column)))
        .map(|(row, column)| pixel_color(test.pixels[row * test.width + column]))
        .collect()
}
