//! Native press/gesture lifetime shared by the panel and auxiliary wheel observers.
use slint::winit_030::winit::event::{
    DeviceId, ElementState, MouseButton, TouchPhase, WindowEvent,
};
use std::collections::HashSet;
#[derive(Default)]
/// Tracks all held native buttons, fingers, and phased touchpad gestures.
pub struct PressState {
    buttons: HashSet<(DeviceId, MouseButton)>,
    fingers: HashSet<(DeviceId, u64)>,
    touchpads: HashSet<DeviceId>,
}

impl PressState {
    pub fn observe(&mut self, event: &WindowEvent) -> bool {
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
        self.active()
    }

    pub fn has_touches(&self) -> bool {
        !self.fingers.is_empty()
    }

    pub fn release_left_button(&mut self) {
        self.buttons
            .retain(|(_, button)| *button != MouseButton::Left);
    }

    pub fn active(&self) -> bool {
        !self.buttons.is_empty() || !self.fingers.is_empty() || !self.touchpads.is_empty()
    }
}
