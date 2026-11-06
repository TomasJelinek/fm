use midnight_core::{Cancellation, DirectoryService, FileSystem, StorageService, VolumeService};
use midnight_platform::LocalFileSystem;
use std::fs;

#[test]
fn selected_adapter_serves_independent_service_trait_objects() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ověření_中文_🧪.txt");
    fs::write(&path, b"service contract").unwrap();
    let adapter = LocalFileSystem::new(Some(root.path().to_owned()));
    let filesystem: &dyn FileSystem = &adapter;
    let directory: &dyn DirectoryService = filesystem;
    let volumes: &dyn VolumeService = filesystem;
    let storage: &dyn StorageService = filesystem;

    let mut entries = Vec::new();
    directory
        .read_directory(root.path(), &Cancellation::default(), &mut |entry| {
            entries.push(entry);
            true
        })
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, path);
    assert_eq!(entries[0].metadata.size, None);
    assert_eq!(directory.entry_metadata(&path).unwrap().size, Some(16));
    assert!(
        volumes
            .list_volumes()
            .unwrap()
            .iter()
            .any(|volume| { volume.id == "home" && volume.root == root.path() })
    );
    let space = storage.space_info(root.path()).unwrap();
    assert!(space.total > 0);
    assert!(space.available <= space.total);
}
