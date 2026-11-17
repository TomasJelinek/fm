//! Native test fixtures without spawning a shell: directory junctions and
//! non-surrogate (filter-style) reparse points, set with `FSCTL_SET_REPARSE_POINT`.
//! Standalone (std and windows-sys only) so integration tests can include it by path.
#![allow(dead_code, reason = "each test binary uses a subset of the fixtures")]
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA,
    },
    System::{
        IO::DeviceIoControl,
        Ioctl::{FSCTL_DELETE_REPARSE_POINT, FSCTL_SET_REPARSE_POINT},
    },
};

/// `IO_REPARSE_TAG_MOUNT_POINT`: a junction, which is a name surrogate.
pub const MOUNT_POINT: u32 = 0xA000_0003;
/// A third-party tag without the Microsoft or name-surrogate bits. Like a Cloud Files
/// placeholder, the directory variant sets the "may have children" bit (0x1000_0000).
pub const FILTER_FILE: u32 = 0x0000_4D45;
pub const FILTER_DIRECTORY: u32 = 0x1000_4D45;
const FIXTURE_GUID: [u8; 16] = *b"midnight-fixture";

fn open_for_reparse(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn control(file: &File, code: u32, input: &[u8]) -> io::Result<()> {
    let mut returned = 0u32;
    // SAFETY: a live handle and an input buffer that outlives this synchronous request;
    // no output buffer is requested.
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            input.as_ptr().cast(),
            input.len() as u32,
            ptr::null_mut(),
            0,
            &mut returned,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Turns the existing empty directory `link` into a junction to the absolute `target`.
pub fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    // NT substitute names are not normalized: use backslashes and no verbatim prefix.
    let target: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .map(|unit| if unit == 47 { 92 } else { unit })
        .collect();
    let plain = target
        .strip_prefix(&[92, 92, 63, 92][..])
        .unwrap_or(&target)
        .to_vec();
    let mut substitute: Vec<u16> = r"\??\".encode_utf16().collect();
    substitute.extend(&plain);
    let substitute_bytes = substitute.len() * 2;
    let print_bytes = plain.len() * 2;
    // PathBuffer holds both names, each followed by a terminating NUL.
    let names = substitute_bytes + 2 + print_bytes + 2;
    let mut buffer = Vec::with_capacity(16 + names);
    buffer.extend(MOUNT_POINT.to_le_bytes());
    buffer.extend(
        u16::try_from(8 + names)
            .map_err(|_| invalid())?
            .to_le_bytes(),
    );
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend((substitute_bytes as u16).to_le_bytes());
    buffer.extend(((substitute_bytes + 2) as u16).to_le_bytes());
    buffer.extend((print_bytes as u16).to_le_bytes());
    for unit in substitute.iter().chain(&[0]).chain(&plain).chain(&[0]) {
        buffer.extend(unit.to_le_bytes());
    }
    control(&open_for_reparse(link)?, FSCTL_SET_REPARSE_POINT, &buffer)
}

fn guid_buffer(tag: u32, data: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(24 + data.len());
    buffer.extend(tag.to_le_bytes());
    buffer.extend((data.len() as u16).to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    buffer.extend(FIXTURE_GUID);
    buffer.extend(data);
    buffer
}

/// Adds a third-party non-surrogate reparse point to an existing file or directory.
/// No filter owns the tag: no-follow opens work, filter-mediated opens fail.
pub fn set_filter_tag(path: &Path, tag: u32) -> io::Result<()> {
    control(
        &open_for_reparse(path)?,
        FSCTL_SET_REPARSE_POINT,
        &guid_buffer(tag, b"midnight"),
    )
}

/// Removes the fixture reparse point again, leaving an ordinary entry.
pub fn clear_filter_tag(path: &Path, tag: u32) -> io::Result<()> {
    control(
        &open_for_reparse(path)?,
        FSCTL_DELETE_REPARSE_POINT,
        &guid_buffer(tag, &[]),
    )
}

/// The repository's `_build/tmp`, spelled without `.`/`..` steps or forward slashes
/// (native mutation validation and junction substitute names reject them).
pub fn build_tmp_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = manifest
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives in <repository>/crates/<name>");
    let root = repository.join("_build").join("tmp");
    std::fs::create_dir_all(&root).expect("create _build/tmp");
    root
}

/// A fresh directory below the repository's `_build/tmp`, removed on drop.
pub fn build_temp(prefix: &str) -> tempfile::TempDir {
    let root = build_tmp_root();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .expect("create fixture directory")
}

/// A Unicode name that also contains an unpaired surrogate.
pub fn unusual_name(stem: &str) -> OsString {
    let mut units: Vec<u16> = stem.encode_utf16().collect();
    units.extend("-文档-".encode_utf16());
    units.push(0xd800);
    OsString::from_wide(&units)
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "junction target is too long")
}
