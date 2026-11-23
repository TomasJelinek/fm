//! Lossless CF_HDROP and Preferred DropEffect, with a thread-owned clipboard window.

use midnight_core::{FileClipboard, FileClipboardIntent, FsError, MAX_FILE_CLIPBOARD_BYTES};
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, GlobalFree, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
            OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        LibraryLoader::GetModuleHandleW,
        Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE},
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
        GetWindowLongPtrW, HWND_MESSAGE, MSG, PM_REMOVE, PeekMessageW, RegisterClassW,
        SetWindowLongPtrW, TranslateMessage, WM_DESTROYCLIPBOARD, WNDCLASSW,
    },
};

#[cfg(test)]
#[path = "file_clipboard_tests.rs"]
mod tests;

fn error(message: impl std::fmt::Display) -> FsError {
    FsError::from_io(
        "Windows file clipboard",
        Path::new(""),
        io::Error::other(message.to_string()),
    )
}

pub(crate) fn encode_hdrop(files: &FileClipboard) -> Result<Vec<u8>, FsError> {
    files.validate()?;
    let mut bytes = vec![0u8; 20];
    bytes[..4].copy_from_slice(&20u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    for path in &files.paths {
        for unit in path.as_os_str().encode_wide().chain(std::iter::once(0)) {
            if bytes.len() > MAX_FILE_CLIPBOARD_BYTES - 4 {
                return Err(error(
                    "The Unicode file clipboard exceeds its 4 MiB resource budget.",
                ));
            }
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    Ok(bytes)
}

pub(crate) fn decode_hdrop(bytes: &[u8]) -> Result<Vec<PathBuf>, FsError> {
    if bytes.len() < 24 || bytes.len() > MAX_FILE_CLIPBOARD_BYTES {
        return Err(error(
            "The native file-list allocation has an invalid size.",
        ));
    }
    let offset = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    if offset < 20
        || offset > bytes.len() - 4
        || !offset.is_multiple_of(2)
        // fWide is a Win32 BOOL: any nonzero value means Unicode. .NET (WinForms, WPF)
        // writes TRUE as -1, and refusing it made every drop from those programs fail.
        || bytes[16..20] == [0; 4]
    {
        return Err(error(
            "The clipboard does not contain a valid Unicode DROPFILES header.",
        ));
    }
    // The allocation's bytes bound the list (checked above); CF_HDROP has no item limit.
    let mut paths = Vec::new();
    let mut name = Vec::new();
    let mut terminated = false;
    for unit in bytes[offset..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
    {
        if unit == 0 {
            if name.is_empty() {
                terminated = true;
                break;
            }
            paths.push(PathBuf::from(OsString::from_wide(&name)));
            name.clear();
        } else {
            name.push(unit);
        }
    }
    if !terminated || !name.is_empty() {
        return Err(error("The native file list is not double-NUL terminated."));
    }
    let files = FileClipboard {
        paths,
        intent: FileClipboardIntent::Copy,
    };
    files.validate()?;
    Ok(files.paths)
}

fn decode_drop_effect(bytes: Option<&[u8]>) -> Result<FileClipboardIntent, FsError> {
    let Some(bytes) = bytes else {
        return Ok(FileClipboardIntent::Copy);
    };
    let Some(&[a, b, c, d]) = bytes.get(..4) else {
        return Err(error("The preferred drop effect is truncated."));
    };
    // Explorer's Copy publishes DROPEFFECT_COPY | DROPEFFECT_LINK (5) and Cut publishes
    // DROPEFFECT_MOVE (2). The file manager never creates links from the clipboard, so
    // the LINK bit is ignored. COPY | MOVE (3, 7) leaves the operation ambiguous and a
    // destructive Move is never guessed; link-only and empty effects name no operation.
    match u32::from_le_bytes([a, b, c, d]) & !DROPEFFECT_LINK {
        DROPEFFECT_COPY => Ok(FileClipboardIntent::Copy),
        DROPEFFECT_MOVE => Ok(FileClipboardIntent::Move),
        _ => Err(error(
            "The clipboard has an unsupported or ambiguous preferred drop effect.",
        )),
    }
}

/// Attempts made while another process holds the clipboard, matching arboard's text path.
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(5);

struct ClipboardLock;
impl ClipboardLock {
    fn open(owner: HWND) -> Result<Self, FsError> {
        retry_open(
            || {
                // SAFETY: the session created this non-null window on the calling worker thread.
                if unsafe { OpenClipboard(owner) } == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            },
            || std::thread::sleep(OPEN_RETRY_DELAY),
        )
        .map(|()| Self)
        .map_err(error)
    }
}

/// Clipboard History, rdpclip, clipboard managers or a reader of data this session just
/// published can hold the clipboard briefly, which fails OpenClipboard with
/// ERROR_ACCESS_DENIED. Retry only that error, a bounded number of times, on the backend
/// session thread; other errors are returned immediately.
fn retry_open(
    mut attempt: impl FnMut() -> io::Result<()>,
    mut pause: impl FnMut(),
) -> io::Result<()> {
    let mut made = 0;
    loop {
        let failure = match attempt() {
            Ok(()) => return Ok(()),
            Err(failure) => failure,
        };
        made += 1;
        if made >= OPEN_ATTEMPTS || failure.raw_os_error() != Some(ERROR_ACCESS_DENIED as i32) {
            return Err(failure);
        }
        pause();
    }
}
impl Drop for ClipboardLock {
    fn drop(&mut self) {
        // SAFETY: each guard owns exactly one successful OpenClipboard call.
        unsafe { CloseClipboard() };
    }
}

struct Memory(HGLOBAL);
impl Memory {
    fn copy(bytes: &[u8]) -> Result<Self, FsError> {
        // SAFETY: the bounded size is nonzero; the returned movable allocation is owned here.
        let memory = Self(unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()) });
        if memory.0.is_null() {
            return Err(error(io::Error::last_os_error()));
        }
        // SAFETY: memory is a live global allocation, exclusively owned until publication.
        let pointer = unsafe { GlobalLock(memory.0) };
        if pointer.is_null() {
            return Err(error(io::Error::last_os_error()));
        }
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            GlobalUnlock(memory.0);
        }
        Ok(memory)
    }

    fn publish(mut self, format: u32) -> Result<(), FsError> {
        // SAFETY: clipboard is open and this is a movable HGLOBAL. Successful publication
        // transfers ownership to Windows, so this guard must no longer free it.
        if unsafe { SetClipboardData(format, self.0) }.is_null() {
            return Err(error(io::Error::last_os_error()));
        }
        self.0 = ptr::null_mut();
        Ok(())
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this guard still owns the allocation because it was not published.
            unsafe { GlobalFree(self.0) };
        }
    }
}

