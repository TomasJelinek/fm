//! File-only inversion across completed, streaming and archive listings.
use super::{tests::request, *};

fn entries() -> Vec<Entry> {
    [
        ("selected folder", EntryKind::Directory),
        ("junction", EntryKind::DirectoryLink),
        ("文档.txt", EntryKind::File),
        ("selected.rs", EntryKind::File),
        ("file link", EntryKind::Symlink),
        ("archive.zip", EntryKind::File),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (path, kind))| Entry::new(EntryId(i as u64 + 100), path.into(), kind))
    .collect()
}

#[test]
fn invert_files_preserves_folders_for_loaded_and_streaming_listings() {
    let rows = entries();
    let index = Arc::new(DirectoryIndex::from_entries(&rows));
    for streaming in [false, true] {
        for (operations, expected) in [
            (vec![SelectionOperation::ClearFilesOnly], 0b000001),
            (
                vec![SelectionOperation::All, SelectionOperation::ClearFilesOnly],
                0b000011,
            ),
            (
                vec![
                    SelectionOperation::FilesOnly,
                    SelectionOperation::ClearFilesOnly,
                ],
                0,
            ),
            (
                vec![
                    SelectionOperation::ClearFilesOnly,
                    SelectionOperation::Invert,
                ],
                0b111101,
            ),
            (vec![SelectionOperation::All], 0b111111),
            (vec![SelectionOperation::FilesOnly], 0b111100),
            (
                vec![SelectionOperation::All, SelectionOperation::FilesOnly],
                0b111100,
            ),
            (
                vec![SelectionOperation::FilesOnly, SelectionOperation::All],
                0b111111,
            ),
            (
                vec![SelectionOperation::FilesOnly, SelectionOperation::Invert],
                0,
            ),
            (vec![SelectionOperation::Invert], 0b110101),
            (vec![SelectionOperation::InvertAll], 0b110110),
            (
                vec![SelectionOperation::InvertAll, SelectionOperation::InvertAll],
                0b001001,
            ),
            (
                vec![SelectionOperation::All, SelectionOperation::InvertAll],
                0,
            ),
            (
                vec![SelectionOperation::FilesOnly, SelectionOperation::InvertAll],
                0b000011,
            ),
            (
                vec![SelectionOperation::Clear, SelectionOperation::InvertAll],
                0b111111,
            ),
            (
                vec![SelectionOperation::Invert, SelectionOperation::Invert],
                0b001001,
            ),
            (
                vec![SelectionOperation::All, SelectionOperation::Invert],
                0b000011,
            ),
            (
                vec![SelectionOperation::Clear, SelectionOperation::Invert],
                0b111100,
            ),
        ] {
            let mut state = State::default();
            let mut revision = 0;
            for row in [0, 3] {
                revision += 1;
                let mut toggle = request(
                    revision,
                    (!streaming).then(|| index.clone()),
                    SelectionAction::Toggle(rows[row].path.clone()),
                );
                toggle.toggle_directory = row == 0;
                state.apply(&toggle).unwrap();
            }
            let mut last = None;
            for operation in operations {
                revision += 1;
                last = state
                    .apply(&request(
                        revision,
                        (!streaming).then(|| index.clone()),
                        SelectionAction::Bulk(operation),
                    ))
                    .unwrap();
            }
            let (partial, defaults) = last.unwrap();
            for (position, entry) in rows.iter().enumerate() {
                let selected = partial.contains_path(&entry.path)
                    || (!partial.basis().by_path.contains_key(&entry.path)
                        && defaults.selects(entry.kind));
                assert_eq!(
                    selected,
                    expected & (1 << position) != 0,
                    "before final rebase: streaming {streaming}, {}",
                    entry.path.display()
                );
            }
            let (snapshot, _) = state
                .apply(&request(
                    revision + 1,
                    Some(index.clone()),
                    SelectionAction::Rebase,
                ))
                .unwrap()
                .unwrap();
            assert_eq!(snapshot.bits(), &[expected], "streaming: {streaming}");
        }
    }
}

#[test]
fn invert_all_archive_entries_preserves_marks_outside_the_visible_view() {
    let rows = entries();
    let visible = [
        rows[0].clone(),
        rows[1].clone(),
        rows[3].clone(),
        rows[5].clone(),
    ];
    let view = DirectoryIndex::from_entries(&visible);
    let selected = [rows[0].id, rows[2].id, rows[3].id].into_iter().collect();
    let result = crate::archive_selection::apply(
        &rows,
        &view,
        &selected,
        ArchiveSelectionAction::Bulk(SelectionOperation::InvertAll),
    )
    .unwrap();
    assert_eq!(
        *result.selected,
        [rows[1].id, rows[2].id, rows[5].id].into_iter().collect()
    );
}

