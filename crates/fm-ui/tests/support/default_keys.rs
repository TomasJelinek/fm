//! Platform-neutral synthetic key dispatch, selected outside the Windows adapter.
use fm_ui::MainWindow;
use slint::{ComponentHandle, SharedString, platform::WindowEvent};

pub(super) fn key(window: &MainWindow, text: SharedString) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
