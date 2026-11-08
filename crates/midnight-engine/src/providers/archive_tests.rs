use super::*;
use crate::providers::MountedFilesystem;
use midnight_core::{ArchiveAddress, ArchiveMount, ArchiveNode, EntryKind, FsError, NodeToken};
use midnight_plugin_host::CapturedFile;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Debug, Default)]
struct State {
    invalid: AtomicBool,
    invalidate_read: AtomicBool,
    reads: AtomicUsize,
    read_mode: AtomicUsize,
    list_mode: AtomicUsize,
    rows: Mutex<HashMap<u64, Vec<ArchiveNode>>>,
}

#[derive(Debug)]
struct FakeMount {
    id: u64,
    state: Arc<State>,
}

fn failure(message: &str) -> FsError {
    FsError::from_io("fake archive", Path::new(""), io::Error::other(message))
}

impl ArchiveMount for FakeMount {
    fn id(&self) -> u64 {
        self.id
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.state.invalid.load(Ordering::Acquire) {
            Err(failure("captured source changed"))
        } else {
            Ok(())
        }
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.validate()?;
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "fake archive list",
                Path::new(""),
                io::ErrorKind::Interrupted.into(),
            ));
        }
        let mut rows = self
            .state
            .rows
            .lock()
            .unwrap()
            .get(&directory)
            .cloned()
            .ok_or_else(|| failure("unknown directory"))?;
        match self.state.list_mode.load(Ordering::Acquire) {
            1 => rows.push(rows[0].clone()),
            2 => rows[0].id.mount += 1,
            3 => rows[0].parent = 99,
            _ => {}
        }
        Ok(rows)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.validate()?;
        if cancel.is_cancelled() || !matches!(entry, 2 | 3 | 5) {
            return Err(failure("unreadable member"));
        }
        self.state.reads.fetch_add(1, Ordering::AcqRel);
        let data = b"hello";
        let begin = usize::try_from(offset).unwrap().min(data.len());
        let end = begin.saturating_add(length as usize).min(data.len());
        let mut bytes = data[begin..end].to_vec();
        match self.state.read_mode.load(Ordering::Acquire) {
            1 => bytes.clear(),
            2 => bytes.push(0),
            _ => {}
        }
        if self.state.invalidate_read.load(Ordering::Acquire) {
            self.state.invalid.store(true, Ordering::Release);
        }
        Ok(bytes)
    }
}

fn node(mount: u64, entry: u64, parent: u64, kind: EntryKind) -> ArchiveNode {
    ArchiveNode {
        id: midnight_core::VirtualEntryId { mount, entry },
        parent,
        raw_name: Arc::from([0xff, entry as u8]),
        display_name: "same caption".into(),
        kind,
        size: (kind == EntryKind::File).then_some(5),
        readable: kind == EntryKind::File,
    }
}

fn fixture(id: u64, directory: u64) -> (ArchiveLocation, Arc<State>) {
    let state = Arc::new(State::default());
    state.rows.lock().unwrap().extend([
        (
            1,
            vec![
                node(id, 2, 1, EntryKind::File),
                node(id, 3, 1, EntryKind::File),
                node(id, 4, 1, EntryKind::Directory),
            ],
        ),
        (4, vec![node(id, 5, 4, EntryKind::File)]),
    ]);
    let trail = if directory == 1 {
        vec![(1, "container".into())]
    } else {
        vec![(1, "container".into()), (4, "directory".into())]
    };
    let container = if cfg!(windows) {
        PathBuf::from(r"D:\not-opened.zip")
    } else {
        PathBuf::from("/not-opened.zip")
    };
    (
        ArchiveLocation {
            mount: Some(Arc::new(FakeMount {
                id,
                state: state.clone(),
            })),
            address: ArchiveAddress {
                provider: "fake.archive".into(),
                container,
                source_version: vec![9, 8, 7],
                components: Vec::new(),
            },
            directory,
            trail,
        },
        state,
    )
}

fn listing(mount: &dyn MountedFilesystem, root: &NodeToken) -> Vec<super::super::ProviderEntry> {
    let mut rows = Vec::new();
    mount
        .list(root, &Cancellation::default(), &mut |row| {
            rows.push(row);
            true
        })
        .unwrap();
    rows
}

fn forged(mount: u64, entry: u64) -> NodeToken {
    NodeToken::new(&[mount.to_le_bytes(), entry.to_le_bytes()].concat()).unwrap()
}

#[test]
fn raw_member_names_and_captions_remain_distinct_without_native_paths() {
    let (location, _) = fixture(11, 1);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].display_name, rows[1].display_name);
    assert_ne!(rows[0].raw_name, rows[1].raw_name);
    assert_ne!(rows[0].token, rows[1].token);
    assert_eq!(rows[0].raw_name.as_ref(), &[0xff, 2]);
    assert_eq!(rows[0].metadata.size, Some(5));
    assert_eq!(
        opened
            .mount
            .parent(&rows[2].token, &Cancellation::default())
            .unwrap(),
        Some(opened.root.clone())
    );
    assert_eq!(
        opened
            .mount
            .parent(&opened.root, &Cancellation::default())
            .unwrap(),
        None
    );
}

