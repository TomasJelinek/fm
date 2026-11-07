#![cfg(target_os = "linux")]
use midnight_core::{Cancellation, TransferService};
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use midnight_platform_linux::LinuxFileSystem;
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    path::Path,
    time::Duration,
};
fn fixture() -> tempfile::TempDir {
    // Linux runner places TMPDIR on its native filesystem: DrvFS cannot preserve
    // arbitrary Unix bytes or create O_TMPFILE. Every owned fixture stays under _build.
    let root = std::env::temp_dir().join("midnight-transfer/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("transfer-linux-")
        .tempdir_in(root)
        .unwrap()
}
fn ready(source: &Path, directory: &Path) -> VerifiedFileCopy {
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = provider.prepare_transfer_source(source, &cancel).unwrap();
    let target = provider
        .prepare_transfer_destination(directory, &cancel)
        .unwrap();
    let mut copy = VerifiedFileCopy::new(source, target.stage_file(&cancel).unwrap()).unwrap();
    while copy.phase() != CopyPhase::ReadyToPublish {
        copy.step(&cancel).unwrap();
    }
    copy
}
#[test]
fn verifies_and_publishes_raw_unix_name_without_deleting_source() {
    let temp = fixture();
    let source = temp
        .path()
        .join(OsString::from_vec(b"source-\xff".to_vec()));
    let name = OsString::from_vec(b"copied-\xfe".to_vec());
    let bytes = vec![137; 700_003];
    fs::write(&source, &bytes).unwrap();
    let mut job = ready(&source, temp.path());
    assert_eq!(job.verified_bytes(), 700_003);
    job.publish(&name, &Cancellation::default()).unwrap();
    assert_eq!(fs::read(temp.path().join(name)).unwrap(), bytes);
    assert_eq!(fs::read(source).unwrap(), bytes);
}
#[test]
fn publication_race_preserves_target_and_allows_another_name() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"original").unwrap();
    let mut job = ready(&source, temp.path());
    fs::write(temp.path().join("target"), b"winner").unwrap();
    assert!(
        job.publish(OsStr::new("target"), &Cancellation::default())
            .is_err()
    );
    assert_eq!(fs::read(temp.path().join("target")).unwrap(), b"winner");
    job.publish(OsStr::new("other"), &Cancellation::default())
        .unwrap();
    assert_eq!(fs::read(temp.path().join("other")).unwrap(), b"original");
}
#[test]
fn changed_source_metadata_prevents_publication_and_leaves_no_staging_names() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(&source, b"original").unwrap();
    let mut job = ready(&source, &target);
    let before = fs::metadata(&source).unwrap();
    fs::write(&source, b"modified").unwrap();
    // Same-length writes can share the filesystem's mtime/ctime quantum.
    // Exercise a detectable metadata change without relying on wall-clock delay.
    fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(before.modified().unwrap() + Duration::from_secs(2))
        .unwrap();
    let after = fs::metadata(&source).unwrap();
    assert_eq!(before.len(), after.len());
    assert_ne!(before.modified().unwrap(), after.modified().unwrap());
    let error = job
        .publish(OsStr::new("copy"), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, std::io::ErrorKind::InvalidInput);
    drop(job);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), b"modified");
}
#[test]
fn cancelled_publication_leaves_source_and_no_staging_names() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(&source, b"original").unwrap();
    let mut job = ready(&source, &target);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        job.publish(OsStr::new("copy"), &cancel).unwrap_err().kind,
        std::io::ErrorKind::Interrupted,
    );
    drop(job);
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), b"original");
}
#[test]
fn links_are_never_opened_and_ancestry_is_checked_by_inode() {
    let temp = fixture();
    let child = temp.path().join("child");
    let link = temp.path().join("link");
    fs::create_dir(&child).unwrap();
    symlink(&child, &link).unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    assert!(provider.prepare_transfer_source(&link, &cancel).is_err());
    let source = provider
        .prepare_transfer_source(temp.path(), &cancel)
        .unwrap();
    let target = provider
        .prepare_transfer_destination(&child, &cancel)
        .unwrap();
    assert!(
        target
            .reject_source_ancestor(source.entry().identity, &cancel)
            .is_err()
    );
    assert!(
        target
            .create_directory(OsStr::new("../escape"), &cancel)
            .is_err()
    );
}
#[test]
fn iteration_preserves_raw_names_and_exclusive_creation_rejects_merge() {
    let temp = fixture();
    let name = OsString::from_vec(b"entry-\xff".to_vec());
    fs::write(temp.path().join(&name), b"original").unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut source = provider
        .prepare_transfer_source(temp.path(), &cancel)
        .unwrap();
    let entry = source.next_child(&cancel).unwrap().unwrap();
    assert_eq!(entry.entry().item.path.file_name(), Some(name.as_os_str()));
    assert!(source.next_child(&cancel).unwrap().is_none());
    let target = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    target.create_directory(OsStr::new("new"), &cancel).unwrap();
    assert!(target.create_directory(OsStr::new("new"), &cancel).is_err());
}