fn read_format(format: u32) -> Result<Option<Vec<u8>>, FsError> {
    // SAFETY: caller holds the clipboard lock throughout lookup, size check and copying.
    if unsafe { IsClipboardFormatAvailable(format) } == 0 {
        return Ok(None);
    }
    let memory = unsafe { GetClipboardData(format) };
    if memory.is_null() {
        return Err(error(io::Error::last_os_error()));
    }
    let length = unsafe { GlobalSize(memory) };
    if length == 0 || length > MAX_FILE_CLIPBOARD_BYTES {
        return Err(error(
            "Clipboard format exceeds its resource budget or has no data.",
        ));
    }
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        return Err(error(io::Error::last_os_error()));
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length).to_vec() };
    unsafe { GlobalUnlock(memory) };
    Ok(Some(bytes))
}

/// Window procedure of the owner window. Its user data is an integer ownership mark
/// (nonzero while the window owns the clipboard): WM_DESTROYCLIPBOARD, sent when any
/// window empties the clipboard this window owns, clears it. Everything else, including
/// creation and destruction, takes the default path.
unsafe extern "system" fn owner_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DESTROYCLIPBOARD {
        // SAFETY: Windows calls the procedure on the window's own thread with that live window.
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
        return 0;
    }
    // SAFETY: unhandled messages keep their original values and default processing.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

pub(super) struct Files {
    window: HWND,
    effect: u32,
}