#[test]
fn captured_subdirectory_keeps_provider_parent_and_foreign_tokens_are_rejected() {
    let (location, _) = fixture(12, 4);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    assert_eq!(rows.len(), 1);
    let parent = opened
        .mount
        .parent(&opened.root, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(listing(opened.mount.as_ref(), &parent).len(), 3);
    for bad in [
        forged(13, 5),
        forged(12, 99),
        NodeToken::new(b"bad").unwrap(),
    ] {
        assert!(
            opened
                .mount
                .open_content(&bad, &Cancellation::default())
                .is_err()
        );
    }
}

#[test]
fn captured_archive_tree_omits_descendant_files_and_rejects_changed_container() {
    use crate::providers::{
        FilesystemProvider, OpenedLocation, ProviderError, ProviderRegistry,
        tree::{DirectoryTree, Disclosure, TreeLimits, TreeRequest},
    };
    use midnight_core::{ProviderId, StoredLocation};
    struct Captured(ArchiveLocation);
    impl FilesystemProvider for Captured {
        fn resolve(
            &self,
            _: &StoredLocation,
            cancel: &Cancellation,
        ) -> Result<OpenedLocation, ProviderError> {
            captured_archive(&self.0, cancel)
        }
    }
    let (location, state) = fixture(91, 1);
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new("test.archive.tree").unwrap(),
        std::num::NonZeroU32::MIN,
        Arc::from([]),
    );
    registry
        .register(address.provider().clone(), Arc::new(Captured(location)))
        .unwrap();
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let mut tree = DirectoryTree::new(mount, TreeLimits::default()).unwrap();
    let result = tree
        .request(tree.root(), TreeRequest::Expand)
        .unwrap()
        .run();
    assert!(tree.apply(result));
    let rows = tree.rows();
    assert_eq!(rows.len(), 3);
    let folder = rows
        .iter()
        .find(|row| row.entry.kind == EntryKind::Directory)
        .unwrap();
    let result = tree.request(folder.id, TreeRequest::Expand).unwrap().run();
    assert!(tree.apply(result));
    assert_eq!(tree.state(folder.id).unwrap().disclosure, Disclosure::Empty);
    assert_eq!(tree.rows().len(), 3);
    state.invalid.store(true, Ordering::Release);
    let result = tree.request(folder.id, TreeRequest::Refresh).unwrap().run();
    assert!(tree.apply(result));
    assert_eq!(
        tree.state(folder.id).unwrap().disclosure,
        Disclosure::Failed
    );
    assert_eq!(
        state.reads.load(Ordering::Acquire),
        0,
        "tree listing must not extract members"
    );
}

#[test]
fn member_content_uses_broker_exact_reads_and_retains_archive_after_location_close() {
    let (location, state) = fixture(14, 1);
    let weak = Arc::downgrade(location.mount.as_ref().unwrap());
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    let source = opened
        .mount
        .open_content(&rows[0].token, &Cancellation::default())
        .unwrap();
    let captured = CapturedFile::from_source(source).unwrap();
    assert!(captured.path().as_os_str().is_empty());
    drop(opened);
    drop(location);
    assert!(weak.upgrade().is_some());
    assert_eq!(captured.read_at(0, 5).unwrap(), b"hello");
    assert_eq!(captured.read_at(4, 64).unwrap(), b"o");
    assert_eq!(captured.read_at(5, 1).unwrap(), b"");
    assert_eq!(state.reads.load(Ordering::Acquire), 2);
    drop(captured);
    assert!(weak.upgrade().is_none());
}

#[test]
fn source_invalidation_before_or_during_read_rejects_old_content() {
    for during in [false, true] {
        let (location, state) = fixture(15, 1);
        let opened = captured_archive(&location, &Cancellation::default()).unwrap();
        let rows = listing(opened.mount.as_ref(), &opened.root);
        let source = opened
            .mount
            .open_content(&rows[0].token, &Cancellation::default())
            .unwrap();
        let captured = CapturedFile::from_source(source).unwrap();
        if during {
            state.invalidate_read.store(true, Ordering::Release);
        } else {
            state.invalid.store(true, Ordering::Release);
        }
        assert!(captured.read_at(0, 5).is_err());
        assert!(
            opened
                .mount
                .parent(&opened.root, &Cancellation::default())
                .is_err()
        );
        assert!(
            opened
                .mount
                .list(&opened.root, &Cancellation::default(), &mut |_| true)
                .is_err()
        );
    }
}

