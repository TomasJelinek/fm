//! Linux uses Chromium's namespace/seccomp or installed setuid helper sandbox.
//! No Windows sandbox pointer applies; no --no-sandbox fallback is added.
pub(crate) type NativeKeyEvent = cef::sys::XEvent;

pub(crate) fn run() -> i32 {
    let args = cef::args::Args::new();
    crate::native::run(args.as_main_args(), std::ptr::null_mut())
}

pub(crate) fn window_info() -> cef::WindowInfo {
    cef::WindowInfo::default().set_as_windowless(0)
}

pub(crate) fn native_key_code(key: &midnight_web_viewer_api::Key) -> i32 {
    // X11 virtual key conversion is explicit and separate from Windows implementation.
    // Wayland/xkb qualification remains a release gate for this platform adapter.
    match key.code.as_str() {
        "ArrowLeft" => 113,
        "ArrowRight" => 114,
        "ArrowUp" => 111,
        "ArrowDown" => 116,
        "Enter" => 36,
        "Space" => 65,
        "Backspace" => 22,
        "Tab" => 23,
        "Escape" => 9,
        _ => 0,
    }
}
