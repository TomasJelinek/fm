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

/// One axis of one wheel event as the platform's CEF reads it (`WheelScale::step`):
/// the units sent and the whole DIPs Chromium scrolls for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WheelStep {
    pub units: i32,
    pub dips: i64,
}

/// The most DIPs one wheel event asks for on an axis, far beyond any page. It bounds the
/// units of an event and the DIPs a gesture carries.
pub(crate) const MAX_WHEEL_DIPS: i64 = 1 << 20;
