use super::*;
use crate::providers::{ProviderError, ProviderRegistry};
use midnight_core::{
    DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, FsError, SpaceInfo, StorageService,
    Volume, VolumeService,
};
use std::{io, path::PathBuf};

#[path = "native_tree_target_tests.rs"]
mod tree_targets;

struct Files;
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        for (index, (name, kind)) in [
            ("same name", EntryKind::File),
            ("folder", EntryKind::Directory),
        ]
        .into_iter()
        .enumerate()
        {
            if !emit(Entry::new(EntryId(index as u64 + 1), path.join(name), kind)) {
                break;
            }
        }
        Ok(())
    }
    fn entry_metadata(&self, _path: &Path) -> Result<EntryMetadata, FsError> {
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
            io::ErrorKind::Unsupported.into(),
        ))
    }
}

fn path() -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("_build")
        .join("provider-codec-test")
}

#[test]
fn native_address_uses_lossless_codec_and_rejects_relative_paths() {
    assert!(native_location(Path::new("relative")).is_err());
    let path = path().join("Žluťoučký kůň 東京");
    let stored = native_location(&path).unwrap();
    assert_eq!(stored.provider().as_str(), codec::PROVIDER);
    assert_eq!(codec::decode(stored.address()).unwrap(), path);
}

#[test]
fn native_listing_is_reusable_without_turning_captions_into_paths() {
    let registry = ProviderRegistry::default();
    let location = native_location(&path()).unwrap();
    registry
        .register(
            location.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(Files))),
        )
        .unwrap();
    let mount = registry
        .resolve(&location, &Cancellation::default())
        .unwrap();
    let rows = mount.list(mount.root(), &Cancellation::default()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].entry.display_name, "same name");
    assert_eq!(
        mount.list(mount.root(), &Cancellation::default()).unwrap()[0].node,
        rows[0].node
    );
    assert_eq!(
        mount
            .parent(&rows[0].node, &Cancellation::default())
            .unwrap()
            .as_ref(),
        Some(mount.root())
    );
    assert!(matches!(
        mount.open_content(&rows[1].node, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn native_targets_preserve_exact_paths_and_retained_targets_reject_provider_loss() {
    let registry = ProviderRegistry::default();
    let root = path().join("東京 Žluťoučký");
    let address = native_location(&root).unwrap();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(Files))),
        )
        .unwrap();
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let rows = mount.list(mount.root(), &Cancellation::default()).unwrap();
    let target = mount
        .resolve_target(&rows[0].node, &Cancellation::default())
        .unwrap();
    let super::super::ProviderTarget::Native(exact) = target.value().unwrap() else {
        panic!("native target must not become an archive");
    };
    assert_eq!(exact, &root.join("same name"));
    drop(registry);
    assert!(matches!(target.value(), Err(ProviderError::StaleReference)));
}

#[test]
fn native_tree_navigation_retains_exact_provider_parent_and_root_files() {
    use crate::providers::tree::{DirectoryTree, TreeLimits, TreeRequest};
    let registry = ProviderRegistry::default();
    let address = native_location(&path().join("東京 Žluťoučký")).unwrap();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(Files))),
        )
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
    assert_eq!(rows.len(), 2);
    let folder = rows
        .iter()
        .find(|row| row.entry.kind == EntryKind::Directory)
        .unwrap();
    let result = tree.request(folder.id, TreeRequest::Expand).unwrap().run();
    assert!(tree.apply(result));
    let rows = tree.rows();
    assert_eq!(rows.len(), 3);
    let descendant = rows.iter().find(|row| row.depth == 1).unwrap();
    let (mount, node) = tree.navigation_target(descendant.id).unwrap();
    assert_eq!(
        mount.parent(&node, &Cancellation::default()).unwrap(),
        Some(folder.node.clone())
    );
    assert_eq!(mount.address(), &address);
}

