#![cfg(target_os = "linux")]
use midnight_core::*;
use midnight_platform_linux::LinuxFileSystem;
use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

#[test]
fn native_find_preserves_raw_names_skips_links_and_revalidates_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let raw = root.join(OsString::from_vec(b"raw-\xff.txt".to_vec()));
    fs::write(&raw, "český obsah").unwrap();
    fs::create_dir(root.join("subdirectory")).unwrap();
    symlink(root.join("subdirectory"), root.join("loop-link")).unwrap();
    let fifo = root.join("special.fifo");
    rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, rustix::fs::Mode::RWXU).unwrap();
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut directory = service.open_find_source(root, &cancel).unwrap();
    let mut found_raw = false;
    let mut found_link = false;
    let mut found_special = false;
    while let Some(child) = directory.next_child(&cancel).unwrap() {
        match child {
            FindChild::Entry(source) if source.entry().path == raw => found_raw = true,
            FindChild::Skipped {
                reason: FindSkipReason::Link,
                ..
            } => found_link = true,
            FindChild::Skipped {
                reason: FindSkipReason::Special,
                ..
            } => found_special = true,
            _ => {}
        }
    }
    assert!(found_raw && found_link && found_special);
    assert!(service.open_find_source(&fifo, &cancel).is_err());
    assert!(
        service
            .open_find_source(&root.join("loop-link"), &cancel)
            .is_err()
    );
    let mut source = service.open_find_source(&raw, &cancel).unwrap();
    let mut bytes = [0; 128];
    let count = source.read_chunk(&mut bytes, &cancel).unwrap();
    assert_eq!(&bytes[..count], "český obsah".as_bytes());
    fs::rename(&raw, root.join("old-object")).unwrap();
    fs::write(&raw, "replacement").unwrap();
    assert!(source.revalidate(&cancel).is_err());
    assert_eq!(
        service
            .open_find_source(std::path::Path::new("/"), &cancel)
            .unwrap()
            .entry()
            .kind,
        EntryKind::Directory
    );
}

#[test]
fn native_find_metadata_uses_existing_backend_locale_date_and_time() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("localized.txt");
    fs::write(&path, b"date metadata").unwrap();
    let service = LinuxFileSystem::new(None);
    let metadata = service.entry_metadata(&path).unwrap();
    let source = service
        .open_find_source(&path, &Cancellation::default())
        .unwrap();
    assert_eq!(
        source.entry().metadata.localized_modified,
        metadata.localized_modified
    );
}
