//! Folder-only listings and presence probes do not enrich or bind regular files.
//! Reclamation forgets tokens the lister no longer holds and never reuses one.
use super::*;
use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
};

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
        mount.nodes.lock().unwrap().len(),
        2,
        "regular files must not spend the mount's node budget"
    );
    mount
        .list(&root, &Cancellation::default(), &mut |_| true)
        .unwrap();
    assert_eq!(mount.nodes.lock().unwrap().len(), 5);
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

fn counted_mount(files: usize, folder: bool) -> (NativeMount, NodeToken) {
    let mount = NativeMount {
        filesystem: counted(files, folder),
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
    (mount, root)
}

fn complete_listing(mount: &NativeMount, root: &NodeToken) -> Vec<crate::providers::ProviderEntry> {
    let mut rows = Vec::new();
    mount
        .list(root, &Cancellation::default(), &mut |row| {
            rows.push(row);
            true
        })
        .unwrap();
    rows
}

fn encoded_len(path: &Path) -> usize {
    path.as_os_str().as_encoded_bytes().len()
}

#[test]
fn retained_native_nodes_forget_others_and_never_reuse_a_token() {
    let (mount, root) = counted_mount(3, true);
    let first = complete_listing(&mount, &root);
    assert_eq!(first.len(), 4);
    assert_eq!(mount.node_count(), Some(5));
    let folder = first
        .iter()
        .find(|row| row.kind == EntryKind::Directory)
        .unwrap()
        .token
        .clone();
    let file = first[0].token.clone();
    mount
        .retain_nodes(&HashSet::from([root.clone(), folder.clone()]))
        .unwrap();
    assert_eq!(mount.node_count(), Some(2));
    assert!(matches!(
        mount.resolve_target(&file, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    assert!(matches!(
        mount.resolve_target(&folder, &Cancellation::default()),
        Ok(crate::providers::ProviderTarget::Native(target)) if target == path().join("folder")
    ));
    assert_eq!(
        mount.nodes.lock().unwrap().path_bytes(),
        encoded_len(&path()) + encoded_len(&path().join("folder")),
        "only kept paths remain charged"
    );

    let second = complete_listing(&mount, &root);
    let first_tokens = first
        .iter()
        .map(|row| row.token.clone())
        .collect::<HashSet<_>>();
    for row in &second {
        if row.kind == EntryKind::Directory {
            assert_eq!(row.token, folder, "a kept node keeps its token");
        } else {
            assert!(
                !first_tokens.contains(&row.token),
                "a forgotten file received its released token again"
            );
        }
    }
    assert_eq!(mount.node_count(), Some(5));
}

#[test]
fn reclaimed_paths_map_to_their_newest_kept_token() {
    let mut nodes = NativeNodes::default();
    let root = nodes.bind(path(), EntryKind::Directory).unwrap();
    let flip = path().join("flip");
    let directory = nodes.bind(flip.clone(), EntryKind::Directory).unwrap();
    let file = nodes.bind(flip.clone(), EntryKind::File).unwrap();
    assert_ne!(directory, file, "a kind change binds a new token");

    nodes.retain(&HashSet::from([
        root.clone(),
        directory.clone(),
        file.clone(),
    ]));
    assert_eq!(
        nodes.bind(flip.clone(), EntryKind::File).unwrap(),
        file,
        "the newest binding of a path stays reachable"
    );

    nodes.retain(&HashSet::from([root.clone(), directory.clone()]));
    assert_eq!(
        nodes.bind(flip.clone(), EntryKind::Directory).unwrap(),
        directory,
        "an older kept token is reachable again once the newer one is released"
    );
    let again = nodes.bind(flip, EntryKind::File).unwrap();
    assert!(
        again != file && again != directory,
        "a released token value is never minted again"
    );
}

#[test]
fn native_reclamation_returns_spare_table_capacity_after_a_burst() {
    let mut nodes = NativeNodes::default();
    let root = nodes.bind(path(), EntryKind::Directory).unwrap();
    for index in 0..10_000 {
        nodes
            .bind(path().join(format!("burst {index}")), EntryKind::File)
            .unwrap();
    }
    let (tokens, paths) = nodes.capacities();
    assert!(tokens >= 10_001 && paths >= 10_001);
    nodes.retain(&HashSet::from([root]));
    assert_eq!(nodes.len(), 1);
    let (tokens, paths) = nodes.capacities();
    assert!(
        tokens < 64 && paths < 64,
        "a reclaimed burst keeps {tokens}/{paths} slots"
    );
}
