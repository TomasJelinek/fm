//! UAC launches a dedicated helper; a local named pipe is bound to both process identities.
use crate::{
    cancelled,
    protocol::{self, Reply, Request},
};
use midnight_core::Cancellation;
use std::{
    ffi::{OsStr, OsString},
    fs::{self, DirEntry, FileType},
    io::{self, Read, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileTypeExt, MetadataExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_PIPE_BUSY, FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
        LocalFree, WAIT_TIMEOUT,
    },
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    },
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Pipes::{
            CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
            WaitNamedPipeW,
        },
        Threading::{
            GetCurrentProcess, GetCurrentProcessId, GetProcessId, GetProcessTimes, OpenProcess,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        },
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::SW_HIDE,
    },
};

#[path = "windows_channel.rs"]
mod channel;
#[path = "windows_children.rs"]
mod children;
#[path = "windows_trust.rs"]
mod trust;
use channel::{Channel, Monitor};
pub(crate) use trust::HELPER_REQUIREMENT;

const TIMEOUT: Duration = Duration::from_secs(300);
/// Wait between attempts while another client holds the single pipe instance.
const BUSY_WAIT: Duration = Duration::from_millis(50);

pub(crate) fn encode_path(path: &Path) -> Vec<u8> {
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}
pub(crate) fn decode_path(bytes: &[u8]) -> io::Result<PathBuf> {
    if !bytes.len().is_multiple_of(2) {
        return Err(protocol::invalid("Odd UTF-16 path byte count"));
    }
    let units: Vec<_> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .map(u16::from_le_bytes)
        .collect();
    if units.contains(&0) {
        return Err(protocol::invalid("NUL in native path"));
    }
    Ok(PathBuf::from(OsString::from_wide(&units)))
}
pub(crate) fn metadata(value: &fs::Metadata) -> (Option<u32>, Option<u32>) {
    (Some(value.file_attributes()), None)
}
/// Directory symbolic links and junctions are directory links by their own attributes. The
/// elevated helper never opens a link target: a target could be another process's pipe server.
pub(crate) fn is_directory_link(_: &DirEntry, file_type: &FileType) -> bool {
    file_type.is_symlink_dir()
}
/// Refuse a helper that a non-administrative account could change before it runs elevated.
pub(crate) fn protected_helper(helper: &Path) -> io::Result<()> {
    trust::pin_helper(helper).map(drop)
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut result: Vec<_> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err(protocol::invalid("NUL in helper argument"));
    }
    result.push(0);
    Ok(result)
}
fn handle(raw: HANDLE) -> io::Result<OwnedHandle> {
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful native creation returned one owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}
fn stamp(process: HANDLE) -> io::Result<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: process has query rights and all output pointers are initialized.
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
fn nonce() -> io::Result<([u8; 32], String)> {
    let mut raw = [0; 32];
    getrandom::fill(&mut raw).map_err(|error| io::Error::other(error.to_string()))?;
    Ok((raw, raw.iter().map(|v| format!("{v:02x}")).collect()))
}
fn parse_nonce(value: &OsString) -> io::Result<([u8; 32], String)> {
    let text = value
        .to_str()
        .ok_or_else(|| protocol::invalid("Invalid channel nonce"))?;
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(protocol::invalid("Invalid channel nonce"));
    }
    let mut raw = [0; 32];
    for (index, byte) in raw.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| protocol::invalid("Invalid channel nonce"))?;
    }
    Ok((raw, text.to_owned()))
}
fn pipe_name(nonce: &str) -> io::Result<Vec<u16>> {
    wide(OsStr::new(&format!(
        r"\\.\pipe\MidnightExplorer.List.{nonce}"
    )))
}
fn query_process(pid: u32) -> io::Result<OwnedHandle> {
    // SAFETY: query/wait access only; ownership is taken only after success.
    handle(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    })
}
fn server(name: &[u16]) -> io::Result<OwnedHandle> {
    let sddl = wide(OsStr::new("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)"))?;
    let mut descriptor = ptr::null_mut();
    // SAFETY: a terminated static descriptor string and valid output pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: descriptor remains valid through creation; handles are never inheritable.
    let raw = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    };
    let error = io::Error::last_os_error();
    // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW allocated the descriptor.
    unsafe {
        LocalFree(descriptor);
    }
    if raw == INVALID_HANDLE_VALUE {
        Err(error)
    } else {
        handle(raw)
    }
}

