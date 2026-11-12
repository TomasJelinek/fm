//! Common Item Dialog on one backend STA. No MAX_PATH buffers or shell commands.
use crate::{WindowsFileSystem, desktop::ComGuard};
use midnight_core::{
    Cancellation, FsError, PathPickerKind, PathPickerOwner, PathPickerRequest, PathPickerService,
};
use std::{
    cell::RefCell,
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoTaskMemFree,
        },
        System::Ole::IOleWindow,
        UI::{
            Shell::{
                Common::COMDLG_FILTERSPEC, FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST,
                FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_NODEREFERENCELINKS, FOS_NOTESTFILECREATE,
                FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog,
                IFileOpenDialog, IFileSaveDialog, IShellItem, SHCreateItemFromParsingName,
                SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetWindowThreadProcessId, HWND_MESSAGE, IsWindow,
                KillTimer, PostMessageW, SetTimer, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
            },
        },
    },
    core::{HRESULT, Interface, PCWSTR, PWSTR, w},
};

const CANCELLED: HRESULT = HRESULT::from_win32(1223);
struct ShellPath(PWSTR);
impl Drop for ShellPath {
    fn drop(&mut self) {
        // SAFETY: GetDisplayName transferred this CoTaskMem allocation to us.
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}
thread_local! {
    static ACTIVE: RefCell<Option<(HWND, usize, IFileDialog, Cancellation)>> = const { RefCell::new(None) };
}

struct DialogTimer {
    window: HWND,
    id: usize,
}
impl DialogTimer {
    fn new(dialog: &IFileDialog, cancel: &Cancellation) -> windows::core::Result<Self> {
        if ACTIVE.with(|slot| slot.borrow().is_some()) {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x800700aau32 as i32,
            )));
        }
        // The Shell modal loop does not dispatch a windowless thread timer.
        // A message-only helper keeps cancellation on the owning STA and receives
        // normal window dispatch without ever being shown or taking focus.
        // SAFETY: built-in STATIC class; this thread owns the returned HWND.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Midnight picker cancellation"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }?;
        // SAFETY: the helper and callback share the dialog's STA.
        let timer = unsafe { SetTimer(Some(window), 1, 50, Some(cancel_tick)) };
        if timer == 0 {
            let cause = windows::core::Error::from_thread();
            // SAFETY: helper creation succeeded on this thread.
            let _ = unsafe { DestroyWindow(window) };
            return Err(cause);
        }
        ACTIVE.with(|slot| {
            *slot.borrow_mut() = Some((window, timer, dialog.clone(), cancel.clone()))
        });
        Ok(Self { window, id: timer })
    }
}
impl Drop for DialogTimer {
    fn drop(&mut self) {
        // SAFETY: this guard and its timer remain on the creating STA thread.
        let _ = unsafe { KillTimer(Some(self.window), self.id) };
        ACTIVE.with(|slot| {
            slot.borrow_mut().take();
        });
        // SAFETY: destruction stays on the helper's creating thread, after its timer.
        let _ = unsafe { DestroyWindow(self.window) };
    }
}

unsafe extern "system" fn cancel_tick(window: HWND, _: u32, id: usize, _: u32) {
    let _ = ACTIVE.try_with(|slot| {
        let Ok(active) = slot.try_borrow() else {
            return;
        };
        if let Some((owner, timer, dialog, cancel)) = active.as_ref()
            && *owner == window
            && *timer == id
            && cancel.is_cancelled()
        {
            // Close() returns S_OK without closing when called by a timer (also
            // reproduced by Mozilla bug 1870660). Post the normal close request
            // to this exact COM-owned dialog on its STA instead. Never terminate
            // a thread or destroy a Shell window; modal extension UI may defer it.
            // SAFETY: retained COM dialog and its window belong to this STA.
            let result = unsafe {
                dialog
                    .cast::<IOleWindow>()
                    .and_then(|owner| owner.GetWindow())
                    .and_then(|dialog_window| {
                        PostMessageW(Some(dialog_window), WM_CLOSE, WPARAM(0), LPARAM(0))
                    })
            };
            if result.is_ok() {
                // SAFETY: only the matching owned timer is stopped after posting.
                let _ = unsafe { KillTimer(Some(window), id) };
            }
        }
    });
}

fn native_owner(owner: PathPickerOwner) -> Option<HWND> {
    let PathPickerOwner::Windows(value) = owner else {
        return None;
    };
    let window = HWND(value as *mut _);
    let mut process = 0;
    // SAFETY: OS validates the borrowed handle; no application memory is dereferenced.
    unsafe {
        if !IsWindow(Some(window)).as_bool() {
            return None;
        }
        GetWindowThreadProcessId(window, Some(&mut process));
    }
    (process == std::process::id()).then_some(window)
}

