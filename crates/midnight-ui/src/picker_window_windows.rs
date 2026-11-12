use midnight_core::PathPickerOwner;
use slint::winit_030::winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

pub(super) fn owner(window: &Window) -> PathPickerOwner {
    match window.window_handle().map(|value| value.as_raw()) {
        Ok(RawWindowHandle::Win32(value)) => PathPickerOwner::Windows(value.hwnd.get() as usize),
        _ => PathPickerOwner::None,
    }
}
