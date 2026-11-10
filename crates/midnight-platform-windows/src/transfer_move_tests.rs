use super::prepare_native;
use midnight_core::{
    Cancellation, TransferMoveCapture, TransferMoveCaptureOutcome, TransferService,
};
use std::{fs, path::PathBuf};

#[test]
fn native_drop_device_observes_actual_volume_without_capturing_a_source() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ordinary file");
    fs::write(&path, b"content must remain unchanged").unwrap();
    let filesystem = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let directory = filesystem
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let expected = Some(directory.identity().device);
    let no_delete = fs::File::options()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_READ_DATA)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .unwrap();
    assert!(
        filesystem.prepare_move_source(&path, &cancel).is_err(),
        "fixture pin denies DELETE access"
    );
    assert_eq!(
        filesystem.native_drop_device(&path, &cancel).unwrap(),
        expected
    );
    assert_eq!(
        filesystem.native_drop_device(temp.path(), &cancel).unwrap(),
        expected
    );
    drop(no_delete);
    drop(filesystem.prepare_move_source(&path, &cancel).unwrap());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(fs::read(&path).unwrap(), b"content must remain unchanged");
    cancel.cancel();
    assert_eq!(
        filesystem
            .native_drop_device(&path, &cancel)
            .unwrap_err()
            .kind,
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn native_move_preserves_identity_and_streams_without_creating_copy_stages() {
    use midnight_core::{TransferNativeMoveOutcome, TransferSource};
    let (_temp, parent) = fixture();
    let source_path = parent.join("source 文档");
    fs::write(&source_path, b"original").unwrap();
    let mut ads = source_path.as_os_str().to_owned();
    ads.push(":notes");
    fs::write(PathBuf::from(ads), b"retained stream").unwrap();
    let destination_path = parent.join("target");
    fs::create_dir(&destination_path).unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&source_path, &cancel).unwrap();
    let expected = source.entry().identity;
    let filesystem = crate::WindowsFileSystem::new(None);
    let destination = filesystem
        .prepare_transfer_destination(&destination_path, &cancel)
        .unwrap();
    assert!(destination.supports_native_move_replacement(expected));
    let outcome = capture
        .try_native_move(
            destination.as_ref(),
            std::ffi::OsStr::new("moved"),
            None,
            &cancel,
        )
        .unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::Moved { ref recovery, .. } if recovery.is_empty())
    );
    let moved = destination
        .inspect(std::ffi::OsStr::new("moved"), &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(moved.identity, expected);
    assert!(!source_path.exists());
    drop(source);
    drop(capture);
    assert_eq!(
        fs::read(destination_path.join("moved")).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(destination_path.join("moved:notes")).unwrap(),
        b"retained stream"
    );
    assert_eq!(fs::read_dir(destination_path).unwrap().count(), 1);
}

