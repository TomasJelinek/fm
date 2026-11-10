#[path = "../src/archive_selection.rs"]
mod archive_selection;
use midnight_core::*;
use std::collections::HashSet;

fn entries() -> Vec<Entry> {
    (0..6)
        .map(|row| {
            Entry::from_archive(ArchiveNode {
                modified: None,
                id: VirtualEntryId {
                    mount: 42,
                    entry: 100 + row * 13,
                },
                parent: 7,
                raw_name: vec![0xff, row as u8].into(),
                display_name: "same 文档.txt".into(),
                kind: EntryKind::File,
                size: Some(row),
                readable: true,
            })
        })
        .collect()
}

#[test]
fn disjoint_archive_rectangle_preserves_duplicate_names_and_opaque_ids() {
    let entries = entries();
    let view = DirectoryIndex::from_entries(&entries);
    assert!(
        view.by_path.is_empty(),
        "archive members must never become native paths"
    );
    let ranges = SelectionRanges::new(
        6,
        vec![
            SelectionSpan { first: 1, last: 2 },
            SelectionSpan { first: 5, last: 5 },
        ],
    )
    .unwrap();
    let prior = HashSet::from([entries[0].id, entries[2].id]);
    for (mode, expected) in [
        (SelectionMode::Replace, vec![1, 2, 5]),
        (SelectionMode::Add, vec![0, 1, 2, 5]),
        (SelectionMode::Toggle, vec![0, 1, 5]),
    ] {
        let result = archive_selection::apply(
            &entries,
            &view,
            &prior,
            ArchiveSelectionAction::Ranges {
                ranges: ranges.clone(),
                mode,
            },
        )
        .unwrap();
        assert_eq!(
            *result.selected,
            expected.iter().map(|row| entries[*row].id).collect()
        );
        assert_eq!(result.totals.files(), expected.len());
        assert_eq!(
            result.totals.file_bytes(),
            Some(expected.iter().map(|row| *row as u64).sum())
        );
    }
}

#[test]
fn changed_archive_extent_fails_without_silent_partial_selection() {
    let entries = entries();
    let view = DirectoryIndex::from_entries(&entries);
    let ranges = SelectionRanges::new(7, vec![SelectionSpan { first: 0, last: 6 }]).unwrap();
    assert!(
        archive_selection::apply(
            &entries,
            &view,
            &HashSet::new(),
            ArchiveSelectionAction::Ranges {
                ranges,
                mode: SelectionMode::Replace
            }
        )
        .is_err()
    );
}
