use super::*;
use midnight_core::{
    DirectoryService, Entry, EntryId, EntryMetadata, FsError, LocalizedDateTime, SpaceInfo,
    StorageService, Volume, VolumeService,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(windows)]
#[path = "native_seed_raw_windows.rs"]
mod raw;
#[cfg(target_os = "linux")]
#[path = "native_seed_raw_linux.rs"]
mod raw;

#[derive(Default)]
struct Files {
    lists: AtomicUsize,
    stats: AtomicUsize,
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.lists.fetch_add(1, Ordering::AcqRel);
        emit(Entry::new(
            EntryId(99),
            path.join("live child"),
            EntryKind::Directory,
        ));
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        self.stats.fetch_add(1, Ordering::AcqRel);
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Files {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(FsError::from_io(
            "test",
            path,
            std::io::ErrorKind::Unsupported.into(),
        ))
    }
}
fn root() -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("_build/native-root-seed")
}
fn entries() -> Vec<Entry> {
    let mut file = Entry::new(
        EntryId(7),
        root().join("東京 % quoted name.txt"),
        EntryKind::File,
    );
    file.display_name = "presentation only".into();
    file.metadata = EntryMetadata {
        size: Some(17),
        modified: Some(std::time::UNIX_EPOCH),
        localized_modified: Some(LocalizedDateTime {
            date: "captured date".into(),
            time: "captured time".into(),
        }),
        ..Default::default()
    };
    vec![
        file,
        Entry::new(EntryId(9), root().join("folder"), EntryKind::Directory),
    ]
}
fn rows(
    opened: &OpenedLocation,
    node: &NodeToken,
    cancel: &Cancellation,
) -> Result<Vec<ProviderEntry>, ProviderError> {
    let mut rows = Vec::new();
    opened.mount.list(node, cancel, &mut |row| {
        rows.push(row);
        true
    })?;
    Ok(rows)
}

#[test]
fn native_seed_preserves_exact_rows_without_listing_or_stat_and_delegates_children() {
    let files = Arc::new(Files::default());
    let provider = NativeProvider::new(files.clone());
    let original = entries();
    let opened = provider
        .capture_root(&root(), &original, &Cancellation::default())
        .unwrap();
    let rows = rows(&opened, &opened.root, &Cancellation::default()).unwrap();
    assert_eq!(rows.len(), 2);
    for (row, expected) in rows.iter().zip(&original) {
        assert_eq!(
            row.raw_name.as_ref(),
            codec::encode(&expected.name).unwrap()
        );
        assert_eq!(row.display_name, expected.display_name);
        assert_eq!(row.metadata, expected.metadata);
        let super::super::ProviderTarget::Native(path) = opened
            .mount
            .resolve_target(&row.token, &Cancellation::default())
            .unwrap()
        else {
            panic!("native target required")
        };
        assert_eq!(path, expected.path);
    }
    assert_eq!(files.lists.load(Ordering::Acquire), 0);
    assert_eq!(files.stats.load(Ordering::Acquire), 0);
    let child = self::rows(&opened, &rows[1].token, &Cancellation::default()).unwrap();
    assert_eq!(child[0].display_name, "live child");
    assert_eq!(files.lists.load(Ordering::Acquire), 1);
    assert_eq!(files.stats.load(Ordering::Acquire), 0);
    let refreshed = self::rows(&opened, &opened.root, &Cancellation::default()).unwrap();
    assert_eq!(refreshed.len(), 1);
    assert_eq!(refreshed[0].display_name, "live child");
    assert_eq!(files.lists.load(Ordering::Acquire), 2);
}

#[test]
fn native_seed_is_immutable_and_precancelled_read_does_not_consume_it() {
    let provider = NativeProvider::new(Arc::new(Files::default()));
    let mut original = entries();
    let opened = provider
        .capture_root(&root(), &original, &Cancellation::default())
        .unwrap();
    original[0].metadata.size = Some(999);
    original[0].path = root().join("replacement");
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(matches!(
        rows(&opened, &opened.root, &cancelled),
        Err(ProviderError::Cancelled)
    ));
    let rows = rows(&opened, &opened.root, &Cancellation::default()).unwrap();
    assert_eq!(rows[0].metadata.size, Some(17));
    assert_eq!(
        rows[0].raw_name.as_ref(),
        codec::encode(&entries()[0].name).unwrap()
    );
}

#[test]
fn native_seed_rejects_mismatched_or_duplicate_snapshots_without_io_or_retained_owners() {
    let files = Arc::new(Files::default());
    let provider = NativeProvider::new(files.clone());
    let owners = Arc::strong_count(&files);
    for mode in 0..5 {
        let mut bad = entries();
        match mode {
            0 => bad[1].id = bad[0].id,
            1 => {
                bad[1].path = bad[0].path.clone();
                bad[1].name = bad[0].name.clone();
            }
            2 => bad[0].path = root().parent().unwrap().join("outside"),
            3 => bad[0].name = "different raw name".into(),
            _ => bad[0].path = PathBuf::from("relative"),
        }
        assert!(
            matches!(
                provider.capture_root(&root(), &bad, &Cancellation::default()),
                Err(ProviderError::InvalidResponse(_))
            ),
            "mode {mode}"
        );
        assert_eq!(Arc::strong_count(&files), owners);
    }
    assert_eq!(files.lists.load(Ordering::Acquire), 0);
    assert_eq!(files.stats.load(Ordering::Acquire), 0);
}

