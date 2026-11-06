//! Windows Winit reports the original key to distinguish AltGr text from shortcuts.
use i_slint_core::input::{InternalKeyEvent, KeyEvent, KeyEventType};
use midnight_ui::MainWindow;
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};

pub(super) fn remapped_altgr(window: &MainWindow) {
    for modifier in [Key::Control, Key::Alt] {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: modifier.into(),
        });
    }
    for event_type in [KeyEventType::KeyPressed, KeyEventType::KeyReleased] {
        let mut key_event = KeyEvent::default();
        key_event.text = "@".into();
        window
            .window()
            .dispatch_event(WindowEvent::internal(InternalKeyEvent {
                key_event,
                text_without_modifiers: "q".into(),
                event_type,
                ..Default::default()
            }));
    }
    for modifier in [Key::Alt, Key::Control] {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: modifier.into(),
        });
    }
}
