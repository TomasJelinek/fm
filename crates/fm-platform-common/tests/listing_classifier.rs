use fm_core::{Cancellation, EntryKind};
use fm_platform_common::LocalServices;
use std::{cell::Cell, fs};

#[test]
fn neutral_listing_uses_the_supplied_cached_classifier_and_honors_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("文档_e\u{301}_🦀");
    fs::write(&path, b"unchanged").unwrap();
    let calls = Cell::new(0);
    let cancel = Cancellation::default();
    let mut entries = Vec::new();
    LocalServices::default()
        .read_directory_with(
            temp.path(),
            &cancel,
            |kind| {
                calls.set(calls.get() + 1);
                assert!(kind.is_file());
                EntryKind::Other
            },
            &mut |entry| {
                entries.push(entry);
                true
            },
        )
        .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(entries[0].kind, EntryKind::Other);
    assert_eq!(entries[0].path, path);
    assert_eq!(fs::read(&path).unwrap(), b"unchanged");
    cancel.cancel();
    LocalServices::default()
        .read_directory_with(
            temp.path(),
            &cancel,
            |_| panic!("cancelled listing classified an entry"),
            &mut |_| panic!("cancelled listing emitted an entry"),
        )
        .unwrap();
}
