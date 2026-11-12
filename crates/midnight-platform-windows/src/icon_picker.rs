//! The native resource chooser runs on one backend STA; selected modules are never executed.
use crate::desktop::ComGuard;
use midnight_core::{Cancellation, PathPickerOwner, PathPickerRequest};
use std::{
    cell::RefCell,
    ffi::OsString,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::PathBuf,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        FILE_TYPE_DISK, GetFileType, GetFinalPathNameByHandleW,
    },
    System::{Com::COINIT_APARTMENTTHREADED, Threading::GetCurrentThreadId},
    UI::{
        Shell::PickIconDlg,
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, EnumThreadWindows, GW_OWNER, GetClassNameW, GetWindow,
            GetWindowThreadProcessId, IsWindow, KillTimer, PostMessageW, SetTimer, WM_CLOSE,
            WS_POPUP,
        },
    },
};
const PATH_UNITS: usize = 32768;
thread_local! {
    static ACTIVE: RefCell<Option<(HWND, usize, Cancellation)>> = const { RefCell::new(None) };
}
struct DialogOwner {
    window: HWND,
    timer: usize,
}
impl DialogOwner {
    fn new(owner: PathPickerOwner, cancel: &Cancellation) -> io::Result<Self> {
        let owner = match owner {
            PathPickerOwner::Windows(value) => {
                let handle = value as HWND;
                let mut process = 0;
                // SAFETY: borrowed native handles are checked by the OS before use.
                if unsafe { IsWindow(handle) } == 0 {
                    return Err(invalid("The icon chooser owner is unavailable."));
                }
                unsafe { GetWindowThreadProcessId(handle, &mut process) };
                if process != std::process::id() {
                    return Err(invalid(
                        "The icon chooser owner belongs to another process.",
                    ));
                }
                handle
            }
            PathPickerOwner::None => ptr::null_mut(),
            _ => return Err(invalid("The icon chooser requires a Windows owner.")),
        };
        if ACTIVE.with(|slot| slot.borrow().is_some()) {
            return Err(invalid("An icon chooser is already active on this thread."));
        }
        let class: Vec<_> = "STATIC".encode_utf16().chain(Some(0)).collect();
        let title: Vec<_> = "Midnight icon chooser owner"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: built-in window class and live terminated strings. The helper has no
        // WS_VISIBLE style; only the user-requested native dialog is ever shown.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the helper and callback belong to this STA's modal message loop.
        let timer = unsafe { SetTimer(window, 1, 50, Some(cancel_tick)) };
        if timer == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: creation succeeded and destruction stays on its owner thread.
            unsafe { DestroyWindow(window) };
            return Err(error);
        }
        ACTIVE.with(|slot| *slot.borrow_mut() = Some((window, timer, cancel.clone())));
        Ok(Self { window, timer })
    }
}
impl Drop for DialogOwner {
    fn drop(&mut self) {
        // SAFETY: this guard owns its timer and hidden helper on the creating STA.
        unsafe { KillTimer(self.window, self.timer) };
        ACTIVE.with(|slot| {
            slot.borrow_mut().take();
        });
        unsafe { DestroyWindow(self.window) };
    }
}
unsafe extern "system" fn cancel_tick(window: HWND, _: u32, timer: usize, _: u32) {
    let close = ACTIVE
        .try_with(|slot| {
            slot.try_borrow().is_ok_and(|active| {
                active.as_ref().is_some_and(|(helper, id, cancel)| {
                    *helper == window && *id == timer && cancel.is_cancelled()
                })
            })
        })
        .unwrap_or(false);
    if close {
        // SAFETY: enumeration is confined to this thread. The callback accepts only
        // dialogs whose owner chain reaches our private, live helper window.
        unsafe {
            EnumThreadWindows(
                GetCurrentThreadId(),
                Some(close_owned_dialog),
                window as LPARAM,
            )
        };
    }
}
unsafe extern "system" fn close_owned_dialog(window: HWND, parameter: LPARAM) -> i32 {
    let mut class = [0u16; 32];
    // SAFETY: fixed writable buffer; the OS validates the enumerated HWND.
    let count = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    if count != 6 || class[..6] != [35, 51, 50, 55, 55, 48] {
        return 1;
    }
    let mut owner = window;
    for _ in 0..8 {
        // SAFETY: owner traversal inspects native handles only.
        owner = unsafe { GetWindow(owner, GW_OWNER) };
        if owner.is_null() {
            break;
        }
        if owner == parameter as HWND {
            // Normal close requests preserve Shell ownership. Nested extension UI
            // may defer cancellation; no window is forcibly destroyed.
            unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
            break;
        }
    }
    1
}
fn select(
    request: &PathPickerRequest,
    cancel: &Cancellation,
) -> io::Result<Option<(PathBuf, i32)>> {
    let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32)
        .ok_or_else(|| io::Error::other("Cannot initialize the icon chooser"))?;
    let owner = DialogOwner::new(request.owner, cancel)?;
    let initial = crate::desktop::system_directory()?.join("shell32.dll");
    let initial: Vec<_> = initial.as_os_str().encode_wide().collect();
    if initial.contains(&0) || initial.len() + 1 >= PATH_UNITS {
        return Err(invalid("The initial icon resource path is too large."));
    }
    let mut buffer = vec![0; PATH_UNITS];
    buffer[..initial.len()].copy_from_slice(&initial);
    let mut index = 0;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    // SAFETY: the path buffer retains its full advertised Unicode capacity, and
    // the native resource chooser writes only an index and selected path.
    let selected = unsafe {
        PickIconDlg(
            owner.window,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut index,
        )
    };
    if selected == 0 || cancel.is_cancelled() {
        return Ok(None);
    }
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .filter(|length| *length > 0 && *length + 1 < buffer.len())
        .ok_or_else(|| invalid("The selected icon resource path is incomplete or too large."))?;
    let path = PathBuf::from(OsString::from_wide(&buffer[..length]));
    midnight_core::validate_picked_path(&path).map_err(io::Error::other)?;
    if index < 0 {
        return Err(invalid("The selected resource index is invalid."));
    }
    Ok(Some((path, index)))
}

