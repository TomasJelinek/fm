//! Bounded metadata capture from the same retained native transfer object.
use std::time::SystemTime;

/// A consistent source timestamp and the native revision used to validate it.
#[derive(Debug, PartialEq, Eq)]
pub struct CapturedModifiedTime<R> {
    pub modified: SystemTime,
    pub revision: R,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ModifiedTimeCaptureError<E> {
    Read(E),
    Changed,
}

/// Capture before data reads, rejecting a continuously changing source after a
/// bounded number of attempts. Each callback must address the same held object.
pub fn capture_modified_time<R: Eq, E>(
    mut read_revision: impl FnMut() -> Result<R, E>,
    mut read_modified: impl FnMut() -> Result<SystemTime, E>,
) -> Result<CapturedModifiedTime<R>, ModifiedTimeCaptureError<E>> {
    for _ in 0..3 {
        let before = read_revision().map_err(ModifiedTimeCaptureError::Read)?;
        let modified = read_modified().map_err(ModifiedTimeCaptureError::Read)?;
        let after = read_revision().map_err(ModifiedTimeCaptureError::Read)?;
        if before == after {
            return Ok(CapturedModifiedTime {
                modified,
                revision: after,
            });
        }
    }
    Err(ModifiedTimeCaptureError::Changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, time::Duration};

    fn time(nanos: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos)
    }

    #[test]
    fn stable_capture_retains_exact_native_revision_and_timestamp() {
        let reads = Cell::new(0);
        let captured = capture_modified_time(
            || {
                reads.set(reads.get() + 1);
                Ok::<_, &str>((17, 23))
            },
            || Ok(time(101)),
        )
        .unwrap();
        assert_eq!(captured.modified, time(101));
        assert_eq!(captured.revision, (17, 23));
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn edit_between_timestamp_and_revision_reads_cannot_capture_stale_time() {
        let generation = Cell::new(0);
        let changed = Cell::new(false);
        let captured = capture_modified_time(
            || Ok::<_, &str>(generation.get()),
            || {
                if !changed.replace(true) {
                    generation.set(1);
                    Ok(time(100))
                } else {
                    Ok(time(200))
                }
            },
        )
        .unwrap();
        assert_eq!(captured.modified, time(200));
        assert_eq!(captured.revision, 1);
    }

    #[test]
    fn continually_changing_source_is_refused_without_an_unbounded_retry() {
        let revision_reads = Cell::new(0);
        let time_reads = Cell::new(0);
        let result = capture_modified_time(
            || {
                let revision = revision_reads.get();
                revision_reads.set(revision + 1);
                Ok::<_, &str>(revision)
            },
            || {
                time_reads.set(time_reads.get() + 1);
                Ok(time(100))
            },
        );
        assert_eq!(result, Err(ModifiedTimeCaptureError::Changed));
        assert_eq!(revision_reads.get(), 6);
        assert_eq!(time_reads.get(), 3);
    }

    #[test]
    fn native_timestamp_error_is_preserved_and_not_retried_as_a_change() {
        let reads = Cell::new(0);
        let result = capture_modified_time(
            || Ok::<_, &str>(7),
            || {
                reads.set(reads.get() + 1);
                Err("timestamp unavailable")
            },
        );
        assert_eq!(
            result,
            Err(ModifiedTimeCaptureError::Read("timestamp unavailable"))
        );
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn failed_revision_read_stops_before_metadata_access() {
        let result = capture_modified_time(
            || Err::<u64, _>("revision unavailable"),
            || panic!("timestamp must not be read without an initial revision"),
        );
        assert_eq!(
            result,
            Err(ModifiedTimeCaptureError::Read("revision unavailable"))
        );
    }
}
