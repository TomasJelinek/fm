//! Explicit desktop activation; called only by the backend's bounded native workers.

use crate::WindowsFileSystem;
use midnight_core::{FsError, OpenEntryResult, OpenService, SystemLocation};
use std::{
    fs::{self, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{FileTypeExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, Prefix},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ACCESS_DENIED},
    Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, GetFileType,
    },
    System::Com::COINIT_APARTMENTTHREADED,
    UI::{
        Shell::{
            Common::ITEMIDLIST, FOLDERID_RecycleBinFolder, ILFree, KF_FLAG_DONT_VERIFY,
            SEE_MASK_FLAG_NO_UI, SEE_MASK_IDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
            SHGetKnownFolderIDList, ShellExecuteExW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

impl OpenService for WindowsFileSystem {
    fn open_system_location(
        &self,
        location: SystemLocation,
    ) -> Result<Option<Box<dyn midnight_core::TerminalProcess>>, FsError> {
        launch_system_location(location)
            .map(|()| None)
            .map_err(|error| FsError::from_io("open system location", Path::new(""), error))
    }

    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        open_with(path, launch_association)
    }

    fn directory_destination(&self, path: &Path) -> Result<std::path::PathBuf, FsError> {
        let operation = || -> io::Result<std::path::PathBuf> {
            validate_path(path)?;
            if !fs::metadata(path)?.is_dir() {
                return Err(unsupported_target());
            }
            match directory_result(path)? {
                OpenEntryResult::Directory => Ok(path.to_owned()),
                OpenEntryResult::DirectoryTarget { path } => Ok(path),
                OpenEntryResult::File { .. } => Err(unsupported_target()),
            }
        };
        operation().map_err(|error| FsError::from_io("open directory", path, error))
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

fn system_folder_id(location: SystemLocation) -> windows_sys::core::GUID {
    match location {
        SystemLocation::RecycleBin => FOLDERID_RecycleBinFolder,
    }
}

struct SystemPidl(*mut ITEMIDLIST);
impl Drop for SystemPidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: only SHGetKnownFolderIDList output enters this owner; ILFree is its pair.
            unsafe { ILFree(self.0) };
        }
    }
}

fn known_folder_status(result: i32, pidl: *mut ITEMIDLIST) -> io::Result<()> {
    if result < 0 {
        return Err(io::Error::other(format!(
            "Cannot resolve system Shell location (HRESULT 0x{:08X}).",
            result as u32
        )));
    }
    if pidl.is_null() {
        return Err(io::Error::other(
            "System Shell location returned no item identifier.",
        ));
    }
    Ok(())
}

fn system_open_info(pidl: *mut ITEMIDLIST) -> SHELLEXECUTEINFOW {
    SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: windows_sys::w!("open"),
        lpIDList: pidl.cast(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    }
}

fn system_launch_result(error: Option<io::Error>) -> io::Result<()> {
    match error {
        None => Ok(()),
        Some(error) if error.raw_os_error() == Some(0) => Err(io::Error::other(
            "The system Shell did not accept the location launch request.",
        )),
        Some(error) => Err(error),
    }
}

