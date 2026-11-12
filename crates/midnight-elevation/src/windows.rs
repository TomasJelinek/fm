//! UAC launches a dedicated helper; a local named pipe is bound to both process identities.
use crate::{
    cancelled,
    protocol::{self, Reply, Request},
};
use midnight_core::Cancellation;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, FILETIME,
        HANDLE, INVALID_HANDLE_VALUE, LocalFree, WAIT_TIMEOUT,
    },
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
    },
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId,
            GetNamedPipeServerProcessId, PIPE_NOWAIT, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PeekNamedPipe, SetNamedPipeHandleState,
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

#[path = "windows_children.rs"]
mod children;

const TIMEOUT: Duration = Duration::from_secs(300);
const TICK: Duration = Duration::from_millis(10);

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
pub(crate) fn is_redirected(value: &fs::Metadata) -> bool {
    value.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
fn wide(value: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
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
    wide(std::ffi::OsStr::new(&format!(
        r"\\.\pipe\MidnightExplorer.List.{nonce}"
    )))
}
fn lock_executable(helper: &Path) -> io::Result<Vec<File>> {
    let mut locks = Vec::new();
    for path in helper.ancestors() {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        if is_redirected(&file.metadata()?) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Redirected helper path",
            ));
        }
        locks.push(file);
    }
    // Revalidate after locks prevent replacement of any checked component.
    midnight_plugin_host::validate_plugin_path(helper).map_err(io::Error::other)?;
    Ok(locks)
}
fn server(name: &[u16]) -> io::Result<OwnedHandle> {
    let sddl = wide(std::ffi::OsStr::new(
        "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)",
    ))?;
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
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    };
    let error = io::Error::last_os_error();
    unsafe {
        LocalFree(descriptor);
    }
    if raw == INVALID_HANDLE_VALUE {
        Err(error)
    } else {
        handle(raw)
    }
}

pub(crate) fn list(helper: &Path, request: &Request, cancel: &Cancellation) -> io::Result<Reply> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let mut child = children::Reservation::acquire()?;
    let _locks = lock_executable(helper)?;
    let (nonce, text) = nonce()?;
    let pipe = server(&pipe_name(&text)?)?;
    let executable = wide(helper.as_os_str())?;
    let directory = wide(
        helper
            .parent()
            .ok_or_else(|| protocol::invalid("Missing helper parent"))?
            .as_os_str(),
    )?;
    let verb = wide(std::ffi::OsStr::new("runas"))?;
    // SAFETY: querying our own process identity does not transfer a handle.
    let parent = unsafe { GetCurrentProcessId() };
    let created = stamp(unsafe { GetCurrentProcess() })?;
    let args = wide(std::ffi::OsStr::new(&format!(
        "--pipe {text} {parent} {created}"
    )))?;
    let mut launch: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    launch.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    launch.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    launch.lpVerb = verb.as_ptr();
    launch.lpFile = executable.as_ptr();
    launch.lpParameters = args.as_ptr();
    launch.lpDirectory = directory.as_ptr();
    launch.nShow = SW_HIDE;
    // SAFETY: all terminated strings remain live; this is the sole explicit UAC launch.
    if unsafe { ShellExecuteExW(&mut launch) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let peer = child.launched(handle(launch.hProcess)?)?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        check(cancel, peer.as_raw_handle(), deadline)?;
        if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) } != 0 {
            break;
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error().map(|v| v as u32) {
            Some(ERROR_PIPE_CONNECTED) => break,
            Some(ERROR_PIPE_LISTENING | ERROR_NO_DATA) => thread::sleep(TICK),
            _ => return Err(error),
        }
    }
    let mut client_pid = 0;
    // SAFETY: pipe and retained launched-process handles are live; output is initialized.
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut client_pid) } == 0
        || client_pid != unsafe { GetProcessId(peer.as_raw_handle()) }
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unexpected helper process",
        ));
    }
    let mut io = Channel {
        pipe,
        peer,
        cancel: cancel.clone(),
        deadline,
    };
    io.write_all(&nonce)?;
    let mut proof = [0; 32];
    io.read_exact(&mut proof)?;
    if proof != nonce {
        return Err(protocol::invalid("Helper nonce mismatch"));
    }
    protocol::write_request(&mut io, request)?;
    protocol::read_reply(&mut io)
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
    let peer = handle(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            parent_pid,
        )
    })?;
    if stamp(peer.as_raw_handle())? != created {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Parent process changed",
        ));
    }
    let name = pipe_name(&text)?;
    let deadline = Instant::now() + TIMEOUT;
    let cancel = Cancellation::default();
    let pipe = loop {
        check(&cancel, peer.as_raw_handle(), deadline)?;
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                0xc000_0000,
                0,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        if raw != INVALID_HANDLE_VALUE {
            break handle(raw)?;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) {
            return Err(error);
        }
        thread::sleep(TICK);
    };
    let mut server_pid = 0;
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut server_pid) } == 0
        || server_pid != parent_pid
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unexpected requesting process",
        ));
    }
    let mode = PIPE_NOWAIT;
    if unsafe { SetNamedPipeHandleState(pipe.as_raw_handle(), &mode, ptr::null(), ptr::null()) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut io = Channel {
        pipe,
        peer,
        cancel: cancel.clone(),
        deadline,
    };
    let mut challenge = [0; 32];
    io.read_exact(&mut challenge)?;
    if challenge != nonce {
        return Err(protocol::invalid("Request nonce mismatch"));
    }
    io.write_all(&nonce)?;
    let request = protocol::read_request(&mut io)?;
    let monitor_pipe = io.pipe.try_clone()?;
    let monitor_peer = io.peer.try_clone()?;
    let monitor_cancel = cancel.clone();
    let monitor = thread::spawn(move || {
        while !monitor_cancel.is_cancelled() {
            if check(&monitor_cancel, monitor_peer.as_raw_handle(), deadline).is_err()
                || unsafe {
                    PeekNamedPipe(
                        monitor_pipe.as_raw_handle(),
                        ptr::null_mut(),
                        0,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                    )
                } == 0
            {
                monitor_cancel.cancel();
                // This dedicated process owns only directory reads and the private channel.
                // End even if a native filesystem call is stuck and cannot poll cancellation.
                terminate();
                break;
            }
            thread::sleep(TICK);
        }
    });
    let reply = crate::helper::reply(request, &cancel);
    let result = protocol::write_reply(&mut io, &reply);
    cancel.cancel();
    let _ = monitor.join();
    result
}

