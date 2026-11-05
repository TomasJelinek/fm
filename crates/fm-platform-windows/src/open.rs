//! Explicit desktop activation; called only by the backend's bounded native workers.

use crate::WindowsFileSystem;
use fm_core::{FsError, OpenEntryResult, OpenService};
use std::{
    fs::{self, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, GetFileType,
    },
    System::Com::COINIT_APARTMENTTHREADED,
    UI::{
        Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

impl OpenService for WindowsFileSystem {
    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        open_with(path, launch_association)
    }
}

struct ShellOpen {
    file: Vec<u16>,
    directory: Vec<u16>,
}

impl ShellOpen {
    fn new(path: &Path) -> io::Result<Self> {
        validate_path(path)?;
        let file = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let directory = path
            .parent()
            .unwrap_or(path)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        Ok(Self { file, directory })
    }

    fn info(&self) -> SHELLEXECUTEINFOW {
        SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
            lpVerb: windows_sys::w!("open"),
            lpFile: self.file.as_ptr(),
            lpParameters: ptr::null(),
            lpDirectory: self.directory.as_ptr(),
            nShow: SW_SHOWNORMAL,
            ..Default::default()
        }
    }
}

fn open_with(
    path: &Path,
    launch: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<OpenEntryResult, FsError> {
    let operation = || -> io::Result<OpenEntryResult> {
        validate_path(path)?;
        // Explicit activation follows links; listing and selection never call this service.
        let metadata = fs::metadata(path)?;
        if metadata.is_dir() {
            return Ok(OpenEntryResult::Directory);
        }
        if !metadata.is_file() {
            return Err(unsupported_target());
        }
        // Zero access queries identity/type without reading file contents or requesting execution.
        // Broad sharing permits normal concurrently open documents and avoids locking user data.
        let file = OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(path)?;
        // SAFETY: File owns a live handle; GetFileType only inspects its native type.
        if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
            return Err(unsupported_target());
        }
        let current = file.metadata()?;
        if current.is_dir() {
            return Ok(OpenEntryResult::Directory);
        }
        if !current.is_file() {
            return Err(unsupported_target());
        }
        launch(path)?;
        Ok(OpenEntryResult::File { process: None })
    };
    operation().map_err(|error| FsError::from_io("open file", path, error))
}

fn validate_path(path: &Path) -> io::Result<()> {
    let prefix = match path.components().next() {
        Some(Component::Prefix(prefix)) => prefix.kind(),
        _ => return Err(invalid_path()),
    };
    if !path.is_absolute()
        || !matches!(
            prefix,
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..)
        )
        || path.as_os_str().encode_wide().any(|unit| unit == 0)
    {
        return Err(invalid_path());
    }
    Ok(())
}

fn invalid_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "activation requires an absolute disk/UNC path without NUL or a device namespace",
    )
}

fn unsupported_target() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "only regular disk files and directories can be activated",
    )
}

