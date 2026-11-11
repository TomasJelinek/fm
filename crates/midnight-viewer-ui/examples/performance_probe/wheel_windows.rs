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

pub(super) fn send(window: &ViewerWindow, delta: i16) -> Result<(), String> {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    eprintln!("wheel_send epoch_ms={} delta={delta}", epoch.as_millis());
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
            let sent = unsafe {
                PostMessageW(hwnd, 0x0200, 0, (x | y << 16) as isize) != 0
                    && PostMessageW(hwnd, 0x020A, (delta as u16 as usize) << 16, 0) != 0
            };
            if sent {
                Ok(())
            } else {
                Err("native wheel injection failed".into())
            }
        })
        .ok_or("probe has no native window")?
}