impl PathPickerService for WindowsFileSystem {
    fn store_command_icon(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
        cancel: &Cancellation,
    ) -> Result<PathBuf, FsError> {
        let result =
            midnight_platform_common::icon_snapshot::SnapshotPixels::from_rgba(width, height, rgba)
                .and_then(|pixels| crate::icon_storage::save(&pixels, cancel));
        result.map_err(|cause| FsError::from_io("store command icon", Path::new(""), cause))
    }

    fn pick_path(
        &self,
        request: &PathPickerRequest,
        cancel: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        request.validate()?;
        if cancel.is_cancelled() {
            return Ok(None);
        }
        if request.kind == PathPickerKind::CommandIconResource {
            return crate::icon_picker::choose(request, cancel)
                .map_err(|cause| FsError::from_io("choose command icon", Path::new(""), cause));
        }
        choose(request, cancel).map_err(|cause| {
            FsError::from_io(
                "choose path",
                Path::new(""),
                io::Error::other(cause.to_string()),
            )
        })
    }
}

fn specific_filter(kind: PathPickerKind) -> Option<(&'static str, String)> {
    match kind {
        PathPickerKind::Program => Some(("Programs", "*.exe;*.com;*.bat;*.cmd".into())),
        PathPickerKind::Image | PathPickerKind::CommandIconImage => Some((
            "Images",
            "*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.ico;*.gif".into(),
        )),
        PathPickerKind::ChecksumOpen => Some((
            "Checksum files",
            "*.sfv;*.md5;*.sha1;*.sha256;*.sha512".into(),
        )),
        PathPickerKind::ChecksumSave(algorithm) => {
            Some((algorithm.label(), format!("*.{}", algorithm.extension())))
        }
        PathPickerKind::TextSave => Some(("Text files", "*.txt".into())),
        _ => None,
    }
}

fn create_dialog(request: &PathPickerRequest) -> windows::core::Result<IFileDialog> {
    // SAFETY: the caller initializes COM and retains this dialog on its STA;
    // every borrowed string remains alive for the native call that copies it.
    unsafe {
        let dialog = if request.kind.save_extension().is_some() {
            CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?
                .cast::<IFileDialog>()?
        } else {
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?
                .cast::<IFileDialog>()?
        };
        let mut options = FOS_FORCEFILESYSTEM
            | FOS_PATHMUSTEXIST
            | FOS_NOCHANGEDIR
            | FOS_DONTADDTORECENT
            | FOS_NODEREFERENCELINKS;
        match request.kind {
            PathPickerKind::Directory => options |= FOS_PICKFOLDERS,
            // Selecting a destination must not create the Shell's write-probe file.
            // The export worker exclusively creates and publishes later.
            PathPickerKind::ChecksumSave(_) | PathPickerKind::TextSave => {
                options |= FOS_NOTESTFILECREATE
            }
            _ => options |= FOS_FILEMUSTEXIST,
        }
        dialog.SetOptions(options)?;
        let title: Vec<u16> = request.title.encode_utf16().chain(Some(0)).collect();
        dialog.SetTitle(PCWSTR(title.as_ptr()))?;
        if let Some((label, pattern)) = specific_filter(request.kind) {
            let label: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
            let pattern: Vec<u16> = pattern.encode_utf16().chain(Some(0)).collect();
            dialog.SetFileTypes(&[
                COMDLG_FILTERSPEC {
                    pszName: PCWSTR(label.as_ptr()),
                    pszSpec: PCWSTR(pattern.as_ptr()),
                },
                COMDLG_FILTERSPEC {
                    pszName: w!("All files"),
                    pszSpec: w!("*.*"),
                },
            ])?;
            dialog.SetFileTypeIndex(1)?;
        }
        if let Some(extension) = request.kind.save_extension() {
            let extension: Vec<u16> = extension.encode_utf16().chain(Some(0)).collect();
            dialog.SetDefaultExtension(PCWSTR(extension.as_ptr()))?;
        }
        if let Some(name) = request.kind.suggested_save_name() {
            let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            dialog.SetFileName(PCWSTR(name.as_ptr()))?;
        }
        if let Some(folder) = &request.initial_directory {
            let wide: Vec<u16> = folder.as_os_str().encode_wide().chain(Some(0)).collect();
            if let Ok(item) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None)
            {
                // A removed/unavailable suggestion must not prevent choosing another folder.
                let _ = dialog.SetFolder(&item);
            }
        }
        Ok(dialog)
    }
}

