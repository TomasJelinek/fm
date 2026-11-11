use crate::{
    archive_source::ArchiveThumbnailSource,
    cache::{Cache, Key},
};
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, EntryKind, FsError, VirtualEntryId};
use midnight_thumbnail_api::Frame;
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Debug)]
struct Mount {
    id: u64,
    changed: AtomicBool,
    reads: AtomicUsize,
    cancel_during_read: Option<Arc<AtomicBool>>,
    short_read: bool,
    change_during_read: bool,
    stable: bool,
}
impl Mount {
    fn new(id: u64) -> Self {
        Self {
            id,
            changed: AtomicBool::new(false),
            reads: AtomicUsize::new(0),
            cancel_during_read: None,
            short_read: false,
            change_during_read: false,
            stable: true,
        }
    }
}
fn failure(kind: io::ErrorKind) -> FsError {
    FsError::from_io("fixture archive", Path::new(""), kind.into())
}
impl ArchiveMount for Mount {
    fn id(&self) -> u64 {
        self.id
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        unreachable!()
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.changed.load(Ordering::Acquire) {
            Err(failure(io::ErrorKind::InvalidData))
        } else {
            Ok(())
        }
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        self.validate()?;
        if expected != [1, 2, 3] {
            return Err(failure(io::ErrorKind::InvalidData));
        }
        Ok(self.stable.then(|| {
            [
                b"container/provider/version/zip/".as_slice(),
                expected,
                &entry.to_le_bytes(),
            ]
            .concat()
        }))
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.validate()?;
        assert!(
            !cancel.is_cancelled(),
            "a viewport cancellation reached the shared archive mount"
        );
        self.reads.fetch_add(1, Ordering::Relaxed);
        if let Some(cancel) = &self.cancel_during_read {
            cancel.store(true, Ordering::Release);
        }
        if self.change_during_read {
            self.changed.store(true, Ordering::Release);
        }
        let data = if entry == 2 { b"first" } else { b"other" };
        let length = if self.short_read {
            length.saturating_sub(1)
        } else {
            length
        };
        Ok(data[offset as usize..offset as usize + length as usize].to_vec())
    }
}
fn descriptor(mount: Arc<Mount>, entry: u64) -> ArchiveThumbnailSource {
    ArchiveThumbnailSource {
        node: ArchiveNode {
            id: VirtualEntryId {
                mount: mount.id(),
                entry,
            },
            parent: 1,
            raw_name: Arc::from(b"same.png".as_slice()),
            display_name: "same.png".into(),
            kind: EntryKind::File,
            size: Some(5),
            readable: true,
            modified: None,
        },
        mount,
        source_version: vec![1, 2, 3],
    }
}

#[test]
fn reopened_member_reuses_persistent_pixels_and_duplicate_names_stay_distinct() {
    let first = descriptor(Arc::new(Mount::new(8)), 2).capture()(&AtomicBool::new(false)).unwrap();
    let reopened =
        descriptor(Arc::new(Mount::new(9)), 2).capture()(&AtomicBool::new(false)).unwrap();
    let duplicate =
        descriptor(Arc::new(Mount::new(9)), 3).capture()(&AtomicBool::new(false)).unwrap();
    assert_eq!(first.version(), reopened.version());
    assert_ne!(first.version(), duplicate.version());
    assert_eq!(first.read_at(0, 5).unwrap(), b"first");
    assert_eq!(duplicate.read_at(0, 5).unwrap(), b"other");
    let root = tempfile::tempdir().unwrap();
    let cache_root = root.path().join("cache");
    let key = Key::new("image", b"archive-preview-v1", &*first, (96, 96), "1", 0).unwrap();
    Cache::open(&cache_root, 1_000_000, 0)
        .unwrap()
        .put(
            &key,
            &*first,
            Frame {
                id: 7,
                frame: 0,
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255],
            },
        )
        .unwrap();
    let mut cache = Cache::open(&cache_root, 1_000_000, 0).unwrap();
    assert_eq!(
        cache.get(&key, &*reopened).unwrap().unwrap().rgba,
        [1, 2, 3, 255]
    );
    let other_key = Key::new(
        "image",
        b"archive-preview-v1",
        &*duplicate,
        (96, 96),
        "1",
        0,
    )
    .unwrap();
    assert!(cache.get(&other_key, &*duplicate).unwrap().is_none());
}

