use crate::WindowsFileSystem;
use crate::terminal_process::{self, LaunchSpec};
use base64::{Engine, engine::general_purpose::STANDARD};
use midnight_core::{
    FsError, LocationKind, LocationService, NavigationLocation, PropertyService, ShellFlavor,
    TerminalMode, TerminalProcess, TerminalRequest, TerminalService,
};
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
    ptr,
};
use windows_sys::{
    Win32::{
        System::{
            Com::{
                COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
            SystemInformation::GetSystemDirectoryW,
        },
        UI::Shell::{
            FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Profile,
            FOLDERID_ProgramFiles, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath, SHOP_FILEPATH,
            SHObjectProperties,
        },
    },
    core::GUID,
};

/// Thread-local COM ownership; never moved out of the native call's scope.
pub(crate) struct ComGuard(std::marker::PhantomData<std::rc::Rc<()>>);
impl ComGuard {
    pub(crate) fn initialize(mode: u32) -> Option<Self> {
        // SAFETY: reserved pointer is null; initialization and its guard stay on this thread.
        let result = unsafe { CoInitializeEx(ptr::null(), mode | COINIT_DISABLE_OLE1DDE as u32) };
        (result >= 0).then(|| Self(std::marker::PhantomData))
    }
}
impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: each guard corresponds to exactly one successful CoInitializeEx call.
        unsafe { CoUninitialize() };
    }
}

impl TerminalService for WindowsFileSystem {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::PowerShell
    }

    fn launch_terminal(
        &self,
        request: &TerminalRequest,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        validate_terminal_request(request).map_err(|error| {
            FsError::from_io("launch terminal", &request.working_directory, error)
        })?;
        let executable = match request.shell {
            ShellFlavor::PowerShell => powershell_path(),
            ShellFlavor::Cmd => command_prompt_path(),
            ShellFlavor::Posix => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "POSIX shell is unavailable on this Windows adapter",
            )),
        }
        .map_err(|error| {
            FsError::from_io("find system shell", &request.working_directory, error)
        })?;
        terminal_command(&executable, request)
            .and_then(|spec| terminal_process::spawn(&spec))
            .map_err(|error| FsError::from_io("launch terminal", &request.working_directory, error))
    }
}

fn terminal_command(executable: &Path, request: &TerminalRequest) -> io::Result<LaunchSpec> {
    validate_terminal_request(request)?;
    if !executable.is_absolute()
        || !request.working_directory.is_absolute()
        || request.command.contains('\0')
        || request
            .working_directory
            .as_os_str()
            .encode_wide()
            .any(|unit| unit == 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal requires absolute native paths and arguments without NUL",
        ));
    }
    let mut arguments = Vec::<OsString>::new();
    let mut raw_tail = None;
    match request.shell {
        ShellFlavor::PowerShell => {
            arguments.extend(
                ["-NoLogo", "-NoProfile", "-NoExit"]
                    .into_iter()
                    .map(OsString::from),
            );
            if !request.command.is_empty() {
                let utf16: Vec<u8> = request
                    .command
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect();
                arguments.push("-EncodedCommand".into());
                arguments.push(STANDARD.encode(utf16).into());
            }
        }
        ShellFlavor::Cmd => {
            arguments.extend(["/D", "/U", "/V:OFF"].into_iter().map(OsString::from));
            if !request.command.is_empty() {
                // cmd /S removes exactly these outer quotes. raw_arg preserves the explicit
                // source inside them; C-runtime argument escaping would alter cmd syntax.
                arguments.extend(["/S", "/K"].into_iter().map(OsString::from));
                raw_tail = Some(format!("\"{}\"", request.command).into());
            }
        }
        ShellFlavor::Posix => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "POSIX shell is unavailable on this Windows adapter",
            ));
        }
    }
    Ok(LaunchSpec {
        application: executable.into(),
        arguments,
        raw_tail,
        working_directory: request.working_directory.clone(),
    })
}

