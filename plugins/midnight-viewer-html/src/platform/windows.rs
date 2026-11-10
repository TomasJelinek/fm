//! Windows sandbox bootstrap ABI from pinned include/cef_sandbox_win.h.
use std::ffi::c_void;

pub(crate) type NativeKeyEvent = cef::sys::MSG;

#[repr(C)]
struct VersionPrefix {
    size: usize,
    cef_major: i32,
    cef_minor: i32,
    cef_patch: i32,
    cef_commit: i32,
    chrome_major: i32,
    chrome_minor: i32,
    chrome_build: i32,
    chrome_patch: i32,
}

/// Exact five-argument C entry called by the official matching CEF bootstrap.
/// No fallback to a null sandbox pointer or unsandboxed launch is permitted.
#[unsafe(no_mangle)]
pub extern "C" fn RunWinMain(
    instance: *mut c_void,
    _command_line: *mut u16,
    _show: i32,
    sandbox_info: *mut c_void,
    version_info: *const c_void,
) -> i32 {
    if instance.is_null() || sandbox_info.is_null() || version_info.is_null() {
        return 70;
    }
    // SAFETY: the official bootstrap owns and passes cef_version_info_t. Its stable prefix
    // begins with size and the eight version integers. No tail field is read.
    let version = unsafe { &*version_info.cast::<VersionPrefix>() };
    if version.size < std::mem::size_of::<VersionPrefix>()
        || version.cef_major != 156
        || version.cef_minor != 0
        || version.cef_patch != 26
        || version.chrome_major != 156
        || version.chrome_build != 8078
        || version.chrome_patch != 88
    {
        return 70;
    }
    let args = cef::MainArgs {
        instance: cef::sys::HINSTANCE(instance.cast()),
    };
    // Bootstrap owns sandbox_info for the full call; the same exact pointer is used for
    // execute_process and initialize, including renderer/GPU/network subprocess dispatch.
    std::panic::catch_unwind(|| crate::native::run(&args, sandbox_info.cast())).unwrap_or(70)
}

pub(crate) fn window_info() -> cef::WindowInfo {
    cef::WindowInfo::default().set_as_windowless(cef::sys::HWND(std::ptr::null_mut()))
}

pub(crate) fn native_key_code(key: &midnight_web_viewer_api::Key) -> i32 {
    // Chromium accepts Win32 virtual keys; non-layout text is delivered separately as UTF-16.
    super::key_code(key)
}
