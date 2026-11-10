use midnight_core::{ArchiveNode, ArchiveTimestamp, Entry, EntryKind, VirtualEntryId};
use std::time::{Duration, UNIX_EPOCH};

fn entry(timestamp: ArchiveTimestamp) -> Entry {
    Entry::from_archive(ArchiveNode {
        id: VirtualEntryId { mount: 1, entry: 2 },
        parent: 1,
        raw_name: b"file".as_slice().into(),
        display_name: "file".into(),
        kind: EntryKind::File,
        size: Some(1),
        readable: true,
        modified: Some(timestamp),
    })
}
#[test]
fn utc_floor_seconds_preserve_subsecond_values_before_and_after_epoch() {
    assert_eq!(
        entry(ArchiveTimestamp::Utc {
            seconds: -1,
            nanoseconds: 500_000_000
        })
        .metadata
        .modified,
        UNIX_EPOCH.checked_sub(Duration::from_millis(500))
    );
    assert_eq!(
        entry(ArchiveTimestamp::Utc {
            seconds: 1,
            nanoseconds: 500_000_000
        })
        .metadata
        .modified,
        UNIX_EPOCH.checked_add(Duration::from_millis(1500))
    );
    assert!(
        entry(ArchiveTimestamp::Utc {
            seconds: 0,
            nanoseconds: 1_000_000_000
        })
        .metadata
        .modified
        .is_none()
    );
}
#[test]
fn civil_archive_time_keeps_its_timezone_unspecified_and_rejects_invalid_dates() {
    let timestamp = ArchiveTimestamp::Local {
        year: 2024,
        month: 2,
        day: 29,
        hour: 12,
        minute: 34,
        second: 56,
    };
    assert!(timestamp.is_valid());
    let entry = entry(timestamp);
    assert!(entry.metadata.modified.is_none());
    assert_eq!(entry.virtual_node.unwrap().modified, Some(timestamp));
    assert!(
        !ArchiveTimestamp::Local {
            year: 1900,
            month: 2,
            day: 29,
            hour: 0,
            minute: 0,
            second: 0
        }
        .is_valid()
    );
}
