//! Post input only to the probe's own native window; do not move the desktop cursor.
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    winit_030::{
        WinitWindowAccessor,
        winit::raw_window_handle::{HasWindowHandle, RawWindowHandle},
    },
};

#[link(name = "user32")]
unsafe extern "system" {
    fn PostMessageW(window: *mut std::ffi::c_void, message: u32, w: usize, l: isize) -> i32;
}

pub(super) fn send(
    window: &ViewerWindow,
    delta: i16,
    timeline: &super::super::timeline::Trace,
) -> Result<(), String> {
    let scale = window.window().scale_factor();
    let x = ((window.get_content_left() + 30.0) * scale) as u32;
    let y = ((window.get_content_top() + 30.0) * scale) as u32;
    window
        .window()
        .with_winit_window(|native| {
            let handle = native.window_handle().map_err(|error| error.to_string())?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return Err("probe has no Win32 window".into());
            };
            let hwnd = handle.hwnd.get() as *mut std::ffi::c_void;
            // SAFETY: The borrowed handle belongs to this live probe window. Posted
            // messages carry integers, retain no pointers, and perform no file work.
            timeline.record("wheel_enqueue", f64::from(delta));
            let sent = unsafe {
                PostMessageW(hwnd, 0x0200, 0, (x | y << 16) as isize) != 0
                    && PostMessageW(hwnd, 0x020A, (delta as u16 as usize) << 16, 0) != 0
            };
            timeline.record("wheel_enqueued", f64::from(delta));
            if sent {
                Ok(())
            } else {
                Err("native wheel injection failed".into())
            }
        })
        .ok_or("probe has no native window")?
}

// These user-provided files are read-only diagnostic inputs on this Windows host.
// Platform-neutral fixture generation must not assume a personal drive layout.
pub(super) fn real_paths() -> Vec<std::path::PathBuf> {
    let directory = std::path::Path::new("D:/temp/ovclash/results/2026-09-07-mfg");
    ["manifest.json", "post-merge-backend-timings.md"]
        .map(|name| directory.join(name))
        .to_vec()
}
