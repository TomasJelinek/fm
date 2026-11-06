//! F11/F12 delegate all network selection and credentials to native Windows dialogs.
use crate::WindowsFileSystem;
use midnight_core::{FsError, NetworkService};
use std::{io, path::Path};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, HWND},
    NetworkManagement::WNet::{RESOURCETYPE_DISK, WNetConnectionDialog, WNetDisconnectDialog},
    System::Threading::GetCurrentProcessId,
    UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
};

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;

impl NetworkService for WindowsFileSystem {
    fn manage_connections(&self, disconnect: bool) -> Result<(), FsError> {
        let owner = foreground_owner()?;
        dispatch(
            disconnect,
            || {
                // SAFETY: owner was checked as a live window of this process. The dialog
                // owns its credential UI; no path, password or mutable buffer is supplied.
                unsafe { WNetConnectionDialog(owner, RESOURCETYPE_DISK) }
            },
            || {
                // SAFETY: same owner check. Windows manages the asynchronous disconnect
                // dialog's own worker; this function does not terminate that worker.
                unsafe { WNetDisconnectDialog(owner, RESOURCETYPE_DISK) }
            },
        )
    }
}

fn foreground_owner() -> Result<HWND, FsError> {
    // SAFETY: these queries read system-owned window/process identifiers and do not retain
    // pointers. A handle is used only after checking that the application owns its window.
    let owner = unsafe { GetForegroundWindow() };
    let mut process = 0;
    // SAFETY: process is a valid writable DWORD; a null HWND yields no valid owner.
    let thread = unsafe { GetWindowThreadProcessId(owner, &mut process) };
    // SAFETY: GetCurrentProcessId takes no pointers and cannot fail.
    if owner.is_null() || thread == 0 || process != unsafe { GetCurrentProcessId() } {
        return Err(FsError::from_io(
            "network connection dialog",
            Path::new(""),
            io::Error::new(
                io::ErrorKind::NotFound,
                "activate the file manager window before opening network connections",
            ),
        ));
    }
    Ok(owner)
}

fn dispatch(
    disconnect: bool,
    connect: impl FnOnce() -> u32,
    disconnect_dialog: impl FnOnce() -> u32,
) -> Result<(), FsError> {
    let code = if disconnect {
        disconnect_dialog()
    } else {
        connect()
    };
    match code {
        0 | u32::MAX | ERROR_CANCELLED => Ok(()),
        code => Err(FsError::from_io(
            "network connection dialog",
            Path::new(""),
            io::Error::from_raw_os_error(code as i32),
        )),
    }
}
