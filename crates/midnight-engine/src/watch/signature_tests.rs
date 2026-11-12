use super::*;
use midnight_core::EntryId;
use std::time::SystemTime;

fn entry(name: &str, kind: EntryKind, size: u64) -> Entry {
    let mut entry = Entry::new(EntryId(1), Path::new("root").join(name), kind);
    entry.metadata.size = Some(size);
    entry
}

#[test]
fn signature_is_order_independent_and_lossless_with_duplicate_counts() {
    let first = entry("Žluťoučký 🦊", EntryKind::File, 7);
    let second = entry("other", EntryKind::Directory, 12);
    let mut left = Signature::default();
    left.add(&first).unwrap();
    left.add(&second).unwrap();
    let mut right = Signature::default();
    right.add(&second).unwrap();
    right.add(&first).unwrap();
    assert_eq!(left, right);
    right.add(&first).unwrap();
    assert_ne!(left, right);
    let mut renamed = first.clone();
    renamed.name = "Zlutoucky 🦊".into();
    let mut changed = Signature::default();
    changed.add(&renamed).unwrap();
    changed.add(&second).unwrap();
    assert_ne!(left, changed);
}

#[test]
fn folder_display_signature_is_separate_from_full_file_observation() {
    let directory = entry("folder", EntryKind::Directory, 1);
    let file = entry("inside.txt", EntryKind::File, 7);
    let mut display = Signature::default();
    display.add(&directory).unwrap();
    let mut full = display;
    full.add(&file).unwrap();
    assert_ne!(display, full);
    let mut computed = directory.clone();
    computed.metadata.size = Some(800);
    let mut same = Signature::default();
    same.add(&computed).unwrap();
    assert_eq!(display, same);
    for change in 0..4 {
        let mut file = file.clone();
        match change {
            0 => file.metadata.size = Some(8),
            1 => file.metadata.modified = Some(SystemTime::UNIX_EPOCH),
            2 => file.metadata.compression = CompressionStatus::Compressed,
            _ => file.metadata.unix_mode = Some(0o100640),
        }
        let mut current = display;
        current.add(&file).unwrap();
        assert_ne!(full, current);
    }
}
