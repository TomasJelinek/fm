//! Required modified time on exact retained native files; never reopen source names.
use super::{destination::recovery::Revision, *};
use midnight_core::TransferModifiedTimePin;
use midnight_platform_common::transfer_modified_time::{
    ModifiedTimeCaptureError, capture_modified_time,
};
use rustix::fs::{Timespec, Timestamps, UTIME_OMIT, futimens};
use std::time::{SystemTime, UNIX_EPOCH};

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
    timestamp(captured.modified, path)?;
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
    let times = Timestamps {
        last_access: Timespec {
            tv_sec: 0,
            tv_nsec: UTIME_OMIT,
        },
        last_modification: timestamp(expected, path)?,
    };
    futimens(file, &times).map_err(|e| error(path, e.into()))
}
fn timestamp(value: SystemTime, path: &Path) -> Result<Timespec, FsError> {
    let nanos = match value.duration_since(UNIX_EPOCH) {
        Ok(v) => i128::try_from(v.as_nanos()).ok(),
        Err(e) => i128::try_from(e.duration().as_nanos())
            .ok()
            .and_then(i128::checked_neg),
    }
    .ok_or_else(|| unsupported(path, "modified time is outside native range"))?;
    let seconds = i64::try_from(nanos.div_euclid(1_000_000_000))
        .map_err(|_| unsupported(path, "modified time is outside native range"))?;
    let nanos = nanos.rem_euclid(1_000_000_000) as i64;
    Ok(Timespec {
        tv_sec: seconds,
        tv_nsec: nanos,
    })
}
