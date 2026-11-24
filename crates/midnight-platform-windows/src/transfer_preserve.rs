//! Preserved times and attributes (`TransferMetadataPolicy::PreserveTimesAndAttributes`):
//! read from a retained source handle, applied through a retained destination handle in
//! one `FILE_BASIC_INFO` call, so Read-only arrives together with the times, last.
use super::*;
use midnight_core::{PRESERVED_WINDOWS_ATTRIBUTES, TransferPreservedMetadata};
use std::{
    os::windows::io::AsRawHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Attributes a basic-information call may carry (`FILE_ATTRIBUTE_VALID_SET_FLAGS`) and
/// the cloud pin states. Directory, Compressed, Encrypted, Sparse and reparse bits are
/// how the object is stored; they are never passed back.
const SETTABLE_ATTRIBUTES: u32 = 0x31a7 | 0x8_0000 | 0x10_0000;
/// FILETIME ticks (100 ns) between 1601-01-01 and the Unix epoch.
const EPOCH_TICKS: i128 = 116_444_736_000_000_000;

/// The three times and the attributes of the object `file` refers to: one
/// `GetFileInformationByHandle` call, without following a link.
pub(super) fn read(file: &File, path: &Path) -> Result<TransferPreservedMetadata, FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    Ok(TransferPreservedMetadata {
        created: time(metadata.creation_time()),
        accessed: time(metadata.last_access_time()),
        modified: time(metadata.last_write_time()),
        windows_attributes: Some(metadata.file_attributes()),
        unix_mode: None,
    })
}

/// Set what `metadata` carries on the object `file` refers to, which must have
/// FILE_READ_ATTRIBUTES and FILE_WRITE_ATTRIBUTES. Times the filesystem stores more
/// coarsely (FAT: 2 s modified time, a date for access) are rounded by it; unknown times
/// are left as they are. The attributes outside `PRESERVED_WINDOWS_ATTRIBUTES` stay.
pub(super) fn apply(
    file: &File,
    path: &Path,
    metadata: &TransferPreservedMetadata,
) -> Result<(), FsError> {
    let mut basic = FILE_BASIC_INFO {
        CreationTime: ticks(metadata.created),
        LastAccessTime: ticks(metadata.accessed),
        LastWriteTime: ticks(metadata.modified),
        ..Default::default()
    };
    if let Some(source) = metadata.windows_attributes {
        let current = file
            .metadata()
            .map_err(|e| error(path, e))?
            .file_attributes();
        let mut next = (current & SETTABLE_ATTRIBUTES & !PRESERVED_WINDOWS_ATTRIBUTES)
            | (source & PRESERVED_WINDOWS_ATTRIBUTES);
        next &= !FILE_ATTRIBUTE_NORMAL;
        // Zero would leave the attributes unchanged; Normal clears them.
        basic.FileAttributes = if next == 0 {
            FILE_ATTRIBUTE_NORMAL
        } else {
            next
        };
    }
    if basic.CreationTime == 0
        && basic.LastAccessTime == 0
        && basic.LastWriteTime == 0
        && basic.FileAttributes == 0
    {
        return Ok(());
    }
    // SAFETY: a live handle with FILE_WRITE_ATTRIBUTES and a fixed-size structure that
    // stays alive during the call; zero times leave those times unchanged.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileBasicInfo,
            (&basic as *const FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(error(path, io::Error::last_os_error()));
    }
    Ok(())
}

/// A FILETIME value as a time; zero (never set) is unknown.
fn time(ticks: u64) -> Option<SystemTime> {
    if ticks == 0 {
        return None;
    }
    let nanos = (i128::from(ticks) - EPOCH_TICKS) * 100;
    let magnitude = Duration::from_nanos(u64::try_from(nanos.unsigned_abs()).ok()?);
    if nanos >= 0 {
        UNIX_EPOCH.checked_add(magnitude)
    } else {
        UNIX_EPOCH.checked_sub(magnitude)
    }
}

/// A time as FILETIME ticks, truncated to 100 ns; 0 (leave unchanged) for an unknown time
/// or one FILETIME cannot represent.
fn ticks(time: Option<SystemTime>) -> i64 {
    let Some(time) = time else {
        return 0;
    };
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i128::try_from(after.as_nanos()).ok(),
        Err(before) => i128::try_from(before.duration().as_nanos())
            .ok()
            .map(|nanos| -nanos),
    };
    nanos
        .map(|nanos| EPOCH_TICKS + nanos.div_euclid(100))
        .filter(|ticks| *ticks > 0 && *ticks <= i128::from(i64::MAX))
        .map_or(0, |ticks| ticks as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_values_round_trip_and_unknown_or_unrepresentable_times_stay_unchanged() {
        for value in [1u64, 116_444_736_000_000_000, 133_000_000_123_456_789] {
            assert_eq!(ticks(time(value)) as u64, value);
        }
        assert_eq!(time(0), None);
        assert_eq!(ticks(None), 0);
        // 1601-01-01 itself is FILETIME zero, which means "unchanged": left unchanged.
        let first = UNIX_EPOCH - Duration::from_secs(11_644_473_600);
        assert_eq!(ticks(Some(first)), 0);
        // Nanoseconds below 100 ns are truncated, as FILETIME stores them.
        let fine = UNIX_EPOCH + Duration::from_nanos(1_234_567_891);
        assert_eq!(ticks(Some(fine)), (EPOCH_TICKS + 12_345_678) as i64);
    }
}
