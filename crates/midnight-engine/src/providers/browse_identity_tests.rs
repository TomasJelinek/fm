//! Injected native-service boundaries; no host filesystem fixtures are needed here.
use super::*;
use midnight_core::{
    DirectoryService, Entry, EntryId, FsError, ProviderInstanceId, SizeDirectoryTarget, SpaceInfo,
    StorageService, Volume, VolumeService,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Files {
    root: PathBuf,
    mode: AtomicUsize,
    identity_paths: Mutex<Vec<PathBuf>>,
    listing_paths: Mutex<Vec<PathBuf>>,
}

impl Files {
    fn new() -> Self {
        Self {
            root: std::env::temp_dir().join("midnight-browse-identity-injected"),
            mode: AtomicUsize::new(0),
            identity_paths: Mutex::default(),
            listing_paths: Mutex::default(),
        }
    }
}

impl DirectoryService for Files {
    fn resolve_size_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        self.identity_paths.lock().unwrap().push(path.to_owned());
        match self.mode.load(Ordering::Relaxed) {
            1 => return Ok(None),
            2 => {
                return Err(FsError::from_io(
                    "resolve fixture",
                    path,
                    std::io::ErrorKind::Unsupported.into(),
                ));
            }
            3 => {
                return Err(FsError::from_io(
                    "resolve fixture",
                    path,
                    std::io::ErrorKind::PermissionDenied.into(),
                ));
            }
            4 => cancel.cancel(),
            _ => {}
        }
        let value = if path == self.root {
            0_u128
        } else if path == self.root.join("alias") {
            1
        } else {
            2
        };
        Ok(Some(SizeDirectoryTarget {
            path: self
                .root
                .join("resolved-target-must-never-be-an-action-path"),
            identity: SizeDirectoryIdentity {
                device: 17,
                object: value.to_le_bytes(),
            },
        }))
    }

    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.listing_paths.lock().unwrap().push(path.to_owned());
        if path == self.root {
            emit(Entry::new(
                EntryId(1),
                path.join("alias"),
                EntryKind::DirectoryLink,
            ));
        } else if path == self.root.join("alias") {
            emit(Entry::new(
                EntryId(2),
                path.join("child"),
                EntryKind::Directory,
            ));
        }
        Ok(())
    }

    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
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
            "space fixture",
            path,
            std::io::ErrorKind::Unsupported.into(),
        ))
    }
}

struct Opened(Mutex<Option<OpenedLocation>>);
impl FilesystemProvider for Opened {
    fn resolve(
        &self,
        _: &StoredLocation,
        _: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        self.0
            .lock()
            .unwrap()
            .take()
            .ok_or(ProviderError::StaleReference)
    }
}

fn mounted(
    files: Arc<Files>,
    captured: bool,
) -> (
    ProviderRegistry,
    StoredLocation,
    ProviderInstanceId,
    MountedLocation,
) {
    let address = native_location(&files.root).unwrap();
    let native = NativeProvider::new(files.clone());
    let cancel = Cancellation::default();
    let opened = if captured {
        native
            .capture_root(
                &files.root,
                &[Entry::new(
                    EntryId(1),
                    files.root.join("alias"),
                    EntryKind::DirectoryLink,
                )],
                &cancel,
            )
            .unwrap()
    } else {
        native.resolve(&address, &cancel).unwrap()
    };
    let registry = ProviderRegistry::default();
    let instance = registry
        .register(
            address.provider().clone(),
            Arc::new(Opened(Mutex::new(Some(opened)))),
        )
        .unwrap();
    let location = registry.resolve(&address, &cancel).unwrap();
    (registry, address, instance, location)
}

#[test]
fn default_browse_hook_keeps_non_directory_unsupported_and_denied_distinct() {
    let files = Files::new();
    let cancel = Cancellation::default();
    assert_eq!(
        files
            .browse_directory_identity(&files.root, &cancel)
            .unwrap()
            .unwrap()
            .device,
        17
    );
    files.mode.store(1, Ordering::Relaxed);
    assert_eq!(
        files
            .browse_directory_identity(&files.root, &cancel)
            .unwrap(),
        None
    );
    for (mode, kind) in [
        (2, std::io::ErrorKind::Unsupported),
        (3, std::io::ErrorKind::PermissionDenied),
    ] {
        files.mode.store(mode, Ordering::Relaxed);
        assert_eq!(
            files
                .browse_directory_identity(&files.root, &cancel)
                .unwrap_err()
                .kind,
            kind
        );
    }
}

#[test]
fn captured_native_root_forwards_identities_and_link_actions_keep_logical_paths() {
    use super::tree::{DirectoryTree, TreeLimits, TreeRequest};
    let files = Arc::new(Files::new());
    let (_registry, _address, _instance, location) = mounted(files.clone(), true);
    let mut tree = DirectoryTree::new(
        location,
        TreeLimits::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    let result = tree
        .request(tree.root(), TreeRequest::Expand)
        .unwrap()
        .run();
    assert!(tree.apply(result));
    assert!(
        files.listing_paths.lock().unwrap().is_empty(),
        "root uses its captured listing"
    );
    let link = tree
        .rows()
        .into_iter()
        .find(|row| row.entry.kind == EntryKind::DirectoryLink)
        .unwrap();
    let result = tree.request(link.id, TreeRequest::Expand).unwrap().run();
    assert!(tree.apply(result));
    let child = tree
        .rows()
        .into_iter()
        .find(|row| row.parent == Some(link.id))
        .unwrap();
    let target = tree
        .resolve_entry_target(child.id, &Cancellation::default())
        .unwrap();
    let ProviderTarget::Native(path) = target.value().unwrap() else {
        panic!("native target");
    };
    assert_eq!(path, &files.root.join("alias").join("child"));
    assert_eq!(
        *files.identity_paths.lock().unwrap(),
        [
            files.root.clone(),
            files.root.clone(),
            files.root.join("alias"),
            files.root.join("alias")
        ]
    );
    assert_eq!(
        *files.listing_paths.lock().unwrap(),
        [files.root.join("alias")]
    );
}

#[test]
fn native_identity_bridge_checks_cancellation_and_revoked_mount_ownership() {
    let files = Arc::new(Files::new());
    let (registry, address, instance, location) = mounted(files.clone(), false);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        location.browse_directory_identity(location.root(), &cancel),
        Err(ProviderError::Cancelled)
    );
    assert!(files.identity_paths.lock().unwrap().is_empty());
    files.mode.store(4, Ordering::Relaxed);
    let cancel = Cancellation::default();
    assert_eq!(
        location.browse_directory_identity(location.root(), &cancel),
        Err(ProviderError::Cancelled)
    );
    assert!(
        !cancel.is_cancelled(),
        "mount request owns its local cancellation"
    );
    assert!(registry.unregister(address.provider(), instance).unwrap());
    assert_eq!(
        location.browse_directory_identity(location.root(), &Cancellation::default()),
        Err(ProviderError::StaleReference)
    );
    assert_eq!(files.identity_paths.lock().unwrap().len(), 1);
}
