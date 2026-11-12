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
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.visited.lock().unwrap().push(path.into());
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
fn foreign_snapshot_navigation_side_link_roots_and_oversized_requests_do_no_io() {
    let (original, snapshots, files) = fixture();
    for case in 0..8 {
        let mut request = original.clone();
        match case {
            0 => request.root.entries = request.root.entries.to_vec().into(),
            1 => request.root.index = Arc::new((*request.root.index).clone()),
            2 => request.root.navigation = NavigationId(2),
            3 => request.root.pane = PaneId::Right,
            4 => request.visible = vec![EntryId(4)],
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
                &Arc::new(Semaphore::new(1))
            )
            .is_none()
        );
    }
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
            &Arc::new(Semaphore::new(1))
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

#[test]
fn archive_presence_uses_captured_member_identity_without_native_fallback() {
    let (mut request, snapshots, files) = fixture();
    let mount = Arc::new(Archive(AtomicUsize::new(0)));
    let entry = Entry::from_archive(ArchiveNode {
        id: VirtualEntryId { mount: 7, entry: 2 },
        parent: 1,
        raw_name: Arc::from(b"raw folder".as_slice()),
        display_name: "Folder caption".into(),
        kind: EntryKind::Directory,
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
    let result = run(
        &request,
        &request.cancel,
        &snapshots,
        &NativeProvider::new(files.clone()),
        &Arc::new(Semaphore::new(1)),
    )
    .unwrap();
    assert_eq!(result, [(EntryId(2), FolderPresence::HasFolders)]);
    assert_eq!(mount.0.load(Ordering::SeqCst), 1);
    assert!(files.visited.lock().unwrap().is_empty());
}
