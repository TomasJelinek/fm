use super::*;
use std::{collections::HashSet, sync::Arc};

#[test]
fn archive_selection_keeps_panel_order_and_duplicate_name_identities() {
    let entries: Arc<[Entry]> = vec![
        Entry::new(EntryId(1), "same.txt".into(), EntryKind::File),
        Entry::new(EntryId(2), "folder".into(), EntryKind::Directory),
        Entry::new(EntryId(3), "same.txt".into(), EntryKind::File),
    ]
    .into();
    let visible = vec![entries[2].clone(), entries[1].clone()];
    let capture = FileListCapture {
        directory: "archive.zip".into(),
        source: FileListSource::Archive {
            entries,
            view: Arc::new(DirectoryIndex::from_entries(&visible)),
            selected: Arc::new(HashSet::from([EntryId(1), EntryId(3)])),
            fallback: Some(EntryId(2)),
        },
    };
    assert_eq!(
        selected(&capture, &Cancellation::default())
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>(),
        [EntryId(3)]
    );
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(selected(&capture, &cancelled).is_err());
}

#[test]
fn directory_dots_are_not_extensions_and_unknown_metadata_stays_unknown() {
    let mut entry = Entry::new(EntryId(1), "folder.txt".into(), EntryKind::Directory);
    entry.metadata.size = Some(4096); // A directory entry's own length is not its recursive size.
    let row = record(&entry, "archive.zip", &BTreeMap::new()).unwrap();
    assert_eq!(row.name_part, "folder.txt");
    assert_eq!(row.extension, "");
    assert_eq!(row.size, "<DIR>");
    assert_eq!(row.date, "-");
    let mut entry = Entry::new(EntryId(2), "文档.rs".into(), EntryKind::File);
    entry.metadata.size = Some(12345);
    let row = record(&entry, "archive.zip", &BTreeMap::new()).unwrap();
    assert_eq!(row.name_part, "文档");
    assert_eq!(row.extension, "rs");
    assert_eq!(row.size, "12345");
}
