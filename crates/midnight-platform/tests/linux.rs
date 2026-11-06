#![cfg(target_os = "linux")]

use midnight_core::{Cancellation, DirectoryService, Entry, EntryKind, VolumeService};
use midnight_platform::LocalFileSystem;
use std::{fs, path::Path};

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
fn raw_non_utf8_names_survive_listing() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let name = OsString::from_vec(b"raw\xff".to_vec());
    let path = dir.path().join(&name);
    fs::write(&path, b"x").unwrap();
    let entries = list(&LocalFileSystem::new(None), dir.path());
    assert_eq!(entries[0].name, name);
    assert_eq!(entries[0].path, path);
}

#[test]
fn symlinks_are_classified_and_metadata_does_not_follow_target() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::write(&target, vec![0; 1024]).unwrap();
    symlink(&target, dir.path().join("link")).unwrap();
    symlink(dir.path().join("missing"), dir.path().join("dangling")).unwrap();
    let provider = LocalFileSystem::new(None);
    let entries = list(&provider, dir.path());
    for name in ["link", "dangling"] {
        let entry = entries.iter().find(|entry| entry.name == name).unwrap();
        assert_eq!(entry.kind, EntryKind::Symlink);
        assert_eq!(provider.entry_metadata(&entry.path).unwrap().size, None);
    }
}
#[test]
fn linux_volumes_include_the_root_directory() {
    assert!(
        LocalFileSystem::new(None)
            .list_volumes()
            .unwrap()
            .iter()
            .any(|volume| volume.root == Path::new("/"))
    );
}
