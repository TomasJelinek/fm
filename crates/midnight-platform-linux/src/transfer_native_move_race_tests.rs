use super::*;
use std::fs;

fn fixture() -> (tempfile::TempDir, Arc<Capture>, Arc<dyn TransferDirectory>) {
    let temp = super::super::native_tests::fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"selected source").unwrap();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let (_, capture) = prepare_native(&source, &Cancellation::default()).unwrap();
    let target = crate::LinuxFileSystem::new(None)
        .prepare_move_destination(&destination, &Cancellation::default())
        .unwrap();
    (temp, capture, target)
}

#[test]
fn substituted_source_is_restored_without_publishing_or_deleting_it() {
    let (temp, capture, target) = fixture();
    let source = temp.path().join("source");
    let outcome = attempt_with(
        &capture,
        &*target,
        OsStr::new("item"),
        None,
        &Cancellation::default(),
        || {
            fs::rename(&source, temp.path().join("original-away")).unwrap();
            fs::write(&source, b"unexpected replacement").unwrap();
        },
        || {},
    )
    .unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::RecoveryRequired {
        destination: None, ref recovery, ..
    } if recovery.is_empty())
    );
    assert_eq!(fs::read(source).unwrap(), b"unexpected replacement");
    assert_eq!(
        fs::read(temp.path().join("original-away")).unwrap(),
        b"selected source"
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
}

#[test]
fn late_target_collision_restores_source_and_repeated_attempt_is_idempotent() {
    let (temp, capture, target) = fixture();
    let outcome = attempt_with(
        &capture,
        &*target,
        OsStr::new("item"),
        None,
        &Cancellation::default(),
        || {},
        || {
            fs::write(target.path().join("item"), b"target winner").unwrap();
        },
    )
    .unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::RecoveryRequired {
        destination: None, ref recovery, ..
    } if recovery.is_empty())
    );
    assert_eq!(
        fs::read(temp.path().join("source")).unwrap(),
        b"selected source"
    );
    assert_eq!(
        fs::read(target.path().join("item")).unwrap(),
        b"target winner"
    );
    let repeated = attempt(
        &capture,
        &*target,
        OsStr::new("different"),
        None,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(matches!(
        repeated,
        TransferNativeMoveOutcome::RecoveryRequired { .. }
    ));
    assert!(!target.path().join("different").exists());
}

#[test]
fn restoration_collision_retains_exact_original_and_both_new_winners() {
    let (temp, capture, target) = fixture();
    let outcome = attempt_with(
        &capture,
        &*target,
        OsStr::new("item"),
        None,
        &Cancellation::default(),
        || {},
        || {
            fs::write(target.path().join("item"), b"target winner").unwrap();
            fs::write(temp.path().join("source"), b"source winner").unwrap();
        },
    )
    .unwrap();
    let TransferNativeMoveOutcome::RecoveryRequired {
        destination: None,
        recovery,
        ..
    } = outcome
    else {
        panic!("unexpected outcome: {outcome:?}")
    };
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].identity, Some(capture.entry.identity));
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"selected source");
    assert_eq!(
        fs::read(temp.path().join("source")).unwrap(),
        b"source winner"
    );
    assert_eq!(
        fs::read(target.path().join("item")).unwrap(),
        b"target winner"
    );
}

#[test]
fn replacement_race_retains_unexpected_target_and_reports_committed_source() {
    let (temp, capture, target) = fixture();
    fs::write(target.path().join("item"), b"consented target").unwrap();
    let cancel = Cancellation::default();
    let expected = target
        .prepare_target(OsStr::new("item"), &cancel)
        .unwrap()
        .unwrap();
    let outcome = attempt_with(
        &capture,
        &*target,
        OsStr::new("item"),
        Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry)),
        &cancel,
        || {},
        || {
            fs::rename(
                target.path().join("item"),
                target.path().join("target-away"),
            )
            .unwrap();
            fs::write(target.path().join("item"), b"unexpected target").unwrap();
        },
    )
    .unwrap();
    let TransferNativeMoveOutcome::RecoveryRequired {
        destination: Some(path),
        recovery,
        ..
    } = outcome
    else {
        panic!("unexpected outcome: {outcome:?}")
    };
    assert_eq!(fs::read(path).unwrap(), b"selected source");
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"unexpected target");
    assert_eq!(
        fs::read(target.path().join("target-away")).unwrap(),
        b"consented target"
    );
    assert!(!temp.path().join("source").exists());
}