#[test]
fn native_seed_budgets_and_cancellation_fail_before_publication() {
    let provider = NativeProvider::new(Arc::new(Files::default()));
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(matches!(
        provider.capture_root(&root(), &entries(), &cancelled),
        Err(ProviderError::Cancelled)
    ));
    let oversized = vec![entries().remove(0); 50_001];
    assert!(matches!(
        provider.capture_root(&root(), &oversized, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    let mut oversized = entries();
    oversized[0].display_name = "x".repeat(32 * 1024 * 1024 + 1);
    assert!(matches!(
        provider.capture_root(&root(), &oversized, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
}

fn assert_raw_seed_name(name: std::ffi::OsString) {
    let files = Arc::new(Files::default());
    let provider = NativeProvider::new(files.clone());
    let entry = Entry::new(EntryId(11), root().join(&name), EntryKind::File);
    let opened = provider
        .capture_root(
            &root(),
            std::slice::from_ref(&entry),
            &Cancellation::default(),
        )
        .unwrap();
    let captured = rows(&opened, &opened.root, &Cancellation::default()).unwrap();
    assert_eq!(captured[0].raw_name.as_ref(), codec::encode(&name).unwrap());
    let super::super::ProviderTarget::Native(path) = opened
        .mount
        .resolve_target(&captured[0].token, &Cancellation::default())
        .unwrap()
    else {
        panic!("native target required")
    };
    assert_eq!(path, entry.path);
    assert_eq!(files.lists.load(Ordering::Acquire), 0);
    assert_eq!(files.stats.load(Ordering::Acquire), 0);
}

#[test]
fn native_seed_cancelled_publication_is_not_successful_or_replayable() {
    let provider = NativeProvider::new(Arc::new(Files::default()));
    let opened = provider
        .capture_root(&root(), &entries(), &Cancellation::default())
        .unwrap();
    let cancel = Cancellation::default();
    let mut emitted = 0;
    assert!(matches!(
        opened.mount.list(&opened.root, &cancel, &mut |_| {
            emitted += 1;
            cancel.cancel();
            true
        }),
        Err(ProviderError::Cancelled)
    ));
    assert_eq!(emitted, 1);
    let refreshed = rows(&opened, &opened.root, &Cancellation::default()).unwrap();
    assert_eq!(refreshed.len(), 1);
    assert_eq!(refreshed[0].display_name, "live child");
}

#[test]
fn native_seed_content_uses_injected_capture_and_never_a_fallback() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let calls = captured.clone();
    let provider =
        NativeProvider::new(Arc::new(Files::default())).with_capture(Arc::new(move |path, _| {
            calls.lock().unwrap().push(path.to_owned());
            Err(ProviderError::Unavailable)
        }));
    let opened = provider
        .capture_root(&root(), &entries(), &Cancellation::default())
        .unwrap();
    let rows = rows(&opened, &opened.root, &Cancellation::default()).unwrap();
    assert!(matches!(
        opened
            .mount
            .open_content(&rows[0].token, &Cancellation::default()),
        Err(ProviderError::Unavailable)
    ));
    assert_eq!(*captured.lock().unwrap(), [entries()[0].path.clone()]);
}

#[test]
fn native_seed_refuses_virtual_rows_nul_paths_and_hidden_large_capacity() {
    let provider = NativeProvider::new(Arc::new(Files::default()));
    let mut bad = entries();
    bad[0].virtual_node = Some(midnight_core::ArchiveNode {
        modified: None,
        id: midnight_core::VirtualEntryId { mount: 1, entry: 2 },
        parent: 1,
        raw_name: Arc::from(&b"virtual"[..]),
        display_name: "virtual".into(),
        kind: EntryKind::File,
        size: Some(5),
        readable: true,
    });
    assert!(matches!(
        provider.capture_root(&root(), &bad, &Cancellation::default()),
        Err(ProviderError::InvalidResponse(_))
    ));
    bad = entries();
    bad[0] = Entry::new(EntryId(7), root().join("bad\0name"), EntryKind::File);
    assert!(matches!(
        provider.capture_root(&root(), &bad, &Cancellation::default()),
        Err(ProviderError::InvalidResponse(_))
    ));
    bad = entries();
    bad[0].display_name = String::with_capacity(32 * 1024 * 1024 + 1);
    bad[0].display_name.push('x');
    assert!(matches!(
        provider.capture_root(&root(), &bad, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    let oversized_path = root().join("x".repeat(32 * 1024 * 1024 + 1));
    assert!(matches!(
        provider.capture_root(&oversized_path, &[], &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
}