fn validate_terminal_request(request: &TerminalRequest) -> io::Result<()> {
    if request.mode != TerminalMode::Standard {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Elevated Windows terminals are not supported",
        ));
    }
    if request.shell == ShellFlavor::Posix {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "POSIX shell is unavailable on this Windows adapter",
        ));
    }
    if request.shell == ShellFlavor::Cmd
        && matches!(request.working_directory.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _)))
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Command Prompt cannot use a UNC working directory; choose PowerShell.",
        ));
    }
    Ok(())
}

pub(crate) fn powershell_path() -> io::Result<PathBuf> {
    let _com = ComGuard::initialize(windows_sys::Win32::System::Com::COINIT_MULTITHREADED as u32);
    if let Some(program_files) = _com
        .as_ref()
        .and_then(|_| known_folder(&FOLDERID_ProgramFiles))
    {
        let pwsh = program_files.join("PowerShell").join("7").join("pwsh.exe");
        if pwsh.is_file() {
            return Ok(pwsh);
        }
    }
    let executable = system_directory()?
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    installed_shell(executable, "PowerShell")
}

fn command_prompt_path() -> io::Result<PathBuf> {
    installed_shell(system_directory()?.join("cmd.exe"), "Command Prompt")
}

fn installed_shell(executable: PathBuf, name: &str) -> io::Result<PathBuf> {
    if executable.is_file() {
        Ok(executable)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name} is not installed in a supported system location"),
        ))
    }
}

pub(crate) fn system_directory() -> io::Result<PathBuf> {
    // Query the OS rather than mutable PATH/SystemRoot/COMSPEC environment variables.
    let mut buffer = vec![0u16; 256];
    loop {
        // SAFETY: buffer is writable for its stated capacity.
        let length =
            unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length < buffer.len() {
            return Ok(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        buffer.resize(length + 1, 0);
    }
}

fn property_path(paths: &[PathBuf]) -> Option<Vec<u16>> {
    let [path] = paths else {
        return None;
    };
    if !path.is_absolute() {
        return None;
    }
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    Some(wide)
}

impl PropertyService for WindowsFileSystem {
    fn show_properties(&self, paths: &[PathBuf]) -> Result<bool, FsError> {
        let Some(wide) = property_path(paths) else {
            return Ok(false);
        };
        // Invoke only the native Properties command, on its own STA worker scope.
        let Some(_com) = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32) else {
            return Ok(false);
        };
        // SAFETY: a live NUL-terminated Unicode path, no owner, and optional default page.
        Ok(unsafe {
            SHObjectProperties(
                ptr::null_mut(),
                SHOP_FILEPATH as u32,
                wide.as_ptr(),
                ptr::null(),
            )
        } != 0)
    }
}

pub(crate) fn known_folder(id: &GUID) -> Option<PathBuf> {
    let mut raw = ptr::null_mut();
    // SAFETY: id and output are valid; OS allocates a NUL-terminated Unicode string.
    let result =
        unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY as u32, ptr::null_mut(), &mut raw) };
    if raw.is_null() {
        return None;
    }
    let path = if result >= 0 {
        let mut length = 0;
        // SAFETY: successful API guarantees a NUL-terminated allocation; no MAX_PATH buffer.
        unsafe {
            while *raw.add(length) != 0 {
                length += 1;
            }
            Some(PathBuf::from(OsString::from_wide(
                std::slice::from_raw_parts(raw, length),
            )))
        }
    } else {
        None
    };
    // SAFETY: SHGetKnownFolderPath allocated this string with COM's allocator.
    unsafe { CoTaskMemFree(raw.cast()) };
    path.filter(|path| path.is_absolute())
}

