use super::*;
use midnight_core::{TransferNativeMoveOutcome, TransferReplacementPolicy};
use std::{fs, os::unix::fs::symlink};

pub(super) fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-native-move/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn native_move_keeps_sparse_inode_and_allows_an_existing_writer() {
    let temp = fixture();
    let source = temp.path().join("large 文档");
    let writer = File::create(&source).unwrap();
    writer.set_len(10 * 1024 * 1024 * 1024).unwrap();
    let before = writer.metadata().unwrap();
    let target = temp.path().join("destination");
    fs::create_dir(&target).unwrap();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = service.prepare_move_source(&source, &cancel).unwrap();
    let destination = service.prepare_move_destination(&target, &cancel).unwrap();
    let outcome = prepared
        .capture
        .try_native_move(&*destination, OsStr::new("large 文档"), None, &cancel)
        .unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::Moved { .. }),
        "{outcome:?}"
    );
    assert!(!source.exists());
    let moved = fs::metadata(target.join("large 文档")).unwrap();
    assert_eq!(
        (moved.dev(), moved.ino(), moved.len(), moved.blocks()),
        (before.dev(), before.ino(), before.len(), before.blocks())
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn native_directory_move_preserves_children_without_following_them() {
    let temp = fixture();
    let source = temp.path().join("tree");
    fs::create_dir(&source).unwrap();
    symlink("/missing-native-move-test", source.join("link")).unwrap();
    let before = fs::metadata(&source).unwrap().ino();
    let target = temp.path().join("destination");
    fs::create_dir(&target).unwrap();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = service.prepare_move_source(&source, &cancel).unwrap();
    let destination = service.prepare_move_destination(&target, &cancel).unwrap();
    let outcome = prepared
        .capture
        .try_native_move(&*destination, OsStr::new("tree"), None, &cancel)
        .unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::Moved { .. }),
        "{outcome:?}"
    );
    assert_eq!(fs::metadata(target.join("tree")).unwrap().ino(), before);
    assert!(
        fs::symlink_metadata(target.join("tree/link"))
            .unwrap()
            .is_symlink()
    );
    assert!(!source.exists());
}

#[test]
fn native_move_collision_is_unchanged_and_explicit_replacement_retains_old_target() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("destination");
    fs::write(&source, b"new bytes").unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("item"), b"old bytes").unwrap();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = service.prepare_move_source(&source, &cancel).unwrap();
    let destination = service.prepare_move_destination(&target, &cancel).unwrap();
    let error = prepared
        .capture
        .try_native_move(&*destination, OsStr::new("item"), None, &cancel)
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&source).unwrap(), b"new bytes");
    assert_eq!(fs::read(target.join("item")).unwrap(), b"old bytes");
    let expected = destination
        .prepare_target(OsStr::new("item"), &cancel)
        .unwrap()
        .unwrap();
    let outcome = prepared
        .capture
        .try_native_move(
            &*destination,
            OsStr::new("item"),
            Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
        )
        .unwrap();
    let TransferNativeMoveOutcome::Moved { recovery, .. } = outcome else {
        panic!("unexpected replacement: {outcome:?}")
    };
    assert_eq!(recovery.len(), 1);
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old bytes");
    assert_eq!(fs::read(target.join("item")).unwrap(), b"new bytes");
    assert!(!source.exists());
}

#[test]
fn native_same_entry_is_distinct_from_a_hard_link_alias() {
    let temp = fixture();
    let source = temp.path().join("Case");
    fs::write(&source, b"bytes").unwrap();
    fs::hard_link(&source, temp.path().join("alias")).unwrap();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = service.prepare_move_source(&source, &cancel).unwrap();
    let destination = service
        .prepare_move_destination(temp.path(), &cancel)
        .unwrap();
    assert!(matches!(
        prepared
            .capture
            .try_native_move(&*destination, OsStr::new("Case"), None, &cancel,)
            .unwrap(),
        TransferNativeMoveOutcome::SameEntry
    ));
    assert!(
        prepared
            .capture
            .try_native_move(&*destination, OsStr::new("alias"), None, &cancel,)
            .is_err()
    );
    assert!(source.exists() && temp.path().join("alias").exists());
    assert!(matches!(
        prepared
            .capture
            .try_native_move(&*destination, OsStr::new("case"), None, &cancel,)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { .. }
    ));
    assert!(!source.exists());
    assert_eq!(fs::read(temp.path().join("case")).unwrap(), b"bytes");
    assert_eq!(fs::read(temp.path().join("alias")).unwrap(), b"bytes");
}

