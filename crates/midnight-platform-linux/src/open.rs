//! Explicit desktop activation; called only by the backend's bounded native workers.

use crate::LinuxFileSystem;
use midnight_core::{FsError, OpenEntryResult, OpenService, SystemLocation, TerminalProcess};
use std::{
    fs,
    io::{self, Read},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    },
    path::Path,
    process::{Command, Stdio},
};

#[cfg(test)]
#[path = "executable_open_tests.rs"]
mod executable_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileActivation {
    Association,
    Program,
    Terminal,
}

fn file_activation(path: &Path, metadata: &fs::Metadata) -> io::Result<FileActivation> {
    use rustix::fs::{Mode, OFlags, open};

    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "only regular files can be activated",
        ));
    }
    if metadata.mode() & 0o111 == 0 || has_extension(path, b"desktop") {
        return Ok(FileActivation::Association);
    }
    // Explicit activation follows links. NONBLOCK prevents a replacement FIFO from
    // waiting for a writer; the opened descriptor must match the known regular file
    // before any bytes are read. Only the four-byte executable header is inspected.
    let file = match open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(descriptor) => fs::File::from(descriptor),
        Err(cause) => {
            let cause = io::Error::from(cause);
            if cause.kind() == io::ErrorKind::PermissionDenied {
                // Execute-only native programs need not grant permission to inspect
                // their contents. The kernel remains responsible for execution access.
                return Ok(FileActivation::Program);
            }
            return Err(cause);
        }
    };
    let check = || -> io::Result<()> {
        let current = file.metadata()?;
        if !same_activation_target(metadata, &current) {
            return Err(io::Error::other(
                "activation target changed during executable inspection",
            ));
        }
        Ok(())
    };
    check()?;
    let mut header = Vec::with_capacity(4);
    (&file).take(4).read_to_end(&mut header)?;
    check()?;
    if header == b"\x7fELF" {
        Ok(FileActivation::Program)
    } else if header.starts_with(b"#!") || has_extension(path, b"sh") {
        Ok(FileActivation::Terminal)
    } else {
        // Some mounts mark every document executable. Execute bits alone must not
        // turn arbitrary text, desktop launchers or other documents into shell input.
        Ok(FileActivation::Association)
    }
}

fn has_extension(path: &Path, extension: &[u8]) -> bool {
    path.extension()
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(extension))
}

fn same_activation_target(expected: &fs::Metadata, current: &fs::Metadata) -> bool {
    current.is_file()
        && expected.dev() == current.dev()
        && expected.ino() == current.ino()
        && expected.len() == current.len()
        && expected.mode() == current.mode()
        && expected.mtime() == current.mtime()
        && expected.mtime_nsec() == current.mtime_nsec()
        && expected.ctime() == current.ctime()
        && expected.ctime_nsec() == current.ctime_nsec()
}

fn program_command(path: &Path) -> io::Result<Command> {
    validate_path(path)?;
    let mut command = Command::new(path);
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

impl OpenService for LinuxFileSystem {
    fn open_system_location(
        &self,
        location: SystemLocation,
    ) -> Result<Option<Box<dyn TerminalProcess>>, FsError> {
        open_system_with(location, DesktopOpener::find_system(), |mut command| {
            command
                .spawn()
                .map(|child| Box::new(child) as Box<dyn TerminalProcess>)
        })
    }

    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        open_with(path, |path, metadata| {
            let mut command = match file_activation(path, metadata)? {
                FileActivation::Association => DesktopOpener::find()?.command(path)?,
                FileActivation::Program => program_command(path)?,
                FileActivation::Terminal => crate::desktop::executable_terminal(path)?,
            };
            let child = command.spawn()?;
            Ok(Some(Box::new(child)))
        })
    }
}

#[derive(Clone, Copy)]
enum DesktopOpener {
    Xdg,
    Gio,
}

