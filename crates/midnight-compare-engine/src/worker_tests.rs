use super::*;
use midnight_core::{DirectoryIndex, Entry, EntryId, SelectionSnapshot};
use std::{path::PathBuf, sync::Arc};

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
    let paths = [
        directory.path().join("left.txt"),
        directory.path().join("right.txt"),
    ];
    std::fs::write(&paths[0], "český 🐈\nold\n").unwrap();
    std::fs::write(&paths[1], "český 🐈\nnew\n").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = crate::Service::new(runtime.handle().clone());
    let session = service
        .open(
            FileCompareLaunch {
                sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
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
