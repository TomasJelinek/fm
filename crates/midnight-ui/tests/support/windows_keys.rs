//! Match the Windows Winit backend's key metadata, including Ctrl+Alt/AltGr
//! disambiguation. No native input is injected into the user's desktop.
use i_slint_core::input::{InternalKeyEvent, KeyEvent, KeyEventType};
use midnight_ui::MainWindow;
use slint::{ComponentHandle, SharedString, platform::WindowEvent};

pub(crate) fn key(window: &MainWindow, text: SharedString) {
    key_edge(window, text.clone(), true);
    key_edge(window, text, false);
}

pub(crate) fn key_edge(window: &MainWindow, text: SharedString, pressed: bool) {
    let mut key_event = KeyEvent::default();
    key_event.text = text.clone();
    window
        .window()
        .dispatch_event(WindowEvent::internal(InternalKeyEvent {
            key_event,
            text_without_modifiers: text,
            event_type: if pressed {
                KeyEventType::KeyPressed
            } else {
                KeyEventType::KeyReleased
            },
            ..Default::default()
        }));
}
