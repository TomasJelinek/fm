use super::*;
use std::{path::PathBuf, sync::Arc};

mod history_models;
mod performance;
#[cfg(unix)]
mod unix;

#[test]
fn locale_snapshot_reaches_rows_details_and_copied_values_without_splitting_spaces() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let mut entry = Entry::new(EntryId(1), root.join("localized.txt"), EntryKind::File);
    entry.metadata.size = Some(u64::MAX);
    entry.metadata.localized_modified = Some(LocalizedDateTime {
        date: "5. listopadu 2026".into(),
        time: "1:07 PM".into(),
    });
    {
        let mut state = presenter.borrow_mut();
        state.number_locale = NumberLocale {
            decimal_separator: ",".into(),
            group_separator: "\u{a0}".into(),
            grouping: vec![3],
            repeat_last_group: true,
        };
        state.preferences.values.size_display = SizeDisplay::Bytes;
        state.panes[0].entries = vec![entry].into();
        state.panes[0].index = Arc::new(DirectoryIndex::from_entries(&state.panes[0].entries));
        state.panes[0].cursor = Some(EntryId(1));
    }
    let row = Rows::new(presenter.clone(), PaneId::Left)
        .row_data(0)
        .unwrap();
    assert_eq!(
        row.size.as_str(),
        "18\u{a0}446\u{a0}744\u{a0}073\u{a0}709\u{a0}551\u{a0}615"
    );
    assert_eq!(row.modified.as_str(), "5. listopadu 2026 1:07 PM");
    let fields = presenter.borrow().panel_details(PaneId::Left);
    for (label, expected) in [
        ("Bytes", row.size.as_str()),
        ("Date", "5. listopadu 2026"),
        ("Time", "1:07 PM"),
    ] {
        let field = fields.iter().find(|field| field.label == label).unwrap();
        assert_eq!(field.value, expected);
        assert!(presenter.borrow_mut().copy_panel_detail(
            PaneId::Left,
            &field.identity,
            &|command| {
                assert!(matches!(command, Command::CopyText { text, .. } if text == expected));
                true
            }
        ));
    }
}

#[test]
fn modified_dates_outside_the_portable_calendar_render_unknown_but_keep_raw_time() {
    use std::time::{Duration, UNIX_EPOCH};
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let time = UNIX_EPOCH
        .checked_add(Duration::from_secs(500_000_000_000))
        .unwrap();
    let mut entry = Entry::new(EntryId(1), root.join("far future date"), EntryKind::File);
    entry.metadata.modified = Some(time);
    presenter.borrow_mut().panes[0].entries = vec![entry].into();
    let row = Rows::new(presenter.clone(), PaneId::Left)
        .row_data(0)
        .unwrap();
    assert_eq!(row.modified.as_str(), "—");
    assert_eq!(
        presenter.borrow().panes[0].entries[0].metadata.modified,
        Some(time)
    );
}

#[test]
fn directory_links_have_folder_appearance_without_queued_or_target_sizes() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let mut entry = Entry::new(
        EntryId(1),
        root.join("folder link"),
        EntryKind::DirectoryLink,
    );
    entry.metadata.size = Some(123);
    entry.metadata.compression = CompressionStatus::Compressed;
    presenter.borrow_mut().panes[0].entries = vec![entry].into();
    presenter.borrow_mut().panes[0].size_scan = Some(FolderSizeScanId(7));
    let row = Rows::new(presenter, PaneId::Left).row_data(0).unwrap();
    assert!(row.directory);
    assert_eq!(row.kind.as_str(), "Folder link");
    assert_eq!(row.size.as_str(), "—");
    assert_eq!(row.size_state, 0);
    assert!(
        row.compressed,
        "compression may coexist with a native link-overlay icon"
    );
}

#[test]
fn compression_badge_uses_only_reported_metadata_and_preserves_logical_lengths() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let mut entries: Vec<_> = ["compressed 文档.txt", "ordinary.zip", "unknown.zip"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut entry = Entry::new(EntryId(index as u64), root.join(name), EntryKind::File);
            entry.metadata.size = Some(123);
            entry
        })
        .collect();
    {
        let mut p = presenter.borrow_mut();
        assert!(p.navigate(PaneId::Left, root.clone(), None, &|_| true));
        p.apply(Event::EntriesBatch {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            entries: entries.clone(),
        });
    }
    let model = Rows::new(presenter.clone(), PaneId::Left);
    assert!((0..3).all(|row| !model.row_data(row).unwrap().compressed));
    for (entry, compression) in entries.iter_mut().zip([
        CompressionStatus::Compressed,
        CompressionStatus::Uncompressed,
        CompressionStatus::Unknown,
    ]) {
        entry.metadata.compression = compression;
    }
    presenter.borrow_mut().apply(Event::MetadataBatch {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries: entries
            .iter()
            .map(|entry| (entry.id, entry.metadata.clone()))
            .collect(),
    });
    assert!(model.row_data(0).unwrap().compressed);
    assert_eq!(
        model.row_data(0).unwrap().name.as_str(),
        "compressed 文档.txt"
    );
    for row in 0..3 {
        assert_eq!(model.row_data(row).unwrap().size.as_str(), "123 B");
        if row > 0 {
            assert!(!model.row_data(row).unwrap().compressed);
        }
    }
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries: entries.into(),
    });
    assert!(model.row_data(0).unwrap().compressed);
    assert_eq!(model.row_data(0).unwrap().size.as_str(), "123 B");
}

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
                ..Default::default()
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

#[test]
fn accepted_toggle_has_first_update_feedback_before_backend_acknowledgement() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR);
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        root.join("delayed.txt"),
        EntryKind::File,
    )]
    .into();
    {
        let mut p = presenter.borrow_mut();
        p.navigate(PaneId::Left, root, None, &|_| true);
        p.apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        p.select_row(PaneId::Left, 0, true, &|_| true);
        assert!(
            p.panes[0].selected.is_empty(),
            "backend snapshot remains authoritative"
        );
    }
    let rows = Rows::new(presenter.clone(), PaneId::Left);
    assert!(
        rows.row_data(0).unwrap().selected,
        "accepted gesture needs immediate visible feedback"
    );
    presenter
        .borrow_mut()
        .select_row(PaneId::Left, 0, true, &|_| true);
    assert!(
        !rows.row_data(0).unwrap().selected,
        "second pending toggle composes with first"
    );
    presenter
        .borrow_mut()
        .select_row(PaneId::Left, 0, true, &|_| false);
    assert!(
        !rows.row_data(0).unwrap().selected,
        "refused gesture cannot alter feedback"
    );
}
