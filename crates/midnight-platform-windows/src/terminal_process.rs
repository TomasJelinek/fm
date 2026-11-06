//! Windows console process ownership, independent of file-manager diagnostic streams.
use midnight_core::TerminalProcess;
use std::{
    ffi::{OsStr, OsString},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::PathBuf,
    process::ExitStatus,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess,
        PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW, WaitForSingleObject,
    },
    UI::WindowsAndMessaging::SW_HIDE,
};

const MAX_PROCESS_UTF16: usize = 32767;

#[derive(Debug)]
pub(crate) struct LaunchSpec {
    pub application: PathBuf,
    pub arguments: Vec<OsString>,
    pub raw_tail: Option<OsString>,
    pub working_directory: PathBuf,
}

impl LaunchSpec {
    fn command_line(&self) -> io::Result<Vec<u16>> {
        let mut line = Vec::new();
        append_argument(&mut line, self.application.as_os_str(), true)?;
        for argument in &self.arguments {
            line.push(b' ' as u16);
            append_argument(&mut line, argument, false)?;
        }
        if let Some(tail) = &self.raw_tail {
            line.push(b' ' as u16);
            line.extend(wide_units(tail)?);
        }
        if line.len() >= MAX_PROCESS_UTF16 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows process command line exceeds its UTF16 limit",
            ));
        }
        line.push(0);
        Ok(line)
    }
}

fn wide_units(value: &OsStr) -> io::Result<Vec<u16>> {
    let units: Vec<_> = value.encode_wide().take(MAX_PROCESS_UTF16 + 1).collect();
    if units.contains(&0) || units.len() >= MAX_PROCESS_UTF16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows process arguments must fit native UTF16 limits and contain no NUL",
        ));
    }
    Ok(units)
}

fn terminated(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut units = wide_units(value)?;
    units.push(0);
    Ok(units)
}

/// Windows C-runtime quoting for ordinary argv; raw Cmd tails bypass this function.
fn append_argument(line: &mut Vec<u16>, argument: &OsStr, force_quotes: bool) -> io::Result<()> {
    let units = wide_units(argument)?;
    if !force_quotes
        && !units.is_empty()
        && !units
            .iter()
            .any(|unit| [b' ' as u16, b'\t' as u16, b'"' as u16].contains(unit))
    {
        line.extend(units);
        return Ok(());
    }
    line.push(b'"' as u16);
    let mut slashes = 0usize;
    for unit in units {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            line.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        line.push(unit);
        slashes = 0;
    }
    line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    line.push(b'"' as u16);
    Ok(())
}

struct NativeProcess {
    handle: OwnedHandle,
    id: u32,
    status: Option<ExitStatus>,
}

impl TerminalProcess for NativeProcess {
    fn id(&self) -> u32 {
        self.id
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        // SAFETY: a live owned process handle, queried without waiting on the backend.
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => return Ok(None),
            WAIT_OBJECT_0 => {}
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => return Err(io::Error::other("Unexpected process wait result")),
        }
        let mut code = 0;
        // SAFETY: the process is signaled, so even259 is a completed exit status.
        if unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let status = ExitStatus::from_raw(code);
        self.status = Some(status);
        Ok(Some(status))
    }
}

pub(crate) fn spawn(spec: &LaunchSpec) -> io::Result<Box<dyn TerminalProcess>> {
    spawn_inner(spec, false)
}

