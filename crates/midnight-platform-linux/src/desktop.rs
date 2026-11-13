use crate::LinuxFileSystem;
use midnight_core::{
    FsError, LocationKind, LocationService, NavigationLocation, PropertyService, ShellFlavor,
    TerminalMode, TerminalProcess, TerminalRequest, TerminalService,
};
use std::{
    fs,
    io::{self, Read},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

const TERMINALS: &[&str] = &[
    "/usr/bin/xterm",
    "/usr/bin/konsole",
    "/usr/bin/gnome-terminal",
    "/usr/bin/xfce4-terminal",
    "/usr/bin/x-terminal-emulator",
];
const COMMAND_WRAPPER: &str = "eval \"$1\"; exec /bin/sh -i";

#[cfg(test)]
#[path = "user_command_terminal_tests.rs"]
mod user_command_tests;

impl TerminalService for LinuxFileSystem {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }

    fn launch_terminal(
        &self,
        request: &TerminalRequest,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        validate_terminal_request(request).map_err(|error| {
            FsError::from_io("launch terminal", &request.working_directory, error)
        })?;
        if request.mode == TerminalMode::Elevated && !is_executable(Path::new("/usr/bin/sudo")) {
            return Err(FsError::from_io(
                "find sudo",
                &request.working_directory,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "sudo is not installed at /usr/bin/sudo",
                ),
            ));
        }
        let executable = TERMINALS.iter().map(Path::new).find(|path| fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0))
            .ok_or_else(|| FsError::from_io("find system terminal", &request.working_directory, io::Error::new(io::ErrorKind::NotFound, "No supported system terminal is installed (xterm, Konsole, GNOME Terminal, Xfce Terminal).")))?;
        terminal_command(executable, request)
            .and_then(|mut command| command.spawn())
            .map(|child| Box::new(child) as Box<dyn TerminalProcess>)
            .map_err(|error| FsError::from_io("launch terminal", &request.working_directory, error))
    }

    fn launch_terminal_with_environment(
        &self,
        request: &TerminalRequest,
        environment: &[(String, std::ffi::OsString)],
        cancel: &midnight_core::Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        let mut command = prepare_user_terminal(request, environment, cancel)?;
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "Custom Command terminal",
                &request.working_directory,
                io::ErrorKind::Interrupted.into(),
            ));
        }
        command
            .spawn()
            .map(|child| Box::new(child) as Box<dyn TerminalProcess>)
            .map_err(|cause| {
                FsError::from_io("Custom Command terminal", &request.working_directory, cause)
            })
    }
}

pub(crate) fn prepare_user_terminal(
    request: &TerminalRequest,
    environment: &[(String, std::ffi::OsString)],
    cancel: &midnight_core::Cancellation,
) -> Result<Command, FsError> {
    let executable = TERMINALS
        .iter()
        .map(Path::new)
        .find(|path| is_executable(path))
        .ok_or_else(|| {
            FsError::from_io(
                "find system terminal",
                &request.working_directory,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "No supported system terminal is installed.",
                ),
            )
        })?;
    prepare_user_terminal_with(executable, request, environment, cancel)
}
fn prepare_user_terminal_with(
    executable: &Path,
    request: &TerminalRequest,
    environment: &[(String, std::ffi::OsString)],
    cancel: &midnight_core::Cancellation,
) -> Result<Command, FsError> {
    midnight_core::UserCommandProcess {
        working_directory: request.working_directory.clone(),
        invocation: midnight_core::UserCommandProcessInvocation::Terminal {
            shell: request.shell,
            script: request.command.clone(),
            environment: environment.to_vec(),
        },
    }
    .check(cancel)?;
    if request.mode != TerminalMode::Standard {
        return Err(FsError::from_io(
            "Custom Command terminal",
            &request.working_directory,
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Custom Commands use normal-user terminals only.",
            ),
        ));
    }
    if !fs::metadata(&request.working_directory)
        .map_err(|cause| FsError::from_io("Custom Command cwd", &request.working_directory, cause))?
        .is_dir()
    {
        return Err(FsError::from_io(
            "Custom Command cwd",
            &request.working_directory,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Command cwd is not a directory.",
            ),
        ));
    }
    let mut command = terminal_command(executable, request).map_err(|cause| {
        FsError::from_io("Custom Command terminal", &request.working_directory, cause)
    })?;
    for key in midnight_core::USER_COMMAND_ENVIRONMENT {
        command.env_remove(key);
    }
    command.envs(environment.iter().map(|(key, value)| (key, value)));
    crate::user_commands::validate_os_arguments(&command).map_err(|cause| {
        FsError::from_io("Custom Command terminal", &request.working_directory, cause)
    })?;
    Ok(command)
}

