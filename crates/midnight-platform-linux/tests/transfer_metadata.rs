#![cfg(target_os = "linux")]
use midnight_core::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use midnight_platform_linux::LinuxFileSystem;
use std::{
    ffi::OsStr,
    fs::{self, File, FileTimes},
    time::{Duration, SystemTime},
};
fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-metadata/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}
#[test]
fn required_modified_time_survives_independent_readback_publication_and_handle_drop() {
    for expected in [
        SystemTime::UNIX_EPOCH + Duration::new(123456, 123456789),
        SystemTime::UNIX_EPOCH - Duration::from_nanos(1),
    ] {
        let temp = fixture();
        let path = temp.path().join("source 文档");
        fs::write(&path, vec![87; 700003]).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(expected))
            .unwrap();
        let fs = LinuxFileSystem::new(None);
        let cancel = Cancellation::default();
        let source = fs.prepare_transfer_source(&path, &cancel).unwrap();
        let pin = source.prepare_modified_time(&cancel).unwrap();
        assert_eq!(pin.modified(), expected);
        let destination = fs
            .prepare_transfer_destination(temp.path(), &cancel)
            .unwrap();
        let stage = destination
            .stage_file_with_metadata(TransferMetadataPolicy::RequireModifiedTime, &cancel)
            .unwrap();
        let mut copy = VerifiedFileCopy::new_with_modified_time(source, stage, pin).unwrap();
        while copy.phase() != CopyPhase::ReadyToPublish {
            copy.step(&cancel).unwrap();
        }
        copy.publish(OsStr::new("target"), &cancel).unwrap();
        drop(copy);
        assert_eq!(
            fs::metadata(temp.path().join("target"))
                .unwrap()
                .modified()
                .unwrap(),
            expected
        );
        assert_eq!(
            fs::read(temp.path().join("target")).unwrap(),
            fs::read(path).unwrap()
        );
    }
}
#[test]
fn source_pin_rejects_late_metadata_edit_and_directory_metadata_is_unsupported() {
    let temp = fixture();
    let path = temp.path().join("source");
    fs::write(&path, b"bytes").unwrap();
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service.prepare_transfer_source(&path, &cancel).unwrap();
    let pin = source.prepare_modified_time(&cancel).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();
    assert!(pin.revalidate(&cancel).is_err());
    let directory = service
        .prepare_transfer_source(temp.path(), &cancel)
        .unwrap();
    assert!(directory.prepare_modified_time(&cancel).is_err());
}
#[test]
fn metadata_capability_never_modifies_a_published_target_or_ignores_cancellation() {
    let temp = fixture();
    let service = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let directory = service
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let mut stage = directory
        .stage_file_with_metadata(TransferMetadataPolicy::RequireModifiedTime, &cancel)
        .unwrap();
    cancel.cancel();
    assert!(
        stage
            .set_modified_time(SystemTime::UNIX_EPOCH, &cancel)
            .is_err()
    );
    stage
        .publish_no_replace(OsStr::new("target"), &Cancellation::default())
        .unwrap();
    assert!(
        stage
            .set_modified_time(SystemTime::UNIX_EPOCH, &Cancellation::default())
            .is_err()
    );
}
