use super::*;
use crate::SnapshotState;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

#[path = "folder_presence_worker_tests.rs"]
mod worker;

#[derive(Default)]
struct Files {
    rows: HashMap<PathBuf, Vec<Entry>>,
    visited: Mutex<Vec<PathBuf>>,
    emitted: AtomicUsize,
    /// Storage of a path (Unknown when absent) and every classified path.
    kinds: Mutex<HashMap<PathBuf, StorageKind>>,
    classified: Mutex<Vec<PathBuf>>,
    /// Resolved device of a directory (unsupported when absent) and every query.
    devices: Mutex<HashMap<PathBuf, u64>>,
    identified: Mutex<Vec<PathBuf>>,
    /// The I/O gate whose state each probe records as exclusive or shared.
    gate: Mutex<Option<Arc<Gate>>>,
    exclusive: Mutex<Vec<(PathBuf, bool)>>,
}
fn isolated() -> Admissions {
    Admissions::new(Arc::new(Gate::new(1)), Arc::new(Gate::new(1)))
}
impl DirectoryService for Files {
    fn browse_directory_identity(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Option<SizeDirectoryIdentity>, FsError> {
        self.identified.lock().unwrap().push(path.into());
        let device = self.devices.lock().unwrap().get(path).copied();
        let device = device.ok_or_else(|| {
            FsError::from_io("identity", path, std::io::ErrorKind::Unsupported.into())
        })?;
        Ok(Some(SizeDirectoryIdentity {
            device,
            object: [0; 16],
        }))
    }
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.visited.lock().unwrap().push(path.into());
        if let Some(gate) = &*self.gate.lock().unwrap() {
            let exclusive = gate.snapshot().exclusive;
            self.exclusive
                .lock()
                .unwrap()
                .push((path.into(), exclusive));
        }
        let rows = self.rows.get(path).ok_or_else(|| {
            FsError::from_io("probe", path, std::io::ErrorKind::PermissionDenied.into())
        })?;
        for entry in rows {
            if cancel.is_cancelled() {
                break;
            }
            self.emitted.fetch_add(1, Ordering::SeqCst);
            if !emit(entry.clone()) {
                break;
            }
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("presence must not stat file contents")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        panic!("no volume query")
    }
}
impl StorageService for Files {
    fn storage_kind(&self, path: &Path) -> Result<StorageKind, FsError> {
        self.classified.lock().unwrap().push(path.into());
        Ok(self
            .kinds
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .unwrap_or_default())
    }
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        panic!("no space query")
    }
}

fn fixture() -> (FolderPresenceRequest, Snapshots, Arc<Files>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/folder-presence");
    let entries: Arc<[Entry]> = [
        (1, "empty", EntryKind::Directory),
        (2, "files", EntryKind::Directory),
        (3, "folders", EntryKind::Directory),
        (4, "link", EntryKind::DirectoryLink),
        (5, "offscreen", EntryKind::Directory),
        (6, "empty-link", EntryKind::DirectoryLink),
        (7, "denied-link", EntryKind::DirectoryLink),
        (8, "file", EntryKind::File),
    ]
    .into_iter()
    .map(|(id, name, kind)| Entry::new(EntryId(id), path.join(name), kind))
    .collect::<Vec<_>>()
    .into();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let mut state = SnapshotState::default();
    state.remember_native_tree_root(root.clone(), path.clone());
    let snapshots = crate::test_snapshots([state, SnapshotState::default()]);
    let mut rows = HashMap::new();
    rows.insert(path.join("empty"), vec![]);
    rows.insert(path.join("empty-link"), vec![]);
    rows.insert(
        path.join("files"),
        vec![Entry::new(
            EntryId(10),
            path.join("files/a"),
            EntryKind::File,
        )],
    );
    rows.insert(
        path.join("folders"),
        vec![
            Entry::new(
                EntryId(11),
                path.join("folders/first"),
                EntryKind::DirectoryLink,
            ),
            Entry::new(
                EntryId(12),
                path.join("folders/never inspected"),
                EntryKind::File,
            ),
        ],
    );
    rows.insert(
        path.join("link"),
        vec![
            Entry::new(
                EntryId(13),
                path.join("link/child-link"),
                EntryKind::DirectoryLink,
            ),
            Entry::new(
                EntryId(14),
                path.join("link/never inspected"),
                EntryKind::File,
            ),
        ],
    );
    (
        FolderPresenceRequest {
            root,
            revision: FolderPresenceRevision(1),
            visible: vec![EntryId(1), EntryId(2), EntryId(3)],
            cancel: Cancellation::default(),
        },
        snapshots,
        Arc::new(Files {
            rows,
            ..Default::default()
        }),
    )
}