#[test]
fn native_content_broker_reads_exact_file_and_rejects_source_change() {
    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path().join("Žluťoučký 東京");
    std::fs::create_dir(&directory).unwrap();
    let source = directory.join("same name");
    std::fs::write(&source, b"original content").unwrap();
    let address = native_location(&directory).unwrap();
    let registry = ProviderRegistry::default();
    registry
        .register(
            address.provider().clone(),
            Arc::new(
                NativeProvider::new(Arc::new(Files)).with_capture(Arc::new(|path, _| {
                    CapturedFile::capture(path)
                        .map_err(|error| ProviderError::Failed(error.to_string()))
                })),
            ),
        )
        .unwrap();
    let mounted = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let rows = mounted
        .list(mounted.root(), &Cancellation::default())
        .unwrap();
    let content = mounted
        .open_content(&rows[0].node, &Cancellation::default())
        .unwrap();
    let captured = CapturedFile::from_source(content).unwrap();
    drop(mounted);
    assert_eq!(captured.read_at(0, 256).unwrap(), b"original content");
    assert_eq!(captured.read_at(9, 256).unwrap(), b"content");
    std::fs::write(&source, b"changed source with a different length").unwrap();
    assert!(captured.read_at(0, 256).is_err());
}

#[test]
fn a_browse_only_native_adapter_cannot_open_files_through_a_hidden_fallback() {
    let registry = ProviderRegistry::default();
    let address = native_location(&path()).unwrap();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(Files))),
        )
        .unwrap();
    let mounted = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let file = mounted
        .list(mounted.root(), &Cancellation::default())
        .unwrap()
        .remove(0);
    assert!(matches!(
        mounted.open_content(&file.node, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn native_read_capture_calls_the_injected_provider_and_keeps_its_failure() {
    let registry = ProviderRegistry::default();
    let address = native_location(&path()).unwrap();
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let calls = captured.clone();
    let provider =
        NativeProvider::new(Arc::new(Files)).with_capture(Arc::new(move |path, cancel| {
            assert!(!cancel.is_cancelled());
            calls.lock().unwrap().push(path.to_owned());
            Err(ProviderError::Unavailable)
        }));
    registry
        .register(address.provider().clone(), Arc::new(provider))
        .unwrap();
    let mounted = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let file = mounted
        .list(mounted.root(), &Cancellation::default())
        .unwrap()
        .remove(0);
    assert!(matches!(
        mounted.open_content(&file.node, &Cancellation::default()),
        Err(ProviderError::Unavailable)
    ));
    assert_eq!(*captured.lock().unwrap(), [path().join("same name")]);
}

#[test]
fn native_capture_cancellation_discards_a_late_successful_source() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("same name"), b"captured bytes").unwrap();
    let address = native_location(fixture.path()).unwrap();
    let registry = ProviderRegistry::default();
    let provider = NativeProvider::new(Arc::new(Files)).with_capture(Arc::new(|path, cancel| {
        let source = CapturedFile::capture(path)
            .map_err(|error| ProviderError::Failed(error.to_string()))?;
        cancel.cancel();
        Ok(source)
    }));
    registry
        .register(address.provider().clone(), Arc::new(provider))
        .unwrap();
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let row = mount
        .list(mount.root(), &Cancellation::default())
        .unwrap()
        .remove(0);
    let outer = Cancellation::default();
    assert!(matches!(
        mount.open_content(&row.node, &outer),
        Err(ProviderError::Cancelled)
    ));
    assert!(
        !outer.is_cancelled(),
        "local request cancellation must not cancel its caller"
    );
    assert!(
        mount.list(mount.root(), &outer).is_ok(),
        "request cancellation must not revoke the mount"
    );
}

struct MetadataFiles {
    complete: bool,
    stats: Arc<std::sync::atomic::AtomicUsize>,
}
impl DirectoryService for MetadataFiles {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        let mut entry = Entry::new(EntryId(1), path.join("name"), EntryKind::File);
        if self.complete {
            entry.metadata.modified = Some(std::time::UNIX_EPOCH);
        }
        emit(entry);
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        self.complete
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        self.stats
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(EntryMetadata {
            modified: Some(std::time::UNIX_EPOCH),
            ..EntryMetadata::default()
        })
    }
}
impl VolumeService for MetadataFiles {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Files.list_volumes()
    }
}
impl StorageService for MetadataFiles {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Files.space_info(path)
    }
}
struct Locale;
impl midnight_core::LocaleService for Locale {
    fn number_locale(&self) -> Result<midnight_core::NumberLocale, FsError> {
        Ok(Default::default())
    }
    fn format_datetime(
        &self,
        _: std::time::SystemTime,
    ) -> Option<midnight_core::LocalizedDateTime> {
        Some(midnight_core::LocalizedDateTime {
            date: "local date".into(),
            time: "local time".into(),
        })
    }
}