#[test]
fn native_move_collision_cancel_case_and_hardlink_alias_keep_exact_entries() {
    use midnight_core::TransferNativeMoveOutcome;
    use std::ffi::OsStr;
    let (_temp, parent) = fixture();
    let source = parent.join("Case.txt");
    fs::write(&source, b"source").unwrap();
    fs::write(parent.join("winner"), b"winner").unwrap();
    fs::hard_link(&source, parent.join("alias")).unwrap();
    let cancel = Cancellation::default();
    let (_source, capture) = prepare_native(&source, &cancel).unwrap();
    let destination = crate::WindowsFileSystem::new(None)
        .prepare_transfer_destination(&parent, &cancel)
        .unwrap();
    assert!(matches!(
        capture
            .try_native_move(destination.as_ref(), OsStr::new("Case.txt"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::SameEntry
    ));
    assert!(
        capture
            .try_native_move(destination.as_ref(), OsStr::new("alias"), None, &cancel)
            .is_err()
    );
    assert!(
        capture
            .try_native_move(destination.as_ref(), OsStr::new("winner"), None, &cancel)
            .is_err()
    );
    let stopped = Cancellation::default();
    stopped.cancel();
    assert!(
        capture
            .try_native_move(
                destination.as_ref(),
                OsStr::new("cancelled"),
                None,
                &stopped
            )
            .is_err()
    );
    assert!(source.exists());
    assert_eq!(fs::read(parent.join("winner")).unwrap(), b"winner");
    assert!(matches!(
        capture
            .try_native_move(destination.as_ref(), OsStr::new("CASE.txt"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { .. }
    ));
    let names: Vec<_> = fs::read_dir(&parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(names.contains(&"CASE.txt".into()));
    assert!(!names.contains(&"Case.txt".into()));
    assert!(names.contains(&"alias".into()));
}

#[test]
fn native_move_replacement_retains_original_and_conflict_gap_winners() {
    use midnight_core::{TransferNativeMoveOutcome, TransferReplacementPolicy};
    use std::ffi::OsStr;
    for gap in ["none", "cancel", "winner"] {
        let (_temp, parent) = fixture();
        let source = parent.join("source");
        fs::write(&source, b"source").unwrap();
        let destination_path = parent.join("target");
        fs::create_dir(&destination_path).unwrap();
        let final_path = destination_path.join("final");
        fs::write(&final_path, b"old target").unwrap();
        let cancel = Cancellation::default();
        let (_source, capture) = prepare_native(&source, &cancel).unwrap();
        let destination = crate::WindowsFileSystem::new(None)
            .prepare_transfer_destination(&destination_path, &cancel)
            .unwrap();
        let expected = destination
            .prepare_target(OsStr::new("final"), &cancel)
            .unwrap()
            .unwrap();
        let outcome = super::native_move::attempt_with(
            &capture,
            destination.as_ref(),
            OsStr::new("final"),
            Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
            || match gap {
                "cancel" => cancel.cancel(),
                "winner" => fs::write(&final_path, b"new winner").unwrap(),
                _ => {}
            },
        );
        match gap {
            "none" => {
                let TransferNativeMoveOutcome::Moved { recovery, .. } = outcome.unwrap() else {
                    panic!("native replacement did not commit")
                };
                assert_eq!(recovery.len(), 1);
                assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old target");
                assert!(!source.exists());
                drop(capture);
                drop(_source);
                assert_eq!(fs::read(&final_path).unwrap(), b"source");
            }
            "cancel" => {
                assert!(
                    matches!(outcome.unwrap(), TransferNativeMoveOutcome::RecoveryRequired { destination: None, recovery, .. } if recovery.is_empty())
                );
                assert!(source.exists());
                assert_eq!(fs::read(&final_path).unwrap(), b"old target");
                assert_eq!(fs::read_dir(&destination_path).unwrap().count(), 1);
            }
            "winner" => {
                let TransferNativeMoveOutcome::RecoveryRequired {
                    destination,
                    recovery,
                    ..
                } = outcome.unwrap()
                else {
                    panic!("gap winner must retain original recovery")
                };
                assert!(destination.is_none());
                assert!(source.exists());
                assert_eq!(fs::read(&final_path).unwrap(), b"new winner");
                assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old target");
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn lazy_move_validation_seeks_the_retained_file_after_capture() {
    use midnight_core::TransferSource;
    let (_temp, parent) = fixture();
    let path = parent.join("retained");
    fs::write(&path, b"original bytes").unwrap();
    let cancel = Cancellation::default();
    let (mut source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.preflight_finalization(&cancel).unwrap();
    capture.capture(&cancel).unwrap();
    fs::write(&path, b"different new namespace winner").unwrap();
    let guard = source.rewind_for_validation(&cancel).unwrap();
    assert_eq!(source.seek_for_validation(9, &cancel).unwrap(), 14);
    let mut bytes = [0; 5];
    assert_eq!(source.read_chunk(&mut bytes, &cancel).unwrap(), 5);
    assert_eq!(&bytes, b"bytes");
    assert!(source.seek_for_validation(15, &cancel).is_err());
    assert_eq!(source.seek_for_validation(0, &cancel).unwrap(), 14);
    guard.revalidate(&cancel).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"different new namespace winner");
}

#[test]
fn native_move_merge_lists_lazily_and_removes_only_the_empty_source_directory() {
    use midnight_core::TransferNativeMoveOutcome;
    use std::ffi::OsStr;
    let (_temp, parent) = fixture();
    let source = parent.join("source-dir");
    let target = parent.join("existing-dir");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("child"), b"moved").unwrap();
    fs::write(target.join("kept"), b"kept").unwrap();
    let filesystem = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut prepared = filesystem.prepare_move_source(&source, &cancel).unwrap();
    let destination = filesystem
        .prepare_move_destination(&target, &cancel)
        .unwrap();
    let child = prepared.source.next_move_child(&cancel).unwrap().unwrap();
    assert!(matches!(
        child
            .capture
            .try_native_move(destination.as_ref(), OsStr::new("child"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { .. }
    ));
    drop(child);
    assert!(prepared.source.next_move_child(&cancel).unwrap().is_none());
    prepared.capture.remove_empty_directory(&cancel).unwrap();
    drop(prepared);
    assert!(!source.exists());
    assert_eq!(fs::read(target.join("child")).unwrap(), b"moved");
    assert_eq!(fs::read(target.join("kept")).unwrap(), b"kept");
}

#[test]
fn native_move_stale_replacement_consent_preserves_source_and_new_target() {
    use midnight_core::TransferReplacementPolicy;
    use std::ffi::OsStr;
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    let target = parent.join("target");
    fs::write(&source, b"source").unwrap();
    fs::write(&target, b"old target").unwrap();
    let cancel = Cancellation::default();
    let prepared = crate::WindowsFileSystem::new(None)
        .prepare_move_source(&source, &cancel)
        .unwrap();
    let destination = crate::WindowsFileSystem::new(None)
        .prepare_transfer_destination(&parent, &cancel)
        .unwrap();
    let consent = destination
        .prepare_target(OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    fs::rename(&target, parent.join("old-away")).unwrap();
    fs::write(&target, b"new target").unwrap();
    let failure = prepared
        .capture
        .try_native_move(
            destination.as_ref(),
            OsStr::new("target"),
            Some((consent, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
        )
        .unwrap_err();
    assert_eq!(failure.kind, std::io::ErrorKind::AlreadyExists);
    assert!(source.exists());
    assert_eq!(fs::read(&target).unwrap(), b"new target");
    assert_eq!(fs::read(parent.join("old-away")).unwrap(), b"old target");
}

#[test]
#[ignore = "requires NATIVE_MOVE_OTHER_VOLUME_ROOT on a distinct Windows volume"]
fn native_move_cross_volume_reports_unchanged_namespace() {
    use midnight_core::TransferNativeMoveOutcome;
    use std::ffi::OsStr;
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"source bytes").unwrap();
    let other = tempfile::tempdir_in(
        std::env::var_os("NATIVE_MOVE_OTHER_VOLUME_ROOT")
            .expect("owned fixture root on a different volume"),
    )
    .unwrap();
    fs::write(other.path().join("target"), b"existing target").unwrap();
    let filesystem = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = filesystem.prepare_move_source(&path, &cancel).unwrap();
    let destination = filesystem
        .prepare_move_destination(other.path(), &cancel)
        .unwrap();
    assert_ne!(
        prepared.source.entry().identity.device,
        destination.identity().device,
        "fixture must use distinct actual volume identities"
    );
    assert!(!destination.supports_native_move_replacement(prepared.source.entry().identity));
    assert!(matches!(
        prepared
            .capture
            .try_native_move(destination.as_ref(), OsStr::new("target"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::CrossDevice
    ));
    assert_eq!(fs::read(&path).unwrap(), b"source bytes");
    assert_eq!(
        fs::read(other.path().join("target")).unwrap(),
        b"existing target"
    );
    assert_eq!(fs::read_dir(other.path()).unwrap().count(), 1);
}

#[test]
fn ordinary_move_cleans_stage_while_an_external_reader_retains_bytes() {
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::*;
    let (_temp, parent) = fixture();
    let path = parent.join("ordinary 文档");
    fs::write(&path, b"verified original").unwrap();
    let mut reader = fs::File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.preflight_finalization(&cancel).unwrap();
    capture.capture(&cancel).unwrap();
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Removed { .. }
    ));
    let cleanup = capture.take_stage_cleanup().unwrap();
    let stage = cleanup.path().to_owned();
    drop(finalizer);
    drop(source);
    drop(capture);
    cleanup.cleanup().unwrap();
    assert!(!stage.exists());
    assert!(!path.exists());
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"verified original");
    assert_eq!(fs::read_dir(parent).unwrap().count(), 1); // fixture seed
}

#[test]
fn late_ads_refusal_restores_exact_original_and_cleans_temporary_stage() {
    for after_prepare in [false, true] {
        let (_temp, parent) = fixture();
        let path = parent.join("ADS 文档");
        fs::write(&path, b"verified unnamed bytes").unwrap();
        let cancel = Cancellation::default();
        let (source, capture) = prepare_native(&path, &cancel).unwrap();
        capture.preflight_finalization(&cancel).unwrap();
        let TransferMoveCaptureOutcome::Captured { recovery, .. } =
            capture.capture(&cancel).unwrap()
        else {
            panic!("not staged")
        };
        let mut finalizer = after_prepare.then(|| capture.prepare_finalization(&cancel).unwrap());
        let mut ads = recovery.path.as_os_str().to_owned();
        ads.push(":late-notes");
        fs::write(PathBuf::from(ads), b"unverified stream").unwrap();
        if let Some(finalizer) = finalizer.as_mut() {
            assert!(matches!(
                finalizer.finalize(&cancel),
                midnight_core::TransferMoveFinalizeOutcome::Retained { .. }
            ));
        } else {
            assert!(capture.prepare_finalization(&cancel).is_err());
        }
        drop(finalizer);
        capture.restore().unwrap();
        let cleanup = capture.take_stage_cleanup().unwrap();
        drop(source);
        drop(capture);
        cleanup.cleanup().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"verified unnamed bytes");
        let mut ads = path.as_os_str().to_owned();
        ads.push(":late-notes");
        assert_eq!(fs::read(PathBuf::from(ads)).unwrap(), b"unverified stream");
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 2);
    }
}

#[test]
fn restore_conflict_never_clobbers_new_winner_or_cleans_nonempty_stage() {
    let (_temp, parent) = fixture();
    let path = parent.join("conflict");
    fs::write(&path, b"selected original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("not staged")
    };
    fs::write(&path, b"new winner").unwrap();
    assert!(capture.restore().is_err());
    let cleanup = capture.take_stage_cleanup().unwrap();
    drop(source);
    drop(capture);
    assert!(cleanup.cleanup().is_err());
    assert_eq!(fs::read(&path).unwrap(), b"new winner");
    assert_eq!(fs::read(recovery.path).unwrap(), b"selected original");
}
#[test]
fn retained_validation_survives_source_capture_and_proves_published_binding() {
    use std::io::{Read, Write};
    let (_temp, parent) = fixture();
    let source = parent.join("retained-source");
    fs::write(&source, b"verified bytes").unwrap();
    let provider = crate::WindowsFileSystem::new(None);
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
    use midnight_core::{TransferPublication, TransferReplacementPolicy};
    use std::io::Write;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    // Bootstrap a protected user-only folder through the existing recovery capability;
    // inherited workspace fixture ACLs intentionally permit other sandbox identities.
    fs::write(temp.path().join("seed"), b"fixture seed").unwrap();
    let provider = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let destination = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let target = destination
        .prepare_target(std::ffi::OsStr::new("seed"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = destination
        .stage_file_for_target(
            std::ffi::OsStr::new("seed"),
            target,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"replacement seed").unwrap();
    let receipt = match stage
        .publish_replace(std::ffi::OsStr::new("seed"), &cancel)
        .unwrap()
    {
        TransferPublication::Replaced { recovery, .. } => recovery,
        other => panic!("fixture recovery creation failed: {other:?}"),
    };
    let parent = receipt.path.parent().unwrap().to_owned();
    drop(stage);
    (temp, parent)
}

#[test]
fn preparation_does_not_capture_or_create_recovery_until_explicit_invocation() {
    let (_temp, parent) = fixture();
    let source = parent.join("source 文档 🦀");
    fs::write(&source, b"original").unwrap();
    let before = fs::read_dir(&parent).unwrap().count();
    let prepared = crate::WindowsFileSystem::new(None)
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
    let prepared = crate::WindowsFileSystem::new(None)
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
    let prepared = crate::WindowsFileSystem::new(None)
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
fn move_root_sharing_rejects_writers_and_source_name_substitution() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let prepared = crate::WindowsFileSystem::new(None)
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap();
    assert!(fs::rename(&source, parent.join("source-away")).is_err());
    assert!(fs::OpenOptions::new().write(true).open(&source).is_err());
    drop(prepared);
    assert_eq!(fs::read(&source).unwrap(), b"original");
}

#[test]
fn long_unicode_source_name_is_retained_without_a_max_path_ceiling() {
    use std::os::windows::ffi::OsStrExt;
    let (_temp, parent) = fixture();
    let source = parent.join("文".repeat(200));
    assert!(source.as_os_str().encode_wide().count() > 260);
    fs::write(&source, b"long source bytes").unwrap();
    let prepared = crate::WindowsFileSystem::new(None)
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
    drop(prepared);
    assert_eq!(fs::read(&recovery.path).unwrap(), b"long source bytes");
}

#[test]
fn cancellation_after_capture_restores_the_exact_original_handle() {
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
fn a_new_source_name_winner_is_never_replaced_during_cancellation_cleanup() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let (_reader, capture) = prepare_native(&source, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    let result = capture
        .capture_with(
            &cancel,
            || {},
            || {
                fs::write(&source, b"new name winner").unwrap();
                cancel.cancel();
            },
        )
        .unwrap();
    let recovery = match result {
        TransferMoveCaptureOutcome::RecoveryRequired { recovery, .. } => recovery,
        other => panic!("unexpected restore outcome: {other:?}"),
    };
    drop(capture);
    assert_eq!(fs::read(&source).unwrap(), b"new name winner");
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"original");
}

#[test]
fn an_existing_incompatible_writer_refuses_move_preparation() {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let writer = fs::OpenOptions::new().write(true).open(&source).unwrap();
    assert!(
        crate::WindowsFileSystem::new(None)
            .prepare_move_source(&source, &Cancellation::default())
            .is_err()
    );
    drop(writer);
    assert_eq!(fs::read(&source).unwrap(), b"original");
}

#[test]
fn unsafe_source_parent_is_refused_before_verified_capture_and_copy_still_works() {
    use std::{
        mem::size_of,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        ptr,
    };
    use windows_sys::Win32::{Security::*, Storage::FileSystem::*};
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let file = fs::OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(&parent)
        .unwrap();
    let mut world = [0u32; 17];
    let mut sid_length = 68;
    // SAFETY: owned writable SID buffer of the native maximum size.
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinWorldSid,
                ptr::null_mut(),
                world.as_mut_ptr().cast(),
                &mut sid_length,
            )
        },
        0
    );
    let length =
        size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid_length as usize;
    let mut acl = vec![0u64; length.div_ceil(8)];
    let acl = acl.as_mut_ptr().cast::<ACL>();
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: ACL/descriptor/SID buffers are live and writable; only this fresh owned
    // fixture parent receives a deliberately unsafe DACL. No workspace/root ACL changes.
    assert_ne!(
        unsafe { InitializeAcl(acl, length as u32, ACL_REVISION) },
        0
    );
    assert_ne!(
        unsafe {
            AddAccessAllowedAce(
                acl,
                ACL_REVISION,
                FILE_ALL_ACCESS,
                world.as_mut_ptr().cast(),
            )
        },
        0
    );
    assert_ne!(unsafe { InitializeSecurityDescriptor(descriptor, 1) }, 0);
    assert_ne!(
        unsafe { SetSecurityDescriptorDacl(descriptor, 1, acl, 0) },
        0
    );
    assert_ne!(
        unsafe {
            SetKernelObjectSecurity(
                file.as_raw_handle(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            )
        },
        0
    );
    drop(file);
    let provider = crate::WindowsFileSystem::new(None);
    // Metadata-only native rename admission does not need recovery staging.
    // Both cross-volume proof admission and direct capture still reject it.
    let prepared = provider
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap();
    let error = prepared
        .capture
        .preflight_finalization(&Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
    assert!(error.message.contains("Copy"));
    assert_eq!(
        prepared
            .capture
            .capture(&Cancellation::default())
            .unwrap_err()
            .kind,
        std::io::ErrorKind::Unsupported
    );
    drop(prepared);
    assert!(
        provider
            .prepare_transfer_source(&source, &Cancellation::default())
            .is_ok()
    );
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 2); // fixture seed + source
}

#[test]
fn explicit_finalization_removes_retained_original_but_never_new_source_winner() {
    let (_temp, parent) = fixture();
    let path = parent.join("finalize 文档 🦀");
    fs::write(&path, b"verified original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.preflight_finalization(&cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("capture failed")
    };
    fs::write(&path, b"new winner").unwrap();
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    finalizer.revalidate(&cancel).unwrap();
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Removed { .. }
    ));
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Removed { .. }
    ));
    drop(finalizer);
    drop(source);
    drop(capture);
    assert!(
        !recovery.path.exists(),
        "the captured original must actually be removed"
    );
    assert_eq!(fs::read(&path).unwrap(), b"new winner");
}

#[test]
fn finalization_cancel_and_drop_preserve_original() {
    for cancelled in [false, true] {
        let (_temp, parent) = fixture();
        let path = parent.join("cancel-finalize");
        fs::write(&path, b"original").unwrap();
        let cancel = Cancellation::default();
        let (source, capture) = prepare_native(&path, &cancel).unwrap();
        let TransferMoveCaptureOutcome::Captured { recovery, .. } =
            capture.capture(&cancel).unwrap()
        else {
            panic!("capture failed")
        };
        let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
        if cancelled {
            cancel.cancel();
            assert!(matches!(
                finalizer.finalize(&cancel),
                midnight_core::TransferMoveFinalizeOutcome::Retained { .. }
            ));
        }
        drop(finalizer);
        drop(source);
        drop(capture);
        assert_eq!(fs::read(recovery.path).unwrap(), b"original");
    }
}

#[test]
fn finalization_preflight_refuses_unverified_hardlink_aliases() {
    let (_temp, parent) = fixture();
    let path = parent.join("linked-source");
    fs::write(&path, b"original").unwrap();
    fs::hard_link(&path, parent.join("alias")).unwrap();
    let cancel = Cancellation::default();
    let (_, capture) = prepare_native(&path, &cancel).unwrap();
    assert!(capture.preflight_finalization(&cancel).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn finalization_preflight_refuses_named_streams_including_empty_streams() {
    for content in [b"".as_slice(), b"unverified stream bytes".as_slice()] {
        let (_temp, parent) = fixture();
        let path = parent.join("streams 文档");
        fs::write(&path, b"verified unnamed data").unwrap();
        let mut named = path.as_os_str().to_owned();
        named.push(":notes");
        fs::write(std::path::PathBuf::from(&named), content).unwrap();
        let (_, capture) = prepare_native(&path, &Cancellation::default()).unwrap();
        assert!(
            capture
                .preflight_finalization(&Cancellation::default())
                .is_err()
        );
        drop(capture);
        assert_eq!(fs::read(std::path::PathBuf::from(named)).unwrap(), content);
        assert_eq!(fs::read(path).unwrap(), b"verified unnamed data");
    }
}

#[test]
fn finalization_preflight_refuses_readonly_without_changing_attributes() {
    let (_temp, parent) = fixture();
    let path = parent.join("readonly-source");
    fs::write(&path, b"original").unwrap();
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let (_, capture) = prepare_native(&path, &Cancellation::default()).unwrap();
    assert!(
        capture
            .preflight_finalization(&Cancellation::default())
            .is_err()
    );
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    drop(capture);
    fs::set_permissions(path, original).unwrap();
}

#[test]
fn finalization_refuses_late_retained_metadata_change_and_preserves_original() {
    use std::{
        fs::{File, FileTimes},
        os::windows::fs::OpenOptionsExt,
        time::Duration,
    };
    use windows_sys::Win32::Storage::FileSystem::*;
    let (_temp, parent) = fixture();
    let path = parent.join("late-metadata");
    fs::write(&path, b"verified original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("capture failed")
    };
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    let attributes = File::options()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&recovery.path)
        .unwrap();
    let changed = attributes.metadata().unwrap().modified().unwrap() + Duration::from_secs(1);
    attributes
        .set_times(FileTimes::new().set_modified(changed))
        .unwrap();
    drop(attributes);
    assert!(finalizer.revalidate(&cancel).is_err());
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Retained { .. }
    ));
    drop(finalizer);
    drop(source);
    drop(capture);
    assert_eq!(fs::read(recovery.path).unwrap(), b"verified original");
}

#[test]
fn normal_disposition_keeps_external_reader_bytes_until_the_reader_closes() {
    use std::{fs::OpenOptions, io::Read, os::windows::fs::OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::*;
    let (_temp, parent) = fixture();
    let path = parent.join("external-reader");
    fs::write(&path, b"verified original").unwrap();
    let mut reader = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("capture failed")
    };
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Removed {
            state: midnight_core::TransferSourceRemoval::DeletePending,
            durability_confirmed: false,
            ..
        }
    ));
    drop(finalizer);
    drop(source);
    drop(capture);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"verified original");
    drop(reader);
    assert!(!recovery.path.exists());
}

#[test]
fn late_named_stream_after_capture_blocks_removal_even_with_unchanged_main_bytes() {
    let (_temp, parent) = fixture();
    let path = parent.join("late-stream");
    fs::write(&path, b"verified unnamed data").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("capture failed")
    };
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    let mut named = recovery.path.as_os_str().to_owned();
    named.push(":late");
    let named = std::path::PathBuf::from(named);
    fs::write(&named, b"unverified bytes").unwrap();
    assert!(finalizer.revalidate(&cancel).is_err());
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Retained { .. }
    ));
    drop(finalizer);
    drop(source);
    drop(capture);
    assert_eq!(fs::read(named).unwrap(), b"unverified bytes");
    assert_eq!(fs::read(recovery.path).unwrap(), b"verified unnamed data");
}