#[test]
fn flat_presence_checks_only_requested_folders_and_stops_at_first_subfolder() {
    let (request, snapshots, files) = fixture();
    let result = run(
        &request,
        &request.cancel,
        &snapshots,
        &NativeProvider::new(files.clone()),
        &Arc::new(Semaphore::new(1)),
        &crate::ReportArena::testing(),
        &mut isolated(),
    )
    .unwrap();
    assert_eq!(
        result,
        [
            (EntryId(1), FolderPresence::Empty),
            (EntryId(2), FolderPresence::Empty),
            (EntryId(3), FolderPresence::HasFolders)
        ]
    );
    assert_eq!(files.visited.lock().unwrap().len(), 3);
    assert_eq!(
        files.emitted.load(Ordering::SeqCst),
        2,
        "regular-file tail after first subfolder is never inspected"
    );
    assert!(
        snapshots.lock().unwrap()[0]
            .tree_root
            .as_ref()
            .unwrap()
            .snapshot()
            .same_snapshot(&request.root)
    );
}

#[test]
fn foreign_snapshot_navigation_side_files_and_oversized_requests_do_no_io() {
    let (original, snapshots, files) = fixture();
    for case in 0..8 {
        let mut request = original.clone();
        match case {
            0 => request.root.entries = request.root.entries.to_vec().into(),
            1 => request.root.index = Arc::new((*request.root.index).clone()),
            2 => request.root.navigation = NavigationId(2),
            3 => request.root.pane = PaneId::Right,
            4 => request.visible = vec![EntryId(1), EntryId(8)],
            5 => request.visible = vec![EntryId(1); FolderPresenceRequest::MAX_VISIBLE + 1],
            6 => request.visible = vec![EntryId(1), EntryId(1)],
            _ => request.visible = vec![EntryId(1), EntryId(999)],
        }
        assert!(
            run(
                &request,
                &request.cancel,
                &snapshots,
                &NativeProvider::new(files.clone()),
                &Arc::new(Semaphore::new(1)),
                &crate::ReportArena::testing(),
                &mut isolated()
            )
            .is_none()
        );
    }
    assert!(files.visited.lock().unwrap().is_empty());
}

#[test]
fn native_mixed_directory_and_link_presence_retains_every_requested_hint() {
    let (mut request, snapshots, files) = fixture();
    request.visible = vec![EntryId(1), EntryId(4), EntryId(2), EntryId(3)];
    let result = run(
        &request,
        &request.cancel,
        &snapshots,
        &NativeProvider::new(files.clone()),
        &Arc::new(Semaphore::new(1)),
        &crate::ReportArena::testing(),
        &mut isolated(),
    )
    .expect("one native directory link must not discard the whole visible batch");
    assert_eq!(
        result,
        [
            (EntryId(1), FolderPresence::Empty),
            (EntryId(4), FolderPresence::HasFolders),
            (EntryId(2), FolderPresence::Empty),
            (EntryId(3), FolderPresence::HasFolders),
        ]
    );
    let root = request.root.entries[0].path.parent().unwrap();
    assert_eq!(
        *files.visited.lock().unwrap(),
        [
            root.join("empty"),
            root.join("link"),
            root.join("files"),
            root.join("folders")
        ],
        "probe only immediate logical roots, never children or the offscreen directory"
    );
    assert_eq!(
        files.emitted.load(Ordering::SeqCst),
        3,
        "both directory roots stop at their first subfolder, including a link child"
    );
}