fn launch_system_location(location: SystemLocation) -> io::Result<()> {
    let _com = crate::desktop::ComGuard::initialize(COINIT_APARTMENTTHREADED as u32)
        .ok_or_else(|| io::Error::other("cannot initialize system location COM apartment"))?;
    let folder = system_folder_id(location);
    let mut raw = ptr::null_mut();
    // SAFETY: folder is a fixed host-known GUID, output storage is valid, and no token is supplied.
    let status = unsafe {
        SHGetKnownFolderIDList(
            &folder,
            KF_FLAG_DONT_VERIFY as u32,
            ptr::null_mut(),
            &mut raw,
        )
    };
    let pidl = SystemPidl(raw);
    known_folder_status(status, pidl.0)?;
    let mut info = system_open_info(pidl.0);
    // SAFETY: the absolute PIDL and static verb outlive this call. File, parameters and directory
    // are null; SEE_MASK_IDLIST selects the PIDL. No caller text or filesystem path is interpreted.
    let succeeded = unsafe { ShellExecuteExW(&mut info) } != 0;
    // Capture thread-local last error before handle/PIDL/COM cleanup can overwrite it.
    let error = (!succeeded).then(io::Error::last_os_error);
    if !info.hProcess.is_null() {
        // SAFETY: any returned process handle is owned. NOCLOSEPROCESS is absent, as for file opens.
        unsafe { CloseHandle(info.hProcess) };
    }
    // NOASYNC does not establish namespace/UI completion; success means accepted launch only.
    system_launch_result(error)
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
            return directory_result(path);
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

fn directory_result(path: &Path) -> io::Result<OpenEntryResult> {
    if !fs::symlink_metadata(path)?.file_type().is_symlink_dir() {
        return Ok(OpenEntryResult::Directory);
    }
    match fs::read_dir(path) {
        Ok(_) => Ok(OpenEntryResult::Directory),
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
            // Compatibility junctions can deny listing the alias itself. Reading
            // their OS reparse destination requests no content, privilege or ACL
            // change. Only this explicit activation may choose another location;
            // scanners and destructive operations never call this service.
            let target = fs::read_link(path)?;
            let target = if target.is_absolute() {
                target
            } else {
                let parent = path.parent().ok_or_else(invalid_path)?;
                fs::canonicalize(parent.join(target))?
            };
            validate_path(&target)?;
            if !fs::metadata(&target)?.is_dir() {
                return Err(unsupported_target());
            }
            // The target's own ACL remains authoritative. Opening its directory
            // must succeed with ordinary caller rights before returning it.
            drop(fs::read_dir(&target)?);
            Ok(OpenEntryResult::DirectoryTarget { path: target })
        }
        Err(error) => Err(error),
    }
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
            .join("midnight-platform/tests/create-junction.ps1");
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

    #[test]
    fn recycle_bin_request_uses_only_fixed_pidl_open_fields() {
        let mut marker = 0_u8;
        let pidl = std::ptr::from_mut(&mut marker).cast::<ITEMIDLIST>();
        let info = system_open_info(pidl);
        assert_eq!(info.cbSize as usize, size_of::<SHELLEXECUTEINFOW>());
        assert_eq!(
            info.fMask,
            SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC
        );
        assert_eq!(info.lpIDList, pidl.cast());
        // SAFETY: request construction supplies the fixed five-unit, terminated static open verb.
        let verb = unsafe { std::slice::from_raw_parts(info.lpVerb, 5) };
        assert_eq!(
            verb,
            &[b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0]
        );
        assert!(info.lpFile.is_null());
        assert!(info.lpParameters.is_null());
        assert!(info.lpDirectory.is_null());
        assert!(info.hwnd.is_null());
        assert!(info.hProcess.is_null());
        assert_eq!(info.nShow, SW_SHOWNORMAL);
    }

    #[test]
    fn recycle_bin_known_folder_is_fixed_and_failure_or_null_is_rejected() {
        let id = system_folder_id(SystemLocation::RecycleBin);
        let expected = FOLDERID_RecycleBinFolder;
        assert_eq!(
            (id.data1, id.data2, id.data3, id.data4),
            (
                expected.data1,
                expected.data2,
                expected.data3,
                expected.data4
            )
        );
        let mut marker = 0_u8;
        let pidl = std::ptr::from_mut(&mut marker).cast::<ITEMIDLIST>();
        assert!(known_folder_status(0, pidl).is_ok());
        assert!(known_folder_status(0, ptr::null_mut()).is_err());
        let error = known_folder_status(0x80070005_u32 as i32, pidl).unwrap_err();
        assert!(error.to_string().contains("80070005"));
    }

    #[test]
    fn recycle_bin_launch_result_preserves_captured_error_and_rejects_zero_error() {
        assert!(system_launch_result(None).is_ok());
        let denied = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32);
        let error = system_launch_result(Some(denied)).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
        let error = system_launch_result(Some(io::Error::from_raw_os_error(0))).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }
}

#[cfg(test)]
#[path = "open_junction_tests.rs"]
mod junction_tests;
