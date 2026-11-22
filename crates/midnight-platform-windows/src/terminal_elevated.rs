//! The terminal button's Ctrl+click on Windows: the selected shell, started as
//! administrator by the Shell's `runas` verb after the user's consent (UAC). This
//! process never elevates itself. The shell's absolute path, its fixed arguments and the
//! working folder travel as separate fields; no command line is built from a path or a
//! name, so no folder name can add a command.
use super::{ComGuard, command_prompt_path, powershell_path, user_command_cwd};
use crate::{terminal_process, windows_argv::wide_units};
use midnight_core::{FsError, ShellFlavor, TerminalMode, TerminalProcess, TerminalRequest};
use std::{
    ffi::OsStr,
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr, thread,
};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, HANDLE, INVALID_HANDLE_VALUE},
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    Storage::FileSystem::{GetDriveTypeW, QueryDosDeviceW},
    System::{
        Com::COINIT_APARTMENTTHREADED,
        Threading::{GetCurrentProcess, OpenProcessToken},
        WindowsProgramming::DRIVE_REMOTE,
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

#[cfg(test)]
#[path = "terminal_elevated_tests.rs"]
mod tests;

/// The `ShellExecuteExW` request for an administrator terminal, as data.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ElevatedLaunch {
    /// Always `runas`: Windows asks for consent and starts the program elevated.
    pub verb: &'static str,
    /// The shell's absolute path, found as for a normal terminal.
    pub file: PathBuf,
    /// The shell's fixed interactive arguments, those of a normal terminal: never a
    /// path, a name or user text.
    pub parameters: &'static str,
    /// The working folder, which the shell receives as its current directory.
    pub directory: PathBuf,
}

/// Whether an elevated process sees a drive letter of this sign-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DriveScope {
    /// A volume, which every sign-in sees.
    Shared,
    /// A network drive mapped for this sign-in only.
    Mapped,
    /// A `subst` drive defined for this sign-in only.
    Substituted,
}

/// Opens `request`'s shell as administrator in its folder. A declined consent prompt
/// returns `midnight_core::terminal_consent_declined`.
pub(super) fn launch(request: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
    let folder = &request.working_directory;
    let file = match request.shell {
        ShellFlavor::PowerShell => powershell_path(),
        ShellFlavor::Cmd => command_prompt_path(),
        ShellFlavor::Posix => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "POSIX shell is unavailable on this Windows adapter",
        )),
    }
    .map_err(|cause| FsError::from_io("find system shell", folder, cause))?;
    // A process already elevated sees the drives of its own sign-in.
    let elevated = process_is_elevated();
    let spec = elevated_launch(request, &file, |letter| {
        if elevated {
            DriveScope::Shared
        } else {
            drive_scope(letter)
        }
    })
    .map_err(|cause| FsError::from_io(LAUNCH, folder, cause))?;
    let process = shell_execute(&spec).map_err(|cause| launch_failure(cause, folder))?;
    terminal_process::tracked(process).map_err(|cause| FsError::from_io(LAUNCH, folder, cause))
}

const LAUNCH: &str = "launch administrator terminal";

/// The launch of `request`'s shell `file` as administrator, refused with a plain
/// reason when the shell could not run in the folder. `drive` tells whether an
/// elevated process sees a drive letter.
pub(super) fn elevated_launch(
    request: &TerminalRequest,
    file: &Path,
    drive: impl Fn(char) -> DriveScope,
) -> io::Result<ElevatedLaunch> {
    let folder = &request.working_directory;
    if request.mode != TerminalMode::Elevated {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Only an elevated terminal request is started as administrator.",
        ));
    }
    if !request.command.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "An administrator terminal opens an interactive shell; run commands in it.",
        ));
    }
    if !file.is_absolute()
        || !folder.is_absolute()
        || folder.as_os_str().encode_wide().any(|unit| unit == 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal requires absolute native paths and arguments without NUL",
        ));
    }
    let parameters = match request.shell {
        ShellFlavor::PowerShell => "-NoLogo -NoProfile -NoExit",
        ShellFlavor::Cmd => "/D /U /V:OFF",
        ShellFlavor::Posix => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "POSIX shell is unavailable on this Windows adapter",
            ));
        }
    };
    if let Some(letter) = drive_letter(folder) {
        match drive(letter) {
            DriveScope::Shared => {}
            DriveScope::Mapped => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "An administrator terminal cannot open drive {letter}:. It is a network \
                         drive mapped for your account only, which programs run as \
                         administrator do not see. Open the folder by its network path instead."
                    ),
                ));
            }
            DriveScope::Substituted => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "An administrator terminal cannot open drive {letter}:. It is a \
                         substituted drive defined for your account only, which programs run \
                         as administrator do not see. Open the folder on its own drive instead."
                    ),
                ));
            }
        }
    }
    let directory = match request.shell {
        // Cmd runs elsewhere when given a verbatim, device or UNC folder.
        ShellFlavor::Cmd => user_command_cwd::for_cmd(folder)?,
        _ => folder.clone(),
    };
    Ok(ElevatedLaunch {
        verb: "runas",
        file: file.to_owned(),
        parameters,
        directory,
    })
}

