//! Windows-specific native identity coverage.
use crate::CapturedFile;
#[path = "tests_windows_capture.rs"]
mod capture;

pub(super) fn make_shared(path: &std::path::Path) {
    crate::trust::fixture_acl(path, "D:P(A;;FA;;;WD)").unwrap();
}

// Ioctl codes from winioctl.h; the windows-sys Ioctl feature is not otherwise needed.
const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00a4;
const FSCTL_SET_SPARSE: u32 = 0x0009_00c4;

fn control(handle: &impl std::os::windows::io::AsRawHandle, code: u32, input: &[u8]) {
    let mut returned = 0;
    // SAFETY: a live handle and an initialized input buffer of the stated length; no output.
    let status = unsafe {
        windows_sys::Win32::System::IO::DeviceIoControl(
            handle.as_raw_handle(),
            code,
            input.as_ptr().cast(),
            input.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(status, 0, "{}", std::io::Error::last_os_error());
}

/// Create a directory junction directly; spawning a shell costs about 0.7 s per link.
pub(crate) fn junction(link: &std::path::Path, target: &std::path::Path) {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
    use windows_sys::Win32::{
        Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
        System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT,
    };
    let target: Vec<u16> = target
        .canonicalize()
        .unwrap()
        .as_os_str()
        .encode_wide()
        .collect();
    // A verbatim DOS path becomes the NT substitute name; the print name is plain DOS.
    let print = target
        .strip_prefix(&[92, 92, 63, 92][..])
        .expect("junction target must be a verbatim DOS path");
    let substitute: Vec<u16> = [92, 63, 63, 92].iter().chain(print).copied().collect();
    let names = (substitute.len() + print.len() + 2) * 2;
    let mut buffer = Vec::with_capacity(16 + names);
    buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend(((8 + names) as u16).to_le_bytes());
    buffer.extend(0_u16.to_le_bytes());
    for field in [
        0,
        substitute.len() * 2,
        (substitute.len() + 1) * 2,
        print.len() * 2,
    ] {
        buffer.extend((field as u16).to_le_bytes());
    }
    for unit in substitute.iter().chain(&[0]).chain(print).chain(&[0]) {
        buffer.extend(unit.to_le_bytes());
    }
    std::fs::create_dir(link).unwrap();
    let directory = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(link)
        .unwrap();
    control(&directory, FSCTL_SET_REPARSE_POINT, &buffer);
}

/// Mark a new file sparse so a distant write does not zero-fill gigabytes on NTFS.
pub(crate) fn sparse(file: &std::fs::File) {
    control(file, FSCTL_SET_SPARSE, &[]);
}

#[test]
fn replacement_with_same_size_and_timestamp_is_still_a_different_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("original");
    std::fs::write(&path, b"original").unwrap();
    let capture = CapturedFile::capture(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::rename(&path, root.path().join("renamed")).unwrap();
    std::fs::write(&path, b"original").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert!(capture.open().is_err());
}
