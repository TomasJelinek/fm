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
