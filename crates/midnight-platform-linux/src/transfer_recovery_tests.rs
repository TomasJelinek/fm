use super::*;
use std::{fs, os::unix::fs::symlink};

fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-transfer/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn stage(path: &Path) -> Stage {
    let cancel = Cancellation::default();
    let directory = Directory::from_file(
        File::from(openat(CWD, path, DIRECTORY, Mode::empty()).unwrap()),
        path.to_owned(),
    )
    .unwrap();
    let expected = prepare_target(&directory, OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let file = File::from(
        openat(
            &*directory.file,
            ".",
            OFlags::RDWR | OFlags::TMPFILE | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap(),
    );
    let mut stage = Stage {
        metadata: false,
        file: Some(file),
        directory,
        published: false,
        published_name: None,
        replacement: None,
    };
    stage.write_all(b"verified bytes").unwrap();
    stage.sync(&cancel).unwrap();
    stage
        .prepare_replacement(
            OsStr::new("target"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage
}

#[test]
fn replacement_at_the_last_revalidation_boundary_is_retained_after_exchange() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"consented original").unwrap();
    let mut stage = stage(temp.path());
    let outcome = stage
        .replace(OsStr::new("target"), &Cancellation::default(), || {
            fs::rename(&target, temp.path().join("original-away")).unwrap();
            fs::write(&target, b"late newcomer").unwrap();
        })
        .unwrap();
    let recovery = match outcome {
        TransferPublication::RecoveryRequired {
            published: Some(path),
            recovery,
            ..
        } => {
            assert_eq!(path, target);
            assert_eq!(recovery.len(), 1);
            recovery[0].clone()
        }
        other => panic!("unexpected recovery outcome: {other:?}"),
    };
    drop(stage);
    assert_eq!(fs::read(&target).unwrap(), b"verified bytes");
    assert_eq!(fs::read(recovery.path).unwrap(), b"late newcomer");
    assert_eq!(
        fs::read(temp.path().join("original-away")).unwrap(),
        b"consented original"
    );
}

#[test]
fn a_late_symlink_is_retained_as_a_link_and_its_target_is_never_opened() {
    let temp = fixture();
    let target = temp.path().join("target");
    let outside = temp.path().join("outside");
    fs::write(&target, b"original").unwrap();
    fs::write(&outside, b"keep outside").unwrap();
    let mut stage = stage(temp.path());
    let outcome = stage
        .replace(OsStr::new("target"), &Cancellation::default(), || {
            fs::rename(&target, temp.path().join("original-away")).unwrap();
            symlink(&outside, &target).unwrap();
        })
        .unwrap();
    let recovery = match outcome {
        TransferPublication::RecoveryRequired {
            published: Some(_),
            recovery,
            ..
        } => recovery,
        other => panic!("unexpected recovery outcome: {other:?}"),
    };
    drop(stage);
    assert!(
        fs::symlink_metadata(&recovery[0].path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&outside).unwrap(), b"keep outside");
}

#[test]
fn a_late_missing_target_retains_verified_bytes_without_deleting_the_original() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    let mut stage = stage(temp.path());
    let outcome = stage
        .replace(OsStr::new("target"), &Cancellation::default(), || {
            fs::rename(&target, temp.path().join("original-away")).unwrap();
        })
        .unwrap();
    let recovery = match outcome {
        TransferPublication::RecoveryRequired {
            published: None,
            recovery,
            ..
        } => recovery,
        other => panic!("unexpected recovery outcome: {other:?}"),
    };
    drop(stage);
    assert!(!target.exists());
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"verified bytes");
    assert_eq!(
        fs::read(temp.path().join("original-away")).unwrap(),
        b"original"
    );
}

#[test]
fn substituted_recovery_stage_is_detected_without_touching_final_destination() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    let mut stage = stage(temp.path());
    let outcome = stage
        .replace(OsStr::new("target"), &Cancellation::default(), || {
            let recovery = fs::read_dir(temp.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .as_encoded_bytes()
                        .starts_with(b".midnight-recovery-")
                })
                .unwrap();
            fs::remove_file(recovery.join("item")).unwrap();
            fs::write(recovery.join("item"), b"unexpected stage").unwrap();
        })
        .unwrap();
    let recovery = match outcome {
        TransferPublication::RecoveryRequired {
            published: None,
            recovery,
            ..
        } => recovery,
        other => panic!("substituted stage was published: {other:?}"),
    };
    drop(stage);
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"unexpected stage");
}

#[test]
fn displayed_target_metadata_and_revision_cannot_straddle_a_concurrent_edit() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"old").unwrap();
    let file =
        File::from(openat(CWD, &target, OFlags::PATH | OFlags::NOFOLLOW, Mode::empty()).unwrap());
    let result = target_snapshot(&file, &target, || {
        fs::write(&target, b"updated longer contents").unwrap()
    });
    assert!(
        result.is_err(),
        "conflict display and consent revision describe different versions"
    );
}