#[test]
fn native_empty_and_denied_links_preserve_their_ordinary_sibling_hint() {
    let (mut request, snapshots, files) = fixture();
    request.visible = vec![EntryId(6), EntryId(7), EntryId(3)];
    let probe = || {
        run(
            &request,
            &request.cancel,
            &snapshots,
            &NativeProvider::new(files.clone()),
            &Arc::new(Semaphore::new(1)),
            &crate::ReportArena::testing(),
            &mut isolated(),
        )
    };
    let result = probe().expect("link availability is an individual hint result");
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], (EntryId(6), FolderPresence::Empty));
    assert_eq!(result[1].0, EntryId(7));
    assert!(
        matches!(&result[1].1, FolderPresence::Unavailable(error) if error.as_str().contains("denied")),
        "a denied link must stay unavailable rather than look empty"
    );
    assert_eq!(result[2], (EntryId(3), FolderPresence::HasFolders));
    let root = request.root.entries[0].path.parent().unwrap();
    assert_eq!(
        *files.visited.lock().unwrap(),
        [
            root.join("empty-link"),
            root.join("denied-link"),
            root.join("folders")
        ]
    );
    assert_eq!(files.emitted.load(Ordering::SeqCst), 1);
    files.visited.lock().unwrap().clear();
    request.cancel.cancel();
    assert!(probe().is_none());
    assert!(files.visited.lock().unwrap().is_empty());
}

#[test]
fn unavailable_directory_is_never_reported_empty_and_cancelled_requests_do_no_io() {
    let (mut request, snapshots, files) = fixture();
    request.visible = vec![EntryId(5)];
    let result = run(
        &request,
        &request.cancel,
        &snapshots,
        &NativeProvider::new(files.clone()),
        &Arc::new(Semaphore::new(1)),
        &crate::ReportArena::testing(),
        &mut isolated(),
    )
    .unwrap();
    assert!(
        matches!(&result[0].1, FolderPresence::Unavailable(error) if error.as_str().contains("denied"))
    );
    files.visited.lock().unwrap().clear();
    request.cancel.cancel();
    assert!(
        run(
            &request,
            &request.cancel,
            &snapshots,
            &NativeProvider::new(files.clone()),
            &Arc::new(Semaphore::new(1)),
            &crate::ReportArena::testing(),
            &mut isolated()
        )
        .is_none()
    );
    assert!(files.visited.lock().unwrap().is_empty());
}

#[derive(Debug)]
struct Archive(AtomicUsize);
impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        7
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        assert_eq!(
            directory, 2,
            "probe the captured member, never the root again"
        );
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![ArchiveNode {
            id: VirtualEntryId { mount: 7, entry: 3 },
            parent: 2,
            raw_name: Arc::from(b"child".as_slice()),
            display_name: "child".into(),
            kind: EntryKind::Directory,
            size: None,
            readable: false,
            modified: None,
        }])
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("presence never extracts archive content")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

fn archive_fixture(
    kind: EntryKind,
) -> (FolderPresenceRequest, Snapshots, Arc<Files>, Arc<Archive>) {
    let (mut request, snapshots, files) = fixture();
    let mount = Arc::new(Archive(AtomicUsize::new(0)));
    let entry = Entry::from_archive(ArchiveNode {
        id: VirtualEntryId { mount: 7, entry: 2 },
        parent: 1,
        raw_name: Arc::from(b"raw folder".as_slice()),
        display_name: "Folder caption".into(),
        kind,
        size: None,
        readable: false,
        modified: None,
    });
    let entries: Arc<[Entry]> = vec![entry].into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    let location = ArchiveLocation {
        mount: Some(mount.clone()),
        address: ArchiveAddress {
            provider: "test.archive".into(),
            format: None,
            container: Path::new(env!("CARGO_MANIFEST_DIR")).join("owned.zip"),
            source_version: vec![1],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, String::new())],
    };
    let listing = ArchiveListing {
        location,
        entries: entries.clone(),
        index: index.clone(),
    };
    let mut states = snapshots.lock().unwrap();
    states[0].archive = Some((NavigationId(1), entries.clone()));
    states[0].remember_archive_tree_root(PaneId::Left, NavigationId(1), &listing);
    drop(states);
    request.root.entries = entries;
    request.root.index = index;
    request.visible = vec![EntryId(2)];
    (request, snapshots, files, mount)
}