fn choose(
    request: &PathPickerRequest,
    cancel: &Cancellation,
) -> windows::core::Result<Option<PathBuf>> {
    let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED.0 as u32)
        .ok_or_else(windows::core::Error::from_thread)?;
    let dialog = create_dialog(request)?;
    // SAFETY: every COM object and borrowed string remains alive on this STA.
    unsafe {
        let _timer = DialogTimer::new(&dialog, cancel)?;
        if cancel.is_cancelled() {
            return Ok(None);
        }
        match dialog.Show(native_owner(request.owner)) {
            Err(error) if error.code() == CANCELLED => return Ok(None),
            result => result?,
        }
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let name = ShellPath(dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?);
        if name.0.is_null() {
            return Err(windows::core::Error::from_hresult(HRESULT(
                0x80004005u32 as i32,
            )));
        }
        let raw = OsString::from_wide(name.0.as_wide());
        Ok(Some(PathBuf::from(raw)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::ChecksumAlgorithm;
    use windows::Win32::UI::Shell::{FOS_NOTESTFILECREATE, FOS_OVERWRITEPROMPT, IFileSaveDialog};

    fn request(kind: PathPickerKind) -> PathPickerRequest {
        PathPickerRequest {
            kind,
            title: "Checksum soubory 中文".into(),
            initial_directory: None,
            owner: PathPickerOwner::None,
        }
    }

    #[test]
    fn text_save_uses_save_dialog_with_text_filter_without_file_creation_or_overwrite_consent() {
        let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED.0 as u32).unwrap();
        let dialog = create_dialog(&request(PathPickerKind::TextSave)).unwrap();
        assert!(dialog.cast::<IFileSaveDialog>().is_ok());
        // SAFETY: configuration and inspection remain on the creating STA; never Show().
        unsafe {
            let options = dialog.GetOptions().unwrap();
            assert_eq!(options & FOS_FILEMUSTEXIST, Default::default());
            assert_eq!(options & FOS_PICKFOLDERS, Default::default());
            assert_eq!(options & FOS_OVERWRITEPROMPT, Default::default());
            assert_ne!(options & FOS_NOTESTFILECREATE, Default::default());
            assert_eq!(dialog.GetFileTypeIndex().unwrap(), 1);
            assert_eq!(
                ShellPath(dialog.GetFileName().unwrap())
                    .0
                    .to_string()
                    .unwrap(),
                "file-list.txt"
            );
        }
        assert_eq!(
            specific_filter(PathPickerKind::TextSave),
            Some(("Text files", "*.txt".into()))
        );
    }

    #[test]
    fn checksum_save_uses_save_dialog_without_creating_files_or_requesting_overwrite() {
        let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED.0 as u32).unwrap();
        for (algorithm, name) in [
            (ChecksumAlgorithm::Crc32, "checksums.sfv"),
            (ChecksumAlgorithm::Md5, "checksums.md5"),
            (ChecksumAlgorithm::Sha1, "checksums.sha1"),
            (ChecksumAlgorithm::Sha256, "checksums.sha256"),
            (ChecksumAlgorithm::Sha512, "checksums.sha512"),
        ] {
            let dialog = create_dialog(&request(PathPickerKind::ChecksumSave(algorithm))).unwrap();
            assert!(dialog.cast::<IFileSaveDialog>().is_ok());
            // SAFETY: configuration and inspection run on this test's owning STA.
            unsafe {
                let options = dialog.GetOptions().unwrap();
                assert_eq!(options & FOS_FILEMUSTEXIST, Default::default());
                assert_eq!(options & FOS_PICKFOLDERS, Default::default());
                assert_eq!(options & FOS_OVERWRITEPROMPT, Default::default());
                assert_eq!(options & FOS_NOTESTFILECREATE, FOS_NOTESTFILECREATE);
                assert_eq!(options & FOS_PATHMUSTEXIST, FOS_PATHMUSTEXIST);
                let actual = ShellPath(dialog.GetFileName().unwrap());
                assert_eq!(actual.0.to_string().unwrap(), name);
                assert_eq!(dialog.GetFileTypeIndex().unwrap(), 1);
            }
        }
    }

    #[test]
    fn checksum_open_uses_existing_file_dialog_with_all_supported_formats() {
        let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED.0 as u32).unwrap();
        let dialog = create_dialog(&request(PathPickerKind::ChecksumOpen)).unwrap();
        assert!(dialog.cast::<IFileOpenDialog>().is_ok());
        // SAFETY: the unshown dialog remains on its creating STA.
        let options = unsafe { dialog.GetOptions() }.unwrap();
        assert_eq!(options & FOS_FILEMUSTEXIST, FOS_FILEMUSTEXIST);
        assert_eq!(options & FOS_PICKFOLDERS, Default::default());
        assert_eq!(
            specific_filter(PathPickerKind::ChecksumOpen),
            Some((
                "Checksum files",
                "*.sfv;*.md5;*.sha1;*.sha256;*.sha512".into()
            ))
        );
    }

    #[test]
    fn checksum_save_filters_follow_the_selected_algorithm() {
        for (algorithm, pattern) in [
            (ChecksumAlgorithm::Crc32, "*.sfv"),
            (ChecksumAlgorithm::Md5, "*.md5"),
            (ChecksumAlgorithm::Sha1, "*.sha1"),
            (ChecksumAlgorithm::Sha256, "*.sha256"),
            (ChecksumAlgorithm::Sha512, "*.sha512"),
        ] {
            let (_, actual) = specific_filter(PathPickerKind::ChecksumSave(algorithm)).unwrap();
            assert_eq!(actual, pattern);
        }
    }
}
