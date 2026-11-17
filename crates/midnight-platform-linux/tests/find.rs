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

/// Directory times come from the kernel's coarse clock, so a change within the same
/// tick can keep them. A fixed past time makes every later change observable.
fn backdate(path: &std::path::Path) {
    let past = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    fs::File::open(path).unwrap().set_modified(past).unwrap();
}

#[test]
fn changes_inside_a_directory_never_abort_its_remaining_enumeration() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    for name in ["a", "b", "c"] {
        fs::write(root.join(name), name).unwrap();
    }
    fs::create_dir(root.join("sub")).unwrap();
    backdate(root);
    backdate(&root.join("sub"));
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut directory = service.open_find_source(root, &cancel).unwrap();
    let mut entries = 0;
    while let Some(child) = directory.next_child(&cancel).unwrap() {
        assert!(matches!(child, FindChild::Entry(_)));
        entries += 1;
        if entries <= 2 {
            // An editor saves into the searched directory via a temporary and rename.
            fs::write(root.join(format!(".save-{entries}")), b"saved").unwrap();
            fs::rename(
                root.join(format!(".save-{entries}")),
                root.join(format!("saved-{entries}")),
            )
            .unwrap();
        }
    }
    // Entries created during enumeration may or may not be reported.
    assert!(
        (4..=6).contains(&entries),
        "enumeration stopped after {entries} entries"
    );

    // A directory result remains valid when its contents change; a file does not.
    let captured = service
        .open_find_source(&root.join("sub"), &cancel)
        .unwrap()
        .entry()
        .clone();
    let file = service.open_find_source(&root.join("a"), &cancel).unwrap();
    fs::write(root.join("sub/added"), b"added").unwrap();
    assert_ne!(
        fs::metadata(root.join("sub")).unwrap().modified().unwrap(),
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000),
        "the directory change must be observable in its times"
    );
    fs::write(root.join("a"), b"rewritten contents").unwrap();
    let reopened = service
        .open_find_source(&root.join("sub"), &cancel)
        .unwrap();
    assert!(reopened.entry().matches_capture(&captured));
    reopened.revalidate(&cancel).unwrap();
    assert!(file.revalidate(&cancel).is_err());

    // A directory replaced before its enumeration starts is still refused.
    let mut stale = service
        .open_find_source(&root.join("sub"), &cancel)
        .unwrap();
    fs::rename(root.join("sub"), root.join("sub-old")).unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    assert!(stale.next_child(&cancel).is_err());
}

#[test]
fn locations_reached_through_ancestor_links_are_searchable_but_never_the_final_link() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    fs::create_dir_all(root.join("real/dir")).unwrap();
    fs::write(root.join("real/dir/file.txt"), b"found").unwrap();
    symlink(root.join("real"), root.join("link")).unwrap();
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    // As on merged-/usr systems (`/lib -> usr/lib`) or after entering a directory link.
    let mut directory = service
        .open_find_source(&root.join("link/dir"), &cancel)
        .unwrap();
    assert_eq!(directory.entry().kind, EntryKind::Directory);
    let Some(FindChild::Entry(child)) = directory.next_child(&cancel).unwrap() else {
        panic!("the file below the linked ancestor was not found");
    };
    assert_eq!(child.entry().path, root.join("link/dir/file.txt"));
    child.revalidate(&cancel).unwrap();
    let result = service
        .open_find_source(&root.join("link/dir/file.txt"), &cancel)
        .unwrap();
    assert!(result.entry().matches_capture(child.entry()));
    assert!(
        service
            .open_find_source(&root.join("link"), &cancel)
            .is_err()
    );
    // procfs magic links are never followed, even as ancestors.
    let magic = std::path::Path::new("/proc/self/root").join(root.strip_prefix("/").unwrap());
    assert!(service.open_find_source(&magic, &cancel).is_err());
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
