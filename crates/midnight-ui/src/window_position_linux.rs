//! Borrowed X11 startup-placement hints. Wayland placement belongs to its compositor.
#![allow(unsafe_code)]

use slint::winit_030::{
    WinitWindowAccessor,
    winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    },
};
use std::{ffi::c_ulong, mem::MaybeUninit};
use x11_dl::xlib;

#[cfg(test)]
#[path = "window_position_linux_tests.rs"]
mod tests;

fn xlib_identity(
    display: RawDisplayHandle,
    window: RawWindowHandle,
) -> Option<(*mut xlib::Display, c_ulong)> {
    match (display, window) {
        (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
            Some((display.display?.as_ptr().cast(), window.window))
        }
        _ => None,
    }
}

fn position_hints(
    mut hints: xlib::XSizeHints,
    position: slint::PhysicalPosition,
) -> xlib::XSizeHints {
    hints.flags |= xlib::USPosition;
    hints.x = position.x;
    hints.y = position.y;
    hints
}

pub(super) fn restore(window: &slint::Window, position: slint::PhysicalPosition) -> bool {
    window
        .with_winit_window(|native| {
            let (Ok(display), Ok(handle)) = (native.display_handle(), native.window_handle())
            else {
                return false;
            };
            let Some((display, window)) = xlib_identity(display.as_raw(), handle.as_raw()) else {
                // Native Wayland exposes neither absolute origins nor a setter for them.
                return false;
            };
            let Ok(xlib) = xlib::Xlib::open() else {
                return false;
            };
            let mut hints = MaybeUninit::<xlib::XSizeHints>::zeroed();
            let mut supplied = 0;
            // SAFETY: this display/XID pair is borrowed from the same live Winit window
            // on its UI thread. Xlib writes only the provided, correctly sized outputs.
            // The initialized integer-only hints are used only after a successful read.
            let status = unsafe {
                (xlib.XGetWMNormalHints)(display, window, hints.as_mut_ptr(), &mut supplied)
            };
            if status == 0 {
                // An absent/unreadable property cannot be replaced without losing hints.
                return false;
            }
            // SAFETY: zero initialization plus the successful XGetWMNormalHints output
            // initializes every integer field, including fields whose flag is unset.
            let mut hints = position_hints(unsafe { hints.assume_init() }, position);
            // SAFETY: the same borrowed display/window remains live. Xlib copies the
            // stack-owned hints synchronously; it acquires no Rust or display ownership.
            // Flush before Winit's move/map so the WM observes USPosition at first show.
            unsafe {
                (xlib.XSetWMNormalHints)(display, window, &mut hints);
                (xlib.XFlush)(display);
            }
            native.set_outer_position(slint::winit_030::winit::dpi::PhysicalPosition::new(
                position.x, position.y,
            ));
            true
        })
        .unwrap_or(false)
}
