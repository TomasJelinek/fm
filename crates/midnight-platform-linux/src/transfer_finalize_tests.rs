//! Native source finalization uses a live lease and exact private-stage binding.
use super::*;
use std::{fs, io::Write, os::unix::fs::OpenOptionsExt};

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let root = std::env::temp_dir().join("midnight-move/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::tempdir_in(root).unwrap();
    let parent = temp.path().to_owned();
    (temp, parent)
}

#[test]
fn ordinary_move_finalization_unlinks_the_selected_source_and_cleans_empty_stage() {
    let (_temp, parent) = fixture();
    let path = parent.join("source 文档");
    fs::write(&path, b"verified original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.preflight_finalization(&cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("not captured")
    };
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Removed {
            state: midnight_core::TransferSourceRemoval::Unlinked,
            durability_confirmed: true,
            ..
        }
    ));
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Removed { .. }
    ));
    let cleanup = capture.take_stage_cleanup().unwrap();
    drop(finalizer);
    drop(source);
    drop(capture);
    cleanup.cleanup().unwrap();
    assert!(!path.exists());
    assert!(!recovery.path.exists());
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
}

#[test]
fn existing_writer_refuses_finalization_and_original_is_restored() {
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"original").unwrap();
    let mut writer = fs::File::options().write(true).open(&path).unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.capture(&cancel).unwrap();
    assert!(capture.prepare_finalization(&cancel).is_err());
    writer.write_all(b"changed!").unwrap();
    capture.restore().unwrap();
    let cleanup = capture.take_stage_cleanup().unwrap();
    drop(source);
    drop(capture);
    cleanup.cleanup().unwrap();
    assert_eq!(fs::read(path).unwrap(), b"changed!");
}

#[test]
#[allow(unsafe_code)]
fn an_existing_writable_mapping_refuses_move_admission_after_writer_fd_closes() {
    use std::os::fd::AsRawFd;
    let (_temp, parent) = fixture();
    let path = parent.join("mapped-source");
    fs::write(&path, vec![42; 4096]).unwrap();
    let writer = fs::File::options()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    // SAFETY: maps exactly the owned test file; the mapping is released below.
    let mapping = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            writer.as_raw_fd(),
            0,
        )
    };
    assert_ne!(mapping, libc::MAP_FAILED);
    drop(writer);
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let refused = capture.preflight_finalization(&cancel).is_err();
    // SAFETY: unmaps the one successfully created fixture mapping.
    assert_eq!(unsafe { libc::munmap(mapping, 4096) }, 0);
    assert!(refused, "a writable mmap must count as an existing writer");
    capture.preflight_finalization(&cancel).unwrap();
    drop(source);
    drop(capture);
    assert_eq!(fs::read(path).unwrap(), vec![42; 4096]);
}

#[test]
fn unverified_hardlink_alias_refuses_move_admission() {
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"original").unwrap();
    fs::hard_link(&path, parent.join("alias")).unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    assert!(capture.preflight_finalization(&cancel).is_err());
    drop(source);
    drop(capture);
    assert_eq!(fs::read(path).unwrap(), b"original");
    assert_eq!(fs::read(parent.join("alias")).unwrap(), b"original");
}

#[test]
fn pending_old_name_writer_prevents_capture_and_resumes_only_after_source_is_retained() {
    use std::time::{Duration, Instant};
    let (_temp, parent) = fixture();
    let path = parent.join("old-source-name");
    fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    capture.preflight_finalization(&cancel).unwrap();
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        let mut writer = fs::File::options().write(true).open(writer_path).unwrap();
        writer.write_all(b"changed!").unwrap();
        writer.sync_all().unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let observed = loop {
        if capture.revalidate(&cancel).is_err() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::yield_now();
    };
    let refused = capture.capture(&cancel).is_err();
    let retained = path.exists();
    drop(source);
    drop(capture);
    writer.join().unwrap();
    assert!(observed, "pending writer must trigger a lease break");
    assert!(
        refused,
        "a detected old-name writer must prevent source staging"
    );
    assert!(retained);
    assert_eq!(fs::read(path).unwrap(), b"changed!");
    assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
}

#[test]
fn an_observed_lease_break_refuses_removal_even_if_writer_open_was_nonblocking() {
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("not captured")
    };
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    let writer = fs::File::options()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&recovery.path);
    assert_eq!(writer.unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(finalizer.revalidate(&cancel).is_err());
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Retained { .. }
    ));
    drop(finalizer);
    capture.restore().unwrap();
    let cleanup = capture.take_stage_cleanup().unwrap();
    drop(source);
    drop(capture);
    cleanup.cleanup().unwrap();
    assert_eq!(fs::read(path).unwrap(), b"original");
}

#[test]
fn cancelling_or_dropping_finalization_preserves_and_restores_original() {
    for cancelled in [false, true] {
        let (_temp, parent) = fixture();
        let path = parent.join("source");
        fs::write(&path, b"original").unwrap();
        let cancel = Cancellation::default();
        let (source, capture) = prepare_native(&path, &cancel).unwrap();
        capture.capture(&cancel).unwrap();
        let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
        if cancelled {
            cancel.cancel();
            assert!(matches!(
                finalizer.finalize(&cancel),
                midnight_core::TransferMoveFinalizeOutcome::Retained { .. }
            ));
        }
        drop(finalizer);
        capture.restore().unwrap();
        let cleanup = capture.take_stage_cleanup().unwrap();
        drop(source);
        drop(capture);
        cleanup.cleanup().unwrap();
        assert_eq!(fs::read(path).unwrap(), b"original");
    }
}

#[test]
fn a_replaced_stage_entry_is_never_unlinked_by_finalization() {
    let (_temp, parent) = fixture();
    let path = parent.join("source");
    fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    let (source, capture) = prepare_native(&path, &cancel).unwrap();
    let TransferMoveCaptureOutcome::Captured { recovery, .. } = capture.capture(&cancel).unwrap()
    else {
        panic!("not captured")
    };
    let mut finalizer = capture.prepare_finalization(&cancel).unwrap();
    let moved = parent.join("original-away");
    fs::rename(&recovery.path, &moved).unwrap();
    fs::write(&recovery.path, b"substitute").unwrap();
    assert!(matches!(
        finalizer.finalize(&cancel),
        midnight_core::TransferMoveFinalizeOutcome::Retained { .. }
    ));
    drop(finalizer);
    assert!(capture.restore().is_err());
    drop(source);
    drop(capture);
    assert_eq!(fs::read(moved).unwrap(), b"original");
    assert_eq!(fs::read(recovery.path).unwrap(), b"substitute");
}
