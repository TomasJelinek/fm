use super::*;

fn fixture() -> (FileListCapture, Snapshots, SharedSizes) {
    let root = std::env::temp_dir().join("file-list-sizes");
    let mut entries: Vec<_> = ["complete", "partial", "errors", "link", "file"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(
                EntryId(index as u64 + 1),
                root.join(name),
                match index {
                    3 => EntryKind::DirectoryLink,
                    4 => EntryKind::File,
                    _ => EntryKind::Directory,
                },
            )
        })
        .collect();
    entries[4].metadata.size = Some(55);
    let entries: Arc<[Entry]> = entries.into();
    let sizes: SharedSizes = Arc::new(Mutex::new(
        entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                (
                    entry.path.clone(),
                    FolderSize {
                        bytes: 10 + index as u64,
                        finished: index != 1,
                        errors: u64::from(index == 2),
                        ..Default::default()
                    },
                )
            })
            .collect(),
    ));
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    snapshots.lock().unwrap()[0].visible = Some(crate::CommittedSnapshot {
        navigation: NavigationId(7),
        path: root,
        entries: entries.clone(),
        sizes: sizes.clone(),
    });
    let capture = FileListCapture {
        directory: String::new(),
        source: FileListSource::Native {
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            selection: Arc::default(),
            fallback: Some(EntryId(1)),
        },
    };
    (capture, snapshots, sizes)
}

fn entries(capture: &FileListCapture) -> Vec<&Entry> {
    let FileListSource::Native { entries, .. } = &capture.source else {
        panic!()
    };
    entries.iter().collect()
}

#[test]
fn completed_native_sizes_are_frozen_once_and_survive_panel_navigation() {
    let (capture, snapshots, sizes) = fixture();
    let captured = Captured::bind(&capture, &snapshots);
    snapshots.lock().unwrap()[0] = Default::default();
    let entries = entries(&capture);
    let frozen = captured.freeze(&entries, &Cancellation::default()).unwrap();
    assert_eq!(frozen.directory_size(entries[0]), Some(10));
    assert_eq!(frozen.directory_size(entries[3]), Some(13));
    for index in [1, 2, 4] {
        assert_eq!(frozen.directory_size(entries[index]), None);
    }
    sizes
        .lock()
        .unwrap()
        .get_mut(&entries[0].path)
        .unwrap()
        .bytes = 100;
    let again = captured.freeze(&entries, &Cancellation::default()).unwrap();
    assert!(Arc::ptr_eq(&frozen, &again));
    assert_eq!(again.directory_size(entries[0]), Some(10));
}

#[test]
fn equal_content_new_allocation_and_non_native_panel_cannot_supply_sizes() {
    let (capture, snapshots, _) = fixture();
    let mut detached = capture.clone();
    let FileListSource::Native {
        entries: detached_entries,
        ..
    } = &mut detached.source
    else {
        panic!()
    };
    *detached_entries = detached_entries.to_vec().into();
    let frozen = Captured::bind(&detached, &snapshots)
        .freeze(&entries(&capture), &Cancellation::default())
        .unwrap();
    assert_eq!(frozen.directory_size(entries(&capture)[0]), None);
    snapshots.lock().unwrap()[0].archive_navigation = Some(NavigationId(8));
    let frozen = Captured::bind(&capture, &snapshots)
        .freeze(&entries(&capture), &Cancellation::default())
        .unwrap();
    assert_eq!(frozen.directory_size(entries(&capture)[0]), None);
}

#[test]
fn cancellation_does_not_publish_an_empty_cache_or_poison_the_next_preview() {
    let (capture, snapshots, _) = fixture();
    let captured = Captured::bind(&capture, &snapshots);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(captured.freeze(&entries(&capture), &cancel).is_err());
    let frozen = captured
        .freeze(&entries(&capture), &Cancellation::default())
        .unwrap();
    assert_eq!(frozen.directory_size(entries(&capture)[0]), Some(10));
}