#[test]
fn native_registry_listing_keeps_locale_parity_without_reopening_complete_rows() {
    for complete in [true, false] {
        let stats = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let provider = NativeProvider::new(Arc::new(MetadataFiles {
            complete,
            stats: stats.clone(),
        }))
        .with_locale(Arc::new(Locale));
        let opened = provider
            .resolve(&native_location(&path()).unwrap(), &Cancellation::default())
            .unwrap();
        let mut rows = Vec::new();
        opened
            .mount
            .list(&opened.root, &Cancellation::default(), &mut |entry| {
                rows.push(entry);
                true
            })
            .unwrap();
        let formatted = rows[0].metadata.localized_modified.as_ref().unwrap();
        assert_eq!(formatted.date, "local date");
        assert_eq!(formatted.time, "local time");
        assert_eq!(
            stats.load(std::sync::atomic::Ordering::Relaxed),
            usize::from(!complete)
        );
    }
}

#[test]
fn foreign_or_unknown_native_codec_never_resolves_as_current_platform() {
    let native = native_location(&path()).unwrap();
    let provider = NativeProvider::new(Arc::new(Files));
    let foreign = StoredLocation::new(
        ProviderId::new("native.foreign").unwrap(),
        native.codec_version(),
        Arc::from(native.address()),
    );
    assert!(matches!(
        provider.resolve(&foreign, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
    let future = StoredLocation::new(
        native.provider().clone(),
        NonZeroU32::new(2).unwrap(),
        Arc::from(native.address()),
    );
    assert!(matches!(
        provider.resolve(&future, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn synthetic_parent_entry_cannot_be_issued_as_a_native_child() {
    struct ParentEntry;
    impl DirectoryService for ParentEntry {
        fn read_directory(
            &self,
            path: &Path,
            _cancel: &Cancellation,
            emit: &mut dyn FnMut(Entry) -> bool,
        ) -> Result<(), FsError> {
            emit(Entry::new(
                EntryId(1),
                path.join(".."),
                EntryKind::Directory,
            ));
            Ok(())
        }
        fn entry_metadata(&self, _path: &Path) -> Result<EntryMetadata, FsError> {
            Ok(EntryMetadata::default())
        }
    }
    impl VolumeService for ParentEntry {
        fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
            Ok(Vec::new())
        }
    }
    impl StorageService for ParentEntry {
        fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
            Err(FsError::from_io(
                "test",
                path,
                io::ErrorKind::Unsupported.into(),
            ))
        }
    }
    let registry = ProviderRegistry::default();
    let address = native_location(&path()).unwrap();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(ParentEntry))),
        )
        .unwrap();
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    assert!(matches!(
        mount.list(mount.root(), &Cancellation::default()),
        Err(ProviderError::InvalidResponse(_))
    ));
}

#[test]
fn native_address_budget_is_checked_before_codec_allocations() {
    let provider = NativeProvider::new(Arc::new(Files));
    let address = native_location(&path()).unwrap();
    // All codecs need at least one native byte for every two encoded bytes.
    // The invalid NUL payload must hit admission before a decoder inspects it.
    let oversized = StoredLocation::new(
        address.provider().clone(),
        address.codec_version(),
        Arc::from(vec![0; 64 * 1024 * 1024 + 1]),
    );
    assert!(matches!(
        provider.resolve(&oversized, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    drop(oversized);
    let oversized_native = path().join("x".repeat(32 * 1024 * 1024));
    assert!(matches!(
        native_location(&oversized_native),
        Err(ProviderError::ResourceLimited)
    ));
}
