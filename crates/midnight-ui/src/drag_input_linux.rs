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
        Ok(RawWindowHandle::Wayland(handle)) => {
            if !wayland_transport_enabled() {
                return NativeDragWindow::Unsupported;
            }
            let Ok(RawDisplayHandle::Wayland(display)) =
                window.display_handle().map(|display| display.as_raw())
            else {
                return NativeDragWindow::Unsupported;
            };
            let scale = window.scale_factor();
            if !(scale.is_finite() && scale > 0.0 && scale < 64.0) {
                return NativeDragWindow::Unsupported;
            }
            // The backend actor lends winit's display and this surface while the window
            // lives (`file_drag_drop_wayland.rs`); the bridge cancels that lifetime first.
            NativeDragWindow::Wayland {
                display: display.display.as_ptr() as usize,
                surface: handle.surface.as_ptr() as usize,
                scale_milli: (scale * 1000.0).round() as u32,
            }
        }
        _ => NativeDragWindow::Unsupported,
    }
}

/// `MIDNIGHT_WAYLAND_DND=0` keeps a native Wayland window to dragging between its own
/// panels, as before Wayland transport existed, should a compositor misbehave.
fn wayland_transport_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("MIDNIGHT_WAYLAND_DND").is_none_or(|value| value != "0"))
}

pub(crate) fn release_input(window: &Window) -> bool {
    let Ok(display) = window.display_handle() else {
        return false;
    };
    let display = match display.as_raw() {
        RawDisplayHandle::Xlib(display) => display,
        // The compositor moves the press's implicit grab to the drag that the actor
        // starts with its serial: there is no grab of this connection to release.
        RawDisplayHandle::Wayland(_) => {
            return matches!(host(window), NativeDragWindow::Wayland { .. });
        }
        _ => return false,
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
