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
fn unsafe_source_parent_is_refused_before_any_capture_and_copy_still_works() {
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
