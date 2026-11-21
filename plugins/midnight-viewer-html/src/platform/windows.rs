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

/// MWV1 event output. Child-process stdout inheritance is a POSIX launch concern
/// (see the Linux adapter); Windows keeps writing the process standard output.
pub(crate) fn protocol_output() -> std::io::Result<impl std::io::Write + Send + 'static> {
    Ok(std::io::stdout())
}

const SPI_GETWHEELSCROLLLINES: u32 = 0x0068;
const SPI_GETWHEELSCROLLCHARS: u32 = 0x006C;
/// `SPI_GETWHEELSCROLLLINES` when a notch scrolls a whole page.
const WHEEL_PAGESCROLL: u32 = u32::MAX;
/// Windows' defaults for both settings.
const DEFAULT_SCROLL_COUNT: u32 = 3;

#[link(name = "user32")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, parameter: u32, value: *mut c_void, ini: u32) -> i32;
}

/// CEF on Windows reads a wheel delta as `WHEEL_DELTA` units (120 per notch), which
/// Chromium scrolls by the system's wheel characters across or lines down, 100/3 DIPs
/// each (CEF's `TranslateWebWheelEvent`). As measured on Windows 11 at three lines, one
/// unit per DIP moved a page only 5/6 of the host's distance, and a notch the host
/// glides in 2.4-DIP frames moved 47 of 60 DIPs: Chromium drops the fraction of a pixel
/// each event scrolls. The host's deltas are already the distance to scroll.
pub(crate) fn wheel_units() -> super::WheelUnits {
    let read = |action| {
        let mut count = DEFAULT_SCROLL_COUNT;
        // SAFETY: both actions write one u32 through the pointer, which outlives the call.
        let read = unsafe { SystemParametersInfoW(action, 0, (&raw mut count).cast(), 0) };
        (read != 0).then_some(count)
    };
    let [across, down] = [
        scroll_count(read(SPI_GETWHEELSCROLLCHARS)),
        scroll_count(read(SPI_GETWHEELSCROLLLINES)),
    ];
    super::WheelUnits {
        per_dip: [units_per_dip(across), units_per_dip(down)],
        quantum: [quantum(across), quantum(down)],
    }
}

/// The lines (or characters) a notch scrolls. None, or a setting that cannot scroll by
/// pixels (no lines, or a page per notch), keeps Windows' default of three.
fn scroll_count(count: Option<u32>) -> u32 {
    count
        .filter(|count| *count != 0 && *count != WHEEL_PAGESCROLL)
        .unwrap_or(DEFAULT_SCROLL_COUNT)
}

/// CEF units per DIP when a notch scrolls `count` lines of 100/3 DIPs: 18/(5 count).
fn units_per_dip(count: u32) -> f64 {
    120.0 / (f64::from(count) * 100.0 / 3.0)
}

/// The fewest units that scroll whole DIPs: `u` units scroll `5 count u / 18` DIPs.
fn quantum(count: u32) -> i32 {
    let mut a = 18;
    let mut b = 5 * count.min(1 << 24);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    (18 / a) as i32
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
