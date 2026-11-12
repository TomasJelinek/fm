//! Lossless CF_HDROP and Preferred DropEffect, with a thread-owned clipboard window.

use midnight_core::{
    FileClipboard, FileClipboardIntent, FsError, MAX_FILE_CLIPBOARD_BYTES, MAX_FILE_CLIPBOARD_PATHS,
};
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HWND},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
            OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        LibraryLoader::GetModuleHandleW,
        Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Ole::CF_HDROP,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, HWND_MESSAGE, MSG,
        PM_REMOVE, PeekMessageW, RegisterClassW, TranslateMessage, WNDCLASSW,
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
        || bytes[16..20] != 1u32.to_le_bytes()
    {
        return Err(error(
            "The clipboard does not contain a valid Unicode DROPFILES header.",
        ));
    }
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
            if paths.len() > MAX_FILE_CLIPBOARD_PATHS {
                return Err(error(
                    "The file clipboard exceeds its 4096-item resource budget.",
                ));
            }
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
    let Some(value) = bytes.get(..4) else {
        return Err(error("The preferred drop effect is truncated."));
    };
    match u32::from_le_bytes(value.try_into().unwrap()) {
        1 => Ok(FileClipboardIntent::Copy),
        2 => Ok(FileClipboardIntent::Move),
        _ => Err(error(
            "The clipboard has an unsupported or ambiguous preferred drop effect.",
        )),
    }
}

struct ClipboardLock;
impl ClipboardLock {
    fn open(owner: HWND) -> Result<Self, FsError> {
        // SAFETY: the session created this non-null window on the calling worker thread.
        if unsafe { OpenClipboard(owner) } == 0 {
            return Err(error(io::Error::last_os_error()));
        }
        Ok(Self)
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
        // SAFETY: the executable and native default window procedure stay loaded.
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        CLASS
            .get_or_init(|| {
                let value = WNDCLASSW {
                    lpfnWndProc: Some(DefWindowProcW),
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
