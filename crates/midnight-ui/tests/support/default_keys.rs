//! Platform-neutral synthetic key dispatch, selected outside the Windows adapter.
use midnight_ui::MainWindow;
use slint::{ComponentHandle, SharedString, platform::WindowEvent};

pub(crate) fn key(window: &MainWindow, text: SharedString) {
    key_edge(window, text.clone(), true);
    key_edge(window, text, false);
}

pub(crate) fn key_edge(window: &MainWindow, text: SharedString, pressed: bool) {
    window.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    });
}
