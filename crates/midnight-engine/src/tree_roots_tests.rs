use super::*;
use crate::{CommittedSnapshot, Outbox, Snapshots, publish_listing};
use midnight_core::{
    ArchiveAddress, ArchiveFormat, ArchiveMount, ArchiveNode, DirectoryIndex, DirectoryService,
    Entry, EntryId, EntryKind, EntryMetadata, Event, FsError, NavigationId, PaneId, SortSpec,
    SpaceInfo, StorageService, VirtualEntryId, Volume, VolumeService,
};
use std::{collections::HashMap, path::Path, sync::Mutex};

struct NoNativeIo;

impl DirectoryService for NoNativeIo {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        panic!("completed root must not be enumerated twice")
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("completed root metadata must be reused")
    }
}
impl VolumeService for NoNativeIo {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        panic!("no volume query belongs to root handoff")
    }
}
impl StorageService for NoNativeIo {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        panic!("no space query belongs to root handoff")
    }
}

fn native_snapshot() -> (TreeRootSnapshot, PathBuf) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tree-root-tests");
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(19),
        path.join("東京 folder"),
        EntryKind::Directory,
    )]
    .into();
    (
        TreeRootSnapshot {
            pane: PaneId::Left,
            navigation: NavigationId(7),
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        },
        path,
    )
}

fn native_state(root: &TreeRootSnapshot, path: PathBuf) -> SnapshotState {
    let mut state = SnapshotState::default();
    drop(state.remember_native_tree_root(root.clone(), path));
    state
}

