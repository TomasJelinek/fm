use super::*;
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, FileCompareSource, SelectionSnapshot,
};
use std::{path::PathBuf, sync::Arc};

#[cfg(unix)]
#[path = "worker_raw_unix_tests.rs"]
mod raw_unix;
#[cfg(windows)]
#[path = "worker_raw_windows_tests.rs"]
mod raw_windows;

fn counterpart(directory: &str, names: &[&str]) -> FileCompareSource {
    let entries: Arc<[Entry]> = names
        .iter()
        .enumerate()
        .map(|(id, name)| {
            Entry::new(
                EntryId(id as u64),
                PathBuf::from(directory).join(name),
                EntryKind::File,
            )
        })
        .collect::<Vec<_>>()
        .into();
    FileCompareSource::SameName {
        directory: directory.into(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    }
}

#[test]
fn same_name_finds_exact_unicode_in_either_opposite_panel_without_changing_order() {
    let name = "Žluťoučký 世界 🐈.txt";
    for active in 0..2 {
        let mut sources = vec![
            counterpart("left", &["unrelated.txt", name]),
            counterpart("right", &[name, "different.txt"]),
        ];
        sources[active] = FileCompareSource::Focused(
            PathBuf::from(if active == 0 { "left" } else { "right" }).join(name),
        );
        let launch = FileCompareLaunch {
            sources,
            disabled_plugins: Arc::from([]),
        };
        assert_eq!(
            resolve(&launch, &AtomicBool::new(false)).unwrap(),
            [
                PathBuf::from("left").join(name),
                PathBuf::from("right").join(name)
            ]
        );
    }
}

#[test]
fn same_name_uses_the_single_selected_file_and_its_captured_snapshot() {
    let entries: Arc<[Entry]> = vec![
        Entry::new(EntryId(40), "left/focused.txt".into(), EntryKind::File),
        Entry::new(EntryId(90), "left/selected.txt".into(), EntryKind::File),
    ]
    .into();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), vec![2]).unwrap());
    let launch = FileCompareLaunch {
        sources: vec![
            FileCompareSource::Selected {
                entries,
                view,
                selection,
            },
            counterpart("right", &["focused.txt", "selected.txt"]),
        ],
        disabled_plugins: Arc::from([]),
    };
    assert_eq!(
        resolve(&launch, &AtomicBool::new(false)).unwrap(),
        [
            PathBuf::from("left/selected.txt"),
            PathBuf::from("right/selected.txt")
        ]
    );
    assert!(resolve(&launch, &AtomicBool::new(true)).is_err());
}

#[test]
fn same_name_rejects_missing_ambiguous_hidden_or_nonidentical_native_names() {
    for names in [
        vec!["different.txt"],
        vec!["same.txt", "same.txt"],
        vec!["SAME.TXT"],
    ] {
        let launch = FileCompareLaunch {
            sources: vec![
                FileCompareSource::Focused("left/same.txt".into()),
                counterpart("right", &names),
            ],
            disabled_plugins: Arc::from([]),
        };
        assert!(resolve(&launch, &AtomicBool::new(false)).is_err());
    }
    let mut hidden = counterpart("right", &["same.txt"]);
    if let FileCompareSource::SameName { view, .. } = &mut hidden {
        *view = Arc::new(DirectoryIndex::default());
    }
    let launch = FileCompareLaunch {
        sources: vec![FileCompareSource::Focused("left/same.txt".into()), hidden],
        disabled_plugins: Arc::from([]),
    };
    assert!(resolve(&launch, &AtomicBool::new(false)).is_err());
    let launch = FileCompareLaunch {
        sources: vec![
            FileCompareSource::Focused("left/é.txt".into()),
            counterpart("right", &["e\u{301}.txt"]),
        ],
        disabled_plugins: Arc::from([]),
    };
    assert!(resolve(&launch, &AtomicBool::new(false)).is_err());
}

#[test]
fn same_name_rejects_nonfiles_and_mismatched_folder_or_index() {
    for kind in [EntryKind::Directory, EntryKind::Symlink, EntryKind::Other] {
        let mut source = counterpart("right", &["same.txt"]);
        if let FileCompareSource::SameName { entries, .. } = &mut source {
            Arc::make_mut(entries)[0].kind = kind;
        }
        let launch = FileCompareLaunch {
            sources: vec![FileCompareSource::Focused("left/same.txt".into()), source],
            disabled_plugins: Arc::from([]),
        };
        assert!(resolve(&launch, &AtomicBool::new(false)).is_err());
    }
    for invalid_folder in [false, true] {
        let mut source = counterpart("right", &["same.txt"]);
        if let FileCompareSource::SameName {
            directory, view, ..
        } = &mut source
        {
            if invalid_folder {
                *directory = "elsewhere".into();
            } else {
                Arc::make_mut(view).by_path.clear();
            }
        }
        let launch = FileCompareLaunch {
            sources: vec![FileCompareSource::Focused("left/same.txt".into()), source],
            disabled_plugins: Arc::from([]),
        };
        assert!(resolve(&launch, &AtomicBool::new(false)).is_err());
    }
}

