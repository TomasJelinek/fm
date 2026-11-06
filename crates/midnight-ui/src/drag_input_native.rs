//! The UI thread releases only its own live native input grab. No payload/path I/O.
#[cfg(not(any(windows, target_os = "linux")))]
use {midnight_core::NativeDragWindow, slint::winit_030::winit::window::Window};

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "drag_input_windows.rs"]
mod implementation;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
#[path = "drag_input_linux.rs"]
mod implementation;
#[cfg(any(windows, target_os = "linux"))]
pub(super) use implementation::{host, release_input};

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn host(_: &Window) -> NativeDragWindow {
    NativeDragWindow::Unsupported
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn release_input(_: &Window) -> bool {
    false
}
