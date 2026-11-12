//! Required modified time using retained handles and exact native FILETIME values.
use super::{destination::recovery::Revision, *};
use midnight_core::TransferModifiedTimePin;
use midnight_platform_common::transfer_modified_time::{
    ModifiedTimeCaptureError, capture_modified_time,
};
use std::{
    os::windows::io::AsRawHandle,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Foundation::FILETIME;

struct Pin {
    file: Arc<File>,
    path: PathBuf,
    revision: Revision,
    modified: SystemTime,
}
impl TransferModifiedTimePin for Pin {
    fn modified(&self) -> SystemTime {
        self.modified
    }
    fn current_modified(&self, cancel: &Cancellation) -> Result<SystemTime, FsError> {
        modified(&self.file, &self.path, cancel)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        if Revision::of(&self.file, &self.path)? != self.revision {
            return Err(invalid(&self.path, "source modified-time revision changed"));
        }
        Ok(())
    }
}
pub(super) fn prepare(
    file: Arc<File>,
    path: &Path,
    cancel: &Cancellation,
) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
    let captured = capture_modified_time(
        || {
            checked(cancel, path)?;
            Revision::of(&file, path)
        },
        || modified(&file, path, cancel),
    )
    .map_err(|e| match e {
        ModifiedTimeCaptureError::Read(error) => error,
        ModifiedTimeCaptureError::Changed => {
            invalid(path, "source changed while capturing modified time")
        }
    })?;
    native_time(captured.modified, path)?;
    Ok(Box::new(Pin {
        file,
        path: path.to_owned(),
        revision: captured.revision,
        modified: captured.modified,
    }))
}
pub(super) fn modified(
    file: &File,
    path: &Path,
    cancel: &Cancellation,
) -> Result<SystemTime, FsError> {
    checked(cancel, path)?;
    file.metadata()
        .and_then(|meta| meta.modified())
        .map_err(|e| error(path, e))
}
pub(super) fn set(
    file: &File,
    path: &Path,
    expected: SystemTime,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    checked(cancel, path)?;
    let time = native_time(expected, path)?;
    // SAFETY: the exact owned stage handle has WRITE_ATTRIBUTES; the fixed-size
    // FILETIME stays live and null creation/access parameters leave them unchanged.
    if unsafe {
        SetFileTime(
            file.as_raw_handle(),
            std::ptr::null(),
            std::ptr::null(),
            &time,
        )
    } == 0
    {
        return Err(error(path, io::Error::last_os_error()));
    }
    Ok(())
}
fn native_time(value: SystemTime, path: &Path) -> Result<FILETIME, FsError> {
    let nanos = match value.duration_since(UNIX_EPOCH) {
        Ok(v) => i128::try_from(v.as_nanos()).ok(),
        Err(e) => i128::try_from(e.duration().as_nanos())
            .ok()
            .and_then(i128::checked_neg),
    }
    .ok_or_else(|| unsupported(path, "modified time is outside native FILETIME range"))?;
    if nanos % 100 != 0 {
        return Err(unsupported(
            path,
            "modified time cannot be represented exactly in FILETIME",
        ));
    }
    let ticks = 116_444_736_000_000_000i128 + nanos / 100;
    if ticks <= 0 || ticks > i64::MAX as i128 {
        return Err(unsupported(
            path,
            "modified time is a reserved or unsupported FILETIME value",
        ));
    }
    let ticks = ticks as u64;
    Ok(FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    })
}
