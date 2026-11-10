use super::*;
use midnight_core::{EntryKind, FsError};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Debug, Default)]
pub(super) struct Mount {
    pub(super) rows: HashMap<u64, Vec<ArchiveNode>>,
    pub(super) reads: AtomicUsize,
    pub(super) lists: AtomicUsize,
    pub(super) changed: AtomicBool,
    pub(super) cancel: Option<Cancellation>,
    pub(super) cancel_on: Option<u64>,
}
impl ArchiveMount for Mount {
    fn id(&self) -> u64 {
        7
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        if let Some(cancel) = &self.cancel
            && self.cancel_on.is_none_or(|id| id == directory)
        {
            cancel.cancel();
        }
        self.rows
            .get(&directory)
            .cloned()
            .ok_or_else(|| failure("unreadable"))
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        panic!("size scan must never read archive contents")
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.changed.load(Ordering::Relaxed) {
            Err(failure("changed"))
        } else {
            Ok(())
        }
    }
}
fn failure(message: &str) -> FsError {
    FsError::from_io(
        "test",
        std::path::Path::new(""),
        std::io::Error::other(message),
    )
}
pub(super) fn node(id: u64, parent: u64, kind: EntryKind, size: Option<u64>) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId {
            mount: 7,
            entry: id,
        },
        parent,
        raw_name: Arc::from(b"same".as_slice()),
        display_name: "same".into(),
        kind,
        size,
        readable: kind == EntryKind::File,
        modified: None,
    }
}
pub(super) fn address() -> ArchiveAddress {
    ArchiveAddress {
        provider: "test.archive".into(),
        format: None,
        container: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/raw archive.zip"),
        source_version: vec![1],
        components: vec![],
    }
}
pub(super) fn run(mount: &Mount, roots: &[ArchiveNode]) -> (bool, HashMap<u64, FolderSize>) {
    let mut result = HashMap::new();
    let ok = compute(
        mount,
        &address(),
        &[1],
        &[],
        roots,
        &Cancellation::default(),
        &mut |id, size| {
            result.insert(id.entry, size);
            true
        },
    );
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
    (ok, result)
}
#[test]
fn nested_empty_and_duplicate_names_have_independent_identity_sums() {
    let mut mount = Mount::default();
    mount.rows.insert(
        2,
        vec![
            node(4, 2, EntryKind::File, Some(7)),
            node(5, 2, EntryKind::Directory, None),
        ],
    );
    mount
        .rows
        .insert(5, vec![node(6, 5, EntryKind::File, Some(11))]);
    mount.rows.insert(3, vec![]);
    let (ok, sizes) = run(
        &mount,
        &[
            node(2, 1, EntryKind::Directory, None),
            node(3, 1, EntryKind::Directory, None),
        ],
    );
    assert!(ok);
    assert_eq!(sizes[&2].bytes, 18);
    assert_eq!(sizes[&2].files, 2);
    assert_eq!(sizes[&5].bytes, 11);
    assert_eq!(sizes[&3].bytes, 0);
    assert!(sizes.values().all(|s| s.finished && s.errors == 0));
}
#[test]
fn links_are_counted_as_skipped_without_traversal() {
    let mut mount = Mount::default();
    mount.rows.insert(
        2,
        vec![
            node(3, 2, EntryKind::DirectoryLink, Some(900)),
            node(4, 2, EntryKind::Symlink, Some(600)),
            node(5, 2, EntryKind::Other, Some(100)),
        ],
    );
    let (ok, sizes) = run(&mount, &[node(2, 1, EntryKind::Directory, None)]);
    assert!(ok);
    assert_eq!(sizes[&2].bytes, 0);
    assert_eq!(sizes[&2].skipped_links, 2);
    assert_eq!(mount.lists.load(Ordering::Relaxed), 1);
}
#[test]
fn unknown_lengths_unreadable_directories_and_overflow_are_partial() {
    let mut mount = Mount::default();
    mount.rows.insert(
        2,
        vec![
            node(3, 2, EntryKind::File, None),
            node(4, 2, EntryKind::File, Some(u64::MAX)),
            node(5, 2, EntryKind::File, Some(1)),
            node(6, 2, EntryKind::Directory, None),
        ],
    );
    let (ok, sizes) = run(&mount, &[node(2, 1, EntryKind::Directory, None)]);
    assert!(ok);
    assert_eq!(sizes[&2].bytes, u64::MAX);
    assert!(sizes[&2].errors >= 3);
    assert!(sizes[&2].finished);
    assert_eq!(sizes[&6].errors, 1);
}
#[test]
fn cycle_duplicate_id_wrong_parent_and_mount_never_double_count() {
    let mut mount = Mount::default();
    let mut foreign = node(9, 2, EntryKind::File, Some(100));
    foreign.id.mount = 99;
    mount.rows.insert(
        2,
        vec![
            node(3, 2, EntryKind::Directory, None),
            node(4, 2, EntryKind::File, Some(5)),
            node(4, 2, EntryKind::File, Some(5)),
            node(8, 99, EntryKind::File, Some(100)),
            foreign,
        ],
    );
    mount
        .rows
        .insert(3, vec![node(2, 3, EntryKind::Directory, None)]);
    let (ok, sizes) = run(&mount, &[node(2, 1, EntryKind::Directory, None)]);
    assert!(ok);
    assert_eq!(sizes[&2].bytes, 5);
    assert!(sizes[&2].errors >= 4);
    assert_eq!(mount.lists.load(Ordering::Relaxed), 2);
}
#[test]
fn stale_source_and_cancel_never_finish_or_publish_complete_data() {
    let mount = Mount::default();
    mount.changed.store(true, Ordering::Relaxed);
    let (ok, sizes) = run(&mount, &[node(2, 1, EntryKind::Directory, None)]);
    assert!(!ok);
    assert!(sizes.values().all(|s| !s.finished));
    let cancel = Cancellation::default();
    let mut mount = Mount {
        cancel: Some(cancel.clone()),
        ..Mount::default()
    };
    mount.rows.insert(2, vec![]);
    let mut sizes = Vec::new();
    assert!(!compute(
        &mount,
        &address(),
        &[1],
        &[],
        &[node(2, 1, EntryKind::Directory, None)],
        &cancel,
        &mut |_, size| {
            sizes.push(size);
            true
        }
    ));
    assert!(sizes.iter().all(|s| !s.finished));
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn oversized_metadata_and_invalid_roots_fail_without_unbounded_traversal() {
    let mut mount = Mount::default();
    let mut huge = node(3, 2, EntryKind::Directory, None);
    huge.raw_name = vec![b'x'; MAX_NAMES + 1].into();
    mount.rows.insert(2, vec![huge]);
    let (ok, sizes) = run(&mount, &[node(2, 1, EntryKind::Directory, None)]);
    assert!(ok);
    assert!(sizes[&2].finished);
    assert_eq!(sizes[&2].errors, 1);
    assert_eq!(mount.lists.load(Ordering::Relaxed), 1);
    let (ok, sizes) = run(&mount, &[node(2, 99, EntryKind::Directory, None)]);
    assert!(!ok);
    assert!(sizes.is_empty());
}

#[test]
fn deep_traversal_is_iterative_and_captured_ancestors_cannot_reappear_as_children() {
    let mut mount = Mount::default();
    for id in 2..4098 {
        mount
            .rows
            .insert(id, vec![node(id + 1, id, EntryKind::Directory, None)]);
    }
    mount
        .rows
        .insert(4098, vec![node(4099, 4098, EntryKind::File, Some(17))]);
    let (ok, sizes) = run(&mount, &[node(2, 1, EntryKind::Directory, None)]);
    assert!(ok);
    assert_eq!(sizes.len(), 4097);
    assert_eq!(sizes[&2].bytes, 17);
    mount
        .rows
        .insert(6, vec![node(2, 6, EntryKind::Directory, None)]);
    let mut result = FolderSize::default();
    let mut parent_address = address();
    parent_address.components = vec![b"parent".to_vec(), b"nested".to_vec()];
    assert!(compute(
        &mount,
        &parent_address,
        &[1, 2, 5],
        &[],
        &[node(6, 5, EntryKind::Directory, None)],
        &Cancellation::default(),
        &mut |_, size| {
            result = size;
            true
        }
    ));
    assert!(result.finished);
    assert_eq!(result.errors, 1);
    assert_eq!(result.bytes, 0);
}
