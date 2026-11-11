//! Uses the Shell's explicit recycle operation, never its permanent RemoveItem method.
//! Shell items are path-based. Identity is checked immediately before the Shell call, but
//! Windows exposes no handle-bound recycle operation: a concurrent namespace replacement
//! inside that call remains an OS boundary limitation. Cancellation is cooperative.
use super::{WindowsMutation, checked, failure};
use crate::desktop::ComGuard;
use midnight_core::{Cancellation, FsError};
use std::{io, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::UI::Shell::{
        BHID_Transfer, COPYENGINE_E_RECYCLE_BIN_NOT_FOUND, COPYENGINE_E_RECYCLE_FORCE_NUKE,
        COPYENGINE_E_RECYCLE_PATH_TOO_LONG, COPYENGINE_E_RECYCLE_SIZE_TOO_BIG,
        FOLDERID_RecycleBinFolder, IShellItem, ITransferSource, KF_FLAG_DEFAULT,
        SHCreateItemFromParsingName, SHGetKnownFolderItem,
    },
    core::PCWSTR,
};

#[cfg(test)]
#[path = "mutation_trash_tests.rs"]
mod tests;

pub(super) fn recycle(
    item: &WindowsMutation,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(&Path),
) -> Result<(), FsError> {
    let _apartment =
        ComGuard::initialize(windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32)
            .ok_or_else(|| {
                failure(
                    "initialize recycle service",
                    &item.item.path,
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "the Windows recycle service requires a COM single-threaded apartment",
                    ),
                )
            })?;
    let _recycled = recycle_shell(&item.item.path, cancel, || item.revalidate(false).map(drop))?;
    progress(&item.item.path);
    Ok(())
}

fn recycle_shell(
    path: &Path,
    cancel: &Cancellation,
    revalidate: impl FnOnce() -> Result<(), FsError>,
) -> Result<IShellItem, FsError> {
    checked(cancel, path)?;
    let wide = shell_path(path);
    // SAFETY: caller has initialized COM on this thread, UTF-16 is NUL-terminated, and
    // all interfaces live through the synchronous call. The dedicated RecycleItem contract
    // returns the recycled item; unlike DeleteItem/RemoveItem it has no permanent route.
    unsafe {
        let source: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
            .map_err(|e| failure("create recycle Shell item", path, io::Error::other(e)))?;
        let parent = source
            .GetParent()
            .map_err(|e| failure("get recycle source parent", path, io::Error::other(e)))?;
        let transfer: ITransferSource = parent
            .BindToHandler(None, &BHID_Transfer)
            .map_err(|e| failure("bind recycle transfer", path, io::Error::other(e)))?;
        let destination: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)
                .map_err(|e| failure("open Windows Recycle Bin", path, io::Error::other(e)))?;
        // Wait before the final path identity check. Pausing after revalidation
        // would extend the Shell's unavoidable path-based race for the entire pause.
        checked(cancel, path)?;
        revalidate()?;
        if cancel.is_cancelled() {
            return Err(failure("recycle", path, io::ErrorKind::Interrupted.into()));
        }
        transfer
            .RecycleItem(&source, &destination, 0)
            .map_err(|error| recycle_error(path, error))
    }
}

fn recycle_error(path: &Path, error: windows::core::Error) -> FsError {
    let reason = match error.code() {
        COPYENGINE_E_RECYCLE_PATH_TOO_LONG => {
            Some("Windows Recycle Bin does not support this path length")
        }
        COPYENGINE_E_RECYCLE_SIZE_TOO_BIG => {
            Some("Windows Recycle Bin cannot accept an item of this size")
        }
        COPYENGINE_E_RECYCLE_FORCE_NUKE => {
            Some("Windows Recycle Bin is disabled for this location")
        }
        COPYENGINE_E_RECYCLE_BIN_NOT_FOUND => {
            Some("Windows Recycle Bin is unavailable for this location")
        }
        _ => None,
    };
    let error = match reason {
        Some(reason) => io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{reason}; the item was not permanently deleted ({})",
                error.code()
            ),
        ),
        None => io::Error::other(error),
    };
    failure("recycle", path, error)
}

fn shell_path(path: &Path) -> Vec<u16> {
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    let verbatim_unc: Vec<_> = "\\\\?\\UNC\\".encode_utf16().collect();
    let verbatim: Vec<_> = "\\\\?\\".encode_utf16().collect();
    let mut result = if units.starts_with(&verbatim_unc) {
        "\\\\"
            .encode_utf16()
            .chain(units[8..].iter().copied())
            .collect()
    } else if units.starts_with(&verbatim) {
        units[4..].to_vec()
    } else {
        units
    };
    result.push(0);
    result
}