impl DesktopOpener {
    fn executable(self) -> &'static Path {
        Path::new(match self {
            Self::Xdg => "/usr/bin/xdg-open",
            Self::Gio => "/usr/bin/gio",
        })
    }

    fn find() -> io::Result<Self> {
        for candidate in [Self::Xdg, Self::Gio] {
            if let Ok(metadata) = fs::metadata(candidate.executable())
                && metadata.is_file()
                && metadata.permissions().mode() & 0o111 != 0
            {
                return Ok(candidate);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no system desktop opener is installed (/usr/bin/xdg-open or /usr/bin/gio)",
        ))
    }

    fn find_system() -> io::Result<Self> {
        Self::find_system_with(|path| {
            fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
    }

    fn find_system_with(mut available: impl FnMut(&Path) -> bool) -> io::Result<Self> {
        // Prefer GIO's direct URI handling; xdg-open is a fallback only when GIO is absent.
        for candidate in [Self::Gio, Self::Xdg] {
            if available(candidate.executable()) {
                return Ok(candidate);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no system Trash opener is installed (/usr/bin/gio or /usr/bin/xdg-open)",
        ))
    }

    fn system_command(self, location: SystemLocation) -> Command {
        let uri = match location {
            SystemLocation::RecycleBin => "trash:///",
        };
        let mut command = Command::new(self.executable());
        if matches!(self, Self::Gio) {
            command.args(["open", "--"]);
        }
        // Fixed typed target only: no panel CWD, caller URI, shell source or Trash filesystem path.
        command
            .arg(uri)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn command(self, path: &Path) -> io::Result<Command> {
        validate_path(path)?;
        let mut command = Command::new(self.executable());
        if matches!(self, Self::Gio) {
            command.args(["open", "--"]);
        }
        // An absolute native path is a literal file argument, never an option or URI scheme.
        // No shell source, Unicode conversion or filename-derived command is constructed.
        command.arg(path);
        if let Some(parent) = path.parent() {
            command.current_dir(parent);
        }
        // Desktop helpers must not consume our input or expose file contents in Midnight Explorer logs.
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Ok(command)
    }
}

/// Called by desktop-context discovery on a backend worker; never launches a helper.
pub(crate) fn supports_system_location(location: SystemLocation) -> bool {
    match location {
        SystemLocation::RecycleBin => DesktopOpener::find_system().is_ok(),
    }
}

fn open_system_with(
    location: SystemLocation,
    opener: io::Result<DesktopOpener>,
    launch: impl FnOnce(Command) -> io::Result<Box<dyn TerminalProcess>>,
) -> Result<Option<Box<dyn TerminalProcess>>, FsError> {
    // The Engine reserves its existing opener slot before this call and owns the returned child.
    // Successful spawn is not a successful desktop launch: the tracker observes the helper exit.
    opener
        .and_then(|opener| launch(opener.system_command(location)))
        .map(Some)
        .map_err(|error| FsError::from_io("open system location", Path::new(""), error))
}

fn open_with(
    path: &Path,
    launch: impl FnOnce(&Path, &fs::Metadata) -> io::Result<Option<Box<dyn TerminalProcess>>>,
) -> Result<OpenEntryResult, FsError> {
    let operation = || -> io::Result<OpenEntryResult> {
        validate_path(path)?;
        // Only explicit activation follows symlinks. Reject special targets before
        // executable classification or desktop association can inspect any contents.
        let metadata = fs::metadata(path)?;
        if metadata.is_dir() {
            return Ok(OpenEntryResult::Directory);
        }
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "only regular files and directories can be activated",
            ));
        }
        Ok(OpenEntryResult::File {
            process: launch(path, &metadata)?,
        })
    };
    operation().map_err(|error| FsError::from_io("open file", path, error))
}