/// Run the consent launch on its own short-lived STA: Shell verbs may be delegated to COM
/// extensions, and a backend worker thread must not keep an apartment afterwards.
fn launch_elevated(
    executable: Vec<u16>,
    parameters: Vec<u16>,
    directory: Vec<u16>,
) -> io::Result<OwnedHandle> {
    let verb = wide(OsStr::new("runas"))?;
    thread::Builder::new()
        .name("authorization-launch".into())
        .spawn(move || {
            // SAFETY: this new thread initializes and later uninitializes its own apartment.
            let apartment = unsafe {
                CoInitializeEx(
                    ptr::null(),
                    (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
                )
            };
            // SAFETY: SHELLEXECUTEINFOW is a plain C structure; all-zero is a valid start.
            let mut launch: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            launch.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
            launch.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
            launch.lpVerb = verb.as_ptr();
            launch.lpFile = executable.as_ptr();
            launch.lpParameters = parameters.as_ptr();
            launch.lpDirectory = directory.as_ptr();
            launch.nShow = SW_HIDE;
            // SAFETY: all terminated strings remain live; this is the sole explicit UAC launch.
            let result = if unsafe { ShellExecuteExW(&mut launch) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                handle(launch.hProcess)
            };
            if apartment >= 0 {
                // SAFETY: balances the successful initialization on this thread.
                unsafe { CoUninitialize() };
            }
            result
        })?
        .join()
        .map_err(|_| io::Error::other("Authorization launch failed"))?
}

pub(crate) fn list(helper: &Path, request: &Request, cancel: &Cancellation) -> io::Result<Reply> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let mut child = children::Reservation::acquire()?;
    // The pins prevent replacement of every validated component until this call returns,
    // which is after the elevated process has opened its image.
    let pins = trust::pin_helper(helper)?;
    let pinned = pins
        .first()
        .ok_or_else(|| protocol::invalid("Missing authorization helper"))?;
    let executable = trust::shell_path(helper, pinned);
    let (nonce, text) = nonce()?;
    let pipe = server(&pipe_name(&text)?)?;
    let directory = executable
        .parent()
        .ok_or_else(|| protocol::invalid("Missing helper parent"))?;
    // SAFETY: querying our own process identity does not transfer a handle.
    let parent = unsafe { GetCurrentProcessId() };
    let created = stamp(unsafe { GetCurrentProcess() })?;
    let parameters = wide(OsStr::new(&format!("--pipe {text} {parent} {created}")))?;
    let process = launch_elevated(
        wide(executable.as_os_str())?,
        parameters,
        wide(directory.as_os_str())?,
    )?;
    let peer = child.launched(process)?;
    let mut io = Channel::new(pipe, cancel.clone(), Instant::now() + TIMEOUT)?;
    io.connect(peer.as_raw_handle())?;
    let mut client_pid = 0;
    // SAFETY: pipe and retained launched-process handles are live; output is initialized.
    if unsafe { GetNamedPipeClientProcessId(io.pipe().as_raw_handle(), &mut client_pid) } == 0
        || client_pid != unsafe { GetProcessId(peer.as_raw_handle()) }
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unexpected helper process",
        ));
    }
    // The nonce is visible in the pipe name and command line; the PID and process-time checks
    // above and in the helper bind the channel. Matching it rejects a stale or foreign helper.
    io.write_all(&nonce)?;
    let mut proof = [0; 32];
    io.read_exact(&mut proof)?;
    if proof != nonce {
        return Err(protocol::invalid("Helper nonce mismatch"));
    }
    protocol::write_request(&mut io, request)?;
    let reply = protocol::read_reply(&mut io);
    drop(pins);
    reply
}

/// Open the requester's pipe as an elevated client. Identification-only quality of service
/// means no pipe server, even one that squats on the name, can impersonate this process.
fn connect_client(name: &[u16], requester: HANDLE, deadline: Instant) -> io::Result<OwnedHandle> {
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "Helper timeout"));
        }
        // SAFETY: requester is retained by the caller; a zero timeout never blocks.
        if unsafe { WaitForSingleObject(requester, 0) } != WAIT_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Requesting process ended",
            ));
        }
        // SAFETY: a terminated pipe name; ownership is taken only on success.
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                ptr::null_mut(),
            )
        };
        if raw != INVALID_HANDLE_VALUE {
            return handle(raw);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) {
            return Err(error);
        }
        // SAFETY: a terminated pipe name; this bounded wait returns when an instance listens.
        unsafe { WaitNamedPipeW(name.as_ptr(), channel::millis(BUSY_WAIT)) };
    }
}

pub(crate) fn serve(args: &[OsString]) -> io::Result<()> {
    serve_with_monitor(args, || std::process::exit(1))
}
fn serve_with_monitor(args: &[OsString], terminate: fn()) -> io::Result<()> {
    if args.len() != 4 || args[0] != "--pipe" {
        return Err(protocol::invalid("Invalid helper arguments"));
    }
    let (nonce, text) = parse_nonce(&args[1])?;
    let parent_pid: u32 = args[2]
        .to_str()
        .and_then(|v| v.parse().ok())
        .filter(|pid| *pid != 0)
        .ok_or_else(|| protocol::invalid("Invalid parent PID"))?;
    let created: u64 = args[3]
        .to_str()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| protocol::invalid("Invalid parent identity"))?;
    let peer = query_process(parent_pid)?;
    if stamp(peer.as_raw_handle())? != created {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Parent process changed",
        ));
    }
    let deadline = Instant::now() + TIMEOUT;
    let pipe = connect_client(&pipe_name(&text)?, peer.as_raw_handle(), deadline)?;
    let mut server_pid = 0;
    // SAFETY: the live pipe handle and an initialized output.
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut server_pid) } == 0
        || server_pid != parent_pid
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unexpected requesting process",
        ));
    }
    let cancel = Cancellation::default();
    let mut io = Channel::new(pipe, cancel.clone(), deadline)?;
    let mut challenge = [0; 32];
    io.read_exact(&mut challenge)?;
    if challenge != nonce {
        return Err(protocol::invalid("Request nonce mismatch"));
    }
    io.write_all(&nonce)?;
    let request = protocol::read_request(&mut io)?;
    let monitor = Monitor::start(io.pipe(), peer, deadline, cancel.clone(), terminate)?;
    let reply = crate::helper::reply(request, &cancel);
    monitor.stop();
    protocol::write_reply(&mut io, &reply)
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
