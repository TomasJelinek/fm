#![cfg(target_os = "linux")]
use midnight_core::{Cancellation, DeleteMode, DirectoryService, EntryKind, MutationService};
use midnight_platform_linux::LinuxFileSystem;
use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

#[test]
fn actual_listing_classifies_directory_links_without_replacing_link_identity_or_metadata() {
    let owned = tempfile::tempdir().unwrap();
    let target = owned.path().join("target 資料");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"target contents").unwrap();
    fs::write(owned.path().join("file"), b"contents").unwrap();
    let link = owned.path().join(OsString::from_vec(vec![b'l', 0xff]));
    symlink(&target, &link).unwrap();
    symlink("file", owned.path().join("file-link")).unwrap();
    symlink("missing", owned.path().join("broken-link")).unwrap();
    symlink("loop-link", owned.path().join("loop-link")).unwrap();
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut entries = Vec::new();
    service
        .read_directory(owned.path(), &cancel, &mut |entry| {
            entries.push(entry);
            true
        })
        .unwrap();
    let directory = entries.iter().find(|entry| entry.path == link).unwrap();
    assert_eq!(directory.kind, EntryKind::DirectoryLink);
    assert_eq!(directory.native_path(), Some(link.as_path()));
    assert_eq!(directory.name, link.file_name().unwrap());
    let metadata = service.entry_metadata(&link).unwrap();
    assert_eq!(metadata.unix_mode.unwrap() & libc::S_IFMT, libc::S_IFLNK);
    assert_eq!(metadata.size, None);
    for name in ["file-link", "broken-link", "loop-link"] {
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.path == owned.path().join(name))
                .unwrap()
                .kind,
            EntryKind::Symlink
        );
    }
    let resolved = service
        .resolve_size_directory(&directory.path, &cancel)
        .unwrap()
        .unwrap();
    let ordinary = service
        .resolve_size_directory(&target, &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.identity, ordinary.identity);
    // Browsing's directory hint never changes the no-follow mutation capture.
    let prepared = service.prepare(&directory.path, &cancel).unwrap();
    assert_eq!(prepared.item().kind, EntryKind::Symlink);
    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |_| {})
        .unwrap();
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"target contents");
}

#[test]
fn cancelled_listing_never_probes_links_or_publishes_entries() {
    let cancel = Cancellation::default();
    cancel.cancel();
    LinuxFileSystem::new(None)
        .read_directory(
            std::path::Path::new("unavailable-before-cancel"),
            &cancel,
            &mut |_| panic!("cancelled listing must not publish a row"),
        )
        .unwrap();
}