#[test]
fn archive_presence_uses_captured_member_identity_without_native_fallback() {
    let (request, snapshots, files, mount) = archive_fixture(EntryKind::Directory);
    let result = run(
        &request,
        &request.cancel,
        &snapshots,
        &NativeProvider::new(files.clone()),
        &Arc::new(Semaphore::new(1)),
        &crate::ReportArena::testing(),
        &mut isolated(),
    )
    .unwrap();
    assert_eq!(result, [(EntryId(2), FolderPresence::HasFolders)]);
    assert_eq!(mount.0.load(Ordering::SeqCst), 1);
    assert!(files.visited.lock().unwrap().is_empty());
}

#[test]
fn archive_presence_keeps_directory_links_and_files_out_of_probe_batches() {
    for kind in [EntryKind::DirectoryLink, EntryKind::File] {
        let (request, snapshots, files, mount) = archive_fixture(kind);
        assert!(
            run(
                &request,
                &request.cancel,
                &snapshots,
                &NativeProvider::new(files.clone()),
                &Arc::new(Semaphore::new(1)),
                &crate::ReportArena::testing(),
                &mut isolated(),
            )
            .is_none(),
            "archive presence retains the ordinary-directory-only contract: {kind:?}"
        );
        assert_eq!(mount.0.load(Ordering::SeqCst), 0);
        assert!(files.visited.lock().unwrap().is_empty());
    }
}

fn probe_with(
    request: &FolderPresenceRequest,
    snapshots: &Snapshots,
    files: &Arc<Files>,
    admissions: &mut Admissions,
) -> Option<Vec<(EntryId, FolderPresence)>> {
    run(
        request,
        &request.cancel,
        snapshots,
        &NativeProvider::new(files.clone()),
        &Arc::new(Semaphore::new(1)),
        &crate::ReportArena::testing(),
        admissions,
    )
}

#[test]
fn ssd_listing_rows_share_the_gate_only_on_proven_ssd_devices_and_scrolling_reuses_classes() {
    let (mut request, snapshots, files) = fixture();
    let root = request.root.entries[0].path.parent().unwrap().to_owned();
    let gate = Arc::new(Gate::new(4));
    *files.gate.lock().unwrap() = Some(gate.clone());
    files.kinds.lock().unwrap().extend([
        (root.clone(), StorageKind::LocalSsd),
        (root.join("files"), StorageKind::Rotational),
        (root.join("link"), StorageKind::LocalSsd),
    ]);
    // "files" is a mount point of a rotational disk that "empty-link" also
    // reaches; "link" leads to another SSD and "denied-link" has no identity.
    files.devices.lock().unwrap().extend([
        (root.clone(), 1),
        (root.join("empty"), 1),
        (root.join("folders"), 1),
        (root.join("offscreen"), 1),
        (root.join("files"), 2),
        (root.join("link"), 3),
        (root.join("empty-link"), 2),
    ]);
    request.visible = [1, 2, 3, 4, 6, 7].map(EntryId).to_vec();
    let mut admissions = Admissions::new(gate, Arc::new(Gate::new(1)));
    let result = probe_with(&request, &snapshots, &files, &mut admissions).unwrap();
    assert_eq!(result.len(), 6);
    assert_eq!(
        *files.exclusive.lock().unwrap(),
        [
            (root.join("empty"), false),
            (root.join("files"), true),
            (root.join("folders"), false),
            (root.join("link"), false),
            (root.join("empty-link"), true),
            (root.join("denied-link"), true),
        ]
    );
    // The listing and each other device are classified once.
    assert_eq!(
        *files.classified.lock().unwrap(),
        [root.clone(), root.join("files"), root.join("link")]
    );
    // Scrolling the same listing reuses its class and device.
    files.exclusive.lock().unwrap().clear();
    request.visible = vec![EntryId(5)];
    request.revision = FolderPresenceRevision(2);
    probe_with(&request, &snapshots, &files, &mut admissions).unwrap();
    assert_eq!(
        *files.exclusive.lock().unwrap(),
        [(root.join("offscreen"), false)]
    );
    assert_eq!(files.classified.lock().unwrap().len(), 3);
    let identified = files.identified.lock().unwrap();
    assert_eq!(identified.iter().filter(|path| **path == root).count(), 1);
}

