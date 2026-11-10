use midnight_core::PathPickerOwner;
use slint::winit_030::winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

pub(super) fn owner(window: &Window) -> PathPickerOwner {
    match window.window_handle().map(|value| value.as_raw()) {
        Ok(RawWindowHandle::Xlib(value)) => PathPickerOwner::X11(value.window),
        Ok(RawWindowHandle::Xcb(value)) => PathPickerOwner::X11(u64::from(value.window.get())),
        // A Wayland surface pointer is not an exported xdg-foreign parent identifier.
        // The native portal remains usable as a separate unparented chooser.
        _ => PathPickerOwner::None,
    }
}
