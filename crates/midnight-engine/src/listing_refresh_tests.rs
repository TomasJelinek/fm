use super::*;
use std::time::{Duration, SystemTime};

fn entry(id: u64, name: &str, size: u64) -> Entry {
    let mut entry = Entry::new(
        EntryId(id),
        PathBuf::from("/shown").join(name),
        EntryKind::File,
    );
    entry.metadata.size = Some(size);
    entry
}

/// Applies `edits` to `before` as a model would, taking added and changed
/// rows from `after`.
fn apply(before: &[Entry], after: &[Entry], edits: &[RowEdit]) -> Vec<(EntryId, PathBuf, u64)> {
    let row = |entry: &Entry| (entry.id, entry.path.clone(), entry.metadata.size.unwrap());
    let mut rows: Vec<_> = before.iter().map(row).collect();
    for edit in edits {
        match *edit {
            RowEdit::Removed { row, count } => {
                rows.drain(row..row + count);
            }
            RowEdit::Added { row: at, count } => {
                for offset in 0..count {
                    rows.insert(at + offset, row(&after[at + offset]));
                }
            }
            RowEdit::Changed { row: at } => {
                assert_eq!(rows[at].0, after[at].id, "a changed row keeps its entry");
                rows[at] = row(&after[at]);
            }
        }
    }
    rows
}

fn check(before: &[Entry], after: &[Entry]) -> Vec<RowEdit> {
    let edits = row_edits(before, after).expect("a few edits");
    let expected: Vec<_> = after
        .iter()
        .map(|entry| (entry.id, entry.path.clone(), entry.metadata.size.unwrap()))
        .collect();
    assert_eq!(apply(before, after, &edits), expected, "{edits:?}");
    edits
}

#[test]
fn unchanged_rows_need_no_edit() {
    let rows = [entry(1, "a", 1), entry(2, "b", 2)];
    assert_eq!(check(&rows, &rows), []);
}

#[test]
fn additions_removals_and_changes_are_merged_runs() {
    let before = [
        entry(1, "a", 1),
        entry(2, "b", 2),
        entry(3, "c", 3),
        entry(4, "d", 4),
        entry(5, "e", 5),
    ];
    let after = [
        entry(1, "a", 1),
        entry(10, "b1", 1),
        entry(11, "b2", 1),
        entry(4, "d", 40),
        entry(5, "e", 5),
        entry(12, "f", 1),
    ];
    assert_eq!(
        check(&before, &after),
        [
            RowEdit::Removed { row: 1, count: 2 },
            RowEdit::Added { row: 1, count: 2 },
            RowEdit::Changed { row: 3 },
            RowEdit::Added { row: 5, count: 1 },
        ]
    );
}

#[test]
fn moved_rows_are_removed_and_added_again() {
    let before = [entry(1, "a", 1), entry(2, "b", 2), entry(3, "c", 3)];
    check(
        &before,
        &[entry(2, "b", 2), entry(3, "c", 3), entry(1, "a", 9)],
    );
    check(
        &before,
        &[entry(3, "c", 3), entry(1, "a", 1), entry(2, "b", 2)],
    );
    check(&before, &[]);
    check(&[], &before);
}

#[test]
fn a_recycled_identity_of_another_path_is_another_row() {
    let before = [entry(1, "a", 1), entry(2, "b", 2)];
    let edits = check(&before, &[entry(1, "a", 1), entry(2, "c", 2)]);
    assert_eq!(
        edits,
        [
            RowEdit::Removed { row: 1, count: 1 },
            RowEdit::Added { row: 1, count: 1 },
        ]
    );
}

#[test]
fn a_shuffled_listing_is_replaced_rather_than_edited() {
    let before: Vec<_> = (0..1_000)
        .map(|id| entry(id, &format!("{id:04}"), 1))
        .collect();
    let mut after = before.clone();
    after.reverse();
    assert_eq!(row_edits(&before, &after), None);
}

#[test]
fn adoption_keeps_shown_identities_and_localized_times() {
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut same = entry(1, "same", 1);
    same.metadata.modified = Some(modified);
    let shown = Shown::new(Arc::from(vec![same.clone(), entry(2, "grown", 2)]));
    let mut adoption = Adoption::default();
    let mut entries = vec![
        Entry::new(EntryId(100), same.path.clone(), EntryKind::File),
        Entry::new(EntryId(101), PathBuf::from("/shown/grown"), EntryKind::File),
        Entry::new(EntryId(102), PathBuf::from("/shown/new"), EntryKind::File),
        // The same path as another kind is another entry.
        Entry::new(
            EntryId(103),
            PathBuf::from("/shown/grown"),
            EntryKind::Directory,
        ),
    ];
    let metadata = |size| EntryMetadata {
        size: Some(size),
        modified: Some(modified),
        ..EntryMetadata::default()
    };
    let read = [
        Some(metadata(1)),
        Some(EntryMetadata {
            size: Some(20),
            ..EntryMetadata::default()
        }),
        Some(metadata(3)),
        None,
    ];
    for (entry, metadata) in entries.iter_mut().zip(read) {
        adoption.adopt(&shown, entry, metadata, None);
    }
    assert!(adoption.finish(&mut entries));
    let ids: Vec<_> = entries.iter().map(|entry| entry.id.0).collect();
    assert_eq!(ids, [1, 2, 102, 103]);
    assert_eq!(entries[0].metadata, same.metadata);
    assert_eq!(entries[1].metadata.size, Some(20));
    assert_eq!(entries[2].metadata.size, Some(3));
}

#[test]
fn a_new_entry_numbered_like_a_shown_one_restores_the_read_identities() {
    let shown = Shown::new(Arc::from(vec![entry(1, "a", 1), entry(7, "b", 1)]));
    let mut adoption = Adoption::default();
    // The provider numbers this listing from 7 again.
    let mut entries = vec![
        Entry::new(EntryId(7), PathBuf::from("/shown/new"), EntryKind::File),
        Entry::new(EntryId(8), PathBuf::from("/shown/b"), EntryKind::File),
    ];
    for entry in &mut entries {
        adoption.adopt(&shown, entry, None, None);
    }
    assert!(!adoption.finish(&mut entries));
    let ids: Vec<_> = entries.iter().map(|entry| entry.id.0).collect();
    assert_eq!(ids, [7, 8]);
}
