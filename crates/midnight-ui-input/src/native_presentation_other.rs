use slint::winit_030::winit::{
    event::WindowEvent,
    window::{Window, WindowId},
};

/// Native presentation needs no workaround on this platform.
#[derive(Default)]
pub struct NativePresentation {}

impl NativePresentation {
    /// Observe a native transition without consuming or drawing the event.
    pub fn observe(
        &mut self,
        _: WindowId,
        _: Option<&Window>,
        _: Option<&slint::Window>,
        _: &WindowEvent,
    ) {
    }
}