#[test]
fn validation_seek_uses_retained_inode_after_its_original_name_changes() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"retained original").unwrap();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut prepared = service.prepare_move_source(&source, &cancel).unwrap();
    fs::rename(&source, temp.path().join("renamed")).unwrap();
    fs::write(&source, b"different object").unwrap();
    let guard = prepared.source.rewind_for_validation(&cancel).unwrap();
    assert_eq!(prepared.source.seek_for_validation(9, &cancel).unwrap(), 17);
    let mut bytes = [0; 8];
    assert_eq!(prepared.source.read_chunk(&mut bytes, &cancel).unwrap(), 8);
    assert_eq!(&bytes, b"original");
    guard.revalidate(&cancel).unwrap();
    assert!(prepared.source.seek_for_validation(18, &cancel).is_err());
    cancel.cancel();
    assert!(prepared.source.seek_for_validation(0, &cancel).is_err());
}

#[test]
fn native_tmpfs_move_needs_no_ext_specific_content_proof_capability() {
    let root = Path::new("/dev/shm/_build/midnight-native-move");
    fs::create_dir_all(root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    let source = temp.path().join("source");
    fs::write(&source, b"keep inode").unwrap();
    let before = fs::metadata(&source).unwrap().ino();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = service.prepare_move_source(&source, &cancel).unwrap();
    let target = service
        .prepare_move_destination(temp.path(), &cancel)
        .unwrap();
    assert!(matches!(
        prepared
            .capture
            .try_native_move(&*target, OsStr::new("destination"), None, &cancel,)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { .. }
    ));
    assert_eq!(
        fs::metadata(temp.path().join("destination")).unwrap().ino(),
        before
    );
}

#[test]
fn native_admission_pins_only_metadata_for_unreadable_raw_named_file() {
    use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
    let temp = fixture();
    let source = temp
        .path()
        .join(OsString::from_vec(b"source-\xff".to_vec()));
    fs::write(&source, b"metadata only").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o0)).unwrap();
    let cancel = Cancellation::default();
    let (retained, capture) = prepare_native(&source, &cancel).unwrap();
    assert!(
        rustix::fs::fcntl_getfl(&*retained.native_file())
            .unwrap()
            .contains(OFlags::PATH)
    );
    assert!(
        rustix::fs::fcntl_getfl(&*capture._file)
            .unwrap()
            .contains(OFlags::PATH)
    );
    let target = crate::LinuxFileSystem::new(None)
        .prepare_move_destination(temp.path(), &cancel)
        .unwrap();
    let renamed = OsString::from_vec(b"destination-\xfe".to_vec());
    assert!(matches!(
        capture
            .try_native_move(&*target, &renamed, None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { .. }
    ));
    assert!(!source.exists());
    assert_eq!(
        fs::metadata(temp.path().join(renamed))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0
    );
}

#[test]
fn native_tmpfs_replacement_is_advertised_without_copy_proof_capability() {
    let root = Path::new("/dev/shm/_build/midnight-native-move");
    fs::create_dir_all(root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"new bytes").unwrap();
    fs::write(&target, b"old bytes").unwrap();
    let source_inode = fs::metadata(&source).unwrap().ino();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = service.prepare_move_source(&source, &cancel).unwrap();
    let destination = service
        .prepare_move_destination(temp.path(), &cancel)
        .unwrap();
    assert!(!destination.capabilities().recoverable_replacement);
    let selected = prepared.source.entry().identity;
    assert!(destination.supports_native_move_replacement(selected));
    assert!(
        !destination.supports_native_move_replacement(TransferIdentity {
            device: selected.device ^ 1,
            ..selected
        })
    );
    let expected = destination
        .prepare_target(OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let outcome = prepared
        .capture
        .try_native_move(
            &*destination,
            OsStr::new("target"),
            Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
        )
        .unwrap();
    let TransferNativeMoveOutcome::Moved { recovery, .. } = outcome else {
        panic!("unexpected native replacement: {outcome:?}")
    };
    assert_eq!(fs::metadata(target).unwrap().ino(), source_inode);
    assert_eq!(recovery.len(), 1);
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old bytes");
    assert!(!source.exists());
}

#[test]
fn native_cross_device_outcome_leaves_names_and_contents_unchanged() {
    let source_root = fixture();
    let other_root = Path::new("/dev/shm/_build/midnight-native-move");
    fs::create_dir_all(other_root).unwrap();
    let destination_root = tempfile::tempdir_in(other_root).unwrap();
    let source = source_root.path().join("source");
    fs::write(&source, b"original").unwrap();
    assert_ne!(
        fs::metadata(source_root.path()).unwrap().dev(),
        fs::metadata(destination_root.path()).unwrap().dev()
    );
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = service.prepare_move_source(&source, &cancel).unwrap();
    let target = service
        .prepare_move_destination(destination_root.path(), &cancel)
        .unwrap();
    assert!(matches!(
        prepared
            .capture
            .try_native_move(&*target, OsStr::new("item"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::CrossDevice
    ));
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read_dir(source_root.path()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(destination_root.path()).unwrap().count(), 0);
}
