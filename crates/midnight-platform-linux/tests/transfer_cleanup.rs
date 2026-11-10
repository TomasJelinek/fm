#![cfg(target_os = "linux")]
//! Native unpublished stages stay anonymous and release their partial contents on cancellation.
use midnight_core::{Cancellation, TransferService, TransferVerification};
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use midnight_platform_linux::LinuxFileSystem;
use std::{
    ffi::OsStr,
    fs,
    io::{self, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-transfer/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("cancel-stage-")
        .tempdir_in(root)
        .unwrap()
}

fn long_directory(root: &Path) -> PathBuf {
    let mut directory = root.to_owned();
    for part in 0..12 {
        directory.push(format!("segment-{part:02}-{}-文档", "x".repeat(54)));
    }
    fs::create_dir_all(&directory).unwrap();
    assert!(directory.as_os_str().as_bytes().len() > 800);
    directory
}

fn copy(source: &Path, target: &Path, verification: TransferVerification) -> VerifiedFileCopy {
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service.prepare_transfer_source(source, &cancel).unwrap();
    let target = service
        .prepare_transfer_destination(target, &cancel)
        .unwrap();
    VerifiedFileCopy::new_with_verification(
        source,
        target.stage_file(&cancel).unwrap(),
        verification,
    )
    .unwrap()
}

#[test]
fn explicit_cleanup_closes_partial_anonymous_stage_and_preserves_existing_target() {
    let temp = fixture();
    let directory = long_directory(temp.path());
    let existing = directory.join("existing");
    fs::write(&existing, b"keep existing bytes").unwrap();
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let target = service
        .prepare_transfer_destination(&directory, &cancel)
        .unwrap();
    let mut stage = target.stage_file(&cancel).unwrap();
    stage.write_all(&vec![73; 1_048_576]).unwrap();
    assert_eq!(stage.len().unwrap(), 1_048_576);
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    cancel.cancel();
    stage.cleanup().unwrap();
    assert!(
        stage.len().is_err(),
        "cleanup must close the retained partial inode"
    );
    stage.cleanup().unwrap();
    drop(stage);
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    assert_eq!(fs::read(existing).unwrap(), b"keep existing bytes");
}

#[test]
fn cancelling_partial_verified_copy_removes_stage_for_both_verification_modes() {
    for mode in [
        TransferVerification::FullSha256,
        TransferVerification::SizeAndSamples,
    ] {
        let temp = fixture();
        let directory = long_directory(temp.path());
        let source = temp.path().join("source 文档");
        let bytes = vec![91; 1_048_579];
        fs::write(&source, &bytes).unwrap();
        let completed = directory.join("already-completed");
        fs::write(&completed, b"completed copy stays").unwrap();
        let mut operation = copy(&source, &directory, mode);
        assert_eq!(
            operation.step(&Cancellation::default()).unwrap(),
            CopyPhase::Copying
        );
        assert!(operation.copied_bytes() > 0);
        assert!(operation.copied_bytes() < bytes.len() as u64);
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            operation.step(&cancel).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
        operation.cleanup().unwrap();
        assert!(
            operation
                .publish(OsStr::new("partial"), &Cancellation::default())
                .is_err()
        );
        drop(operation);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert_eq!(fs::read(completed).unwrap(), b"completed copy stays");
        assert_eq!(fs::read(source).unwrap(), bytes);
    }
}

#[test]
fn dropping_partial_copy_without_explicit_cleanup_leaves_no_visible_partial_file() {
    let temp = fixture();
    let directory = long_directory(temp.path());
    let source = temp.path().join("source");
    let bytes = vec![29; 1_048_581];
    fs::write(&source, &bytes).unwrap();
    let mut operation = copy(&source, &directory, TransferVerification::default());
    operation.step(&Cancellation::default()).unwrap();
    assert!(operation.copied_bytes() > 0);
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    drop(operation);
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), bytes);
}

#[test]
fn verified_publication_beyond_max_path_survives_cleanup() {
    let temp = fixture();
    let directory = long_directory(temp.path());
    let source = temp.path().join("source");
    let bytes = vec![43; 524_291];
    fs::write(&source, &bytes).unwrap();
    let mut operation = copy(&source, &directory, TransferVerification::default());
    let cancel = Cancellation::default();
    while operation.phase() != CopyPhase::ReadyToPublish {
        operation.step(&cancel).unwrap();
    }
    operation
        .publish(OsStr::new("complete 文档"), &cancel)
        .unwrap();
    operation.cleanup().unwrap();
    drop(operation);
    assert_eq!(fs::read(directory.join("complete 文档")).unwrap(), bytes);
    assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
    assert_eq!(fs::read(source).unwrap(), bytes);
}