/// A declined consent prompt is the user's choice; any other error names its cause.
pub(super) fn launch_failure(cause: io::Error, folder: &Path) -> FsError {
    if cause.raw_os_error() == Some(ERROR_CANCELLED as i32) {
        midnight_core::terminal_consent_declined(folder)
    } else {
        FsError::from_io(LAUNCH, folder, cause)
    }
}

fn drive_letter(path: &Path) -> Option<char> {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Some(char::from(letter).to_ascii_uppercase())
            }
            _ => None,
        },
        _ => None,
    }
}

/// How an elevated process of this user sees `letter`: network drives and `subst`
/// drives belong to the sign-in that defined them.
pub(super) fn drive_scope(letter: char) -> DriveScope {
    let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain([0]).collect();
    // SAFETY: a NUL-terminated drive root; the call only classifies it.
    if unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE {
        return DriveScope::Mapped;
    }
    let device: Vec<u16> = format!("{letter}:").encode_utf16().chain([0]).collect();
    let mut target = vec![0u16; 32 * 1024];
    // SAFETY: a NUL-terminated device name and a writable buffer of the stated length.
    let length =
        unsafe { QueryDosDeviceW(device.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
    // `subst` links a drive letter to a path (`\??\C:\folder`); volumes are devices.
    let prefix: Vec<u16> = OsStr::new("\\??\\").encode_wide().collect();
    if length as usize > prefix.len() && target.starts_with(&prefix) {
        DriveScope::Substituted
    } else {
        DriveScope::Shared
    }
}

/// Whether this process already runs as administrator.
fn process_is_elevated() -> bool {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: the current-process pseudo handle needs no closing; the token is owned below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    // SAFETY: a token handle this call opened and nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = 0u32;
    // SAFETY: a writable TOKEN_ELEVATION of the stated size.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&raw mut elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
    };
    read != 0 && elevation.TokenIsElevated != 0
}

fn terminated(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut units = wide_units(value)?;
    units.push(0);
    Ok(units)
}

/// Runs the `runas` launch on its own short-lived STA thread: Shell verbs may be
/// delegated to COM extensions, and a backend worker keeps no apartment afterwards. It
/// returns when the user has answered the consent prompt.
fn shell_execute(spec: &ElevatedLaunch) -> io::Result<OwnedHandle> {
    let verb = terminated(OsStr::new(spec.verb))?;
    let file = terminated(spec.file.as_os_str())?;
    let parameters = terminated(OsStr::new(spec.parameters))?;
    let directory = terminated(spec.directory.as_os_str())?;
    thread::Builder::new()
        .name("administrator-terminal".into())
        .spawn(move || {
            let _apartment = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32)
                .ok_or_else(|| io::Error::other("cannot initialize the Shell's COM apartment"))?;
            // SAFETY: SHELLEXECUTEINFOW is a plain C structure; all-zero is a valid start.
            let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
            // NOASYNC: this thread ends right after; FLAG_NO_UI: errors return, no dialog.
            info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpParameters = parameters.as_ptr();
            info.lpDirectory = directory.as_ptr();
            info.nShow = SW_SHOWNORMAL;
            // SAFETY: every terminated string outlives the call; the only process this
            // starts is the absolute shell, elevated by Windows after the user's consent.
            if unsafe { ShellExecuteExW(&mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if info.hProcess.is_null() || info.hProcess == INVALID_HANDLE_VALUE {
                return Err(io::Error::other(
                    "Windows started the administrator terminal without a process to track.",
                ));
            }
            // SAFETY: NOCLOSEPROCESS transfers the returned process handle to the caller.
            Ok(unsafe { OwnedHandle::from_raw_handle(info.hProcess) })
        })?
        .join()
        .map_err(|_| io::Error::other("The administrator terminal launch failed."))?
}