#[test]
fn exact_completed_native_root_reuses_rows_without_native_io() {
    let (root, path) = native_snapshot();
    let state = native_state(&root, path.clone());
    let captured = state.capture_tree_root(&root).unwrap();
    assert!(captured.snapshot().same_snapshot(&root));
    let opened = captured
        .open(
            &NativeProvider::new(Arc::new(NoNativeIo)),
            &Cancellation::default(),
        )
        .unwrap();
    let mut rows = Vec::new();
    opened
        .mount
        .list(&opened.root, &Cancellation::default(), &mut |row| {
            rows.push(row);
            true
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name, "東京 folder");
    assert!(matches!(
        opened.mount.resolve_target(&rows[0].token, &Cancellation::default()),
        Ok(crate::providers::ProviderTarget::Native(actual)) if actual == path.join("東京 folder")
    ));
}

#[test]
fn equal_contents_in_foreign_allocations_or_generations_are_rejected() {
    let (root, path) = native_snapshot();
    let state = native_state(&root, path);
    let mut foreign = root.clone();
    foreign.entries = root.entries.to_vec().into();
    assert!(matches!(
        state.capture_tree_root(&foreign),
        Err(ProviderError::StaleReference)
    ));
    foreign = root.clone();
    foreign.index = Arc::new((*root.index).clone());
    assert!(matches!(
        state.capture_tree_root(&foreign),
        Err(ProviderError::StaleReference)
    ));
    foreign = root.clone();
    foreign.pane = PaneId::Right;
    assert!(matches!(
        state.capture_tree_root(&foreign),
        Err(ProviderError::StaleReference)
    ));
    foreign = root;
    foreign.navigation = NavigationId(8);
    assert!(matches!(
        state.capture_tree_root(&foreign),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn pending_navigation_blocks_admission_but_not_accepted_capture() {
    let (root, path) = native_snapshot();
    let mut state = native_state(&root, path);
    let accepted = state.capture_tree_root(&root).unwrap();
    state.pending = Some(NavigationId(8));
    assert!(matches!(
        state.capture_tree_root(&root),
        Err(ProviderError::StaleReference)
    ));
    state.pending = None;
    state.archive_navigation = Some(NavigationId(8));
    assert!(matches!(
        state.capture_tree_root(&root),
        Err(ProviderError::StaleReference)
    ));
    state.archive_navigation = None;
    state.network_navigation = Some(NavigationId(8));
    assert!(matches!(
        state.capture_tree_root(&root),
        Err(ProviderError::StaleReference)
    ));
    drop(state);
    assert!(accepted.snapshot().same_snapshot(&root));
    assert!(
        accepted
            .open(
                &NativeProvider::new(Arc::new(NoNativeIo)),
                &Cancellation::default()
            )
            .is_ok()
    );
}

#[test]
fn cancelled_open_does_not_call_native_services() {
    let (root, path) = native_snapshot();
    let captured = native_state(&root, path).capture_tree_root(&root).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        captured.open(&NativeProvider::new(Arc::new(NoNativeIo)), &cancel),
        Err(ProviderError::Cancelled)
    ));
}

#[test]
fn completed_listing_records_the_exact_index_before_waking_the_ui() {
    let (root, path) = native_snapshot();
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    let observed = snapshots.clone();
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(move || {
            assert!(observed.lock().unwrap()[0].tree_root.is_some());
        }),
    };
    publish_listing(
        &outbox,
        &snapshots,
        None,
        root.pane,
        SortSpec::default(),
        &Cancellation::default(),
        CommittedSnapshot {
            navigation: root.navigation,
            path,
            entries: root.entries,
            sizes: Arc::new(Mutex::new(HashMap::new())),
        },
    );
    let Event::ListingFinished {
        pane,
        navigation,
        entries,
        index,
    } = events.try_recv().unwrap()
    else {
        panic!("listing completion expected")
    };
    let published = TreeRootSnapshot {
        pane,
        navigation,
        entries,
        index,
    };
    let states = snapshots.lock().unwrap();
    assert!(states[0].capture_tree_root(&published).is_ok());
    assert!(states[1].capture_tree_root(&published).is_err());
}

#[derive(Debug)]
struct Archive;

impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        23
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("archive handoff must reuse the completed listing")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("root handoff must not extract archive data")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

#[test]
fn archive_handoff_retains_source_mount_and_exact_virtual_index_after_navigation() {
    let archive = Arc::new(Archive);
    let retained = Arc::downgrade(&archive);
    let location = ArchiveLocation {
        mount: Some(archive),
        address: ArchiveAddress {
            format: Some(ArchiveFormat::Zip),
            provider: "test.archive".into(),
            container: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/source.zip"),
            source_version: vec![1],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, "source.zip".into())],
    };
    let entries: Arc<[Entry]> = vec![Entry::from_archive(ArchiveNode {
        modified: None,
        id: VirtualEntryId {
            mount: 23,
            entry: 17,
        },
        parent: 1,
        raw_name: Arc::from(&b"folder"[..]),
        display_name: "folder".into(),
        kind: EntryKind::Directory,
        size: None,
        readable: false,
    })]
    .into();
    let listing = ArchiveListing {
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        location,
    };
    let mut state = SnapshotState::default();
    drop(state.remember_archive_tree_root(PaneId::Left, NavigationId(4), &listing));
    state.archive = Some((NavigationId(4), listing.entries.clone()));
    state.archive_navigation = Some(NavigationId(4));
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(4),
        entries: listing.entries.clone(),
        index: listing.index.clone(),
    };
    let accepted = state.capture_tree_root(&root).unwrap();
    state.network_navigation = Some(NavigationId(5));
    assert!(matches!(
        state.capture_tree_root(&root),
        Err(ProviderError::StaleReference)
    ));
    // An older discovery snapshot may be retained while a newer archive is visible.
    state.network_navigation = Some(NavigationId(3));
    assert!(state.capture_tree_root(&root).is_ok());
    drop(listing);
    drop(state);
    assert!(retained.upgrade().is_some());
    let opened = accepted
        .open(
            &NativeProvider::new(Arc::new(NoNativeIo)),
            &Cancellation::default(),
        )
        .unwrap();
    let mut rows = Vec::new();
    opened
        .mount
        .list(&opened.root, &Cancellation::default(), &mut |row| {
            rows.push(row);
            true
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name, "folder");
    drop(opened);
    drop(accepted);
    assert!(retained.upgrade().is_none());
}
