use super::prepare_native;
use midnight_core::{
    Cancellation, TransferMoveCapture, TransferMoveCaptureOutcome, TransferService,
};
use std::{fs, path::PathBuf};
#[test]
fn retained_validation_survives_source_capture_and_proves_published_binding() {
    use std::io::{Read, Write};
    let (_temp, parent) = fixture();
    let source = parent.join("retained-source");
    fs::write(&source, b"verified bytes").unwrap();
    let provider = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut prepared = provider.prepare_move_source(&source, &cancel).unwrap();
    let guard = prepared.source.rewind_for_validation(&cancel).unwrap();
    let mut bytes = [0; 14];
    assert_eq!(prepared.source.read_chunk(&mut bytes, &cancel).unwrap(), 14);
    guard.revalidate(&cancel).unwrap();
    drop(guard);
    prepared.capture.capture(&cancel).unwrap();
    let guard = prepared.source.rewind_for_validation(&cancel).unwrap();
    assert_eq!(prepared.source.read_chunk(&mut bytes, &cancel).unwrap(), 14);
    assert_eq!(&bytes, b"verified bytes");
    guard.revalidate(&cancel).unwrap();
    let destination = provider.prepare_move_destination(&parent, &cancel).unwrap();
    let mut stage = destination.stage_file(&cancel).unwrap();
    let guard = stage.rewind_for_validation(&cancel).unwrap();
    guard.revalidate(&cancel).unwrap();
    drop(guard);
    stage.write_all(b"verified bytes").unwrap();
    assert!(stage.revalidate_publication(&cancel).is_err());
    stage
        .publish_no_replace(std::ffi::OsStr::new("published"), &cancel)
        .unwrap();
    let identity = stage.revalidate_publication(&cancel).unwrap();
    assert_eq!(
        destination
            .inspect(std::ffi::OsStr::new("published"), &cancel)
            .unwrap()
            .unwrap()
            .identity,
        identity
    );
    let guard = stage.rewind_for_validation(&cancel).unwrap();
    let mut bytes = Vec::new();
    stage.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"verified bytes");
    guard.revalidate(&cancel).unwrap();
}

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let root = std::env::temp_dir().join("midnight-move/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    let parent = temp.path().to_owned();
    (temp, parent)
}

#[test]
fn proof_rejects_live_writer_changes_and_substituted_published_names() {
    use std::io::Write;
    let (_temp, parent) = fixture();
    let source = parent.join("mutable-source");
    fs::write(&source, b"original").unwrap();
    let provider = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut prepared = provider.prepare_move_source(&source, &cancel).unwrap();
    let guard = prepared.source.rewind_for_validation(&cancel).unwrap();
    fs::write(&source, b"modified length").unwrap();
    assert!(guard.revalidate(&cancel).is_err());
    let destination = provider.prepare_move_destination(&parent, &cancel).unwrap();
    let mut stage = destination.stage_file(&cancel).unwrap();
    stage.write_all(b"original").unwrap();
    stage
        .publish_no_replace(std::ffi::OsStr::new("published"), &cancel)
        .unwrap();
    let guard = stage.rewind_for_validation(&cancel).unwrap();
    fs::write(parent.join("published"), b"modified length").unwrap();
    assert!(guard.revalidate(&cancel).is_err());
    fs::rename(parent.join("published"), parent.join("retained-target")).unwrap();
    std::os::unix::fs::symlink("retained-target", parent.join("published")).unwrap();
    assert!(stage.revalidate_publication(&cancel).is_err());
    drop(stage);
    assert_eq!(
        fs::read(parent.join("retained-target")).unwrap(),
        b"modified length"
    );
}

#[test]
fn preparation_does_not_capture_or_create_recovery_until_explicit_invocation() {
    let (_temp, parent) = fixture();
    let source = parent.join("source 文档 🦀");
    fs::write(&source, b"original").unwrap();
    let before = fs::read_dir(&parent).unwrap().count();
    let prepared = crate::LinuxFileSystem::new(None)
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap();
    assert_eq!(
        prepared.source.entry().identity,
        prepared.capture.entry().identity
    );
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read_dir(&parent).unwrap().count(), before);
}

#[test]
fn whole_root_capture_retains_every_child_and_survives_all_pin_drops() {
    let (_temp, parent) = fixture();
    let source = parent.join("source 文档 🦀");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/data"), b"original bytes").unwrap();
    let prepared = crate::LinuxFileSystem::new(None)
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap();
    let identity = prepared.capture.entry().identity;
    let outcome = prepared.capture.capture(&Cancellation::default()).unwrap();
    let recovery = match &outcome {
        TransferMoveCaptureOutcome::Captured {
            source: actual,
            recovery,
        } => {
            assert_eq!(actual, &source);
            assert_eq!(recovery.identity, Some(identity));
            recovery.clone()
        }
        other => panic!("unexpected capture: {other:?}"),
    };
    assert_eq!(recovery.path.file_name(), source.file_name());
    assert_eq!(recovery.path.parent().unwrap().parent(), source.parent());
    // A second invocation must not capture a newly created source entry.
    fs::create_dir(&source).unwrap();
    fs::write(source.join("winner"), b"new source name").unwrap();
    let repeated = prepared.capture.capture(&Cancellation::default()).unwrap();
    match repeated {
        TransferMoveCaptureOutcome::Captured {
            recovery: repeated, ..
        } => assert_eq!(repeated.path, recovery.path),
        other => panic!("repeat changed committed outcome: {other:?}"),
    }
    drop(prepared);
    assert_eq!(
        fs::read(recovery.path.join("nested/data")).unwrap(),
        b"original bytes"
    );
    assert_eq!(fs::read(source.join("winner")).unwrap(), b"new source name");
}

