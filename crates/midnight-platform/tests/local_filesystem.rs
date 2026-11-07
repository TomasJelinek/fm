use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryKind, StorageService, VolumeService,
};
use midnight_platform::LocalFileSystem;
use std::{collections::HashSet, fs, io::ErrorKind, path::Path};

fn list(provider: &LocalFileSystem, path: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    provider
        .read_directory(path, &Cancellation::default(), &mut |entry| {
            entries.push(entry);
            true
        })
        .unwrap();
    entries
}

#[test]
fn lists_files_and_directories_with_optional_cached_metadata() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file.txt"), b"hello").unwrap();
    fs::create_dir(dir.path().join("folder")).unwrap();
    let provider = LocalFileSystem::new(None);
    let entries = list(&provider, dir.path());
    assert_eq!(entries.len(), 2);
    let file = entries
        .iter()
        .find(|entry| entry.name == "file.txt")
        .unwrap();
    assert_eq!(file.path, dir.path().join("file.txt"));
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.name == "folder")
            .unwrap()
            .kind,
        EntryKind::Directory
    );
    assert!(file.metadata.size.is_none_or(|size| size == 5));
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.id)
            .collect::<HashSet<_>>()
            .len(),
        2
    );
}

#[test]
fn queries_current_metadata_without_mutating_the_initial_listing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payload");
    fs::write(&path, b"hello").unwrap();
    let provider = LocalFileSystem::new(None);
    let initial = list(&provider, dir.path());
    let snapshot = initial[0].metadata.clone();
    assert!(snapshot.size.is_none_or(|size| size == 5));
    fs::write(&path, b"updated contents").unwrap();
    let metadata = provider.entry_metadata(&path).unwrap();
    assert_eq!(metadata.size, Some(16));
    assert!(metadata.modified.is_some());
    assert_eq!(initial[0].metadata, snapshot);
}

#[test]
fn cancellation_before_open_returns_without_emitting_or_opening_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    LocalFileSystem::new(None)
        .read_directory(&dir.path().join("missing"), &cancel, &mut |_| {
            panic!("cancelled listing emitted")
        })
        .unwrap();
}

#[test]
fn cancellation_during_listing_stops_after_current_entry() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        fs::write(dir.path().join(name), b"").unwrap();
    }
    let cancel = Cancellation::default();
    let mut count = 0;
    LocalFileSystem::new(None)
        .read_directory(dir.path(), &cancel, &mut |_| {
            count += 1;
            cancel.cancel();
            true
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn consumer_can_stop_listing_by_returning_false() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        fs::write(dir.path().join(name), b"").unwrap();
    }
    let mut count = 0;
    LocalFileSystem::new(None)
        .read_directory(dir.path(), &Cancellation::default(), &mut |_| {
            count += 1;
            false
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn missing_paths_return_contextual_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let provider = LocalFileSystem::new(None);
    let listing_error = provider
        .read_directory(&missing, &Cancellation::default(), &mut |_| true)
        .unwrap_err();
    assert_eq!(listing_error.kind, ErrorKind::NotFound);
    assert_eq!(listing_error.path, missing);
    let metadata_error = provider.entry_metadata(&missing).unwrap_err();
    assert_eq!(metadata_error.kind, ErrorKind::NotFound);
    assert_eq!(metadata_error.path, missing);
}

#[test]
fn disappearing_entry_does_not_abort_listing_or_other_metadata() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        fs::write(dir.path().join(name), b"hello").unwrap();
    }
    let provider = LocalFileSystem::new(None);
    let mut entries = Vec::new();
    provider
        .read_directory(dir.path(), &Cancellation::default(), &mut |entry| {
            if entries.is_empty() {
                fs::remove_file(&entry.path).unwrap();
            }
            entries.push(entry);
            true
        })
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        provider.entry_metadata(&entries[0].path).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        provider.entry_metadata(&entries[1].path).unwrap().size,
        Some(5)
    );
}

#[test]
fn volumes_include_injected_home_without_requiring_it_to_exist() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("not-created-home");
    let volumes = LocalFileSystem::new(Some(home.clone()))
        .list_volumes()
        .unwrap();
    assert!(
        volumes
            .iter()
            .any(|volume| volume.label == "Home" && volume.root == home)
    );
    assert_eq!(
        volumes
            .iter()
            .map(|volume| &volume.id)
            .collect::<HashSet<_>>()
            .len(),
        volumes.len()
    );
}

#[test]
fn capacity_is_a_separate_query_and_reports_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let provider = LocalFileSystem::new(None);
    let space = provider.space_info(dir.path()).unwrap();
    assert!(space.total > 0);
    assert!(space.available <= space.total);
    assert!(provider.space_info(&dir.path().join("missing")).is_err());
}
