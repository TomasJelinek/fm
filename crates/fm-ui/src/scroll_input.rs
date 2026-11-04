//! Observe native interaction without consuming row clicks or touch gestures.
use crate::MainWindow;
use slint::{
    ComponentHandle,
    winit_030::{
        EventResult, WinitWindowAccessor,
        winit::event::{DeviceId, ElementState, MouseButton, TouchPhase, WindowEvent},
    },
};
use std::collections::HashSet;

#[derive(Default)]
struct PressState {
    buttons: HashSet<(DeviceId, MouseButton)>,
    fingers: HashSet<(DeviceId, u64)>,
    touchpads: HashSet<DeviceId>,
}

impl PressState {
    fn observe(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
            } => {
                let key = (*device_id, *button);
                if *state == ElementState::Pressed {
                    self.buttons.insert(key);
                } else {
                    self.buttons.remove(&key);
                }
            }
            WindowEvent::Touch(touch) => {
                let key = (touch.device_id, touch.id);
                match touch.phase {
                    TouchPhase::Started => {
                        self.fingers.insert(key);
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.fingers.remove(&key);
                    }
                    TouchPhase::Moved => {}
                }
            }
            WindowEvent::MouseWheel {
                device_id, phase, ..
            } => match phase {
                TouchPhase::Started => {
                    self.touchpads.insert(*device_id);
                }
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    self.touchpads.remove(device_id);
                }
                TouchPhase::Moved => {}
            },
            WindowEvent::Focused(false) | WindowEvent::Destroyed => {
                self.buttons.clear();
                self.fingers.clear();
                self.touchpads.clear();
            }
            _ => {}
        }
        !self.buttons.is_empty() || !self.fingers.is_empty() || !self.touchpads.is_empty()
    }
}

pub(crate) fn install(window: &MainWindow) {
    let weak = window.as_weak();
    let mut state = PressState::default();
    let mut shortcuts = crate::native_shortcuts::ShortcutState::default();
    window.window().on_winit_window_event(move |_, event| {
        let active = state.observe(event);
        if let Some(window) = weak.upgrade() {
            // This callback runs only for a real Winit adapter. Software-window
            // tests retain the Slint logical-key fallback.
            window.set_native_shortcuts_authoritative(true);
            window.set_scroll_interaction_active(active);
            if shortcuts.handle(&window, event) {
                return EventResult::PreventDefault;
            }
        }
        EventResult::Propagate
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase},
    };

    fn mouse(button: MouseButton, state: ElementState) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button,
        }
    }

    fn touch(id: u64, phase: TouchPhase) -> WindowEvent {
        WindowEvent::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(10.0, 10.0),
            force: None,
            id,
        })
    }

    #[test]
    fn held_mouse_buttons_block_settling_until_every_button_is_released() {
        let mut state = PressState::default();
        assert!(state.observe(&mouse(MouseButton::Left, ElementState::Pressed)));
        assert!(state.observe(&mouse(MouseButton::Right, ElementState::Pressed)));
        assert!(state.observe(&mouse(MouseButton::Left, ElementState::Released)));
        assert!(!state.observe(&mouse(MouseButton::Right, ElementState::Released)));
    }

    #[test]
    fn held_touches_remain_active_until_release_cancel_or_focus_loss() {
        let mut state = PressState::default();
        assert!(state.observe(&touch(7, TouchPhase::Started)));
        assert!(state.observe(&touch(8, TouchPhase::Started)));
        assert!(state.observe(&touch(7, TouchPhase::Ended)));
        assert!(!state.observe(&touch(8, TouchPhase::Cancelled)));
        assert!(state.observe(&touch(8, TouchPhase::Started)));
        assert!(!state.observe(&WindowEvent::Focused(false)));
        assert!(!state.observe(&touch(8, TouchPhase::Moved)));
    }

    #[test]
    fn phased_touchpad_gesture_blocks_settling_but_unphased_wheel_does_not() {
        let mut state = PressState::default();
        let wheel = |phase| WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -3.5)),
            phase,
        };
        assert!(!state.observe(&wheel(TouchPhase::Moved)));
        assert!(state.observe(&wheel(TouchPhase::Started)));
        assert!(state.observe(&wheel(TouchPhase::Moved)));
        assert!(!state.observe(&wheel(TouchPhase::Ended)));
    }
}
