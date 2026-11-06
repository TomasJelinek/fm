//! Match the Windows Winit backend's key metadata, including Ctrl+Alt/AltGr
//! disambiguation. No native input is injected into the user's desktop.
use i_slint_core::input::{InternalKeyEvent, KeyEvent, KeyEventType};
use midnight_ui::MainWindow;
use slint::{ComponentHandle, SharedString, platform::WindowEvent};

pub(crate) fn key(window: &MainWindow, text: SharedString) {
    for event_type in [KeyEventType::KeyPressed, KeyEventType::KeyReleased] {
        let mut key_event = KeyEvent::default();
        key_event.text = text.clone();
        window
            .window()
            .dispatch_event(WindowEvent::internal(InternalKeyEvent {
                key_event,
                text_without_modifiers: text.clone(),
                event_type,
                ..Default::default()
            }));
    }
}
