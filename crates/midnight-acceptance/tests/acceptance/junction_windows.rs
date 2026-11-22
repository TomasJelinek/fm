//! Native NTFS directory junctions for the Windows scenarios, set with
//! `FSCTL_SET_REPARSE_POINT` as SECURITY.md requires of test fixtures: no shell or
//! command text sees the paths, so `%`, `!`, `^` and `&` in them are only data. This is
//! the fixture of `crates/midnight-worker-platform/tests/support/junction_windows.rs`.
//! Its one Win32 call is declared here: the suite's `windows` dependency carries only the
//! features of the video scenario's Media Foundation encoder. The suite's other unsafe
//! code, each with its own allowance, is the video scenario's `FSCTL_SET_COMPRESSION`
//! call (`scenarios/viewers_video.rs`) and its Media Foundation calls
//! (`scenarios/viewers_video_h264.rs`).
#![allow(unsafe_code, reason = "one audited DeviceIoControl call")]
use std::{
    ffi::c_void,
    fs::{self, OpenOptions},
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
pub fn create(link: &Path, target: &Path) -> io::Result<()> {
    let target = std::path::absolute(target)?;
    fs::create_dir(link)?;
    let result = set_mount_point(link, &target);
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