fn terminal_command(executable: &Path, request: &TerminalRequest) -> io::Result<Command> {
    validate_terminal_request(request)?;
    if !executable.is_absolute()
        || !request.working_directory.is_absolute()
        || request.command.contains('\0')
        || request
            .working_directory
            .as_os_str()
            .as_bytes()
            .contains(&0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal requires absolute native paths and arguments without NUL",
        ));
    }
    let mut command = Command::new(executable);
    match executable.file_name().and_then(|name| name.to_str()) {
        Some("gnome-terminal") => {
            command.args(["--wait", "--"]);
        }
        Some("konsole") => {
            command.args(["--nofork", "-e"]);
        }
        Some("xfce4-terminal") => {
            command.args(["--disable-server", "--execute"]);
        }
        _ => {
            command.arg("-e");
        }
    }
    if request.mode == TerminalMode::Elevated {
        // Password interaction belongs entirely to sudo inside the external terminal.
        // No -S flag, password pipe, output capture, or implicit elevation is used.
        command.args(["/usr/bin/sudo", "--"]);
    }
    if request.command.is_empty() {
        command.args(["/bin/sh", "-i"]);
    } else {
        // The fixed wrapper is not composed from filenames or user source. The submitted
        // command is one positional argument, explicitly evaluated only after submission.
        command
            .args(["/bin/sh", "-c", COMMAND_WRAPPER, "midnight-terminal"])
            .arg(&request.command);
    }
    command.current_dir(&request.working_directory);
    Ok(command)
}

fn validate_terminal_request(request: &TerminalRequest) -> io::Result<()> {
    if request.shell != ShellFlavor::Posix {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "This Linux adapter supports only its POSIX terminal",
        ));
    }
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

impl PropertyService for LinuxFileSystem {
    fn show_properties(&self, _paths: &[PathBuf]) -> Result<bool, FsError> {
        Ok(false)
    }
}

