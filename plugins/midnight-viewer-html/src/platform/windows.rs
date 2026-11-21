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

/// How CEF on Windows scrolls a wheel delta on one axis. It reads the delta as
/// `WHEEL_DELTA` units (120 per notch), which Chromium scrolls by the system's wheel
/// characters across or lines down, 100/3 DIPs each, computed in `f32` (CEF's
/// `TranslateWebWheelEvent`), and Chromium drops the fraction of a DIP that each event
/// scrolls. As measured on Windows 11 at three lines, one unit per DIP moved a page only
/// 5/6 of the host's distance, and a notch the host glides in 2.4-DIP frames, sent in
/// whole units, moved 47 of 60 DIPs. A unit scrolls 5/18 of a DIP per line, so above
/// three lines not every whole number of DIPs is a whole number of units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WheelScale {
    /// The lines (or characters) a notch scrolls.
    lines: u32,
}

/// The current settings, across and down: CEF reads them for every event, and so does
/// the provider.
pub(crate) fn wheel_scales() -> [WheelScale; 2] {
    let read = |action| {
        let mut count = DEFAULT_SCROLL_COUNT;
        // SAFETY: both actions write one u32 through the pointer, which outlives the call.
        let read = unsafe { SystemParametersInfoW(action, 0, (&raw mut count).cast(), 0) };
        (read != 0).then_some(count)
    };
    [SPI_GETWHEELSCROLLCHARS, SPI_GETWHEELSCROLLLINES].map(|action| WheelScale {
        lines: scroll_count(read(action)),
    })
}

/// The lines (or characters) a notch scrolls. None, or a setting that cannot scroll by
/// pixels (no lines, or a page per notch), keeps Windows' default of three.
fn scroll_count(count: Option<u32>) -> u32 {
    count
        .filter(|count| *count != 0 && *count != WHEEL_PAGESCROLL)
        .unwrap_or(DEFAULT_SCROLL_COUNT)
        .min(1 << 24)
}

impl WheelScale {
    #[cfg(test)]
    pub(crate) fn with_lines(lines: u32) -> Self {
        Self {
            lines: scroll_count(Some(lines)),
        }
    }

    /// The whole DIPs Chromium scrolls for one event of `units`, in CEF's `f32`
    /// arithmetic: a quotient or product just below a whole DIP loses that DIP.
    pub(crate) fn dips(self, units: u32) -> u32 {
        let notches = units as f32 / 120.0;
        let per_notch = self.lines as f32 * (100.0_f32 / 3.0);
        // Truncates toward zero, as Chromium does; saturates far beyond any page.
        (notches * per_notch) as u32
    }

    /// The fewest units that scroll the most whole DIPs not past `dips`, and that
    /// scroll: `dips` exactly whenever some number of units scrolls it, which three lines
    /// or fewer always allow. Zero units when not even one DIP fits, so an event never
    /// carries units whose scroll Chromium would drop entirely (or, for a remainder,
    /// would point the wrong way).
    pub(crate) fn step(self, dips: i64) -> super::WheelStep {
        let wanted = dips.unsigned_abs().min(super::MAX_WHEEL_DIPS as u64) as u32;
        // From the exact rate of 5 lines / 18 DIPs per unit, then corrected for `f32`.
        let rate = 5.0 * f64::from(self.lines) / 18.0;
        let mut units = ((f64::from(wanted) + 1.0) / rate) as u32 + 1;
        while units > 0 && self.dips(units) > wanted {
            units -= 1;
        }
        while self.dips(units + 1) <= wanted {
            units += 1;
        }
        let scrolled = self.dips(units);
        while units > 0 && self.dips(units - 1) == scrolled {
            units -= 1;
        }
        let sign = if dips < 0 { -1 } else { 1 };
        super::WheelStep {
            units: sign * units as i32,
            dips: sign as i64 * i64::from(scrolled),
        }
    }
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
