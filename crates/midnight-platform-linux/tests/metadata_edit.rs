#![cfg(target_os = "linux")]
use midnight_core::*;
use midnight_platform_linux::LinuxFileSystem;
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, UNIX_EPOCH},
};

#[test]
fn combined_rename_permissions_and_timestamp_preserve_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("CASE.txt");
    fs::write(&path, b"unchanged contents").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o764)).unwrap();
    let cancel = Cancellation::default();
    let item = LinuxFileSystem::new(None).prepare(&path, &cancel).unwrap();
    let time = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let patch = MetadataPatch {
        modified: TimeEdit::Set(time),
        unix_mask: 0o020,
        unix_value: 0,
        ..Default::default()
    };
    let result = item.edit_metadata(&patch, Some(OsStr::new("case.txt")), &cancel);
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.changes.renamed && result.changes.modified && result.changes.permissions);
    assert_eq!(
        fs::read(&result.current_path).unwrap(),
        b"unchanged contents"
    );
    let actual = fs::metadata(&result.current_path).unwrap();
    assert_eq!(actual.permissions().mode() & 0o7777, 0o744);
    assert_eq!(actual.modified().unwrap(), time);
    assert!(!path.exists());
}

#[test]
fn hardlink_collision_and_cancel_preserve_all_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SOURCE");
    let other = dir.path().join("target");
    fs::write(&path, b"source").unwrap();
    fs::hard_link(&path, &other).unwrap();
    let cancel = Cancellation::default();
    let item = LinuxFileSystem::new(None).prepare(&path, &cancel).unwrap();
    let patch = MetadataPatch {
        unix_mask: 0o777,
        unix_value: 0o600,
        ..Default::default()
    };
    let before = fs::metadata(&path).unwrap().permissions().mode();
    let collision = item.edit_metadata(&patch, Some(OsStr::new("target")), &cancel);
    assert_eq!(
        collision.error.unwrap().kind,
        std::io::ErrorKind::AlreadyExists
    );
    assert!(!collision.changes.any());
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode(), before);
    cancel.cancel();
    assert_eq!(
        item.edit_metadata(&patch, None, &cancel)
            .error
            .unwrap()
            .kind,
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read(&other).unwrap(), b"source");
}

#[test]
fn metadata_does_not_follow_symlinks_or_replaced_entries() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let link = dir.path().join("link");
    fs::write(&target, b"untouched").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let cancel = Cancellation::default();
    let service = LinuxFileSystem::new(None);
    let patch = MetadataPatch {
        unix_mask: 0o777,
        unix_value: 0o600,
        ..Default::default()
    };
    let before = fs::metadata(&target).unwrap().permissions().mode();
    let linked = service.prepare(&link, &cancel).unwrap();
    assert_eq!(
        linked
            .edit_metadata(&patch, None, &cancel)
            .error
            .unwrap()
            .kind,
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(fs::metadata(&target).unwrap().permissions().mode(), before);
    let prepared = service.prepare(&target, &cancel).unwrap();
    fs::rename(&target, dir.path().join("original")).unwrap();
    fs::write(&target, b"replacement").unwrap();
    assert!(
        prepared
            .edit_metadata(&patch, None, &cancel)
            .error
            .is_some()
    );
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
}

#[test]
fn a_child_replaced_by_a_symlink_after_the_snapshot_is_pinned_as_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("PARENT");
    let outside = dir.path().join("OUTSIDE");
    fs::create_dir_all(parent.join("S")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("OUT.txt"), b"outside").unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&parent, &cancel)
        .unwrap();
    let mut children = prepared
        .metadata_children(&cancel, MAX_METADATA_NAMES, MAX_METADATA_NAMES_BYTES)
        .unwrap();
    assert_eq!(children.names(), ["S"]);
    // Another program replaces the captured folder with a link to OUTSIDE.
    fs::rename(parent.join("S"), dir.path().join("S_old")).unwrap();
    std::os::unix::fs::symlink(&outside, parent.join("S")).unwrap();
    let state = |path: &std::path::Path| {
        let metadata = fs::metadata(path).unwrap();
        (metadata.permissions().mode(), metadata.modified().unwrap())
    };
    let before = (state(&outside), state(&outside.join("OUT.txt")));
    let pinned = children.pin(0, &cancel).unwrap();
    assert_eq!(pinned.item().path, parent.join("S"));
    assert_eq!(pinned.item().kind, EntryKind::Symlink);
    assert_eq!(
        pinned
            .metadata_children(&cancel, MAX_METADATA_NAMES, MAX_METADATA_NAMES_BYTES)
            .err()
            .unwrap()
            .kind,
        std::io::ErrorKind::InvalidInput,
        "a link is never listed"
    );
    let patch = MetadataPatch {
        unix_mask: 0o777,
        unix_value: 0o700,
        modified: TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_600_000_000)),
        recursive: true,
        ..Default::default()
    };
    let outcome = pinned.edit_metadata(&patch, None, &cancel);
    assert_eq!(outcome.error.unwrap().kind, std::io::ErrorKind::Unsupported);
    assert!(!outcome.changes.any());
    assert_eq!((state(&outside), state(&outside.join("OUT.txt"))), before);
}

#[test]
fn another_hard_link_spelled_like_the_new_case_is_still_a_conflict() {
    // The probe skips its directory scan only for an entry with no other name: a hard
    // link spelled like the requested case stays a conflict.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("AB");
    fs::write(&path, b"source").unwrap();
    fs::hard_link(&path, dir.path().join("ab")).unwrap();
    let cancel = Cancellation::default();
    let item = LinuxFileSystem::new(None).prepare(&path, &cancel).unwrap();
    assert_eq!(
        item.metadata_name_available(OsStr::new("ab"), &cancel)
            .unwrap_err()
            .kind,
        std::io::ErrorKind::AlreadyExists
    );
    let lower = MetadataPatch {
        case: Some(CaseChange::default()),
        ..Default::default()
    };
    let outcome = item.edit_metadata(&lower, Some(OsStr::new("ab")), &cancel);
    assert_eq!(
        outcome.error.unwrap().kind,
        std::io::ErrorKind::AlreadyExists
    );
    assert!(!outcome.changes.any());
    assert_eq!(fs::read(&path).unwrap(), b"source");
    assert_eq!(fs::read(dir.path().join("ab")).unwrap(), b"source");
}