#[test]
fn copied_directories_refuse_nonsticky_shared_parents_before_creating_anything() {
    let temp = fixture();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let target = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let error = match target.create_directory(OsStr::new("child"), &cancel) {
        Ok(_) => panic!("copy adopted a directory created in an unsafe shared parent"),
        Err(error) => error,
    };
    assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
    assert!(!temp.path().join("child").exists());
}

#[test]
fn owned_sticky_parent_allows_private_copy_directory() {
    let temp = fixture();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o1777)).unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let target = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let child = target
        .create_directory(OsStr::new("child"), &cancel)
        .unwrap();
    assert_eq!(
        fs::metadata(child.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn explicit_merge_pins_directory_identity_and_retains_existing_children() {
    let temp = fixture();
    let directory = temp.path().join("merge");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("existing"), b"keep").unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    assert!(parent.capabilities().directory_merge);
    let expected = parent
        .prepare_target(OsStr::new("merge"), &cancel)
        .unwrap()
        .unwrap();
    let merged = parent
        .open_merge_directory(OsStr::new("merge"), expected.entry(), &cancel)
        .unwrap();
    assert_eq!(merged.identity(), expected.entry().identity);
    merged.create_directory(OsStr::new("new"), &cancel).unwrap();
    assert_eq!(fs::read(directory.join("existing")).unwrap(), b"keep");
    assert!(directory.join("new").is_dir());
}

#[test]
fn recoverable_replacement_retains_the_original_after_stage_drop() {
    use midnight_core::{TransferPublication, TransferReplacementPolicy};
    use std::io::{Read, Seek, SeekFrom, Write};
    let temp = fixture();
    let target = temp.path().join("target 文档");
    fs::write(&target, b"original").unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    assert!(parent.capabilities().recoverable_replacement);
    assert!(!parent.capabilities().conditional_overwrite);
    let expected = parent
        .prepare_target(OsStr::new("target 文档"), &cancel)
        .unwrap()
        .unwrap();
    let original_identity = expected.entry().identity;
    let mut stage = parent
        .stage_file_for_target(
            OsStr::new("target 文档"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"verified replacement").unwrap();
    stage.sync(&cancel).unwrap();
    stage.seek(SeekFrom::Start(0)).unwrap();
    let mut readback = Vec::new();
    stage.read_to_end(&mut readback).unwrap();
    assert_eq!(readback, b"verified replacement");
    let outcome = stage
        .publish_replace(OsStr::new("target 文档"), &cancel)
        .unwrap();
    let recovery = match outcome {
        TransferPublication::Replaced { path, recovery } => {
            assert_eq!(path, target);
            recovery
        }
        other => panic!("unexpected replacement outcome: {other:?}"),
    };
    drop(stage);
    assert_eq!(recovery.identity, Some(original_identity));
    assert_eq!(fs::read(&target).unwrap(), b"verified replacement");
    assert_eq!(fs::read(&recovery.path).unwrap(), b"original");
}

#[test]
fn strict_conditional_replacement_is_explicitly_unsupported() {
    use midnight_core::TransferReplacementPolicy;
    let temp = fixture();
    fs::write(temp.path().join("target"), b"original").unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let expected = parent
        .prepare_target(OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let error = match parent.stage_file_for_target(
        OsStr::new("target"),
        expected,
        TransferReplacementPolicy::ConditionalIdentity,
        &cancel,
    ) {
        Ok(_) => panic!("provider silently weakened conditional replacement"),
        Err(error) => error,
    };
    assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
    assert_eq!(fs::read(temp.path().join("target")).unwrap(), b"original");
}

#[test]
fn replacement_rejects_changed_target_before_publication() {
    use midnight_core::TransferReplacementPolicy;
    use std::io::Write;
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let expected = parent
        .prepare_target(OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = parent
        .stage_file_for_target(
            OsStr::new("target"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"replacement").unwrap();
    fs::rename(&target, temp.path().join("original-away")).unwrap();
    fs::write(&target, b"newcomer").unwrap();
    assert!(
        stage
            .publish_replace(OsStr::new("target"), &cancel)
            .is_err()
    );
    drop(stage);
    assert_eq!(fs::read(&target).unwrap(), b"newcomer");
    assert_eq!(
        fs::read(temp.path().join("original-away")).unwrap(),
        b"original"
    );
}

#[test]
fn a_verified_stage_can_bind_a_late_conflict_without_recopying() {
    use midnight_core::{TransferPublication, TransferReplacementPolicy};
    use std::io::Write;
    let temp = fixture();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let mut stage = parent.stage_file(&cancel).unwrap();
    stage.write_all(b"already verified").unwrap();
    stage.sync(&cancel).unwrap();
    fs::write(temp.path().join("late"), b"old").unwrap();
    assert!(
        stage
            .publish_no_replace(OsStr::new("late"), &cancel)
            .is_err()
    );
    let expected = parent
        .prepare_target(OsStr::new("late"), &cancel)
        .unwrap()
        .unwrap();
    stage
        .prepare_replacement(
            OsStr::new("late"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    assert!(matches!(
        stage.publish_replace(OsStr::new("late"), &cancel).unwrap(),
        TransferPublication::Replaced { .. }
    ));
    drop(stage);
    assert_eq!(
        fs::read(temp.path().join("late")).unwrap(),
        b"already verified"
    );
}

#[test]
fn explicit_no_replace_ignores_an_old_replacement_binding() {
    use midnight_core::TransferReplacementPolicy;
    use std::io::Write;
    let temp = fixture();
    fs::write(temp.path().join("target"), b"keep").unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let expected = parent
        .prepare_target(OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = parent
        .stage_file_for_target(
            OsStr::new("target"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"new bytes").unwrap();
    stage
        .publish_no_replace(OsStr::new("other"), &cancel)
        .unwrap();
    drop(stage);
    assert_eq!(fs::read(temp.path().join("target")).unwrap(), b"keep");
    assert_eq!(fs::read(temp.path().join("other")).unwrap(), b"new bytes");
}

#[test]
fn target_revision_change_prevents_old_consent_from_authorizing_replacement() {
    use midnight_core::TransferReplacementPolicy;
    use std::io::Write;
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"old bytes").unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let expected = parent
        .prepare_target(OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = parent
        .stage_file_for_target(
            OsStr::new("target"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"replacement").unwrap();
    fs::write(&target, b"new bytes").unwrap();
    let modified = fs::metadata(&target).unwrap().modified().unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&target)
        .unwrap()
        .set_modified(modified + std::time::Duration::from_secs(2))
        .unwrap();
    assert!(
        stage
            .publish_replace(OsStr::new("target"), &cancel)
            .is_err()
    );
    drop(stage);
    assert_eq!(fs::read(&target).unwrap(), b"new bytes");
}

#[test]
fn replaced_merge_directory_identity_cannot_be_reused_by_old_consent() {
    let temp = fixture();
    let directory = temp.path().join("directory");
    fs::create_dir(&directory).unwrap();
    let provider = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let parent = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let expected = parent
        .prepare_target(OsStr::new("directory"), &cancel)
        .unwrap()
        .unwrap();
    fs::rename(&directory, temp.path().join("directory-away")).unwrap();
    fs::create_dir(&directory).unwrap();
    assert!(
        parent
            .open_merge_directory(OsStr::new("directory"), expected.entry(), &cancel)
            .is_err()
    );
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
}
