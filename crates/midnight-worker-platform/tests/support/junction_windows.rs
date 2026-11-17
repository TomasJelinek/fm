//! Native NTFS junction fixture: no shell, PowerShell or command text is involved.
use std::{
    fs::{self, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
    System::IO::DeviceIoControl,
};

const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00a4;
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xa000_0003;

/// Create `link` as a junction to the absolute `target`, which need not exist.
pub fn create(link: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir(link)?;
    let result = set_mount_point(link, target);
    if result.is_err() {
        let _ = fs::remove_dir(link);
    }
    result
}

fn set_mount_point(link: &Path, target: &Path) -> io::Result<()> {
    let directory = OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(link)?;
    // The print name is the Win32 path; the substitute name is its NT form `\??\C:\...`.
    let units: Vec<u16> = target.as_os_str().encode_wide().collect();
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
    // Reserved, then the substitute name offset (zero).
    buffer.extend([0, 0, 0, 0]);
    buffer.extend(length(substitute.len())?.to_le_bytes());
    buffer.extend(length(substitute.len() + 1)?.to_le_bytes());
    buffer.extend(length(print.len())?.to_le_bytes());
    buffer.extend(names.iter().flat_map(|unit| unit.to_le_bytes()));
    let mut returned = 0;
    // SAFETY: a live directory handle and a complete mount-point reparse buffer.
    if unsafe {
        DeviceIoControl(
            directory.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
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
