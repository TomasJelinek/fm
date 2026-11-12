use midnight_core::NativeDragWindow;
use slint::winit_030::winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

pub(crate) fn host(window: &Window) -> NativeDragWindow {
    match window.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => NativeDragWindow::Windows {
            hwnd: handle.hwnd.get() as usize,
        },
        _ => NativeDragWindow::Unsupported,
    }
}

pub(crate) fn release_input(window: &Window) -> bool {
    if !matches!(host(window), NativeDragWindow::Windows { .. }) {
        return false;
    }
    // SAFETY: Called synchronously on the UI event thread. ReleaseCapture acts
    // on this thread's own current grab, borrows no pointer and retains nothing.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
    }
    true
}
