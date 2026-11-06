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
