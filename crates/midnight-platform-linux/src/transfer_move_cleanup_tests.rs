//! Source staging must be reversible without retaining a recovery product.
use super::*;
use std::{ffi::OsString, fs, os::unix::ffi::OsStringExt};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = std::env::temp_dir().join("midnight-move/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    let parent = temp.path().to_owned();
    (temp, parent)
}

#[test]
fn explicit_restore_preserves_raw_source_name_and_removes_only_its_empty_stage() {
    let (_temp, parent) = fixture();
    let path = parent.join(OsString::from_vec(b"source-\xff".to_vec()));
    fs::write(&path, b"selected original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.restore().unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("source was not staged")
    };
    capture.restore().unwrap();
    capture.restore().unwrap();
    let cleanup = capture.take_stage_cleanup().unwrap();
    let stage = cleanup.path().to_owned();
    assert_eq!(stage, recovery.path.parent().unwrap());
    assert!(capture.take_stage_cleanup().is_none());
    drop(source);
    drop(capture);
    cleanup.cleanup().unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"selected original");
    assert!(!stage.exists());
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
}

#[test]
fn cancelled_capture_has_no_empty_stage_left_after_cleanup() {
    for after_rename in [false, true] {
        let (_temp, parent) = fixture();
        let path = parent.join("source 文档");
        fs::write(&path, b"original").unwrap();
        let cancel = Cancellation::default();
        let (source, capture) = prepare_native(&path, &cancel).unwrap();
        let result = capture.capture_with(
            &cancel,
            || {
                if !after_rename {
                    cancel.cancel();
                }
            },
            || cancel.cancel(),
        );
        if after_rename {
            assert!(matches!(
                result.unwrap(),
                TransferMoveCaptureOutcome::Restored { .. }
            ));
        } else {
            assert!(result.is_err());
        }
        capture.restore().unwrap();
        let cleanup = capture.take_stage_cleanup();
        drop(source);
        drop(capture);
        if let Some(cleanup) = cleanup {
            cleanup.cleanup().unwrap();
        }
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    }
}

#[test]
fn restore_conflict_preserves_new_winner_and_nonempty_temporary_directory() {
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"selected original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("source was not staged")
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
fn cleanup_refuses_a_replaced_stage_name_without_deleting_either_directory() {
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.capture(&cancel).unwrap();
    capture.restore().unwrap();
    let cleanup = capture.take_stage_cleanup().unwrap();
    let stage = cleanup.path().to_owned();
    let moved = parent.join("moved-stage");
    fs::rename(&stage, &moved).unwrap();
    fs::create_dir(&stage).unwrap();
    drop(source);
    drop(capture);
    assert!(cleanup.cleanup().is_err());
    assert!(stage.is_dir());
    assert!(moved.is_dir());
    assert_eq!(fs::read(path).unwrap(), b"original");
}

#[test]
fn restoration_sync_error_keeps_the_completed_namespace_fact_and_empty_stage_cleanup() {
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.capture(&cancel).unwrap();
    let failure = capture
        .restore_with(|_| {
            Err(error(
                &path,
                std::io::Error::other("injected restoration directory sync failure"),
            ))
        })
        .unwrap_err();
    assert!(failure.message.contains("sync failure"));
    assert!(capture.restoration_completed());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    let cleanup = capture.take_stage_cleanup().unwrap();
    drop(source);
    drop(capture);
    cleanup.cleanup().unwrap();
    assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
}