fn check(cancel: &Cancellation, peer: HANDLE, deadline: Instant) -> io::Result<()> {
    if cancel.is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Authorized listing cancelled",
        ));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "Helper timeout"));
    }
    // SAFETY: peer is retained for this call; a zero timeout never blocks.
    if unsafe { WaitForSingleObject(peer, 0) } != WAIT_TIMEOUT {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "Peer process ended",
        ));
    }
    Ok(())
}
struct Channel {
    pipe: OwnedHandle,
    peer: OwnedHandle,
    cancel: Cancellation,
    deadline: Instant,
}
impl Read for Channel {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        loop {
            if self.cancel.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Authorized listing cancelled",
                ));
            }
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Helper timeout"));
            }
            let mut count = 0;
            let ok = unsafe {
                ReadFile(
                    self.pipe.as_raw_handle(),
                    data.as_mut_ptr(),
                    data.len().min(u32::MAX as usize) as u32,
                    &mut count,
                    ptr::null_mut(),
                )
            };
            if ok != 0 && count != 0 {
                return Ok(count as usize);
            }
            let error = io::Error::last_os_error();
            if ok == 0 && error.raw_os_error() != Some(ERROR_NO_DATA as i32) {
                return Err(error);
            }
            check(&self.cancel, self.peer.as_raw_handle(), self.deadline)?;
            thread::sleep(TICK);
        }
    }
}
impl Write for Channel {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        loop {
            check(&self.cancel, self.peer.as_raw_handle(), self.deadline)?;
            let mut count = 0;
            let ok = unsafe {
                WriteFile(
                    self.pipe.as_raw_handle(),
                    data.as_ptr(),
                    data.len().min(64 * 1024) as u32,
                    &mut count,
                    ptr::null_mut(),
                )
            };
            if ok != 0 && count != 0 {
                return Ok(count as usize);
            }
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            thread::sleep(TICK);
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_paths_preserve_unpaired_surrogates_and_reject_nul() {
        let path = PathBuf::from(OsString::from_wide(&[b'C' as u16, 58, 92, 0xd800]));
        assert_eq!(decode_path(&encode_path(&path)).unwrap(), path);
        assert!(decode_path(&[0]).is_err());
        assert!(decode_path(&[0, 0]).is_err());
    }
    #[test]
    fn authenticated_channel_reads_unicode_directory_without_elevation() {
        let qa = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/elevation");
        fs::create_dir_all(&qa).unwrap();
        let fixture = tempfile::Builder::new()
            .prefix("pipe-")
            .tempdir_in(qa)
            .unwrap();
        fs::write(fixture.path().join("Příliš_中文_🧪.txt"), b"test").unwrap();
        let (nonce, text) = nonce().unwrap();
        let pipe = server(&pipe_name(&text).unwrap()).unwrap();
        let pid = unsafe { GetCurrentProcessId() };
        let created = stamp(unsafe { GetCurrentProcess() }).unwrap();
        let args = vec![
            "--pipe".into(),
            text.into(),
            pid.to_string().into(),
            created.to_string().into(),
        ];
        let helper = thread::spawn(move || serve_with_monitor(&args, || {}));
        let peer = handle(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        })
        .unwrap();
        let cancel = Cancellation::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            check(&cancel, peer.as_raw_handle(), deadline).unwrap();
            if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) } != 0
                || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32)
            {
                break;
            }
            thread::sleep(TICK);
        }
        let mut peer_pid = 0;
        assert_ne!(
            unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &mut peer_pid) },
            0
        );
        assert_eq!(peer_pid, pid);
        let mut channel = Channel {
            pipe,
            peer,
            cancel,
            deadline,
        };
        channel.write_all(&nonce).unwrap();
        let mut proof = [0; 32];
        channel.read_exact(&mut proof).unwrap();
        assert_eq!(proof, nonce);
        protocol::write_request(
            &mut channel,
            &Request {
                path: encode_path(fixture.path()),
            },
        )
        .unwrap();
        let entries = protocol::read_reply(&mut channel).unwrap().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            decode_path(&entries[0].name).unwrap(),
            PathBuf::from("Příliš_中文_🧪.txt")
        );
        assert_eq!(entries[0].size, Some(4));
        helper.join().unwrap().unwrap();
    }

    fn test_channel() -> (Channel, OwnedHandle) {
        let (_, text) = nonce().unwrap();
        let name = pipe_name(&text).unwrap();
        let pipe = server(&name).unwrap();
        let client = handle(unsafe {
            CreateFileW(
                name.as_ptr(),
                0xc000_0000,
                0,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        })
        .unwrap();
        let connected = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) };
        assert!(
            connected != 0
                || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32)
        );
        let peer = handle(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                GetCurrentProcessId(),
            )
        })
        .unwrap();
        (
            Channel {
                pipe,
                peer,
                cancel: Cancellation::default(),
                deadline: Instant::now() + Duration::from_secs(2),
            },
            client,
        )
    }
    #[test]
    fn complete_buffered_reply_survives_closed_peer_and_empty_read_cancels() {
        let (mut channel, client) = test_channel();
        let reply: Reply = Ok(vec![]);
        let mut bytes = Vec::new();
        protocol::write_reply(&mut bytes, &reply).unwrap();
        let mut written = 0;
        assert_ne!(
            unsafe {
                WriteFile(
                    client.as_raw_handle(),
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut written,
                    ptr::null_mut(),
                )
            },
            0
        );
        assert_eq!(written as usize, bytes.len());
        drop(client);
        assert_eq!(protocol::read_reply(&mut channel).unwrap(), reply);
        let (mut channel, _client) = test_channel();
        let cancel = channel.cancel.clone();
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            cancel.cancel();
        });
        assert_eq!(
            channel.read_exact(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
        stopper.join().unwrap();
        assert_eq!(
            channel.write_all(&[1]).unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
    }

    #[test]
    fn helper_rejects_untrusted_startup_arguments_before_connecting() {
        assert!(serve(&[]).is_err());
        assert!(serve(&["--pipe".into(), "bad".into(), "1".into(), "1".into()]).is_err());
    }
}
