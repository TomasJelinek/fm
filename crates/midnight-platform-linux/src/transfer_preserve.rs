//! Preserved times and permissions (`TransferMetadataPolicy::PreserveTimesAndAttributes`):
//! read from a retained source descriptor, applied through the copy's own descriptor.
//! Linux sets the access and modified times at nanosecond precision; no local filesystem
//! lets a process set a file's birth time, so the creation time is not preserved.
use super::*;
use midnight_core::{PRESERVED_UNIX_MODE_BITS, TransferPreservedMetadata};
use rustix::fs::{Timespec, Timestamps, UTIME_OMIT, fchmod, fcntl_getfl, fcntl_setfl, futimens};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The access and modified times and the mode of the object `file` refers to (an
/// `O_PATH` descriptor too), without following a link.
pub(super) fn read(file: &File, path: &Path) -> Result<TransferPreservedMetadata, FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    Ok(TransferPreservedMetadata {
        created: None,
        accessed: time(metadata.atime(), metadata.atime_nsec()),
        modified: time(metadata.mtime(), metadata.mtime_nsec()),
        windows_attributes: None,
        unix_mode: Some(metadata.mode()),
    })
}

/// Set the access and modified times, then the permission bits (`extra` adds bits the
/// copy keeps from its destination, such as a folder's inherited set-group-ID), on the
/// object `file` refers to, which this user owns. The mode comes last: a mode without
/// write permission does not stop the owner, but it is the change that makes the copy
/// read-only.
pub(super) fn apply(
    file: &File,
    path: &Path,
    metadata: &TransferPreservedMetadata,
    extra: u32,
) -> Result<(), FsError> {
    if metadata.accessed.is_some() || metadata.modified.is_some() {
        let times = Timestamps {
            last_access: timespec(metadata.accessed),
            last_modification: timespec(metadata.modified),
        };
        futimens(file, &times).map_err(|e| error(path, e.into()))?;
    }
    if let Some(mode) = metadata.unix_mode {
        fchmod(
            file,
            Mode::from_raw_mode((mode & PRESERVED_UNIX_MODE_BITS) | extra),
        )
        .map_err(|e| error(path, e.into()))?;
    }
    Ok(())
}

/// Reads through `file` no longer change its access time (`O_NOATIME`, allowed to the
/// owner): a Move's proof reads the copy after its times are set. Best effort.
pub(super) fn keep_access_time(file: &File) {
    if let Ok(flags) = fcntl_getfl(file) {
        let _ = fcntl_setfl(file, flags | OFlags::NOATIME);
    }
}

fn time(seconds: i64, nanos: i64) -> Option<SystemTime> {
    let nanos = u32::try_from(nanos)
        .ok()
        .filter(|nanos| *nanos < 1_000_000_000)?;
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(seconds.unsigned_abs(), nanos))
    } else {
        UNIX_EPOCH
            .checked_sub(Duration::from_secs(seconds.unsigned_abs()))?
            .checked_add(Duration::from_nanos(u64::from(nanos)))
    }
}

/// A time for `futimens`; an unknown or unrepresentable one is left unchanged.
fn timespec(time: Option<SystemTime>) -> Timespec {
    let omit = Timespec {
        tv_sec: 0,
        tv_nsec: UTIME_OMIT,
    };
    let Some(time) = time else {
        return omit;
    };
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i128::try_from(after.as_nanos()).ok(),
        Err(before) => i128::try_from(before.duration().as_nanos())
            .ok()
            .map(|nanos| -nanos),
    };
    let Some(nanos) = nanos else {
        return omit;
    };
    match i64::try_from(nanos.div_euclid(1_000_000_000)) {
        Ok(seconds) => Timespec {
            tv_sec: seconds,
            tv_nsec: nanos.rem_euclid(1_000_000_000) as _,
        },
        Err(_) => omit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nanosecond_times_round_trip_including_before_the_epoch() {
        for (seconds, nanos) in [(0, 0), (1_700_000_000, 123_456_789), (-5, 250_000_000)] {
            let value = time(seconds, nanos).unwrap();
            let spec = timespec(Some(value));
            assert_eq!((spec.tv_sec, spec.tv_nsec), (seconds, nanos));
        }
        assert_eq!(timespec(None).tv_nsec, UTIME_OMIT);
    }
}
