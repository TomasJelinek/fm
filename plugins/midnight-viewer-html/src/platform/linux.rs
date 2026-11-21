//! Linux uses Chromium's namespace/seccomp or installed setuid helper sandbox.
//! No Windows sandbox pointer applies; no --no-sandbox fallback is added.
use std::ffi::{CString, c_char, c_int};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::ffi::OsStringExt;

pub(crate) type NativeKeyEvent = cef::sys::XEvent;

pub(crate) fn run() -> i32 {
    // Raw argv bytes: a non-UTF-8 installation or profile path must not panic before
    // CEF starts (std::env::args, used by cef::args::Args, panics on such entries).
    let Ok(source) = std::env::args_os()
        .map(|argument| CString::new(argument.into_vec()))
        .collect::<Result<Vec<_>, _>>()
    else {
        eprintln!("HTML viewer arguments contain a NUL byte");
        return 70;
    };
    let mut argv: Vec<*mut c_char> = source
        .iter()
        .map(|argument| argument.as_ptr().cast_mut())
        .collect();
    let Ok(argc) = c_int::try_from(argv.len()) else {
        return 70;
    };
    argv.push(std::ptr::null_mut());
    // `source` and `argv` outlive run(), which owns the whole CEF process lifetime.
    let args = cef::MainArgs {
        argc,
        argv: argv.as_mut_ptr(),
    };
    crate::native::run(&args, std::ptr::null_mut())
}

pub(crate) fn window_info() -> cef::WindowInfo {
    cef::WindowInfo::default().set_as_windowless(0)
}

/// On Linux CEF reads a wheel delta as DIPs: a unit is a DIP, on both axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WheelScale;

pub(crate) fn wheel_scales() -> [WheelScale; 2] {
    [WheelScale; 2]
}

impl WheelScale {
    /// `dips` whole DIPs, as as many units.
    pub(crate) fn step(self, dips: i64) -> super::WheelStep {
        let dips = dips.clamp(-super::MAX_WHEEL_DIPS, super::MAX_WHEEL_DIPS);
        super::WheelStep {
            units: dips as i32,
            dips,
        }
    }
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

unsafe extern "C" {
    fn dup2(old: c_int, new: c_int) -> c_int;
}

/// Reserves the MWV1 pipe for the browser-process writer before CEF starts any child.
/// Chromium children inherit descriptors 0-2, so a stray stdout write from the zygote,
/// a renderer, the GPU process or a native library would corrupt the protocol. Keep a
/// private close-on-exec duplicate and point descriptor 1 at stderr (or /dev/null).
pub(crate) fn protocol_output() -> std::io::Result<impl std::io::Write + Send + 'static> {
    let protocol = std::io::stdout().as_fd().try_clone_to_owned()?;
    // SAFETY: dup2 only atomically replaces descriptor 1 with a duplicate of an open
    // descriptor; it reads or writes no Rust-owned memory.
    if unsafe { dup2(2, 1) } < 0 {
        let null = std::fs::OpenOptions::new().write(true).open("/dev/null")?;
        // SAFETY: as above; `null` stays open for the duration of the call.
        if unsafe { dup2(null.as_raw_fd(), 1) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(std::io::BufWriter::with_capacity(
        64 * 1024,
        std::fs::File::from(protocol),
    ))
}
