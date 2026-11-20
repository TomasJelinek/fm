//! Main-window presentation of the shared native shortcut state.
use crate::MainWindow;
use slint::winit_030::winit::{event::WindowEvent, keyboard::Key};

#[derive(Default)]
pub(crate) struct ShortcutState(midnight_ui_input::ShortcutState);

impl ShortcutState {
    pub(crate) fn handle(&mut self, window: &MainWindow, event: &WindowEvent) -> bool {
        // Before the dispatch below: a held panel keeps this text with the key's
        // shortcut (`held_input`). It is what Slint itself would type for a
        // character key that no shortcut takes.
        if let WindowEvent::KeyboardInput { event: key, .. } = event {
            window.set_native_key_text(match &key.logical_key {
                Key::Character(text) => text.as_str().into(),
                _ => key.text.as_deref().unwrap_or_default().into(),
            });
        }
        let consumed = self.0.handle(event, |chord| {
            window.invoke_physical_shortcut(
                chord.key.into(),
                chord.control,
                chord.alt,
                chord.shift,
                chord.meta,
            )
        });
        window.set_native_digit_event(self.0.physical_digit());
        window.set_native_alt_gr(self.0.alt_gr());
        window.set_control_layer(self.0.control_layer());
        window.set_function_modifiers(i32::from(self.0.function_modifiers()));
        consumed
    }
}
