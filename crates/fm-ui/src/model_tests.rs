use super::*;
use std::{path::PathBuf, sync::Arc};

#[test]
fn completed_partial_size_is_distinct_from_other_folders_still_queued() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    {
        let mut p = presenter.borrow_mut();
        p.panes[0].entries = ["drivers", "other"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                Entry::new(EntryId(index as u64), root.join(name), EntryKind::Directory)
            })
            .collect::<Vec<_>>()
            .into();
        p.panes[0].size_scan = Some(FolderSizeScanId(1));
        p.panes[0].size_scan_attempted = true;
        p.panes[0].folder_sizes.insert(
            root.join("drivers"),
            FolderSize {
                bytes: 175_438_846,
                files: 769,
                errors: 1,
                finished: true,
                ..Default::default()
            },
        );
    }
    let model = Rows::new(presenter, PaneId::Left);
    let finished = model.row_data(0).unwrap();
    assert_eq!(
        finished.size_state, 2,
        "one inaccessible folder does not leave the scan running"
    );
    assert_eq!(finished.size.as_str(), "≈ 167.3 MB");
    assert_eq!(
        model.row_data(1).unwrap().size_state,
        1,
        "other roots can still be queued"
    );
}

#[test]
fn unfinished_directory_sizes_are_marked_until_their_own_clean_completion() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let model = Rows::new(presenter.clone(), PaneId::Left);
    {
        let mut p = presenter.borrow_mut();
        p.panes[0].entries = (1..=3)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    root.join(format!("folder{id}")),
                    EntryKind::Directory,
                )
            })
            .collect::<Vec<_>>()
            .into();
        p.panes[0].size_scan = Some(FolderSizeScanId(1));
        p.panes[0].size_scan_attempted = true;
        p.panes[0].folder_sizes.insert(
            root.join("folder1"),
            FolderSize {
                bytes: 1024,
                finished: false,
                ..Default::default()
            },
        );
        p.panes[0].folder_sizes.insert(
            root.join("folder2"),
            FolderSize {
                bytes: 2048,
                finished: true,
                ..Default::default()
            },
        );
    }
    assert_eq!(model.row_data(0).unwrap().size.as_str(), "≈ 1.0 KB");
    assert_eq!(model.row_data(0).unwrap().size_state, 1);
    assert!(model.row_data(0).unwrap().size_hint.contains("Calculating"));
    assert_eq!(model.row_data(1).unwrap().size.as_str(), "2.0 KB");
    assert_eq!(model.row_data(1).unwrap().size_state, 0);
    assert_eq!(model.row_data(2).unwrap().size.as_str(), "…");
    assert_eq!(model.row_data(2).unwrap().size_state, 1);
    assert!(model.row_data(2).unwrap().size_hint.contains("Queued"));
    presenter.borrow_mut().panes[0].size_scan = None;
    assert_eq!(model.row_data(0).unwrap().size.as_str(), "≈ 1.0 KB");
    assert_eq!(model.row_data(0).unwrap().size_state, 2);
    assert!(model.row_data(0).unwrap().size_hint.contains("cancelled"));
    assert_eq!(model.row_data(2).unwrap().size.as_str(), "—");
    assert_eq!(model.row_data(2).unwrap().size_state, 2);
    presenter.borrow_mut().panes[0]
        .folder_sizes
        .get_mut(&root.join("folder1"))
        .unwrap()
        .finished = true;
    assert_eq!(model.row_data(0).unwrap().size_state, 0);
    assert_eq!(model.row_data(0).unwrap().size.as_str(), "1.0 KB");
    presenter.borrow_mut().panes[0]
        .folder_sizes
        .get_mut(&root.join("folder1"))
        .unwrap()
        .errors = 1;
    assert_eq!(model.row_data(0).unwrap().size_state, 2);
    assert_eq!(model.row_data(0).unwrap().size.as_str(), "≈ 1.0 KB");
    assert!(model.row_data(0).unwrap().size_hint.contains("partial"));
    assert!(model.row_data(0).unwrap().size_hint.contains("1 errors"));
}

#[test]
fn parent_is_a_lazy_navigation_row_outside_the_real_entries() {
    let path = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("home");
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
    let model = Rows::new(presenter.clone(), PaneId::Left);
    assert_eq!(
        model.row_count(),
        1,
        "an empty non-root directory must still offer Up"
    );
    let parent = model.row_data(0).unwrap();
    assert_eq!(parent.name.as_str(), "..");
    assert!(!parent.selected);
    assert!(
        parent.cursor,
        "the only navigation row must accept Enter in an empty folder"
    );
    assert!(presenter.borrow().panes[0].rows().is_empty());
    presenter.borrow_mut().panes[0].path = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    assert_eq!(model.row_count(), 0);
}

#[test]
fn pointer_identity_distinguishes_parent_full_width_ids_and_refresh_generations() {
    let path = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("home");
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
    let model = Rows::new(presenter.clone(), PaneId::Left);
    presenter.borrow_mut().panes[0].entries = vec![
        Entry::new(EntryId(0), path.join("zero"), EntryKind::File),
        Entry::new(EntryId(u64::MAX), path.join("last"), EntryKind::File),
    ]
    .into();
    let parent = model.row_data(0).unwrap().identity;
    let first = model.row_data(1).unwrap().identity;
    let last = model.row_data(2).unwrap().identity;
    assert_ne!(
        parent, first,
        "EntryId zero is a real entry, not the parent sentinel"
    );
    assert_ne!(
        first, last,
        "Slint float/int conversion must never truncate the raw id"
    );
    presenter.borrow_mut().panes[0].generation += 1;
    assert_ne!(model.row_data(1).unwrap().identity, first);
    assert_ne!(model.row_data(0).unwrap().identity, parent);
}