#[test]
fn rows_of_an_unproven_listing_stay_exclusive_without_row_identity_queries() {
    for (kind, device) in [
        (StorageKind::Rotational, Some(1)),
        (StorageKind::LocalSsd, None),
    ] {
        let (mut request, snapshots, files) = fixture();
        let root = request.root.entries[0].path.parent().unwrap().to_owned();
        let gate = Arc::new(Gate::new(4));
        *files.gate.lock().unwrap() = Some(gate.clone());
        files.kinds.lock().unwrap().extend([
            (root.clone(), kind),
            (root.join("link"), StorageKind::LocalSsd),
        ]);
        let mut devices = files.devices.lock().unwrap();
        devices.extend(device.map(|device| (root.clone(), device)));
        devices.insert(root.join("link"), 3);
        drop(devices);
        request.visible = vec![EntryId(1), EntryId(4)];
        let mut admissions = Admissions::new(gate, Arc::new(Gate::new(1)));
        probe_with(&request, &snapshots, &files, &mut admissions).unwrap();
        assert_eq!(
            *files.exclusive.lock().unwrap(),
            [(root.join("empty"), true), (root.join("link"), true)],
            "{kind:?}"
        );
        assert_eq!(
            *files.classified.lock().unwrap(),
            std::slice::from_ref(&root)
        );
        // Only a shared listing queries its own device; rows are never queried.
        let identified = files.identified.lock().unwrap();
        assert_eq!(identified.len(), usize::from(kind == StorageKind::LocalSsd));
    }
}

/// Cost of the per-row identity query that proves a shared row's device,
/// next to the presence probe itself, on real native directories. Run with
/// `cargo test -p midnight-engine --lib -- folder_presence::tests::presence_identity_cost_probe --ignored --nocapture`.
#[test]
#[ignore = "timing probe for manual measurements"]
fn presence_identity_cost_probe() {
    let owned = tempfile::tempdir().unwrap();
    let rows = (0..200)
        .map(|index| {
            let row = owned.path().join(format!("row-{index:03}"));
            std::fs::create_dir_all(row.join("child")).unwrap();
            for file in 0..10 {
                std::fs::write(row.join(format!("file-{file}")), [7; 3]).unwrap();
            }
            row
        })
        .collect::<Vec<_>>();
    let filesystem: Arc<dyn FileSystem> = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let native = NativeProvider::new(filesystem.clone());
    let cancel = Cancellation::default();
    let median = |run: &dyn Fn()| {
        let mut samples = (0..5)
            .map(|_| {
                let started = std::time::Instant::now();
                run();
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort();
        samples[samples.len() / 2]
    };
    let probes = median(&|| {
        for row in &rows {
            assert!(native.has_subfolders(row, &cancel).unwrap());
        }
    });
    let identities = median(&|| {
        for row in &rows {
            assert!(device(filesystem.as_ref(), row, &cancel).is_some());
        }
    });
    println!("200 rows: presence probes median {probes:?}, identity queries median {identities:?}");
}
