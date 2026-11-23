//! The native windows this process shows outside Slint, such as the Shell's property
//! sheet that Properties opens on Windows (`Config::native_properties`): their class,
//! title and controls' text, a picture of one for the report, and a polite close. The
//! harness's own windows are software-rendered and have no native window, so every
//! visible top-level window of this process is one the system opened.
#![allow(unsafe_code, reason = "audited Win32 window enumeration and capture")]
use std::ffi::c_void;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
            DESKTOPHORZRES, DESKTOPVERTRES, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
            GetDIBits, GetDeviceCaps, HORZRES, ReleaseDC, SelectObject, VERTRES,
        },
        UI::WindowsAndMessaging::{
            EnumChildWindows, EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW,
            GetWindowThreadProcessId, IsWindow, IsWindowVisible, PostMessageW, WM_CLOSE,
        },
    },
    core::BOOL,
};

/// Render the window's whole content, including DWM-composed parts.
const PW_RENDERFULLCONTENT: u32 = 2;

// Declared here: the suite's `windows` dependency leaves out `Win32_Storage_Xps`, the
// feature that carries this one user32 function.
#[link(name = "user32")]
unsafe extern "system" {
    fn PrintWindow(window: *mut c_void, context: *mut c_void, flags: u32) -> i32;
}

/// One visible top-level window of this process.
#[derive(Clone, Debug)]
pub struct NativeWindow {
    handle: isize,
    pub class: String,
    pub title: String,
    /// Every child control's non-empty text, in Z order.
    pub texts: Vec<String>,
}

impl NativeWindow {
    fn hwnd(&self) -> HWND {
        HWND(self.handle as *mut c_void)
    }

    /// Whether the window still exists.
    pub fn exists(&self) -> bool {
        // SAFETY: Win32 validates the handle.
        unsafe { IsWindow(Some(self.hwnd())) }.as_bool()
    }

    /// Ask the window to close, as its title bar's × does.
    pub fn close(&self) {
        // SAFETY: a posted WM_CLOSE carries no pointers; Win32 validates the handle.
        let _ = unsafe { PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }

    /// The window's pixels as shown: width, height and rows top to bottom.
    pub fn capture(&self) -> Result<(usize, usize, Vec<slint::Rgb8Pixel>), String> {
        let window = self.hwnd();
        let mut bounds = RECT::default();
        // SAFETY: bounds is a valid output; Win32 validates the handle.
        unsafe { GetWindowRect(window, &mut bounds) }.map_err(|error| error.to_string())?;
        // SAFETY: the screen DC is released below, after its last use.
        let screen = unsafe { GetDC(None) };
        // A process that is not DPI aware gets scaled-down window bounds, while
        // PrintWindow renders the window at the display's own pixel size.
        let scale = |physical, logical| {
            // SAFETY: the screen DC is live; these indexes only read.
            let (physical, logical) = unsafe {
                (
                    GetDeviceCaps(Some(screen), physical),
                    GetDeviceCaps(Some(screen), logical),
                )
            };
            move |length: i32| {
                if logical > 0 && physical > logical {
                    (i64::from(length) * i64::from(physical) / i64::from(logical)) as i32
                } else {
                    length
                }
            }
        };
        let width = scale(DESKTOPHORZRES, HORZRES)(bounds.right.saturating_sub(bounds.left));
        let height = scale(DESKTOPVERTRES, VERTRES)(bounds.bottom.saturating_sub(bounds.top));
        if width <= 0 || height <= 0 {
            // SAFETY: the screen DC was obtained above.
            unsafe { ReleaseDC(None, screen) };
            return Err(format!("the window measures {width}×{height}"));
        }
        let (columns, rows) = (width as usize, height as usize);
        let mut bgra = vec![0_u8; columns * rows * 4];
        // SAFETY: every GDI object made here is selected out and released before return,
        // and GetDIBits writes at most `rows` top-down 32-bit rows of `columns` pixels,
        // the size of `bgra`.
        let (printed, copied) = unsafe {
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let previous = SelectObject(memory, bitmap.into());
            let printed = PrintWindow(window.0, memory.0, PW_RENDERFULLCONTENT) != 0;
            SelectObject(memory, previous);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let copied = GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(bgra.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            (printed, copied)
        };
        if !printed || copied != height {
            return Err(format!(
                "PrintWindow {printed}, {copied} of {height} rows copied"
            ));
        }
        let pixels = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[b, g, r, _]| slint::Rgb8Pixel { r, g, b })
            .collect();
        Ok((columns, rows, pixels))
    }
}

fn text(window: HWND) -> String {
    let mut buffer = [0_u16; 1024];
    // SAFETY: the buffer is writable for its length; Win32 validates the handle.
    let length = unsafe { GetWindowTextW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap_or(0)])
}

fn class(window: HWND) -> String {
    let mut buffer = [0_u16; 256];
    // SAFETY: as in `text`.
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap_or(0)])
}

unsafe extern "system" fn collect(window: HWND, list: LPARAM) -> BOOL {
    // SAFETY: `list` is the Vec that `handles` passes, live through the enumeration.
    unsafe { &mut *(list.0 as *mut Vec<HWND>) }.push(window);
    true.into()
}

fn handles(parent: Option<HWND>) -> Vec<HWND> {
    let mut list: Vec<HWND> = Vec::new();
    let pointer = LPARAM(&raw mut list as isize);
    // SAFETY: the callback only appends to `list`, which outlives the enumeration.
    unsafe {
        match parent {
            None => {
                let _ = EnumWindows(Some(collect), pointer);
            }
            Some(parent) => {
                let _ = EnumChildWindows(Some(parent), Some(collect), pointer);
            }
        }
    }
    list
}

/// This process's visible top-level windows.
pub fn windows() -> Vec<NativeWindow> {
    handles(None)
        .into_iter()
        .filter(|window| {
            let mut process = 0;
            // SAFETY: Win32 validates the handle; process is a valid output.
            unsafe { GetWindowThreadProcessId(*window, Some(&mut process)) };
            // SAFETY: Win32 validates the handle.
            process == std::process::id() && unsafe { IsWindowVisible(*window) }.as_bool()
        })
        .map(|window| NativeWindow {
            handle: window.0 as isize,
            class: class(window),
            title: text(window),
            texts: handles(Some(window))
                .into_iter()
                .map(text)
                .filter(|text| !text.trim_matches('\0').is_empty())
                .collect(),
        })
        .collect()
}