#[test]
fn archive_sizes_use_opaque_mount_identity_and_never_size_links() {
    let id = VirtualEntryId {
        mount: 97,
        entry: 2,
    };
    let mut entry = Entry::from_archive(ArchiveNode {
        id,
        parent: 1,
        raw_name: Arc::from(b"folder".as_slice()),
        display_name: "folder".into(),
        kind: EntryKind::Directory,
        size: Some(999),
        readable: false,
        modified: None,
    });
    let sizes = Arc::new(ArchiveSizeSnapshot::new(
        97,
        3,
        HashMap::from([(
            id,
            FolderSize {
                bytes: 42,
                finished: true,
                ..Default::default()
            },
        )]),
    ));
    let captured = Captured {
        state: Mutex::new(State {
            archive: Some(sizes),
            ..Default::default()
        }),
    };
    let frozen = captured
        .freeze(&[&entry], &Cancellation::default())
        .unwrap();
    assert_eq!(frozen.directory_size(&entry), Some(42));
    entry.kind = EntryKind::DirectoryLink;
    assert_eq!(frozen.directory_size(&entry), None);
    entry.kind = EntryKind::Directory;
    entry.virtual_node.as_mut().unwrap().id.mount = 98;
    assert_eq!(frozen.directory_size(&entry), None);
}

#[test]
fn archive_binding_survives_root_retirement_but_rejects_replacement_mount_with_same_id() {
    #[derive(Debug)]
    struct Mount;
    impl ArchiveMount for Mount {
        fn id(&self) -> u64 {
            97
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            panic!("size binding must not enumerate")
        }
        fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
            panic!("size binding must not read")
        }
        fn validate(&self) -> Result<(), FsError> {
            Ok(())
        }
    }
    let source: Arc<dyn ArchiveMount> = Arc::new(Mount);
    let node = VirtualEntryId {
        mount: 97,
        entry: 2,
    };
    let entries: Arc<[Entry]> = vec![Entry::from_archive(ArchiveNode {
        id: node,
        parent: 1,
        raw_name: Arc::from(b"folder".as_slice()),
        display_name: "folder".into(),
        kind: EntryKind::Directory,
        size: None,
        readable: false,
        modified: None,
    })]
    .into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    let mut listing = ArchiveListing {
        location: ArchiveLocation {
            mount: Some(source.clone()),
            directory: 1,
            address: ArchiveAddress {
                provider: "fixture".into(),
                format: None,
                container: "fixture.zip".into(),
                source_version: vec![],
                components: vec![],
            },
            trail: vec![],
        },
        entries: entries.clone(),
        index: index.clone(),
    };
    let sizes = Arc::new(ArchiveSizeSnapshot::new(
        97,
        1,
        HashMap::from([(
            node,
            FolderSize {
                bytes: 42,
                finished: true,
                ..Default::default()
            },
        )]),
    ));
    let snapshots = crate::test_snapshots(Default::default());
    {
        let mut states = snapshots.lock().unwrap();
        states[0].remember_archive_tree_root(PaneId::Left, NavigationId(1), &listing);
        states[0].archive_sort_sizes = Some(crate::archive_sizes::published::Published::testing(
            &source, &sizes,
        ));
    }
    let capture = FileListCapture {
        directory: String::new(),
        source: FileListSource::Archive {
            entries: entries.clone(),
            view: index,
            selected: Arc::default(),
            fallback: Some(entries[0].id),
        },
    };
    let accepted = Captured::bind(&capture, &snapshots);
    listing.location.mount = Some(Arc::new(Mount));
    let old_root = snapshots.lock().unwrap()[0].remember_archive_tree_root(
        PaneId::Left,
        NavigationId(2),
        &listing,
    );
    std::thread::spawn(move || {
        drop(old_root);
        drop(source);
    })
    .join()
    .unwrap();
    drop(sizes);
    let replacement = Captured::bind(&capture, &snapshots);
    assert_eq!(
        accepted
            .freeze(&[&entries[0]], &Cancellation::default())
            .unwrap()
            .directory_size(&entries[0]),
        Some(42)
    );
    assert_eq!(
        replacement
            .freeze(&[&entries[0]], &Cancellation::default())
            .unwrap()
            .directory_size(&entries[0]),
        None,
        "same numeric id is not the accepted mount capability"
    );
}