#[test]
fn select_files_only_replaces_archive_selection_with_visible_files() {
    let rows = entries();
    let visible = [
        rows[0].clone(),
        rows[1].clone(),
        rows[3].clone(),
        rows[5].clone(),
    ];
    let view = DirectoryIndex::from_entries(&visible);
    let all = crate::archive_selection::apply(
        &rows,
        &view,
        &HashSet::new(),
        ArchiveSelectionAction::Bulk(SelectionOperation::All),
    )
    .unwrap();
    assert_eq!(
        *all.selected,
        [rows[0].id, rows[1].id, rows[3].id, rows[5].id]
            .into_iter()
            .collect()
    );
    let mut selected = (*all.selected).clone();
    selected.insert(rows[2].id); // Previously selected file now hidden by the filter.
    let files = crate::archive_selection::apply(
        &rows,
        &view,
        &selected,
        ArchiveSelectionAction::Bulk(SelectionOperation::FilesOnly),
    )
    .unwrap();
    assert_eq!(
        *files.selected,
        [rows[3].id, rows[5].id].into_iter().collect()
    );
}

#[test]
fn select_files_only_masks_partial_bitset_words() {
    let rows: Vec<_> = (0..66)
        .map(|i| {
            Entry::new(
                EntryId(i),
                format!("entry-{i}").into(),
                if i % 2 == 0 {
                    EntryKind::DirectoryLink
                } else {
                    EntryKind::File
                },
            )
        })
        .collect();
    let view = Arc::new(DirectoryIndex::from_entries(&rows));
    let mut state = State::default();
    state
        .apply(&request(
            1,
            Some(view.clone()),
            SelectionAction::Bulk(SelectionOperation::All),
        ))
        .unwrap();
    let (selected, _) = state
        .apply(&request(
            2,
            Some(view),
            SelectionAction::Bulk(SelectionOperation::FilesOnly),
        ))
        .unwrap()
        .unwrap();
    assert_eq!(selected.bits(), &[0xaaaa_aaaa_aaaa_aaaa, 2]);
    assert_eq!(selected.len(), 33);
}

#[test]
fn invert_files_in_archive_preserves_folders_and_respects_filtered_view() {
    let rows = entries();
    // The unselected file and directory link are hidden by the panel filter.
    let visible = [
        rows[0].clone(),
        rows[3].clone(),
        rows[4].clone(),
        rows[5].clone(),
    ];
    let index = DirectoryIndex::from_entries(&visible);
    let selected = [rows[0].id, rows[2].id, rows[3].id].into_iter().collect();
    let result = crate::archive_selection::apply(
        &rows,
        &index,
        &selected,
        ArchiveSelectionAction::Bulk(SelectionOperation::Invert),
    )
    .unwrap();
    assert_eq!(
        *result.selected,
        [rows[0].id, rows[2].id, rows[4].id, rows[5].id]
            .into_iter()
            .collect()
    );
}

#[test]
fn clear_files_only_retains_streaming_folder_exceptions_and_unknown_row_defaults() {
    let rows = entries();
    let index = Arc::new(DirectoryIndex::from_entries(&rows));
    let mut state = State::default();
    state
        .apply(&request(
            1,
            None,
            SelectionAction::Bulk(SelectionOperation::All),
        ))
        .unwrap();
    for (revision, row, directory) in [(2, 0, true), (3, 3, false)] {
        let mut toggle = request(
            revision,
            None,
            SelectionAction::Toggle(rows[row].path.clone()),
        );
        toggle.toggle_directory = directory;
        state.apply(&toggle).unwrap();
    }
    let (partial, defaults) = state
        .apply(&request(
            4,
            None,
            SelectionAction::Bulk(SelectionOperation::ClearFilesOnly),
        ))
        .unwrap()
        .unwrap();
    assert!(
        !partial.contains_path(&rows[0].path),
        "a toggled-off folder must remain off"
    );
    assert!(!partial.contains_path(&rows[3].path));
    assert_eq!(
        defaults,
        SelectionDefaults {
            files: false,
            directories: true
        }
    );
    let (complete, _) = state
        .apply(&request(5, Some(index), SelectionAction::Rebase))
        .unwrap()
        .unwrap();
    assert_eq!(
        complete.bits(),
        &[0b000010],
        "retain directory-link default without restoring cleared file exceptions"
    );
}

#[test]
fn clear_files_only_preserves_known_directory_links_across_partial_words() {
    let rows: Vec<_> = (0..66)
        .map(|i| {
            Entry::new(
                EntryId(i),
                format!("entry-{i}").into(),
                if i % 2 == 0 {
                    EntryKind::DirectoryLink
                } else {
                    EntryKind::File
                },
            )
        })
        .collect();
    let view = Arc::new(DirectoryIndex::from_entries(&rows));
    for streaming in [false, true] {
        let mut state = State::default();
        state
            .apply(&request(
                1,
                Some(view.clone()),
                SelectionAction::Bulk(SelectionOperation::All),
            ))
            .unwrap();
        state
            .apply(&request(
                2,
                (!streaming).then(|| view.clone()),
                SelectionAction::Bulk(SelectionOperation::ClearFilesOnly),
            ))
            .unwrap();
        let (selected, defaults) = state
            .apply(&request(3, Some(view.clone()), SelectionAction::Rebase))
            .unwrap()
            .unwrap();
        assert_eq!(selected.bits(), &[0x5555_5555_5555_5555, 1]);
        assert_eq!(selected.len(), 33);
        assert_eq!(selected.folder_count(), 33);
        assert_eq!(defaults, SelectionDefaults::NONE);
    }
}
