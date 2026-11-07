//! Main-window presentation of the shared native shortcut state.
use crate::MainWindow;
use slint::winit_030::winit::event::WindowEvent;

#[derive(Default)]
pub(crate) struct ShortcutState(midnight_ui_input::ShortcutState);

impl ShortcutState {
    pub(crate) fn handle(&mut self, window: &MainWindow, event: &WindowEvent) -> bool {
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
        consumed
    }
}