impl Files {
    pub(super) fn read_text(&mut self) -> Result<String, FsError> {
        let _lock = ClipboardLock::open(self.window)?;
        let bytes = read_format(windows_sys::Win32::System::Ole::CF_UNICODETEXT as u32)?
            .ok_or_else(|| error("The clipboard contains no Unicode text."))?;
        crate::clipboard_text::decode(&bytes)
    }
    pub(super) fn new() -> Result<Self, FsError> {
        static CLASS: OnceLock<Result<u16, String>> = OnceLock::new();
        let class = windows_sys::w!("MidnightExplorerFileClipboardOwner");
        // SAFETY: the executable and its owner window procedure stay loaded.
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        CLASS
            .get_or_init(|| {
                let value = WNDCLASSW {
                    lpfnWndProc: Some(owner_procedure),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                };
                let atom = unsafe { RegisterClassW(&value) };
                if atom == 0 {
                    Err(io::Error::last_os_error().to_string())
                } else {
                    Ok(atom)
                }
            })
            .as_ref()
            .map_err(error)?;
        // SAFETY: a hidden message-only owner is created and destroyed on this worker.
        let window = unsafe {
            CreateWindowExW(
                0,
                class,
                class,
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(error(io::Error::last_os_error()));
        }
        let mut files = Self { window, effect: 0 };
        files.effect = unsafe { RegisterClipboardFormatW(windows_sys::w!("Preferred DropEffect")) };
        if files.effect == 0 {
            return Err(error(io::Error::last_os_error()));
        }
        Ok(files)
    }

    pub(super) fn copy(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        let data = Memory::copy(&encode_hdrop(files)?)?;
        let value: u32 = match files.intent {
            FileClipboardIntent::Copy => 1,
            FileClipboardIntent::Move => 2,
        };
        let effect = Memory::copy(&value.to_le_bytes())?;
        let _lock = ClipboardLock::open(self.window)?;
        // SAFETY: a non-null worker window owns this lock, as required by SetClipboardData.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(error(io::Error::last_os_error()));
        }
        // EmptyClipboard made this window the owner. It first sent WM_DESTROYCLIPBOARD to the
        // previous owner, which may have been this window, so mark ownership only now.
        self.mark_owner();
        // Publish intent first; if publishing the list fails, no partial file list can be pasted.
        effect.publish(self.effect)?;
        data.publish(CF_HDROP as u32)
    }

    pub(super) fn read(&mut self) -> Result<FileClipboard, FsError> {
        let _lock = ClipboardLock::open(self.window)?;
        let data = read_format(CF_HDROP as u32)?
            .ok_or_else(|| error("The clipboard contains no native file list."))?;
        let effect = read_format(self.effect)?;
        let files = FileClipboard {
            paths: decode_hdrop(&data)?,
            intent: decode_drop_effect(effect.as_deref())?,
        };
        files.validate()?;
        Ok(files)
    }

    /// Whether this window owns the clipboard: it emptied the clipboard and has not
    /// received WM_DESTROYCLIPBOARD since. Reading never takes ownership.
    pub(super) fn owns_clipboard(&self) -> bool {
        // SAFETY: reads the integer user data of this live window of the calling thread.
        unsafe { GetWindowLongPtrW(self.window, GWLP_USERDATA) != 0 }
    }

    /// Record that a successful EmptyClipboard made this window the clipboard owner.
    pub(super) fn mark_owner(&self) {
        // SAFETY: stores an integer in the user data of this live window, on its thread.
        unsafe { SetWindowLongPtrW(self.window, GWLP_USERDATA, 1) };
    }

    #[cfg(test)]
    pub(super) fn window(&self) -> HWND {
        self.window
    }

    pub(super) fn service_pending(&mut self) {
        let mut message = MSG::default();
        // SAFETY: pump only this thread-owned clipboard window, with a per-tick event cap.
        unsafe {
            for _ in 0..128 {
                if PeekMessageW(&mut message, self.window, 0, 0, PM_REMOVE) == 0 {
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        // SAFETY: clipboard sessions are created, serviced and dropped on one backend thread.
        unsafe { DestroyWindow(self.window) };
    }
}