fn launch_association(path: &Path) -> io::Result<()> {
    let request = ShellOpen::new(path)?;
    // Shell extensions can require STA. NOASYNC keeps their initialization within this scope;
    // no UI message loop or long-lived COM thread is required from the backend caller.
    let _com = crate::desktop::ComGuard::initialize(COINIT_APARTMENTTHREADED as u32)
        .ok_or_else(|| io::Error::other("cannot initialize desktop association COM apartment"))?;
    let mut info = request.info();
    // SAFETY: all strings are terminated, owned by request/static storage and remain live through
    // this synchronous call; all optional pointers are null. No shell parameters are supplied.
    let succeeded = unsafe { ShellExecuteExW(&mut info) } != 0;
    let error = (!succeeded).then(io::Error::last_os_error);
    if !info.hProcess.is_null() {
        // SAFETY: any returned hProcess is an owned process handle. No process wait is requested.
        // NOCLOSEPROCESS is intentionally absent, so this is normally null.
        unsafe { CloseHandle(info.hProcess) };
    }
    if let Some(error) = error {
        return Err(if error.raw_os_error() == Some(0) {
            io::Error::other("the system could not open this file association")
        } else {
            error
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsString,
        fs,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::PathBuf,
    };
    use windows_sys::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC};

    #[test]
    fn association_fields_keep_unicode_metacharacters_and_parameters_separate() {
        let path = Path::new("C:\\文档\\Žluťoučký & % ! ' (🦀).txt");
        let request = ShellOpen::new(path).unwrap();
        let expected: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_eq!(request.file, expected);
        let info = request.info();
        assert_eq!(info.lpFile, request.file.as_ptr());
        assert_eq!(info.lpDirectory, request.directory.as_ptr());
        assert!(info.lpParameters.is_null());
        assert_eq!(info.fMask, SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC);
        assert!(info.hProcess.is_null());
    }

    #[test]
    fn dynamic_native_path_preserves_long_paths_and_unpaired_utf16() {
        let long = PathBuf::from(format!("C:\\{}\\data.txt", "文档\\".repeat(300)));
        assert!(long.as_os_str().encode_wide().count() > 260);
        let request = ShellOpen::new(&long).unwrap();
        assert_eq!(
            request.file.len(),
            long.as_os_str().encode_wide().count() + 1
        );
        let mut units: Vec<u16> = "C:\\data\\".encode_utf16().collect();
        units.extend([0xd800, b'.' as u16, b't' as u16]);
        let raw = PathBuf::from(OsString::from_wide(&units));
        assert_eq!(
            ShellOpen::new(&raw).unwrap().file,
            [units, vec![0]].concat()
        );
        for accepted in [
            r"\\?\C:\data\file.txt",
            r"\\server\share\file.txt",
            r"\\?\UNC\server\share\file.txt",
        ] {
            assert!(ShellOpen::new(Path::new(accepted)).is_ok());
        }
    }

    #[test]
    fn rejects_relative_nul_and_device_namespaces_before_launch() {
        for input in [
            "relative.txt",
            "C:relative.txt",
            "\\rooted.txt",
            "C:\\bad\0.txt",
            r"\\.\pipe\name",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\file",
        ] {
            assert!(
                ShellOpen::new(Path::new(input)).is_err(),
                "accepted {input:?}"
            );
            assert!(open_with(Path::new(input), |_| panic!("must not launch")).is_err());
        }
    }

    #[test]
    fn actual_directory_is_internal_and_regular_file_launches_once_with_exact_path() {
        let temp = tempfile::tempdir().unwrap();
        assert!(matches!(
            open_with(temp.path(), |_| panic!("directory must stay internal")),
            Ok(OpenEntryResult::Directory)
        ));
        let file = temp.path().join("Žluťoučký & % ! ' 🦀.txt");
        fs::write(&file, b"not executed").unwrap();
        let mut calls = 0;
        let result = open_with(&file, |argument| {
            calls += 1;
            assert_eq!(argument, file);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(matches!(result, OpenEntryResult::File { process: None }));
        assert!(open_with(&temp.path().join("missing"), |_| panic!("must not launch")).is_err());
    }

    #[test]
    fn launch_failures_keep_native_path_and_context() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("unknown-extension");
        fs::write(&file, b"data").unwrap();
        let result = open_with(&file, |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "association denied",
            ))
        });
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert_eq!(error.path, file);
        assert!(error.to_string().contains("association denied"));
    }

    #[test]
    fn actual_unicode_file_beyond_max_path_is_classified_without_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let component = "dlouhá_文档_🦀_".repeat(5);
        let mut parent = temp.path().to_owned();
        // The repository-local test directory can be shorter than a user's TEMP.
        // Build a genuinely long native path independently of either location.
        while parent.as_os_str().encode_wide().count() <= 260 {
            parent.push(&component);
        }
        fs::create_dir_all(&parent).unwrap();
        let file = parent.join("document & %.txt");
        fs::write(&file, b"data").unwrap();
        assert!(file.as_os_str().encode_wide().count() > 260);
        assert!(matches!(
            open_with(&file, |path| {
                assert_eq!(path.as_os_str(), file.as_os_str());
                Ok(())
            }),
            Ok(OpenEntryResult::File { .. })
        ));
    }

    #[test]
    fn explicit_directory_junction_navigates_internally() {
        use std::{os::windows::process::CommandExt, process::Command};
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target_文档");
        fs::create_dir(&target).unwrap();
        let link = temp.path().join("link_🦀");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("fm-platform/tests/create-junction.ps1");
        let output = Command::new(crate::desktop::powershell_path().unwrap())
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-File",
            ])
            .arg(fixture)
            .arg("-JunctionPath")
            .arg(&link)
            .arg("-TargetPath")
            .arg(&target)
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(matches!(
            open_with(&link, |_| panic!("directory must stay internal")),
            Ok(OpenEntryResult::Directory)
        ));
    }

    #[test]
    fn dos_device_alias_cannot_launch_an_association() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            open_with(&temp.path().join("NUL"), |_| panic!(
                "must not launch a device"
            ))
            .is_err()
        );
    }
}