#[test]
fn cancellation_after_capture_restores_without_copy_or_publication() {
    let (temp, capture, target) = fixture();
    let cancel = Cancellation::default();
    let outcome = attempt_with(
        &capture,
        &*target,
        OsStr::new("item"),
        None,
        &cancel,
        || {},
        || cancel.cancel(),
    )
    .unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::RecoveryRequired {
        destination: None, ref recovery, ref error,
    } if recovery.is_empty() && error.kind == io::ErrorKind::Interrupted)
    );
    assert_eq!(
        fs::read(temp.path().join("source")).unwrap(),
        b"selected source"
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
}

#[test]
fn native_sync_error_preserves_committed_move_and_displaced_bytes_without_replay() {
    let (temp, capture, target) = fixture();
    let cancel = Cancellation::default();
    fs::write(target.path().join("item"), b"old destination").unwrap();
    let expected = target
        .prepare_target(OsStr::new("item"), &cancel)
        .unwrap()
        .unwrap();
    let outcome = execute(
        &capture,
        &*target,
        OsStr::new("item"),
        Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry)),
        &cancel,
        (|| {}, || {}),
        |directories: &[&destination::Directory]| {
            Err(error(
                directories[directories.len() - 1].path(),
                io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected directory sync failure",
                ),
            ))
        },
    )
    .unwrap();
    let TransferNativeMoveOutcome::RecoveryRequired {
        destination: Some(path),
        recovery,
        error,
    } = outcome
    else {
        panic!("sync failure lost its committed error: {outcome:?}")
    };
    assert_eq!(error.kind, io::ErrorKind::StorageFull);
    assert_eq!(error.message, "injected directory sync failure");
    assert_eq!(fs::read(path).unwrap(), b"selected source");
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old destination");
    assert!(!temp.path().join("source").exists());
    assert!(matches!(
        attempt(&capture, &*target, OsStr::new("different"), None, &cancel).unwrap(),
        TransferNativeMoveOutcome::RecoveryRequired {
            destination: Some(_),
            ..
        }
    ));
    assert!(!target.path().join("different").exists());
}

#[test]
fn native_sync_failure_still_attempts_all_directory_syncs_and_retains_first_error() {
    let (_temp, capture, _) = fixture();
    let mut attempts = 0;
    let error = sync_each(
        &[&capture.parent, &capture.parent, &capture.parent],
        |directory| {
            attempts += 1;
            Err(error(
                directory.path(),
                io::Error::other(format!("failure {attempts}")),
            ))
        },
    )
    .unwrap_err();
    assert_eq!(attempts, 3);
    assert_eq!(error.message, "failure 1");
}

/// The private `.midnight-move-*` stages in `folder`.
fn stages(folder: &Path) -> Vec<PathBuf> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".midnight-move-"))
        })
        .collect()
}

#[test]
fn published_move_removes_its_empty_stage_before_syncing_each_folder_once() {
    let (temp, capture, target) = fixture();
    let mut synced = Vec::new();
    let outcome = execute(
        &capture,
        &*target,
        OsStr::new("item"),
        None,
        &Cancellation::default(),
        (|| {}, || {}),
        |directories: &[&destination::Directory]| {
            let identities: Vec<_> = directories.iter().map(|folder| folder.identity()).collect();
            synced.push((identities, stages(temp.path()).len()));
            sync_directories(directories)
        },
    )
    .unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::Moved {
            durability_confirmed: true, ref recovery, ..
        } if recovery.is_empty()),
        "{outcome:?}"
    );
    // One sync of the source and target folders, after the stage was removed.
    assert_eq!(
        synced,
        [(vec![capture.parent.identity(), target.identity()], 0)]
    );
    assert_eq!(
        fs::read(target.path().join("item")).unwrap(),
        b"selected source"
    );
    assert!(!temp.path().join("source").exists());
}