impl LocationService for LinuxFileSystem {
    fn supports_system_location(&self, location: midnight_core::SystemLocation) -> bool {
        crate::open::supports_system_location(location)
    }

    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        let mut locations: Vec<_> = crate::platform_volumes()?
            .into_iter()
            .map(|volume| NavigationLocation {
                id: volume.id,
                label: volume.label,
                path: volume.root,
                kind: LocationKind::Volume,
                drive_letter: None,
            })
            .collect();
        let temporary = std::env::temp_dir();
        if temporary.is_absolute() && temporary.is_dir() {
            locations.push(NavigationLocation {
                id: "temporary".into(),
                label: "Temporary files".into(),
                path: temporary,
                kind: LocationKind::Temporary,
                drive_letter: None,
            });
        }
        let Some(home) = self.home.as_ref().filter(|home| home.is_absolute()) else {
            return Ok(locations);
        };
        locations.push(NavigationLocation {
            id: "home".into(),
            label: "Home".into(),
            path: home.clone(),
            kind: LocationKind::Home,
            drive_letter: None,
        });
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let config = read_bounded(&config_root.join("user-dirs.dirs"), 64 * 1024)
            .and_then(|bytes| String::from_utf8(bytes).ok());
        for (key, label, kind) in [
            ("DESKTOP", "Desktop", LocationKind::Desktop),
            ("DOCUMENTS", "Documents", LocationKind::Documents),
            ("DOWNLOAD", "Downloads", LocationKind::Downloads),
            ("PICTURES", "Pictures", LocationKind::Pictures),
            ("VIDEOS", "Videos", LocationKind::Videos),
            ("MUSIC", "Music", LocationKind::Music),
        ] {
            // When configured to $HOME, XDG marks a location disabled. Do not fabricate
            // an English directory in that case. Conventional defaults apply only when
            // the configuration itself is unavailable.
            let path = match config.as_deref() {
                Some(input) => user_directory(input, key, home),
                None => Some(home.join(label)),
            };
            if let Some(path) = path.filter(|path| path.is_dir()) {
                locations.push(NavigationLocation {
                    id: label.to_ascii_lowercase(),
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

pub(crate) fn read_bounded(path: &Path, limit: usize) -> Option<Vec<u8>> {
    use rustix::fs::{Mode, OFlags, open};

    // A config symlink can resolve to a FIFO. Open without waiting for a writer,
    // then validate the opened descriptor, so a path replacement cannot bypass
    // the regular-file check. Regular-file symlinks remain supported.
    let descriptor = open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let file = fs::File::from(descriptor);
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

/// Parse the documented quoted user-directory format as data, never as shell code.
fn user_directory(input: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let wanted = format!("XDG_{key}_DIR");
    for line in input.lines().rev() {
        let Some((name, value)) = line.trim().split_once('=') else {
            continue;
        };
        if name.trim() != wanted {
            continue;
        }
        let value = value.trim().strip_prefix('"')?;
        let mut escaped = false;
        let mut end = None;
        for (index, ch) in value.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                end = Some(index);
                break;
            }
        }
        let end = end?;
        let tail = value[end + 1..].trim();
        if !tail.is_empty() && !tail.starts_with('#') {
            return None;
        }
        let raw = &value[..end];
        if raw == "$HOME" {
            return None;
        }
        let (relative, body) = match raw.strip_prefix("$HOME/") {
            Some(body) => (true, body),
            None => (false, raw),
        };
        let mut decoded = String::new();
        let mut chars = body.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    let escaped = chars.next()?;
                    if !['\\', '"', '$', '`'].contains(&escaped) {
                        return None;
                    }
                    decoded.push(escaped);
                }
                '$' | '`' | '\0' => return None,
                _ => decoded.push(ch),
            }
        }
        let path = if relative {
            home.join(decoded)
        } else {
            PathBuf::from(decoded)
        };
        return (path.is_absolute() && path != home).then_some(path);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn empty_terminal_launch_uses_interactive_shell_without_command_evaluation() {
        let directory = PathBuf::from("/home/文档/👩🏽‍💻 ; $()");
        for (executable, prefix) in [
            ("/usr/bin/xterm", vec!["-e"]),
            ("/usr/bin/konsole", vec!["--nofork", "-e"]),
            ("/usr/bin/gnome-terminal", vec!["--wait", "--"]),
            (
                "/usr/bin/xfce4-terminal",
                vec!["--disable-server", "--execute"],
            ),
            ("/usr/bin/x-terminal-emulator", vec!["-e"]),
        ] {
            for mode in [TerminalMode::Standard, TerminalMode::Elevated] {
                let request = TerminalRequest {
                    shell: ShellFlavor::Posix,
                    mode,
                    command: String::new(),
                    working_directory: directory.clone(),
                };
                let command = terminal_command(Path::new(executable), &request).unwrap();
                let mut expected = prefix.clone();
                if mode == TerminalMode::Elevated {
                    expected.extend(["/usr/bin/sudo", "--"]);
                }
                expected.extend(["/bin/sh", "-i"]);
                let arguments: Vec<_> = command.get_args().collect();
                assert_eq!(
                    arguments,
                    expected.iter().map(OsStr::new).collect::<Vec<_>>()
                );
                assert_eq!(command.get_program(), executable);
                assert_eq!(command.get_current_dir(), Some(directory.as_path()));
            }
        }
    }

    #[test]
    fn empty_terminal_launch_keeps_native_path_and_nul_validation() {
        for (command, directory) in [
            ("", "relative"),
            ("", "/bad\0path"),
            ("echo\0bad", "/valid"),
        ] {
            let request = TerminalRequest {
                shell: ShellFlavor::Posix,
                mode: TerminalMode::Standard,
                command: command.into(),
                working_directory: directory.into(),
            };
            assert_eq!(
                terminal_command(Path::new("/usr/bin/xterm"), &request)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    fn assert_fifo_read_does_not_wait_for_a_writer(use_symlink: bool) {
        use rustix::fs::{CWD, Mode, OFlags, mkfifoat, open};
        use std::{os::unix::fs::symlink, sync::mpsc, thread, time::Duration};

        let temporary = tempfile::tempdir().unwrap();
        let fifo = temporary.path().join("config-fifo");
        mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
        let path = if use_symlink {
            let link = temporary.path().join("user-dirs.dirs");
            symlink(&fifo, &link).unwrap();
            link
        } else {
            fifo.clone()
        };
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = read_bounded(&path, 64 * 1024);
            result_tx.send(result).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let immediate = result_rx.recv_timeout(Duration::from_millis(500));
        if let Ok(result) = &immediate {
            assert_eq!(result, &None);
        } else {
            // Release the original blocking File::open before failing this regression.
            // NONBLOCK on this writer keeps the test cleanup from itself waiting.
            let writer = open(
                &fifo,
                OFlags::WRONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap();
            assert_eq!(
                result_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
                None
            );
            drop(writer);
        }
        worker.join().unwrap();
        assert!(
            immediate.is_ok(),
            "read_bounded waited for a FIFO writer (symlink: {use_symlink})"
        );
    }

    #[test]
    fn bounded_config_read_rejects_fifo_without_waiting_for_writer() {
        assert_fifo_read_does_not_wait_for_a_writer(false);
    }

    #[test]
    fn bounded_config_read_rejects_symlink_to_fifo_without_waiting_for_writer() {
        assert_fifo_read_does_not_wait_for_a_writer(true);
    }

    #[test]
    fn bounded_config_read_accepts_regular_file_symlink_and_enforces_byte_limit() {
        let temporary = tempfile::tempdir().unwrap();
        let regular = temporary.path().join("regular-config");
        let link = temporary.path().join("config-link");
        fs::write(&regular, b"abc").unwrap();
        std::os::unix::fs::symlink(&regular, &link).unwrap();
        assert_eq!(read_bounded(&link, 3), Some(b"abc".to_vec()));
        assert_eq!(read_bounded(&link, 2), None);
        assert_eq!(read_bounded(temporary.path(), 3), None);
        assert_eq!(read_bounded(Path::new("/dev/null"), 3), None);
    }

    #[test]
    fn terminal_passes_command_as_data_and_preserves_native_working_directory() {
        let directory = PathBuf::from("/home/文档/‘ ’ ; $()");
        let text = "printf '%s' '👩🏽‍💻 e\u{301}'; echo done";
        let request = TerminalRequest {
            shell: ShellFlavor::Posix,
            mode: midnight_core::TerminalMode::Standard,
            command: text.into(),
            working_directory: directory.clone(),
        };
        let command = terminal_command(Path::new("/usr/bin/xterm"), &request).unwrap();
        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            &args[..3],
            &[OsStr::new("-e"), OsStr::new("/bin/sh"), OsStr::new("-c")]
        );
        assert_eq!(args[5], OsStr::new(text));
        assert!(!args[3].to_str().unwrap().contains(text));
        assert!(
            !args[3]
                .to_str()
                .unwrap()
                .contains(directory.to_str().unwrap())
        );
    }

    #[test]
    fn sudo_mode_is_explicit_and_passes_only_exact_positional_command_to_terminal() {
        let source = "printf '%s' '文档 👩🏽‍💻 e\u{301} ! $x'";
        let request = TerminalRequest {
            command: source.into(),
            working_directory: PathBuf::from("/home/文档 ; $()"),
            shell: ShellFlavor::Posix,
            mode: midnight_core::TerminalMode::Elevated,
        };
        let command = terminal_command(Path::new("/usr/bin/xterm"), &request).unwrap();
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            &args[..5],
            &[
                OsStr::new("-e"),
                OsStr::new("/usr/bin/sudo"),
                OsStr::new("--"),
                OsStr::new("/bin/sh"),
                OsStr::new("-c")
            ]
        );
        assert_eq!(args[5], OsStr::new(COMMAND_WRAPPER));
        assert_eq!(args[6], OsStr::new("midnight-terminal"));
        assert_eq!(args[7], OsStr::new(source));
        assert_eq!(args.len(), 8);
        assert_eq!(
            command.get_current_dir(),
            Some(request.working_directory.as_path())
        );
        assert!(!args.iter().any(|argument| *argument == OsStr::new("-S")));
    }

    #[test]
    fn linux_rejects_foreign_shells_without_implicit_fallback() {
        for shell in [ShellFlavor::PowerShell, ShellFlavor::Cmd] {
            let request = TerminalRequest {
                command: "echo 文档".into(),
                working_directory: PathBuf::from("/tmp"),
                shell,
                mode: midnight_core::TerminalMode::Standard,
            };
            assert_eq!(
                terminal_command(Path::new("/usr/bin/xterm"), &request)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn xdg_config_is_parsed_without_shell_expansion_or_loss_of_unicode() {
        let home = Path::new("/home/Žluťoučký");
        assert_eq!(
            user_directory("XDG_DESKTOP_DIR=\"$HOME/文档\\\" 👩🏽‍💻\"", "DESKTOP", home),
            Some(home.join("文档\" 👩🏽‍💻"))
        );
        assert_eq!(
            user_directory("XDG_DOWNLOAD_DIR=\"/srv/Downloads\"", "DOWNLOAD", home),
            Some(PathBuf::from("/srv/Downloads"))
        );
        for bad in [
            "XDG_DESKTOP_DIR=\"$(touch /tmp/pwn)\"",
            "XDG_DESKTOP_DIR=\"$OTHER/path\"",
            "XDG_DESKTOP_DIR=\"relative\"",
            "XDG_DESKTOP_DIR=\"$HOME\"",
            "XDG_DESKTOP_DIR=\"$HOME/x\"; evil",
        ] {
            assert!(user_directory(bad, "DESKTOP", home).is_none(), "{bad}");
        }
    }
}
