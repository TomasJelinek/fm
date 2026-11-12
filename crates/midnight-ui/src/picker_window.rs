//! UI geometry only; picker enumeration and native dialog ownership stay in backend services.
use crate::MainWindow;
use midnight_core::PathPickerOwner;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};

#[cfg(windows)]
#[path = "picker_window_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "picker_window_linux.rs"]
mod native;

pub(crate) fn owner(window: &MainWindow) -> PathPickerOwner {
    native_path_picker_owner(window.window())
}

/// Extract presentation-only native ownership for backend file pickers.
pub fn native_path_picker_owner(window: &slint::Window) -> PathPickerOwner {
    let mut result = PathPickerOwner::None;
    window.with_winit_window(|window| result = native::owner(window));
    result
}