fn spawn_inner(spec: &LaunchSpec, hidden: bool) -> io::Result<Box<dyn TerminalProcess>> {
    if !spec.application.is_absolute() || !spec.working_directory.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Native terminal startup requires absolute paths",
        ));
    }
    let application = terminated(spec.application.as_os_str())?;
    let directory = terminated(spec.working_directory.as_os_str())?;
    let mut line = spec.command_line()?;
    let mut startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    // Do not set STARTF_USESTDHANDLES: the new console supplies its own keyboard
    // and screen buffers even when the file-manager streams point to log files.
    if hidden {
        startup.dwFlags = STARTF_USESHOWWINDOW;
        startup.wShowWindow = SW_HIDE as u16;
    }
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: mutable terminated Unicode command line, trusted explicit application,
    // exact native cwd and initialized structures. No handles are inherited; the
    // parent's Unicode environment is inherited without copying or logging it.
    let success = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
            ptr::null(),
            directory.as_ptr(),
            &startup,
            &mut information,
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreateProcessW transfers two valid owned handles to us.
    // Close the primary thread immediately; process tracking owns the process handle.
    let process = unsafe { OwnedHandle::from_raw_handle(information.hProcess) };
    drop(unsafe { OwnedHandle::from_raw_handle(information.hThread) });
    Ok(Box::new(NativeProcess {
        handle: process,
        id: information.dwProcessId,
        status: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            process::CommandExt,
        },
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    use windows_sys::Win32::{
        Storage::FileSystem::{FILE_TYPE_CHAR, FILE_TYPE_DISK, GetFileType},
        System::Console::{
            GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        },
    };

    #[test]
    fn command_line_preserves_unicode_crt_arguments_and_exact_raw_cmd_tail() {
        let spec = LaunchSpec {
            application: PathBuf::from(r"C:\Program Files\文档\pwsh.exe"),
            arguments: ["plain", "e\u{301} 文档\\", "a\"b", ""]
                .into_iter()
                .map(OsString::from)
                .collect(),
            raw_tail: None,
            working_directory: PathBuf::from(r"C:\文档"),
        };
        let encoded = spec.command_line().unwrap();
        let expected =
            "\"C:\\Program Files\\文档\\pwsh.exe\" plain \"e\u{301} 文档\\\\\" \"a\\\"b\" \"\"";
        assert_eq!(
            encoded,
            expected.encode_utf16().chain(Some(0)).collect::<Vec<_>>()
        );
        let raw_tail = "\"\"C:\\文档\\tool.exe\" \"! & e\u{301}\"\"";
        let spec = LaunchSpec {
            application: PathBuf::from(r"C:\Windows\System32\cmd.exe"),
            arguments: ["/D", "/U", "/V:OFF", "/S", "/K"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            raw_tail: Some(raw_tail.into()),
            working_directory: PathBuf::from(r"C:\文档"),
        };
        let encoded = spec.command_line().unwrap();
        assert_eq!(
            OsString::from_wide(&encoded[..encoded.len() - 1]),
            OsString::from(format!(
                "\"C:\\Windows\\System32\\cmd.exe\" /D /U /V:OFF /S /K {raw_tail}"
            ))
        );
    }

    #[test]
    fn command_line_keeps_long_native_paths_and_refuses_nul() {
        let application = PathBuf::from(format!("C:\\{}\\tool.exe", "文档\\".repeat(100)));
        let mut spec = LaunchSpec {
            application: application.clone(),
            arguments: vec![],
            raw_tail: None,
            working_directory: PathBuf::from(r"C:\"),
        };
        let encoded = spec.command_line().unwrap();
        assert!(application.as_os_str().encode_wide().count() > 260);
        assert_eq!(
            &encoded[1..encoded.len() - 2],
            application.as_os_str().encode_wide().collect::<Vec<_>>()
        );
        spec.raw_tail = Some("bad\0tail".into());
        assert_eq!(
            spec.command_line().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn child_gets_own_hidden_console_when_parent_standard_streams_are_redirected() {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().join("console_文档_👩🏽‍💻_e\u{301}");
        fs::create_dir_all(&cwd).unwrap();
        let log = directory.path().join("redirected-parent.log");
        assert_redirected_console(&cwd, &log);
    }

    #[test]
    fn long_native_cwd_is_not_truncated_or_replaced_when_startup_is_unsupported() {
        let directory = tempfile::tempdir().unwrap();
        let component = "console_文档_👩🏽‍💻_e\u{301}_".repeat(4);
        let cwd = directory
            .path()
            .join(&component)
            .join(&component)
            .join(&component)
            .join(&component);
        fs::create_dir_all(&cwd).unwrap();
        assert!(cwd.as_os_str().encode_wide().count() > 260);
        let spec = LaunchSpec {
            application: std::env::current_exe().unwrap(),
            arguments: [
                "--exact",
                "terminal_process::tests::console_probe_fixture",
                "--ignored",
                "--nocapture",
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
            raw_tail: None,
            working_directory: cwd,
        };
        let mut process = match spawn_inner(&spec, true) {
            Ok(process) => process,
            Err(error) => {
                // The SDK documents CreateProcessW cwd limitations separately from
                // long-path file I/O opt-in. Preserve its error and never launch in
                // a shortened/other directory. This host reports267 even with an
                // extracted longPathAware manifest and LongPathsEnabled=1.
                assert!(
                    matches!(error.raw_os_error(), Some(267 | 206)),
                    "unexpected native startup error: {error}"
                );
                return;
            }
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = process.try_wait().unwrap() {
                assert_eq!(status.code(), Some(259));
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_redirected_console(cwd: &std::path::Path, log: &std::path::Path) {
        let output = fs::File::create(log).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "terminal_process::tests::redirected_parent_fixture",
                "--ignored",
                "--nocapture",
            ])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stderr(output.try_clone().unwrap())
            .stdout(output)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "redirected parent fixture failed: {}",
            fs::read_to_string(log).unwrap()
        );
    }

    #[test]
    #[ignore = "subprocess fixture executed by child_gets_own_hidden_console_when_parent_standard_streams_are_redirected"]
    fn redirected_parent_fixture() {
        // SAFETY: query this process's standard output handle; no handle ownership changes.
        assert_eq!(
            unsafe { GetFileType(GetStdHandle(STD_OUTPUT_HANDLE)) },
            FILE_TYPE_DISK
        );
        let spec = LaunchSpec {
            application: std::env::current_exe().unwrap(),
            arguments: [
                "--exact",
                "terminal_process::tests::console_probe_fixture",
                "--ignored",
                "--nocapture",
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
            raw_tail: None,
            working_directory: std::env::current_dir().unwrap(),
        };
        let mut child = spawn_inner(&spec, true).unwrap();
        assert_ne!(child.id(), 0);
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "hidden console fixture did not terminate"
            );
            thread::sleep(Duration::from_millis(10));
        };
        // 259 must be treated as a real exit code once the process is signaled.
        assert_eq!(status.code(), Some(259));
        assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(259));
    }

    #[test]
    #[ignore = "hidden new-console subprocess fixture executed by redirected_parent_fixture"]
    fn console_probe_fixture() {
        for standard in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE, STD_INPUT_HANDLE] {
            // SAFETY: queries on this fixture's live standard handles only.
            let handle = unsafe { GetStdHandle(standard) };
            assert_eq!(
                unsafe { GetFileType(handle) },
                FILE_TYPE_CHAR,
                "standard stream inherited a redirected parent handle"
            );
            let mut mode = 0;
            assert_ne!(
                unsafe { GetConsoleMode(handle, &mut mode) },
                0,
                "standard stream is not attached to the new console"
            );
        }
        assert!(
            std::env::current_dir()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("文档")
        );
        std::process::exit(259);
    }
}
