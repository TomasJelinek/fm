//! Audited native volume strings and thread-local media-dialog suppression.
use midnight_filesystem_plugin_api::{
    DEFAULT_REPLY_BYTES, ErrorCode, Failure, MAX_VOLUME_TEXT, NameEncoding, NativeText,
};
use midnight_filesystem_plugin_sdk::CallContext;
use std::{io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeInformationW, GetVolumePathNameW},
    System::Diagnostics::Debug::{GetThreadErrorMode, SEM_FAILCRITICALERRORS, SetThreadErrorMode},
};

pub fn check(context: &CallContext<'_>) -> Result<(), Failure> {
    if context.is_cancelled() {
        Err(Failure::new(ErrorCode::Cancelled, "Volume query canceled."))
    } else {
        Ok(())
    }
}
pub fn text(units: &[u16]) -> NativeText {
    NativeText {
        encoding: NameEncoding::WindowsUtf16Le,
        bytes: units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
    }
}
pub fn root(path: &Path, context: &CallContext<'_>) -> Result<Vec<u16>, Failure> {
    check(context)?;
    // Canonicalize the literal raw path first; missing descendants never select a parent volume.
    let canonical = std::fs::canonicalize(crate::path::literal(path)?).map_err(Failure::from_io)?;
    let input: Vec<_> = canonical.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut root = bounded_buffer(context, DEFAULT_REPLY_BYTES as usize / 2, |output| {
        // SAFETY: terminated input and writable dynamically sized output live through this call.
        if unsafe { GetVolumePathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) }
            == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    })?;
    root.push(0);
    Ok(root)
}
pub fn wide_buffer(
    context: &CallContext<'_>,
    query: impl FnMut(&mut [u16]) -> io::Result<()>,
) -> Result<Vec<u16>, Failure> {
    bounded_buffer(context, MAX_VOLUME_TEXT / 2, query)
}
fn bounded_buffer(
    context: &CallContext<'_>,
    maximum: usize,
    mut query: impl FnMut(&mut [u16]) -> io::Result<()>,
) -> Result<Vec<u16>, Failure> {
    let mut buffer = vec![0; 64];
    loop {
        check(context)?;
        match query(&mut buffer) {
            Ok(()) => {
                let end = buffer.iter().position(|unit| *unit == 0).ok_or_else(|| {
                    Failure::new(ErrorCode::Invalid, "Unterminated native volume string.")
                })?;
                buffer.truncate(end);
                return Ok(buffer);
            }
            Err(error) if matches!(error.raw_os_error(), Some(122 | 206 | 234)) => {
                let length = buffer.len() * 2;
                if length > maximum {
                    return Err(Failure::new(
                        ErrorCode::ResourceLimit,
                        "Native volume string budget exceeded.",
                    ));
                }
                buffer.resize(length, 0);
            }
            Err(error) => return Err(Failure::from_io(error)),
        }
    }
}
pub fn label(path: &Path, context: &CallContext<'_>) -> Result<Option<NativeText>, Failure> {
    let _mode = ErrorMode::new().map_err(Failure::from_io)?;
    let root = root(path, context)?;
    let units = wide_buffer(context, |output| {
        // SAFETY: terminated root, exact writable output capacity and optional null outputs.
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                output.as_mut_ptr(),
                output.len() as u32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    })?;
    Ok((!units.is_empty()).then(|| text(&units)))
}
pub fn space(path: &Path, context: &CallContext<'_>) -> Result<(u64, u64), Failure> {
    let _mode = ErrorMode::new().map_err(Failure::from_io)?;
    let root = root(path, context)?;
    check(context)?;
    let (mut available, mut total) = (0, 0);
    // SAFETY: terminated root and exact writable outputs. Legacy total deliberately honors quota.
    if unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut available, &mut total, ptr::null_mut()) }
        == 0
    {
        return Err(Failure::from_io(io::Error::last_os_error()));
    }
    Ok((available, total))
}
pub struct ErrorMode(u32);
impl ErrorMode {
    pub fn new() -> io::Result<Self> {
        // SAFETY: affects only this synchronous thread, output is acquired only on success.
        unsafe {
            let previous = GetThreadErrorMode();
            let mut actual_previous = 0;
            if SetThreadErrorMode(previous | SEM_FAILCRITICALERRORS, &mut actual_previous) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(actual_previous))
        }
    }
}
impl Drop for ErrorMode {
    fn drop(&mut self) {
        // SAFETY: same synchronous thread; successful constructor owns the saved mode.
        unsafe {
            SetThreadErrorMode(self.0, ptr::null_mut());
        }
    }
}
