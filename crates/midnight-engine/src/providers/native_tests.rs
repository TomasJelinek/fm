use super::*;
use crate::providers::{ProviderError, ProviderRegistry};
use midnight_core::{
    DirectoryService, Entry, EntryId, EntryKind, EntryMetadata, FsError, SpaceInfo, StorageService,
    Volume, VolumeService,
};
use std::{io, path::PathBuf};

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
            Arc::new(NativeProvider::new(Arc::new(Files))),
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
