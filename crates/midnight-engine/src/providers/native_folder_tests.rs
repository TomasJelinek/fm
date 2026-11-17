//! Folder-only listings and presence probes do not enrich or bind regular files.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counted {
    files: usize,
    folder: bool,
    metadata: AtomicUsize,
}
impl DirectoryService for Counted {
    fn read_directory(
        &self,
        path: &Path,
        _cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        for index in 0..self.files {
            let name = format!("file {index}");
            if !emit(Entry::new(
                EntryId(index as u64),
                path.join(name),
                EntryKind::File,
            )) {
                return Ok(());
            }
        }
        if self.folder {
            emit(Entry::new(
                EntryId(u64::MAX),
                path.join("folder"),
                EntryKind::Directory,
            ));
        }
        Ok(())
    }
    fn entry_metadata(&self, _path: &Path) -> Result<EntryMetadata, FsError> {
        self.metadata.fetch_add(1, Ordering::SeqCst);
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Counted {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Counted {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(FsError::from_io(
            "test",
            path,
            io::ErrorKind::Unsupported.into(),
        ))
    }
}

fn counted(files: usize, folder: bool) -> Arc<Counted> {
    Arc::new(Counted {
        files,
        folder,
        metadata: AtomicUsize::new(0),
    })
}

#[test]
fn folder_only_listing_skips_files_before_metadata_and_token_binding() {
    let files = counted(3, true);
    let mount = NativeMount {
        filesystem: files.clone(),
        capture: None,
        locale: None,
        nodes: Mutex::new(NativeNodes::default()),
        audited: false,
    };
    let root = mount
        .nodes
        .lock()
        .unwrap()
        .bind(path(), EntryKind::Directory)
        .unwrap();
    let mut rows = Vec::new();
    mount
        .list_folders(&root, &Cancellation::default(), &mut |row| {
            rows.push(row);
            true
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name, "folder");
    assert_eq!(files.metadata.load(Ordering::SeqCst), 1);
    assert_eq!(
        mount.nodes.lock().unwrap().node_count(),
        2,
        "regular files must not spend the mount's node budget"
    );
    mount
        .list(&root, &Cancellation::default(), &mut |_| true)
        .unwrap();
    assert_eq!(mount.nodes.lock().unwrap().node_count(), 5);
}

#[test]
fn registry_presence_probe_stops_at_the_first_folder_without_issuing_references() {
    let registry = ProviderRegistry::default();
    let address = native_location(&path()).unwrap();
    let with_folder = counted(2, true);
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(with_folder.clone())),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    assert_eq!(
        location.has_folders(location.root(), &Cancellation::default()),
        Ok(true)
    );
    assert_eq!(with_folder.metadata.load(Ordering::SeqCst), 1);
    let rows = location
        .list_folders(location.root(), &Cancellation::default(), 8)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        location.has_folders(location.root(), &cancel),
        Err(ProviderError::Cancelled)
    );

    let files_only = ProviderRegistry::default();
    files_only
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(counted(4, false))),
        )
        .unwrap();
    let location = files_only
        .resolve(&address, &Cancellation::default())
        .unwrap();
    assert_eq!(
        location.has_folders(location.root(), &Cancellation::default()),
        Ok(false)
    );
}
