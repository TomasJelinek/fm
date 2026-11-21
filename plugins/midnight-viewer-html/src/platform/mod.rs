#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;

/// Windows virtual key for CEF's `windows_key_code` on every platform.
pub(crate) fn key_code(key: &midnight_web_viewer_api::Key) -> i32 {
    crate::key_codes::windows_key_code(key)
}

/// How the platform's CEF reads a wheel delta, across and down: its units per DIP, and
/// the fewest units that scroll a whole number of DIPs. Chromium drops a fraction of a
/// pixel from each wheel event, so every event carries whole quanta only.
pub(crate) struct WheelUnits {
    pub per_dip: [f64; 2],
    pub quantum: [i32; 2],
}
