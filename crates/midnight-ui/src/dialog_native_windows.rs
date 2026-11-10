//! Audited UI-thread user32 monitor query; ownership and enable use safe Winit APIs.
#![allow(unsafe_code)]
use slint::winit_030::winit::{
    platform::windows::{MonitorHandleExtWindows, WindowAttributesExtWindows, WindowExtWindows},
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::{Window, WindowAttributes},
};
use std::ffi::c_void;

#[cfg(test)]
#[path = "dialog_native_windows_tests.rs"]
mod tests;

pub(super) fn configure(
    attributes: WindowAttributes,
    owner: &Window,
    role: super::WindowRole,
) -> WindowAttributes {
    if role == super::WindowRole::WorkingTool {
        return attributes
            .with_skip_taskbar(false)
            .with_drag_and_drop(false);
    }
    match owner.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => attributes
            .with_owner_window(handle.hwnd.get())
            .with_skip_taskbar(true)
            .with_drag_and_drop(false),
        _ => attributes,
    }
}

pub(crate) fn set_enabled(window: &Window, enabled: bool) {
    window.set_enable(enabled);
}

pub(super) fn attach(_owner: &Window, child: &Window, role: super::WindowRole) {
    // The creation hook supplied owner HWND, avoiding WS_EX_APPWINDOW in Winit itself.
    child.set_skip_taskbar(role == super::WindowRole::Dialog);
}

#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetMonitorInfoW(monitor: *mut c_void, information: *mut MonitorInfo) -> i32;
    fn IsIconic(window: isize) -> i32;
    fn ShowWindowAsync(window: isize, command: i32) -> i32;
    fn SetForegroundWindow(window: isize) -> i32;
}

pub(super) fn activate(window: &Window) -> bool {
    let Ok(RawWindowHandle::Win32(handle)) = window.window_handle().map(|handle| handle.as_raw())
    else {
        return false;
    };
    // SAFETY: HWND is borrowed from a live Winit window on its UI thread. These
    // asynchronous/scalar requests retain no Rust pointers. SetForegroundWindow
    // obeys Windows foreground rules; no synthetic keys or thread attachment is used.
    unsafe {
        if IsIconic(handle.hwnd.get()) != 0 {
            ShowWindowAsync(handle.hwnd.get(), 9); // SW_RESTORE
        }
        SetForegroundWindow(handle.hwnd.get()) != 0
    }
}

pub(super) fn work_area(window: &Window) -> Option<(i32, i32, u32, u32)> {
    let monitor = window
        .current_monitor()
        .or_else(|| window.primary_monitor())?;
    let mut information = MonitorInfo {
        size: std::mem::size_of::<MonitorInfo>() as u32,
        monitor: Rect::default(),
        work: Rect::default(),
        flags: 0,
    };
    // SAFETY: the monitor belongs to a borrowed live Winit window; the correctly sized
    // writable MONITORINFO lives throughout this synchronous call and is never retained.
    if unsafe { GetMonitorInfoW(monitor.hmonitor() as *mut c_void, &mut information) } == 0 {
        return None;
    }
    let rect = information.work;
    Some((
        rect.left,
        rect.top,
        u32::try_from(i64::from(rect.right) - i64::from(rect.left)).ok()?,
        u32::try_from(i64::from(rect.bottom) - i64::from(rect.top)).ok()?,
    ))
}
