#![cfg(windows)]
use midnight_core::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use midnight_platform_windows::WindowsFileSystem;
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
    let expected = SystemTime::UNIX_EPOCH + Duration::new(123456, 123456700);
    let temp = fixture();
    let path = temp.path().join("source 文档");
    fs::write(&path, vec![87; 700003]).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(expected))
        .unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service.prepare_transfer_source(&path, &cancel).unwrap();
    let pin = source.prepare_modified_time(&cancel).unwrap();
    assert_eq!(pin.modified(), expected);
    let directory = service
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let stage = directory
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
#[test]
fn directory_metadata_is_unsupported_and_cancelled_or_published_stages_cannot_be_changed() {
    let temp = fixture();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service
        .prepare_transfer_source(temp.path(), &cancel)
        .unwrap();
    assert!(source.prepare_modified_time(&cancel).is_err());
    drop(source);
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
#[test]
fn reserved_filetime_is_refused_without_changing_the_owned_stage() {
    let temp = fixture();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let directory = service
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let mut stage = directory
        .stage_file_with_metadata(TransferMetadataPolicy::RequireModifiedTime, &cancel)
        .unwrap();
    let before = stage.modified_time(&cancel).unwrap();
    let reserved = SystemTime::UNIX_EPOCH - Duration::from_secs(11_644_473_600);
    assert!(stage.set_modified_time(reserved, &cancel).is_err());
    assert_eq!(stage.modified_time(&cancel).unwrap(), before);
}
#[test]
fn source_metadata_edit_invalidates_the_pre_read_pin() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_WRITE_ATTRIBUTES,
    };
    let temp = fixture();
    let path = temp.path().join("source");
    fs::write(&path, b"source bytes").unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service.prepare_transfer_source(&path, &cancel).unwrap();
    let pin = source.prepare_modified_time(&cancel).unwrap();
    let writer = File::options()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    writer
        .set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();
    drop(writer);
    assert!(pin.revalidate(&cancel).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"source bytes");
}