#[test]
fn pointer_identity_changes_when_refresh_commits_recycled_entry_ids() {
    let path = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("home");
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
    let model = Rows::new(presenter.clone(), PaneId::Left);
    let finish = |navigation, name| {
        let entries = vec![Entry::new(EntryId(7), path.join(name), EntryKind::File)];
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            entries: entries.into(),
            index,
        });
    };
    presenter
        .borrow_mut()
        .navigate(PaneId::Left, path.clone(), None, &|_| true);
    finish(NavigationId(1), "old");
    let old = model.row_data(1).unwrap().identity;
    presenter
        .borrow_mut()
        .navigate(PaneId::Left, path.clone(), None, &|_| true);
    let during_refresh = model.row_data(1).unwrap().identity;
    assert_ne!(
        old, during_refresh,
        "refresh start invalidates a prior press"
    );
    finish(NavigationId(2), "replacement");
    assert_ne!(
        model.row_data(1).unwrap().identity,
        during_refresh,
        "a replacement must not inherit a press made on committed rows during refresh"
    );
}

#[test]
fn command_selection_replacement_preserves_utf8_bytes_and_trailing_text() {
    let text = "echo 文档 Cafe\u{301} 👩🏽‍💻 tail";
    let selected = "文档 Cafe\u{301} 👩🏽‍💻";
    let end = 5 + selected.len();
    let (updated, offset) = replace_command_selection(text, end as i32, 5, "'Žlutý'");
    assert_eq!(updated, "echo 'Žlutý' tail");
    assert_eq!(offset, 14);
    let (updated, offset) = replace_command_selection("文档 tail", 1, 5, "'x'");
    assert_eq!(updated, "'x'档 tail");
    assert_eq!(offset, 3);
    assert_eq!(
        replace_command_selection("hi", -2, 90, "'x'"),
        ("'x'".into(), 3)
    );
}

#[test]
fn multilingual_lazy_names_preserve_exact_unicode_sequences() {
    let names = [
        "Žluťoučký kůň.txt",
        "東京の資料.txt",
        "ملفّ عربي.txt",
        "קובץ עברית.txt",
        "👩🏽‍💻-📁-🏳️‍🌈.txt",
        "Cafe\u{301}.txt",
    ];
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .navigate(PaneId::Left, "left".into(), None, &|_| true);
    presenter.borrow_mut().apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries: names
            .iter()
            .enumerate()
            .map(|(i, name)| Entry::new(EntryId(i as u64), PathBuf::from(name), EntryKind::File))
            .collect(),
    });
    let model = Rows::new(presenter, PaneId::Left);
    assert_eq!(model.row_count(), names.len());
    for (index, expected) in names.iter().enumerate() {
        assert_eq!(
            model.row_data(index).unwrap().name.as_bytes(),
            expected.as_bytes()
        );
    }
    assert_ne!(model.row_data(5).unwrap().name.as_str(), "Café.txt");
}

#[test]
fn lazy_rows_read_current_shared_state_and_only_the_requested_pane() {
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    let model = Rows::new(presenter.clone(), PaneId::Left);
    presenter
        .borrow_mut()
        .navigate(PaneId::Left, "left".into(), None, &|_| true);
    presenter.borrow_mut().apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries: (0..1000)
            .map(|i| {
                Entry::new(
                    EntryId(i),
                    PathBuf::from(format!("item-{i}.txt")),
                    EntryKind::File,
                )
            })
            .collect(),
    });
    assert_eq!(model.row_count(), 1000);
    assert!(model.row_data(1000).is_none());
    assert_eq!(model.row_data(999).unwrap().name.as_str(), "item-999.txt");
    assert_eq!(model.row_data(999).unwrap().size.as_str(), "—");
    presenter.borrow_mut().apply(Event::MetadataBatch {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries: vec![(
            EntryId(999),
            EntryMetadata {
                size: Some(2048),
                modified: None,
            },
        )],
    });
    assert_eq!(model.row_data(999).unwrap().size.as_str(), "2.0 KB");
    let right = Rows::new(presenter, PaneId::Right);
    assert_eq!(right.row_count(), 0);
}

#[test]
fn final_snapshot_rows_show_identity_selection_after_reordering() {
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    let model = Rows::new(presenter.clone(), PaneId::Left);
    presenter
        .borrow_mut()
        .navigate(PaneId::Left, "left".into(), None, &|_| true);
    let entries: Arc<[Entry]> = vec![
        Entry::new(EntryId(2), "folder".into(), EntryKind::Directory),
        Entry::new(EntryId(1), "readme.md".into(), EntryKind::File),
    ]
    .into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries,
        index,
    });
    presenter.borrow_mut().panes[0].cursor = Some(EntryId(1));
    {
        let mut state = presenter.borrow_mut();
        state.panes[0].selected = Arc::new(
            SelectionSnapshot::from_bits(state.panes[0].index.clone(), vec![0b10]).unwrap(),
        );
    }
    assert_eq!(model.row_count(), 2);
    assert_eq!(model.row_data(0).unwrap().size.as_str(), "<DIR>");
    let row = model.row_data(1).unwrap();
    assert!(row.cursor && row.selected);
    assert_eq!(row.kind.as_str(), "MD");
}