#[test]
fn invalid_archive_descriptors_never_gain_source_authority() {
    let mount = Arc::new(Mount::new(8));
    for index in 0..7 {
        let mut source = descriptor(mount.clone(), 2);
        match index {
            0 => source.node.id.mount = 9,
            1 => source.node.id.entry = 0,
            2 => source.node.kind = EntryKind::Symlink,
            3 => source.node.readable = false,
            4 => source.node.size = None,
            5 => source.source_version = vec![4],
            _ => source.source_version.clear(),
        }
        assert!(
            source.capture()(&AtomicBool::new(false)).is_err(),
            "case {index}"
        );
    }
    let mount = Arc::new(Mount {
        stable: false,
        ..Mount::new(8)
    });
    assert!(descriptor(mount.clone(), 2).capture()(&AtomicBool::new(false)).is_err());
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn changed_sources_and_invalid_or_short_reads_fail_closed() {
    let mount = Arc::new(Mount::new(8));
    let source = descriptor(mount.clone(), 2).capture()(&AtomicBool::new(false)).unwrap();
    for (offset, length) in [(0, 0), (0, 256 * 1024 + 1), (4, 2), (u64::MAX, 1)] {
        assert!(source.read_at(offset, length).is_err());
    }
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
    mount.changed.store(true, Ordering::Release);
    assert!(source.validate().is_err());
    assert!(source.read_at(0, 1).is_err());
    let short = Arc::new(Mount {
        short_read: true,
        ..Mount::new(8)
    });
    let source = descriptor(short, 2).capture()(&AtomicBool::new(false)).unwrap();
    assert!(source.read_at(0, 5).is_err());
    let changed = Arc::new(Mount {
        change_during_read: true,
        ..Mount::new(8)
    });
    let source = descriptor(changed, 2).capture()(&AtomicBool::new(false)).unwrap();
    assert!(source.read_at(0, 5).is_err());
}

#[test]
fn cancelled_capture_does_not_read_or_poison_shared_mount() {
    let cancel = Arc::new(AtomicBool::new(true));
    let mount = Arc::new(Mount {
        cancel_during_read: Some(cancel.clone()),
        ..Mount::new(8)
    });
    let capture = descriptor(mount.clone(), 2).capture();
    assert!(matches!(capture(&cancel), Err(error) if error.kind() == io::ErrorKind::Interrupted));
    assert_eq!(mount.reads.load(Ordering::Relaxed), 0);
    cancel.store(false, Ordering::Release);
    let source = capture(&cancel).unwrap();
    // The already-admitted shared mount call drains; the decoder/service rejects publication
    // using its live cancellation flag after this read. The mount stays valid for navigation.
    assert_eq!(source.read_at(0, 5).unwrap(), b"first");
    assert!(cancel.load(Ordering::Acquire));
    assert!(mount.validate().is_ok());
    assert_eq!(
        mount.read(2, 0, 5, &Cancellation::default()).unwrap(),
        b"first"
    );
}

#[derive(Debug)]
struct CooperativeMount {
    inner: Mount,
    flag: Arc<AtomicBool>,
}
impl ArchiveMount for CooperativeMount {
    fn id(&self) -> u64 {
        self.inner.id()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.inner.list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.inner.read(entry, offset, length, cancel)
    }
    fn read_cooperative(
        &self,
        _: u64,
        _: u64,
        _: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        self.flag.store(true, Ordering::Release);
        assert!(cancelled(), "live request flag did not reach archive read");
        Err(FsError::from_io(
            "fixture",
            std::path::Path::new(""),
            io::ErrorKind::Interrupted.into(),
        ))
    }
    fn validate(&self) -> Result<(), FsError> {
        self.inner.validate()
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        revision: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        self.inner.persistent_member_identity(entry, revision)
    }
}
#[test]
fn archive_member_forwards_cooperative_cancel_without_using_hard_read() {
    let flag = Arc::new(AtomicBool::new(false));
    let inner = Arc::new(Mount::new(8));
    let mut request = descriptor(inner, 2);
    let mount = Arc::new(CooperativeMount {
        inner: Mount::new(8),
        flag: flag.clone(),
    });
    request.mount = mount.clone();
    let source = request.capture()(&AtomicBool::new(false)).unwrap();
    assert!(
        matches!(source.read_at_cooperative(0, 5, &|| flag.load(Ordering::Acquire)), Err(error) if error.kind() == io::ErrorKind::Interrupted)
    );
    assert_eq!(mount.inner.reads.load(Ordering::Relaxed), 0);
    assert!(mount.validate().is_ok());
}