impl LocationService for WindowsFileSystem {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        let mut locations: Vec<_> = crate::platform_volumes()?
            .into_iter()
            .map(|volume| {
                let drive_letter = volume
                    .root
                    .as_os_str()
                    .encode_wide()
                    .next()
                    .and_then(|ch| char::from_u32(u32::from(ch)))
                    .filter(char::is_ascii_alphabetic);
                NavigationLocation {
                    id: volume.id,
                    label: volume.label,
                    path: volume.root,
                    kind: LocationKind::Volume,
                    drive_letter,
                }
            })
            .collect();
        let _com =
            ComGuard::initialize(windows_sys::Win32::System::Com::COINIT_MULTITHREADED as u32);
        for (id, label, kind, folder) in [
            ("home", "Home", LocationKind::Home, FOLDERID_Profile),
            (
                "desktop",
                "Desktop",
                LocationKind::Desktop,
                FOLDERID_Desktop,
            ),
            (
                "documents",
                "Documents",
                LocationKind::Documents,
                FOLDERID_Documents,
            ),
            (
                "downloads",
                "Downloads",
                LocationKind::Downloads,
                FOLDERID_Downloads,
            ),
        ] {
            let path = _com
                .as_ref()
                .and_then(|_| known_folder(&folder))
                .or_else(|| {
                    if kind == LocationKind::Home {
                        self.home.clone()
                    } else {
                        None
                    }
                });
            if let Some(path) = path {
                locations.push(NavigationLocation {
                    id: id.into(),
                    label: label.into(),
                    path,
                    kind,
                    drive_letter: None,
                });
            }
        }
        Ok(locations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_com_initialization_preserves_the_callers_existing_apartment() {
        std::thread::spawn(|| {
            use windows_sys::Win32::System::Com::CoGetApartmentType;
            let _apartment = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32)
                .expect("initialize the isolated test thread's STA");
            let apartment_type = || {
                let mut apartment = 0;
                let mut qualifier = 0;
                // SAFETY: both output pointers are live and the current thread owns its STA.
                let result = unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) };
                (result, apartment, qualifier)
            };
            let before = apartment_type();
            assert!(before.0 >= 0);
            assert!(
                ComGuard::initialize(windows_sys::Win32::System::Com::COINIT_MULTITHREADED as u32)
                    .is_none(),
                "a conflicting MTA initialization must fail"
            );
            assert_eq!(
                apartment_type(),
                before,
                "failed initialization must not uninitialize the existing caller-owned STA"
            );
        })
        .join()
        .unwrap();
    }
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::PathBuf};

    #[test]
    fn empty_terminal_launch_has_only_interactive_shell_arguments() {
        let directory = PathBuf::from(r"C:\文档\👩🏽‍💻 & !");
        for (shell, executable, expected) in [
            (
                ShellFlavor::PowerShell,
                r"C:\Program Files\PowerShell\7\pwsh.exe",
                ["-NoLogo", "-NoProfile", "-NoExit"],
            ),
            (
                ShellFlavor::Cmd,
                r"C:\Windows\System32\cmd.exe",
                ["/D", "/U", "/V:OFF"],
            ),
        ] {
            let request = TerminalRequest {
                shell,
                mode: TerminalMode::Standard,
                command: String::new(),
                working_directory: directory.clone(),
            };
            let command = terminal_command(Path::new(executable), &request).unwrap();
            let arguments: Vec<_> = command.arguments.iter().map(OsString::as_os_str).collect();
            assert_eq!(arguments, expected.map(OsStr::new));
            assert!(command.raw_tail.is_none());
            assert_eq!(command.application, Path::new(executable));
            assert_eq!(command.working_directory, directory);
        }
    }

    #[test]
    fn empty_terminal_launch_keeps_native_path_and_nul_validation() {
        for shell in [ShellFlavor::PowerShell, ShellFlavor::Cmd] {
            for (command, directory) in [
                ("", "relative"),
                ("", "C:\\bad\0path"),
                ("echo\0bad", "C:\\valid"),
            ] {
                let request = TerminalRequest {
                    shell,
                    mode: TerminalMode::Standard,
                    command: command.into(),
                    working_directory: directory.into(),
                };
                assert_eq!(
                    terminal_command(Path::new(r"C:\Windows\System32\cmd.exe"), &request)
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::InvalidInput
                );
            }
        }
    }

    #[test]
    fn terminal_passes_exact_unicode_command_as_utf16le_base64_and_separate_cwd() {
        let text = "Write-Output '文档 👩🏽‍💻 e\u{301} ‘quote’' ; $x = 1";
        let directory = PathBuf::from("C:\\文档\\$() ' ; ‘ ’");
        let request = TerminalRequest {
            shell: ShellFlavor::PowerShell,
            mode: midnight_core::TerminalMode::Standard,
            command: text.into(),
            working_directory: directory.clone(),
        };
        let command = terminal_command(
            Path::new("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
            &request,
        )
        .unwrap();
        assert_eq!(command.working_directory, directory);
        let args: Vec<_> = command.arguments.iter().map(OsString::as_os_str).collect();
        assert_eq!(
            &args[..4],
            &[
                OsStr::new("-NoLogo"),
                OsStr::new("-NoProfile"),
                OsStr::new("-NoExit"),
                OsStr::new("-EncodedCommand")
            ]
        );
        let decoded = STANDARD.decode(args[4].to_str().unwrap()).unwrap();
        let expected: Vec<_> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decoded, expected);
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn properties_preserve_long_unicode_path_and_refuse_multi_selection_or_nul() {
        let path = PathBuf::from(format!("C:\\{}\\文档👩🏽‍💻.txt", "segment\\".repeat(80)));
        let native = property_path(std::slice::from_ref(&path)).unwrap();
        let mut expected: Vec<u16> = path.as_os_str().encode_wide().collect();
        expected.push(0);
        assert_eq!(native, expected);
        assert!(property_path(&[path.clone(), path]).is_none());
        assert!(property_path(&[PathBuf::from("C:\\bad\0.txt")]).is_none());
        assert!(property_path(&[PathBuf::from("relative.txt")]).is_none());
    }

    #[test]
    fn cmd_uses_exact_outer_quoted_source_with_autorun_and_delayed_expansion_off() {
        let source = "\"C:\\文档\\tool.exe\" \"e\u{301} & data!\" & echo %USERPROFILE%";
        let request = TerminalRequest {
            command: source.into(),
            working_directory: PathBuf::from("C:\\👩🏽‍💻 ‘ ’ & !"),
            shell: ShellFlavor::Cmd,
            mode: midnight_core::TerminalMode::Standard,
        };
        let command =
            terminal_command(Path::new("C:\\Windows\\System32\\cmd.exe"), &request).unwrap();
        let args: Vec<_> = command.arguments.iter().map(OsString::as_os_str).collect();
        assert_eq!(
            &args[..5],
            &[
                OsStr::new("/D"),
                OsStr::new("/U"),
                OsStr::new("/V:OFF"),
                OsStr::new("/S"),
                OsStr::new("/K")
            ]
        );
        assert_eq!(args.len(), 5);
        assert_eq!(
            command.raw_tail.as_deref(),
            Some(OsStr::new(&format!("\"{source}\"")))
        );
        assert_eq!(command.working_directory, request.working_directory);
    }

    #[test]
    fn windows_rejects_unsupported_shell_and_elevation_without_fallback() {
        for (shell, mode) in [
            (ShellFlavor::Posix, midnight_core::TerminalMode::Standard),
            (
                ShellFlavor::PowerShell,
                midnight_core::TerminalMode::Elevated,
            ),
            (ShellFlavor::Cmd, midnight_core::TerminalMode::Elevated),
        ] {
            let request = TerminalRequest {
                command: "echo 文档".into(),
                working_directory: PathBuf::from("C:\\"),
                shell,
                mode,
            };
            assert_eq!(
                terminal_command(Path::new("C:\\Windows\\System32\\cmd.exe"), &request)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn cmd_rejects_unc_working_directory_instead_of_running_in_another_directory() {
        for command in ["", "echo 文档"] {
            let request = TerminalRequest {
                command: command.into(),
                working_directory: PathBuf::from(r"\\nonexistent.invalid\share\文档"),
                shell: ShellFlavor::Cmd,
                mode: TerminalMode::Standard,
            };
            assert_eq!(
                terminal_command(Path::new(r"C:\Windows\System32\cmd.exe"), &request)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Unsupported
            );
        }
    }
}