fn validate_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "activation requires an absolute native path without NUL",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::{OsStr, OsString},
        fs,
        os::unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::symlink,
        },
        path::PathBuf,
    };

    #[test]
    fn native_association_uses_trusted_executable_and_literal_absolute_argument() {
        let path = Path::new("/tmp/文档/Žluťoučký ' & ; $(touch nope) % 🦀.txt");
        let command = DesktopOpener::Xdg.command(path).unwrap();
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/xdg-open"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![path.as_os_str()]
        );
        let command = DesktopOpener::Gio.command(path).unwrap();
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/gio"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("open"), OsStr::new("--"), path.as_os_str()]
        );
    }

    #[test]
    fn literal_argument_preserves_non_utf8_and_long_native_path() {
        let path = PathBuf::from(OsString::from_vec(b"/tmp/raw_\xff\xfe.txt".to_vec()));
        let command = DesktopOpener::Xdg.command(&path).unwrap();
        assert_eq!(
            command.get_args().next().unwrap().as_bytes(),
            path.as_os_str().as_bytes()
        );
        let path = PathBuf::from(format!("/tmp/{}file.txt", "文档/".repeat(200)));
        assert_eq!(
            DesktopOpener::Xdg.command(&path).unwrap().get_args().next(),
            Some(path.as_os_str())
        );
    }

    #[test]
    fn relative_uri_and_nul_are_rejected_without_launch() {
        for input in [
            "relative",
            "https://example.com",
            "file:///tmp/file",
            "/tmp/bad\0file",
        ] {
            assert!(DesktopOpener::Xdg.command(Path::new(input)).is_err());
            assert!(open_with(Path::new(input), |_, _| panic!("must not launch")).is_err());
        }
    }

    #[test]
    fn actual_directory_stays_internal_and_regular_file_is_launched_once() {
        let temp = tempfile::tempdir().unwrap();
        assert!(matches!(
            open_with(temp.path(), |_, _| panic!("directory must stay internal")),
            Ok(OpenEntryResult::Directory)
        ));
        let file = temp.path().join("文档 ' & ; $(touch nope) 🦀.txt");
        fs::write(&file, b"not executed").unwrap();
        let mut calls = 0;
        let result = open_with(&file, |argument, _| {
            calls += 1;
            assert_eq!(argument, file);
            Ok(None)
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(matches!(result, OpenEntryResult::File { process: None }));
        assert!(
            open_with(&temp.path().join("missing"), |_, _| panic!(
                "must not launch"
            ))
            .is_err()
        );
    }

    #[test]
    fn explicit_links_follow_targets_but_missing_links_and_special_files_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("target");
        fs::write(&file, b"data").unwrap();
        let file_link = temp.path().join("file-link");
        symlink(&file, &file_link).unwrap();
        assert!(matches!(
            open_with(&file_link, |argument, _| {
                assert_eq!(argument, file_link);
                Ok(None)
            }),
            Ok(OpenEntryResult::File { .. })
        ));
        let dir_link = temp.path().join("dir-link");
        symlink(temp.path(), &dir_link).unwrap();
        assert!(matches!(
            open_with(&dir_link, |_, _| panic!(
                "directory link must stay internal"
            )),
            Ok(OpenEntryResult::Directory)
        ));
        let missing_link = temp.path().join("dangling");
        symlink(temp.path().join("missing"), &missing_link).unwrap();
        assert!(open_with(&missing_link, |_, _| panic!("must not launch")).is_err());
        let fifo = temp.path().join("fifo");
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        for special in [&fifo, Path::new("/dev/null")] {
            let error = open_with(special, |_, _| panic!("must not launch"))
                .err()
                .unwrap();
            assert!(error.to_string().contains("regular"));
            assert_eq!(error.kind, io::ErrorKind::Unsupported);
        }
        // The per-test TMPDIR may exceed Linux's sockaddr_un path budget.
        let socket_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
        fs::create_dir_all(&socket_root).unwrap();
        let socket_directory = tempfile::tempdir_in(socket_root.canonicalize().unwrap()).unwrap();
        let socket = socket_directory.path().join("socket");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert_eq!(
            open_with(&socket, |_, _| panic!("must not launch a socket"))
                .err()
                .unwrap()
                .kind,
            io::ErrorKind::Unsupported
        );
        let special_link = temp.path().join("fifo-link");
        symlink(&fifo, &special_link).unwrap();
        assert!(open_with(&special_link, |_, _| panic!("must not launch a FIFO link")).is_err());
    }

    #[test]
    fn missing_and_launch_errors_keep_the_original_native_path() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing_文档");
        let error = open_with(&missing, |_, _| panic!("must not launch"))
            .err()
            .unwrap();
        assert_eq!(error.path, missing);
        assert_eq!(error.kind, io::ErrorKind::NotFound);
        let file = temp
            .path()
            .join(OsString::from_vec(b"raw_\xff.txt".to_vec()));
        fs::write(&file, b"data").unwrap();
        let error = open_with(&file, |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "association denied",
            ))
        })
        .err()
        .unwrap();
        assert_eq!(error.path, file);
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("association denied"));
    }

    #[test]
    fn trash_commands_use_fixed_trusted_programs_and_literal_uri_only() {
        for (opener, expected_program, expected_arguments) in [
            (
                DesktopOpener::Gio,
                "/usr/bin/gio",
                vec!["open", "--", "trash:///"],
            ),
            (DesktopOpener::Xdg, "/usr/bin/xdg-open", vec!["trash:///"]),
        ] {
            let command = opener.system_command(SystemLocation::RecycleBin);
            assert_eq!(command.get_program(), OsStr::new(expected_program));
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                expected_arguments
                    .iter()
                    .map(OsStr::new)
                    .collect::<Vec<_>>()
            );
            assert!(command.get_current_dir().is_none());
        }
    }

    #[test]
    fn trash_prefers_gio_and_only_probes_xdg_when_gio_is_unavailable() {
        let mut probes = Vec::new();
        let chosen = DesktopOpener::find_system_with(|path| {
            probes.push(path.to_owned());
            true
        })
        .unwrap();
        assert!(matches!(chosen, DesktopOpener::Gio));
        assert_eq!(probes, [PathBuf::from("/usr/bin/gio")]);
        let mut probes = Vec::new();
        let chosen = DesktopOpener::find_system_with(|path| {
            probes.push(path.to_owned());
            path == Path::new("/usr/bin/xdg-open")
        })
        .unwrap();
        assert!(matches!(chosen, DesktopOpener::Xdg));
        assert_eq!(
            probes,
            [
                PathBuf::from("/usr/bin/gio"),
                PathBuf::from("/usr/bin/xdg-open")
            ]
        );
        assert!(DesktopOpener::find_system_with(|_| false).is_err());
    }

    struct PendingSystemOpener(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl TerminalProcess for PendingSystemOpener {
        fn id(&self) -> u32 {
            77
        }
        fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
            Ok(None)
        }
    }
    impl Drop for PendingSystemOpener {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn trash_launch_transfers_the_live_child_to_the_engine_owner() {
        let drops = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut child = open_system_with(
            SystemLocation::RecycleBin,
            Ok(DesktopOpener::Gio),
            |command| {
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    [
                        OsStr::new("open"),
                        OsStr::new("--"),
                        OsStr::new("trash:///")
                    ]
                );
                Ok(Box::new(PendingSystemOpener(drops.clone())))
            },
        )
        .unwrap()
        .expect("the process owner must be returned");
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(child.id(), 77);
        assert!(child.try_wait().unwrap().is_none());
        drop(child);
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn missing_trash_opener_and_launch_errors_are_reported_without_native_paths() {
        let missing = DesktopOpener::find_system_with(|_| false);
        let error = open_system_with(SystemLocation::RecycleBin, missing, |_| {
            panic!("must not launch")
        })
        .err()
        .expect("missing capability must fail");
        assert_eq!(error.kind, io::ErrorKind::NotFound);
        assert!(error.path.as_os_str().is_empty());
        let error = open_system_with(SystemLocation::RecycleBin, Ok(DesktopOpener::Gio), |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "launch denied",
            ))
        })
        .err()
        .expect("failed launch must fail");
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
        assert!(error.message.contains("launch denied"));
        assert!(error.path.as_os_str().is_empty());
    }
}
