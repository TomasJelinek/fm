//! Synthetic user input dispatched as window events, exactly as a platform backend
//! delivers keys and pointer events; nothing here calls presenter methods.
use super::inspect;
use i_slint_core::items::AccessibleRole;
use slint::{
    LogicalPosition, SharedString,
    platform::{Key, PointerEventButton, WindowEvent},
};

pub fn press(window: &slint::Window, text: impl Into<SharedString>) {
    window.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
}

pub fn release(window: &slint::Window, text: impl Into<SharedString>) {
    window.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

/// Press and release one key (a character or a [`Key`]).
pub fn key(window: &slint::Window, text: impl Into<SharedString>) {
    let text = text.into();
    press(window, text.clone());
    release(window, text);
}

/// Hold `modifiers`, press `text`, release in reverse order.
pub fn chord(window: &slint::Window, modifiers: &[Key], text: impl Into<SharedString>) {
    for modifier in modifiers {
        press(window, *modifier);
    }
    key(window, text);
    for modifier in modifiers.iter().rev() {
        release(window, *modifier);
    }
}

/// Type text one character at a time.
pub fn type_text(window: &slint::Window, text: &str) {
    for character in text.chars() {
        key(window, SharedString::from(character.to_string()));
    }
}

/// Select everything in the focused editor and replace it with `text`.
pub fn replace_text(window: &slint::Window, text: &str) {
    chord(window, &[Key::Control], "a");
    key(window, Key::Backspace);
    type_text(window, text);
}

pub fn move_to(window: &slint::Window, position: LogicalPosition) {
    window.dispatch_event(WindowEvent::PointerMoved { position });
}

pub fn click_at(window: &slint::Window, position: LogicalPosition) {
    let button = PointerEventButton::Left;
    move_to(window, position);
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// A click with modifier keys held (for example Ctrl+click on a tree expander).
pub fn click_with(window: &slint::Window, modifiers: &[Key], position: LogicalPosition) {
    for modifier in modifiers {
        press(window, *modifier);
    }
    click_at(window, position);
    for modifier in modifiers.iter().rev() {
        release(window, *modifier);
    }
}

#[allow(dead_code)] // Extension API for scenarios that open rows by mouse.
pub fn double_click_at(window: &slint::Window, position: LogicalPosition) {
    click_at(window, position);
    click_at(window, position);
}

/// Press at `from`, move in steps to `to`, release there.
#[allow(dead_code)] // Extension API for drag-and-drop scenarios.
pub fn drag(window: &slint::Window, from: LogicalPosition, to: LogicalPosition, steps: u32) {
    let button = PointerEventButton::Left;
    move_to(window, from);
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    for step in 1..=steps.max(1) {
        let fraction = step as f32 / steps.max(1) as f32;
        move_to(
            window,
            LogicalPosition::new(
                from.x + (to.x - from.x) * fraction,
                from.y + (to.y - from.y) * fraction,
            ),
        );
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
}

/// Click the visible control with this role and label, as a user would: a control a
/// scroll view shows only in part is first scrolled into view with the mouse wheel,
/// then the middle of its visible part is clicked.
pub fn click(window: &slint::Window, role: AccessibleRole, label: &str) -> Result<(), String> {
    let find = || {
        inspect::find(window, role, label).ok_or_else(|| {
            format!(
                "no visible {role:?} {label:?}; visible text: {:?}",
                inspect::visible_texts(window)
            )
        })
    };
    let mut item = find()?;
    let mut wheel = -48.0_f32;
    let mut previous_gap = f32::MAX;
    for _ in 0..24 {
        let full = inspect::rect(&item);
        let shown = inspect::visible_rect(&item);
        if shown.is_some_and(|(_, _, w, h)| h >= full.3 * 0.9 && w >= full.2.min(48.0) * 0.9) {
            break;
        }
        let Some(clip) = inspect::clip_rect(&item) else {
            break;
        };
        let clip_middle = clip.1 + clip.3 / 2.0;
        let gap = (full.1 + full.3 / 2.0 - clip_middle).abs();
        if gap > previous_gap + 0.5 {
            // That wheel direction moved the control away: turn around.
            wheel = -wheel;
        }
        previous_gap = gap;
        let direction = if full.1 + full.3 / 2.0 > clip_middle {
            1.0
        } else {
            -1.0
        };
        window.dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(clip.0 + clip.2 / 2.0, clip_middle),
            delta_x: 0.0,
            delta_y: wheel * direction,
        });
        inspect::settle_tree(window);
        item = find()?;
    }
    let position = inspect::visible_rect(&item).map_or_else(
        || inspect::center(&item),
        |(x, y, w, h)| LogicalPosition::new(x + w / 2.0, y + h / 2.0),
    );
    let size = window.size().to_logical(window.scale_factor());
    if position.x < 0.0 || position.y < 0.0 || position.x >= size.width || position.y >= size.height
    {
        return Err(format!(
            "{role:?} {label:?} is outside the window at {position:?}"
        ));
    }
    click_at(window, position);
    Ok(())
}

pub fn click_button(window: &slint::Window, label: &str) -> Result<(), String> {
    click(window, AccessibleRole::Button, label)
}

/// Focus a text field by clicking it, then replace its text.
pub fn fill(window: &slint::Window, label: &str, text: &str) -> Result<(), String> {
    click(window, AccessibleRole::TextInput, label)?;
    replace_text(window, text);
    Ok(())
}
