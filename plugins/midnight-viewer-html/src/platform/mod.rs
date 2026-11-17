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
