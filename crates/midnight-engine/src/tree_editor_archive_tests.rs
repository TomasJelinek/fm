//! An archive file can have a File row but never grants native editor authority.
use super::*;
use midnight_core::TreeEditorRequest;

#[test]
fn tree_editor_capture_rejects_readable_regular_archive_file() {
    let directory = tempfile::tempdir().unwrap();
    let mut file = archive_node(4, 1, b"editable-looking.txt");
    file.kind = EntryKind::File;
    file.readable = true;
    let mount = Arc::new(ArchiveFixture {
        rows: HashMap::from([(1, vec![file.clone()])]),
        invalid: AtomicBool::new(false),
    });
    let location = ArchiveLocation {
        mount: Some(mount),
        address: ArchiveAddress {
            provider: "test.archive".into(),
            format: None,
            container: directory.path().join("container.zip"),
            source_version: vec![3, 8],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, "container.zip".into())],
    };
    let entries: Arc<[Entry]> = Arc::from([Entry::from_archive(file)]);
    let root = TreeRootSnapshot {
        pane: PaneId::Right,
        navigation: NavigationId(30),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let opened = captured_archive_root(&location, &root.entries, &Cancellation::default()).unwrap();
    let mut session = Session::new(
        root,
        opened,
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let view = session.publication().unwrap().view;
    assert!(session.commit(&view));
    assert_eq!(view.rows[0].entry.kind, EntryKind::File);
    let request = TreeEditorRequest {
        row: view.rows[0].id,
        view,
        request: TreeRequestId(1),
        cancel: Cancellation::default(),
    };
    let capacity = Arc::new(Semaphore::new(1));
    assert!(matches!(
        session.capture_editor(&request, &capacity, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
    assert_eq!(capacity.available_permits(), 1);
}