#[test]
fn unknown_size_unreadable_and_links_never_gain_content_authority() {
    let (location, state) = fixture(16, 1);
    {
        let mut rows = state.rows.lock().unwrap();
        let files = rows.get_mut(&1).unwrap();
        files[0].size = None;
        files[1].readable = false;
        files.push(node(16, 8, 1, EntryKind::Symlink));
    }
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    for row in rows {
        assert!(matches!(
            opened
                .mount
                .open_content(&row.token, &Cancellation::default()),
            Err(ProviderError::Unsupported)
        ));
    }
    assert_eq!(state.reads.load(Ordering::Acquire), 0);
}

#[test]
fn broker_rejects_short_and_oversized_provider_reads() {
    for mode in [1, 2] {
        let (location, state) = fixture(17, 1);
        let opened = captured_archive(&location, &Cancellation::default()).unwrap();
        let rows = listing(opened.mount.as_ref(), &opened.root);
        let source = opened
            .mount
            .open_content(&rows[0].token, &Cancellation::default())
            .unwrap();
        let captured = CapturedFile::from_source(source).unwrap();
        state.read_mode.store(mode, Ordering::Release);
        assert!(captured.read_at(0, 5).is_err());
    }
}

#[test]
fn malformed_or_cancelled_listing_does_not_cache_partial_members() {
    for mode in [1, 2, 3] {
        let (location, state) = fixture(18, 1);
        let opened = captured_archive(&location, &Cancellation::default()).unwrap();
        state.list_mode.store(mode, Ordering::Release);
        assert!(
            opened
                .mount
                .list(&opened.root, &Cancellation::default(), &mut |_| true)
                .is_err()
        );
        assert!(
            opened
                .mount
                .open_content(&forged(18, 2), &Cancellation::default())
                .is_err()
        );
    }
    let (location, _) = fixture(19, 1);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    assert!(matches!(
        opened.mount.list(&opened.root, &cancel, &mut |_| {
            cancel.cancel();
            true
        }),
        Err(ProviderError::Cancelled)
    ));
    assert!(
        opened
            .mount
            .open_content(&forged(19, 2), &Cancellation::default())
            .is_err()
    );
}

#[test]
fn missing_mount_cancelled_capture_and_invalid_source_are_explicit_errors() {
    let (mut location, state) = fixture(20, 1);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        captured_archive(&location, &cancel),
        Err(ProviderError::Cancelled)
    ));
    state.invalid.store(true, Ordering::Release);
    assert!(captured_archive(&location, &Cancellation::default()).is_err());
    location.mount = None;
    assert!(matches!(
        captured_archive(&location, &Cancellation::default()),
        Err(ProviderError::Unavailable)
    ));
}

#[test]
fn stopped_listing_and_contradictory_reissue_preserve_cache_transaction() {
    let (location, state) = fixture(21, 1);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    opened
        .mount
        .list(&opened.root, &Cancellation::default(), &mut |_| false)
        .unwrap();
    assert!(
        opened
            .mount
            .open_content(&forged(21, 2), &Cancellation::default())
            .is_err()
    );
    let rows = listing(opened.mount.as_ref(), &opened.root);
    {
        let mut records = state.rows.lock().unwrap();
        let records = records.get_mut(&1).unwrap();
        records[1].kind = EntryKind::Directory;
        records.push(node(21, 9, 1, EntryKind::File));
    }
    let mut emitted = 0;
    assert!(matches!(
        opened
            .mount
            .list(&opened.root, &Cancellation::default(), &mut |_| {
                emitted += 1;
                true
            }),
        Err(ProviderError::InvalidResponse(_))
    ));
    assert_eq!(emitted, 0);
    assert!(
        opened
            .mount
            .open_content(&forged(21, 9), &Cancellation::default())
            .is_err()
    );
    let original = opened
        .mount
        .open_content(&rows[1].token, &Cancellation::default())
        .unwrap();
    assert_eq!(
        CapturedFile::from_source(original)
            .unwrap()
            .read_at(0, 5)
            .unwrap(),
        b"hello"
    );
}

#[test]
fn interleaved_members_have_distinct_versions_and_enforce_direct_read_bounds() {
    let (location, state) = fixture(22, 1);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    let first = opened
        .mount
        .open_content(&rows[0].token, &Cancellation::default())
        .unwrap();
    let second = opened
        .mount
        .open_content(&rows[1].token, &Cancellation::default())
        .unwrap();
    assert_ne!(first.version(), second.version());
    assert_eq!(first.read_at(2, 2).unwrap(), b"ll");
    assert_eq!(second.read_at(0, 2).unwrap(), b"he");
    assert_eq!(first.read_at(4, 8).unwrap(), b"o");
    let before = state.reads.load(Ordering::Acquire);
    for (offset, count) in [(0, 0), (0, 256 * 1024 + 1), (u64::MAX, 2)] {
        assert!(first.read_at(offset, count).is_err());
    }
    assert_eq!(state.reads.load(Ordering::Acquire), before);
}