pub(crate) fn choose(
    request: &PathPickerRequest,
    cancel: &Cancellation,
) -> io::Result<Option<PathBuf>> {
    let Some((path, index)) = select(request, cancel)? else {
        return Ok(None);
    };
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let pixels = capture_pixels(&path, index, cancel)?;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let snapshot = crate::icon_storage::save(&pixels, cancel)?;
    Ok((!cancel.is_cancelled()).then_some(snapshot))
}
fn capture_pixels(
    path: &std::path::Path,
    index: i32,
    cancel: &Cancellation,
) -> io::Result<midnight_platform_common::icon_snapshot::SnapshotPixels> {
    if cancel.is_cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let source = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = source.metadata()?;
    // SAFETY: source owns a live handle; this only queries its native file type.
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || unsafe { GetFileType(source.as_raw_handle()) } != FILE_TYPE_DISK
    {
        return Err(invalid("Choose an ordinary executable, DLL, or icon file."));
    }
    // Resolve from the retained handle, never reopen through a retargetable junction.
    // The source handle denies write/delete sharing throughout resource extraction.
    let resolved = final_path(&source)?;
    crate::icons::load_resource(&resolved, index, 128, cancel)
        .ok_or_else(|| invalid("The selected icon resource could not be decoded."))
}
fn final_path(source: &File) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 256];
    loop {
        // SAFETY: the file handle and initialized writable UTF-16 buffer remain live.
        let count = unsafe {
            GetFinalPathNameByHandleW(
                source.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        } as usize;
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= PATH_UNITS {
            return Err(invalid("The resource's native final path is too large."));
        }
        if count >= buffer.len() {
            buffer.resize(count + 1, 0);
            continue;
        }
        return Ok(PathBuf::from(OsString::from_wide(&buffer[..count])));
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_resource_index_is_rasterized_at128_and_snapshot_survives_source_removal() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("resource 文档.dll");
        let system = crate::desktop::system_directory().unwrap();
        // Modern Windows keeps shell32 artwork in its resource-only MUN sidecar.
        // Copy the self-contained resources, not a DLL whose sidecar would be lost.
        let sidecar = system
            .parent()
            .unwrap()
            .join("SystemResources/shell32.dll.mun");
        let fixture = if sidecar.is_file() {
            sidecar
        } else {
            system.join("shell32.dll")
        };
        std::fs::copy(fixture, &source).unwrap();
        let cancel = Cancellation::default();
        let first = capture_pixels(&source, 0, &cancel).unwrap();
        let second = capture_pixels(&source, 1, &cancel).unwrap();
        assert_eq!(
            (first.width, first.height, first.rgba.len()),
            (128, 128, 65536)
        );
        assert_ne!(
            first.rgba, second.rgba,
            "the selected native resource index must matter"
        );
        let saved =
            crate::icon_storage::save_test(&temporary.path().join("managed"), &second, &cancel)
                .unwrap();
        let before = std::fs::read(&saved).unwrap();
        std::fs::remove_file(&source).unwrap();
        let after = std::fs::read(&saved).unwrap();
        assert_eq!(before, after);
        assert_eq!(&after[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(after[16..20].try_into().unwrap()), 128);
        assert_eq!(u32::from_be_bytes(after[20..24].try_into().unwrap()), 128);
        cancel.cancel();
        assert!(
            matches!(capture_pixels(&source, 0, &cancel), Err(error) if error.kind() == io::ErrorKind::Interrupted)
        );
    }
}