#[test]
fn failed_folder_sync_after_stage_removal_reports_the_published_move_for_recovery() {
    let (temp, capture, target) = fixture();
    let outcome = execute(
        &capture,
        &*target,
        OsStr::new("item"),
        None,
        &Cancellation::default(),
        (|| {}, || {}),
        |directories: &[&destination::Directory]| {
            Err(error(
                directories[0].path(),
                io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected directory sync failure",
                ),
            ))
        },
    )
    .unwrap();
    let TransferNativeMoveOutcome::RecoveryRequired {
        destination: Some(path),
        recovery,
        error,
    } = outcome
    else {
        panic!("the published Move lost its durability failure: {outcome:?}")
    };
    assert!(recovery.is_empty());
    assert_eq!(error.kind, io::ErrorKind::StorageFull);
    assert_eq!(fs::read(path).unwrap(), b"selected source");
    assert!(stages(temp.path()).is_empty());
    assert!(!temp.path().join("source").exists());
}

#[test]
fn a_stage_that_is_no_longer_empty_is_synced_with_both_folders_and_reported() {
    let (temp, capture, target) = fixture();
    let mut synced = Vec::new();
    let outcome = execute(
        &capture,
        &*target,
        OsStr::new("item"),
        None,
        &Cancellation::default(),
        (
            || {},
            || fs::write(stages(temp.path())[0].join("intruder"), b"another program").unwrap(),
        ),
        |directories: &[&destination::Directory]| {
            synced.push(
                directories
                    .iter()
                    .map(|folder| folder.identity())
                    .collect::<Vec<_>>(),
            );
            sync_directories(directories)
        },
    )
    .unwrap();
    let TransferNativeMoveOutcome::RecoveryRequired {
        destination: Some(path),
        recovery,
        error,
    } = outcome
    else {
        panic!("a retained stage must be reported: {outcome:?}")
    };
    assert!(
        recovery.is_empty(),
        "an ordinary Move reports its stage in the error"
    );
    assert!(!error.message.is_empty());
    assert_eq!(fs::read(path).unwrap(), b"selected source");
    let stage = stages(temp.path()).remove(0);
    assert_eq!(
        fs::read(stage.join("intruder")).unwrap(),
        b"another program"
    );
    let stage_identity = identity(&File::open(&stage).unwrap(), &stage).unwrap();
    assert_eq!(
        synced,
        [vec![
            stage_identity,
            capture.parent.identity(),
            target.identity()
        ]]
    );
}

#[test]
fn replacing_move_syncs_the_stage_holding_the_displaced_entry_with_both_folders() {
    let (temp, capture, target) = fixture();
    let cancel = Cancellation::default();
    fs::write(target.path().join("item"), b"old destination").unwrap();
    let expected = target
        .prepare_target(OsStr::new("item"), &cancel)
        .unwrap()
        .unwrap();
    let mut synced = Vec::new();
    let outcome = execute(
        &capture,
        &*target,
        OsStr::new("item"),
        Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry)),
        &cancel,
        (|| {}, || {}),
        |directories: &[&destination::Directory]| {
            synced.push(
                directories
                    .iter()
                    .map(|folder| folder.identity())
                    .collect::<Vec<_>>(),
            );
            sync_directories(directories)
        },
    )
    .unwrap();
    let TransferNativeMoveOutcome::Moved {
        recovery,
        durability_confirmed: true,
        ..
    } = outcome
    else {
        panic!("unexpected outcome: {outcome:?}")
    };
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old destination");
    let stage = recovery[0].path.parent().unwrap();
    let stage_identity = identity(&File::open(stage).unwrap(), stage).unwrap();
    assert_eq!(
        synced,
        [vec![
            stage_identity,
            capture.parent.identity(),
            target.identity()
        ]]
    );
    assert_eq!(stages(temp.path()), [stage.to_owned()]);
}
