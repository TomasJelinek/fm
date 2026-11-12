//! X11 transient/work-area hints. Wayland uses compositor placement and the application gate.
#![allow(unsafe_code)]
use slint::winit_030::winit::{
    platform::x11::{WindowAttributesExtX11, WindowType},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle},
    window::{Window, WindowAttributes},
};
use std::{
    ffi::{CStr, c_ulong},
    ptr,
};
use x11_dl::xlib;

pub(super) fn configure(
    attributes: WindowAttributes,
    _owner: &Window,
    role: super::WindowRole,
) -> WindowAttributes {
    attributes.with_x11_window_type(vec![if role == super::WindowRole::Dialog {
        WindowType::Dialog
    } else {
        WindowType::Normal
    }])
}

pub(crate) fn set_enabled(_window: &Window, _enabled: bool) {
    // X11 and Wayland do not expose a native input-enable API. The shared owner
    // lease drives the actual Slint input gate, including native shortcut filtering.
}

pub(super) fn activate(window: &Window) -> bool {
    window.set_minimized(false);
    window.focus_window();
    // Public Winit cannot request Wayland activation. X11 processes this request asynchronously.
    xlib_handles(window).is_some()
}

fn xlib_handles(window: &Window) -> Option<(*mut xlib::Display, c_ulong)> {
    match (
        window.display_handle().ok()?.as_raw(),
        window.window_handle().ok()?.as_raw(),
    ) {
        (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
            Some((display.display?.as_ptr().cast(), window.window))
        }
        _ => None,
    }
}

pub(super) fn attach(owner: &Window, child: &Window, role: super::WindowRole) {
    if role == super::WindowRole::WorkingTool {
        return;
    }
    let (Some((display, owner_id)), Some((child_display, child_id))) =
        (xlib_handles(owner), xlib_handles(child))
    else {
        return;
    };
    if display != child_display {
        return;
    }
    let Ok(xlib) = xlib::Xlib::open() else { return };
    // SAFETY: these XIDs/display pointers are borrowed from live Winit windows on
    // their UI thread. Xlib consumes the numbers synchronously and retains no Rust data.
    unsafe {
        (xlib.XSetTransientForHint)(display, child_id, owner_id);
        (xlib.XFlush)(display);
    }
}

/// Copy a bounded CARDINAL property, releasing Xlib's allocation on every path.
fn cardinal_property(
    xlib: &xlib::Xlib,
    display: *mut xlib::Display,
    name: &CStr,
) -> Option<Vec<c_ulong>> {
    let mut actual_type = 0;
    let mut format = 0;
    let mut count = 0;
    let mut remaining = 0;
    let mut bytes = ptr::null_mut();
    // SAFETY: caller holds a live borrowed Xlib display on its owning UI thread.
    // Output pointers reference initialized stack locals; XGetWindowProperty's
    // allocation is copied with its returned length and then freed with XFree.
    unsafe {
        let atom = (xlib.XInternAtom)(display, name.as_ptr(), xlib::True);
        if atom == 0 {
            return None;
        }
        let status = (xlib.XGetWindowProperty)(
            display,
            (xlib.XDefaultRootWindow)(display),
            atom,
            0,
            1024,
            xlib::False,
            xlib::XA_CARDINAL,
            &mut actual_type,
            &mut format,
            &mut count,
            &mut remaining,
            &mut bytes,
        );
        let result = if status == xlib::Success as i32
            && actual_type == xlib::XA_CARDINAL
            && format == 32
            && count <= 1024
            && !bytes.is_null()
        {
            Some(std::slice::from_raw_parts(bytes.cast::<c_ulong>(), count as usize).to_vec())
        } else {
            None
        };
        if !bytes.is_null() {
            (xlib.XFree)(bytes.cast());
        }
        result
    }
}

pub(super) fn work_area(window: &Window) -> Option<(i32, i32, u32, u32)> {
    // Absolute top-level placement is unavailable on public Winit/Wayland.
    let (display, _) = xlib_handles(window)?;
    let monitor = window
        .current_monitor()
        .or_else(|| window.primary_monitor())?;
    let position = monitor.position();
    let size = monitor.size();
    let fallback = (position.x, position.y, size.width, size.height);
    let Ok(xlib) = xlib::Xlib::open() else {
        return Some(fallback);
    };
    let desktop = cardinal_property(&xlib, display, c"_NET_CURRENT_DESKTOP")
        .and_then(|values| values.first().copied())
        .unwrap_or(0);
    let Some(work_areas) = cardinal_property(&xlib, display, c"_NET_WORKAREA") else {
        return Some(fallback);
    };
    let offset = usize::try_from(desktop).ok()?.checked_mul(4)?;
    let Some(area) = work_areas.get(offset..offset.checked_add(4)?) else {
        return Some(fallback);
    };
    // EWMH positions are signed 32-bit values transported in CARDINAL slots.
    let left = i64::from(area[0] as u32 as i32).max(i64::from(position.x));
    let top = i64::from(area[1] as u32 as i32).max(i64::from(position.y));
    let right = (i64::from(area[0] as u32 as i32) + i64::from(area[2] as u32))
        .min(i64::from(position.x) + i64::from(size.width));
    let bottom = (i64::from(area[1] as u32 as i32) + i64::from(area[3] as u32))
        .min(i64::from(position.y) + i64::from(size.height));
    if right <= left || bottom <= top {
        return Some(fallback);
    }
    Some((
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    ))
}
