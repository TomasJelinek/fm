use midnight_core::NativeDragWindow;
use slint::winit_030::winit::{
    raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle},
    window::Window,
};

pub(crate) fn host(window: &Window) -> NativeDragWindow {
    match window.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Xlib(handle)) => u32::try_from(handle.window)
            .ok()
            .map_or(NativeDragWindow::Unsupported, |window| {
                NativeDragWindow::X11 { window }
            }),
        _ => NativeDragWindow::Unsupported,
    }
}

pub(crate) fn release_input(window: &Window) -> bool {
    let Ok(display) = window.display_handle() else {
        return false;
    };
    let RawDisplayHandle::Xlib(display) = display.as_raw() else {
        return false;
    };
    let Some(display) = display.display else {
        return false;
    };
    let Ok(xlib) = x11_dl::xlib::Xlib::open() else {
        return false;
    };
    // SAFETY: Winit lends its live Xlib connection for this UI-thread call. The
    // pointer is never stored, freed or shared; only this connection's pointer
    // grab is released and flushed before the worker acquires its own grab.
    unsafe {
        (xlib.XUngrabPointer)(display.as_ptr().cast(), x11_dl::xlib::CurrentTime);
        (xlib.XFlush)(display.as_ptr().cast());
    }
    true
}
