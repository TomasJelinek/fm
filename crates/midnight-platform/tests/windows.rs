#![cfg(windows)]

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

/// A directory junction, which needs no privilege, set natively by the platform crate's
/// fixture: no shell or command line sees either path, so shell syntax such as `&`, `^`
/// and `%NAME%` stays literal.
fn create_junction(link: &Path, target: &Path) {
    if let Err(error) = midnight_platform_windows::new_junction(link, target) {
        panic!("junction {link:?} to {target:?}: {error}");
    }
    assert!(link.is_dir(), "the junction {link:?} leads to {target:?}");
}

#[test]
fn cached_listing_metadata_matches_the_native_query() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cached_文档.txt");
    fs::write(&path, b"cached metadata").unwrap();
    let provider = LocalFileSystem::new(None);
    let entries = list(&provider, directory.path());
    assert_eq!(entries.len(), 1);
    assert!(provider.listing_metadata_complete(&entries[0]));
    assert_eq!(entries[0].metadata.size, Some(15));
    assert_eq!(entries[0].metadata, provider.entry_metadata(&path).unwrap());
}

#[test]
fn directory_junction_is_classified_as_directory_link_and_dangling_metadata_is_safe() {
    // A legal path containing shell syntax must remain literal fixture data.
    let dir = tempfile::Builder::new()
        .prefix("midnight&echo.safe-100%PATH%^!-")
        .tempdir()
        .unwrap();
    let target = dir.path().join("target");
    let junction = dir.path().join("junction");
    fs::create_dir(&target).unwrap();
    // Directory junctions exercise reparse points without requiring symlink privilege.
    create_junction(&junction, &target);
    let provider = LocalFileSystem::new(None);
    let entries = list(&provider, dir.path());
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.path == junction)
            .unwrap()
            .kind,
        EntryKind::DirectoryLink
    );
    fs::remove_dir(target).unwrap();
    assert_eq!(provider.entry_metadata(&junction).unwrap().size, None);
}

#[test]
fn windows_unicode_paths_beyond_260_utf16_units_are_listed_and_navigated() {
    use std::os::windows::ffi::OsStrExt;

    let root = tempfile::tempdir().unwrap();
    let component = "dlouhá_中文_🧪_".repeat(5);
    assert!(component.encode_utf16().count() < 255);
    let directory = root
        .path()
        .join(&component)
        .join(&component)
        .join(&component)
        .join(&component);
    assert!(directory.as_os_str().encode_wide().count() > 260);
    if let Err(error) = fs::create_dir_all(&directory) {
        if error.kind() == std::io::ErrorKind::Unsupported || error.raw_os_error() == Some(206) {
            eprintln!("SKIP: temporary filesystem does not support this long path: {error}");
            return;
        }
        panic!("long-path fixture creation failed: {error}");
    }
    let name = "Příliš_中文_日本語_العربية_עברית_🧪.txt";
    fs::write(directory.join(name), b"long Unicode path").unwrap();
    let provider = LocalFileSystem::new(None);
    let parent_entries = list(&provider, directory.parent().unwrap());
    let returned_directory = parent_entries
        .iter()
        .find(|entry| entry.name == component.as_str())
        .unwrap();
    assert_eq!(returned_directory.kind, EntryKind::Directory);
    assert_eq!(returned_directory.path, directory);
    let entries = list(&provider, &returned_directory.path);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, name);
    assert_eq!(entries[0].path, directory.join(name));
    assert_eq!(
        provider.entry_metadata(&entries[0].path).unwrap().size,
        Some(17)
    );
    assert_eq!(fs::read(&entries[0].path).unwrap(), b"long Unicode path");
}
#[test]
fn logical_volumes_include_the_temporary_directory_drive() {
    let directory = tempfile::tempdir().unwrap();
    let volumes = LocalFileSystem::new(None).list_volumes().unwrap();
    assert!(
        volumes
            .iter()
            .any(|volume| directory.path().starts_with(&volume.root))
    );
}

#[test]
fn raw_utf16_names_survive_listing_and_metadata() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let directory = tempfile::tempdir().unwrap();
    let name = OsString::from_wide(&[b'r' as u16, b'a' as u16, b'w' as u16, 0xd800]);
    let path = directory.path().join(&name);
    fs::write(&path, b"raw name").unwrap();
    let provider = LocalFileSystem::new(None);
    let entries = list(&provider, directory.path());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, name);
    assert_eq!(entries[0].path, path);
    assert_eq!(provider.entry_metadata(&path).unwrap().size, Some(8));
}