#[test]
fn cancellation_before_capture_retains_the_original_name_and_bytes() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let prepared = crate::LinuxFileSystem::new(None)
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    let error = prepared.capture.capture(&cancel).unwrap_err();
    assert_eq!(error.kind, std::io::ErrorKind::Interrupted);
    drop(prepared);
    assert_eq!(fs::read(source).unwrap(), b"original");
}

#[test]
fn raw_source_name_and_an_existing_writer_remain_lossless_after_capture() {
    use std::{
        ffi::OsString,
        io::{Seek, SeekFrom, Write},
        os::unix::ffi::OsStringExt,
    };
    let (_temp, parent) = fixture();
    let source = parent.join(OsString::from_vec(b"source-\xff".to_vec()));
    fs::write(&source, b"original bytes").unwrap();
    let mut writer = fs::OpenOptions::new().write(true).open(&source).unwrap();
    let prepared = crate::LinuxFileSystem::new(None)
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap();
    let recovery = match prepared.capture.capture(&Cancellation::default()).unwrap() {
        TransferMoveCaptureOutcome::Captured {
            source: actual,
            recovery,
        } => {
            assert_eq!(actual, source);
            recovery
        }
        other => panic!("unexpected capture: {other:?}"),
    };
    writer.seek(SeekFrom::Start(0)).unwrap();
    writer.write_all(b"changed! bytes").unwrap();
    writer.sync_all().unwrap();
    drop(writer);
    drop(prepared);
    assert!(!source.exists());
    assert_eq!(fs::read(recovery.path).unwrap(), b"changed! bytes");
}

#[test]
fn late_substituted_source_is_restored_without_deleting_either_inode() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"selected original").unwrap();
    let (_reader, capture) = prepare_native(&source, &Cancellation::default()).unwrap();
    let result = capture
        .capture_with(
            &Cancellation::default(),
            || {
                fs::rename(&source, parent.join("original-away")).unwrap();
                fs::write(&source, b"raced entry").unwrap();
            },
            || {},
        )
        .unwrap();
    assert!(matches!(
        result,
        TransferMoveCaptureOutcome::Restored { .. }
    ));
    drop(capture);
    assert_eq!(fs::read(&source).unwrap(), b"raced entry");
    assert_eq!(
        fs::read(parent.join("original-away")).unwrap(),
        b"selected original"
    );
}

#[test]
fn a_new_source_name_winner_blocks_restoration_and_all_objects_remain() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"selected original").unwrap();
    let (_reader, capture) = prepare_native(&source, &Cancellation::default()).unwrap();
    let result = capture
        .capture_with(
            &Cancellation::default(),
            || {
                fs::rename(&source, parent.join("original-away")).unwrap();
                fs::write(&source, b"raced entry").unwrap();
            },
            || fs::write(&source, b"new name winner").unwrap(),
        )
        .unwrap();
    let recovery = match result {
        TransferMoveCaptureOutcome::RecoveryRequired { recovery, .. } => recovery,
        other => panic!("unexpected restore outcome: {other:?}"),
    };
    drop(capture);
    assert_eq!(fs::read(&source).unwrap(), b"new name winner");
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"raced entry");
    assert_eq!(
        fs::read(parent.join("original-away")).unwrap(),
        b"selected original"
    );
}

#[test]
fn cancellation_after_capture_restores_without_replacement() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let (_reader, capture) = prepare_native(&source, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    let result = capture
        .capture_with(&cancel, || {}, || cancel.cancel())
        .unwrap();
    assert!(matches!(
        result,
        TransferMoveCaptureOutcome::Restored { .. }
    ));
    assert_eq!(fs::read(&source).unwrap(), b"original");
}

#[test]
fn unsafe_source_parent_is_refused_before_any_capture_and_copy_still_works() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
    let provider = crate::LinuxFileSystem::new(None);
    let error = match provider.prepare_move_source(&source, &Cancellation::default()) {
        Ok(_) => panic!("unsafe parent admitted"),
        Err(error) => error,
    };
    assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
    assert!(error.message.contains("Copy"));
    assert!(
        provider
            .prepare_transfer_source(&source, &Cancellation::default())
            .is_ok()
    );
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
}

#[test]
fn capture_preserves_a_descendant_link_without_following_its_target() {
    use std::os::unix::fs::symlink;
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    let outside = parent.join("outside");
    fs::write(&outside, b"outside bytes").unwrap();
    fs::create_dir(&source).unwrap();
    symlink(&outside, source.join("link")).unwrap();
    let prepared = crate::LinuxFileSystem::new(None)
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap();
    let recovery = match prepared.capture.capture(&Cancellation::default()).unwrap() {
        TransferMoveCaptureOutcome::Captured { recovery, .. } => recovery,
        other => panic!("unexpected capture: {other:?}"),
    };
    drop(prepared);
    assert_eq!(fs::read_link(recovery.path.join("link")).unwrap(), outside);
    assert_eq!(fs::read(&outside).unwrap(), b"outside bytes");
}

#[test]
fn capture_drop_without_explicit_finalization_preserves_the_original() {
    let (_temp, parent) = fixture();
    let path = parent.join("source 文档");
    fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("capture failed")
    };
    drop(capture);
    drop(source);
    assert_eq!(fs::read(recovery.path).unwrap(), b"original");
}
