#![cfg(windows)]
use midnight_core::{Cancellation, TransferPublication, TransferService};
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use midnight_platform_windows::WindowsFileSystem;
use std::{ffi::OsStr, fs, path::Path};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("transfer-")
        .tempdir_in(root)
        .unwrap()
}
fn ready(source: &Path, directory: &Path) -> VerifiedFileCopy {
    let provider = WindowsFileSystem::new(None);
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
fn verifies_bytes_before_unicode_publication_and_never_removes_source() {
    let temp = fixture();
    let source = temp.path().join("source 🦀.bin");
    let target = temp.path().join("destination");
    fs::create_dir(&target).unwrap();
    let bytes = vec![137; 700_003];
    fs::write(&source, &bytes).unwrap();
    let mut job = ready(&source, &target);
    assert!(!target.join("copied 文档.bin").exists());
    assert_eq!(job.verified_bytes(), bytes.len() as u64);
    assert!(matches!(
        job.publish(OsStr::new("copied 文档.bin"), &Cancellation::default())
            .unwrap(),
        TransferPublication::Published(_)
            | TransferPublication::PublishedDurabilityUnconfirmed { .. }
    ));
    assert_eq!(fs::read(target.join("copied 文档.bin")).unwrap(), bytes);
    assert_eq!(fs::read(source).unwrap(), bytes);
}
#[test]
fn publication_race_preserves_existing_file_and_can_retry_another_name() {
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
fn cancellation_cleans_owned_stage_and_preserves_source() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(&source, b"original").unwrap();
    let job = ready(&source, &target);
    drop(job);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), b"original");
}
#[test]
fn native_ancestry_rejects_copying_directory_into_its_descendant() {
    let temp = fixture();
    let child = temp.path().join("child");
    fs::create_dir(&child).unwrap();
    let provider = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
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
}
#[test]
fn directory_iteration_is_native_and_destination_creation_never_merges() {
    let temp = fixture();
    fs::write(temp.path().join("a"), b"A").unwrap();
    fs::write(temp.path().join("文档"), b"B").unwrap();
    let provider = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut source = provider
        .prepare_transfer_source(temp.path(), &cancel)
        .unwrap();
    let mut names = vec![];
    while let Some(child) = source.next_child(&cancel).unwrap() {
        names.push(child.entry().item.path.file_name().unwrap().to_owned());
    }
    names.sort();
    assert_eq!(names, vec![OsStr::new("a"), OsStr::new("文档")]);
    let destination = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    destination
        .create_directory(OsStr::new("new"), &cancel)
        .unwrap();
    assert!(
        destination
            .create_directory(OsStr::new("new"), &cancel)
            .is_err()
    );
    assert!(
        destination
            .create_directory(OsStr::new("../escape"), &cancel)
            .is_err()
    );
}

#[test]
fn explicit_merge_pins_directory_identity_and_retains_existing_children() {
    let temp = fixture();
    let directory = temp.path().join("merge");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("existing"), b"keep").unwrap();
    let provider = WindowsFileSystem::new(None);
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
    let provider = WindowsFileSystem::new(None);
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
    let provider = WindowsFileSystem::new(None);
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
    let provider = WindowsFileSystem::new(None);
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
    let provider = WindowsFileSystem::new(None);
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
    let provider = WindowsFileSystem::new(None);
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
    let provider = WindowsFileSystem::new(None);
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
    let provider = WindowsFileSystem::new(None);
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