#[test]
fn same_name_preserves_long_paths_and_ignores_display_labels() {
    let directory = format!("right/{}", "long-directory/".repeat(30));
    let mut source = counterpart(&directory, &["same.txt"]);
    if let FileCompareSource::SameName { entries, .. } = &mut source {
        Arc::make_mut(entries)[0].display_name = "different display caption".into();
    }
    let launch = FileCompareLaunch {
        sources: vec![FileCompareSource::Focused("left/same.txt".into()), source],
        disabled_plugins: Arc::from([]),
    };
    assert_eq!(
        resolve(&launch, &AtomicBool::new(false)).unwrap()[1],
        PathBuf::from(directory).join("same.txt")
    );
}

#[test]
fn selected_pair_preserves_display_order_and_rejects_directories() {
    let entries: Arc<[Entry]> = vec![
        Entry::new(EntryId(0), PathBuf::from("z/český.txt"), EntryKind::File),
        Entry::new(EntryId(1), PathBuf::from("z/世界.txt"), EntryKind::File),
    ]
    .into();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), vec![3]).unwrap());
    let launch = FileCompareLaunch {
        sources: vec![FileCompareSource::Selected {
            entries: entries.clone(),
            view,
            selection,
        }],
        disabled_plugins: Arc::from([]),
    };
    assert_eq!(
        resolve(&launch, &AtomicBool::new(false)).unwrap(),
        [entries[0].path.clone(), entries[1].path.clone()]
    );
    assert!(resolve(&launch, &AtomicBool::new(true)).is_err());
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(0),
        PathBuf::from("folder"),
        EntryKind::Directory,
    )]
    .into();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap());
    let launch = FileCompareLaunch {
        sources: vec![FileCompareSource::Selected {
            entries,
            view,
            selection,
        }],
        disabled_plugins: Arc::from([]),
    };
    assert!(resolve(&launch, &AtomicBool::new(false)).is_err());
}

#[test]
fn service_reads_and_renders_real_unicode_sources_off_thread() {
    let directory = tempfile::tempdir().unwrap();
    let name = "Žluťoučký 世界 🐈.txt";
    let paths = [
        directory.path().join("left").join(name),
        directory.path().join("right").join(name),
    ];
    for path in &paths {
        std::fs::create_dir(path.parent().unwrap()).unwrap();
    }
    std::fs::write(&paths[0], "český 🐈\nold\n").unwrap();
    std::fs::write(&paths[1], "český 🐈\nnew\n").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = crate::Service::new(runtime.handle().clone());
    let session = service
        .open(
            FileCompareLaunch {
                sources: vec![
                    FileCompareSource::Focused(paths[0].clone()),
                    counterpart(paths[1].parent().unwrap().to_str().unwrap(), &[name]),
                ],
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [30, 30, 30, 255],
                selection: [50, 80, 120, 255],
                accent: [90, 150, 220, 255],
            },
            ViewerGeometry {
                width: 640,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        match session.mailbox.take() {
            Some(Output::Frame { frame, .. }) => {
                assert!(frame.accessible_text.contains("český 🐈"));
                assert!(
                    frame.accessible_text.contains("old") && frame.accessible_text.contains("new")
                );
                assert!(frame.status.contains("1 difference"));
                assert_eq!(frame.pixels.len(), 640 * 200 * 4);
                assert!(!frame.image);
                break;
            }
            Some(Output::Status(error)) => assert!(!error.contains("Cannot"), "{error}"),
            Some(Output::ChooseFiles(choice)) => panic!("Unexpected file choice: {choice:?}"),
            None => {}
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Comparison frame timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    session.close();
}

#[test]
fn unresolved_pair_keeps_session_available_for_manual_file_selection() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = crate::Service::new(runtime.handle().clone());
    let session = service
        .open(
            FileCompareLaunch {
                sources: Vec::new(),
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [30, 30, 30, 255],
                selection: [50, 80, 120, 255],
                accent: [90, 150, 220, 255],
            },
            ViewerGeometry {
                width: 640,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    // A missing pair must wait for the user instead of terminating its receiver.
    let start = std::time::Instant::now();
    loop {
        if let Some(Output::ChooseFiles(choice)) = session.mailbox.take() {
            assert!(choice.paths.iter().all(|path| path.as_os_str().is_empty()));
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "No file choice was offered"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        session.send(Input::Resize(
            ViewerGeometry {
                width: 640,
                height: 200,
                scale: 1.0
            },
            1,
        )),
        "Missing files should open the file chooser, not terminate the session"
    );
    session.close();
}

#[test]
fn root_same_name_uses_only_visible_native_aliases_and_preserves_opposite_file() {
    for visible in [false, true] {
        let FileCompareSource::SameName {
            directory,
            entries,
            view,
        } = counterpart("right", &["文档.txt", "other"])
        else {
            unreachable!()
        };
        let launch = FileCompareLaunch {
            disabled_plugins: Arc::from([]),
            sources: vec![
                FileCompareSource::Focused("left/文档.txt".into()),
                FileCompareSource::RootSameName {
                    directory,
                    entries,
                    view,
                    visible: if visible {
                        Arc::from([EntryId(0)])
                    } else {
                        Arc::from([EntryId(1)])
                    },
                },
            ],
        };
        let result = resolve(&launch, &AtomicBool::new(false));
        if visible {
            assert_eq!(
                result.unwrap(),
                [
                    PathBuf::from("left/文档.txt"),
                    PathBuf::from("right/文档.txt")
                ]
            );
        } else {
            let choice = result.unwrap_err();
            assert_eq!(choice.paths[0], PathBuf::from("left/文档.txt"));
            assert!(choice.paths[1].as_os_str().is_empty());
        }
    }
}
