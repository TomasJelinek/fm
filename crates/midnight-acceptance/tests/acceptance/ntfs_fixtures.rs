//! Create NTFS junctions and mark sparse fixture files using their open handles.
#![allow(
    unsafe_code,
    reason = "audited synchronous DeviceIoControl calls on fixture handles"
)]
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};

/// Open the directory itself (`FILE_FLAG_BACKUP_SEMANTICS`), not what a reparse point
/// there names (`FILE_FLAG_OPEN_REPARSE_POINT`).
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00a4;
const FSCTL_SET_SPARSE: u32 = 0x0009_00c4;
/// A junction: a mount-point reparse point.
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xa000_0003;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        device: *mut c_void,
        code: u32,
        input: *const c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

/// Create `link` as a junction to `target`, which need not exist; a relative target is
/// made absolute against the working folder, as `mklink /J` does.
pub fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    let target = std::path::absolute(target)?;
    fs::create_dir(link)?;
    let result = set_mount_point(link, &target);
    if result.is_err() {
        let _ = fs::remove_dir(link);
    }
    result
}

/// Mark the newly created file sparse before extending it to its logical size.
pub fn mark_sparse(file: &File) -> io::Result<()> {
    let mut returned = 0;
    // SAFETY: the caller owns a live, writable synchronous handle throughout
    // this call. FSCTL_SET_SPARSE sets the flag with no input or output buffer.
    // `returned` is writable until the call finishes; OVERLAPPED is null.
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            ptr::null(),
            0,
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

fn set_mount_point(link: &Path, target: &Path) -> io::Result<()> {
    let directory = OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(link)?;
    // The print name is the Win32 path; the substitute name is its NT form `\??\C:\...`.
    // NT names are not normalized: backslashes only, without a verbatim `\\?\` prefix.
    let units: Vec<u16> = target
        .as_os_str()
        .encode_wide()
        .map(|unit| if unit == 47 { 92 } else { unit })
        .collect();
    let print = units.strip_prefix(&[92, 92, 63, 92][..]).unwrap_or(&units);
    let substitute: Vec<u16> = r"\??\"
        .encode_utf16()
        .chain(print.iter().copied())
        .collect();
    let names: Vec<u16> = substitute
        .iter()
        .chain(&[0])
        .chain(print)
        .chain(&[0])
        .copied()
        .collect();
    let length = |units: usize| u16::try_from(units * 2).map_err(io::Error::other);
    let mut buffer = Vec::new();
    buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    buffer.extend(
        u16::try_from(8 + names.len() * 2)
            .map_err(io::Error::other)?
            .to_le_bytes(),
    );
    // Reserved, then the substitute name's offset (zero).
    buffer.extend([0, 0, 0, 0]);
    buffer.extend(length(substitute.len())?.to_le_bytes());
    buffer.extend(length(substitute.len() + 1)?.to_le_bytes());
    buffer.extend(length(print.len())?.to_le_bytes());
    buffer.extend(names.iter().flat_map(|unit| unit.to_le_bytes()));
    let size = u32::try_from(buffer.len()).map_err(io::Error::other)?;
    let mut returned = 0;
    // SAFETY: `directory` is a live handle for the call's duration; `buffer` is a
    // complete mount-point reparse buffer of `size` bytes that outlives this synchronous
    // call (no OVERLAPPED), and no output buffer is requested.
    if unsafe {
        DeviceIoControl(
            directory.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            size,
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
