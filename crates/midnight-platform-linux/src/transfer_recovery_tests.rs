use super::*;
use std::{fs, os::unix::fs::symlink};

#[test]
fn preserving_output_retains_mode_and_extended_attributes_after_replacement() {
    use rustix::fs::{XattrFlags, fchmod, fgetxattr, fsetxattr};
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    let original = File::open(&target).unwrap();
    fchmod(&original, Mode::from_raw_mode(0o640)).unwrap();
    // Linux POSIX ACL format: owner rw, one named user r, owning group none,
    // mask r, others none. This must survive, including its named-user entry.
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, permission, id) in [
        (1u16, 6u16, u32::MAX),
        (2, 4, 65534),
        (4, 0, u32::MAX),
        (16, 4, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        acl.extend(tag.to_le_bytes());
        acl.extend(permission.to_le_bytes());
        acl.extend(id.to_le_bytes());
    }
    fsetxattr(
        &original,
        "system.posix_acl_access",
        &acl,
        XattrFlags::empty(),
    )
    .unwrap();
    fsetxattr(
        &original,
        "user.midnight-test",
        b"retained",
        XattrFlags::empty(),
    )
    .unwrap();
    let directory = Directory::from_file(
        File::from(openat(CWD, temp.path(), DIRECTORY, Mode::empty()).unwrap()),
        temp.path().to_owned(),
    )
    .unwrap();
    let cancel = Cancellation::default();
    let expected = prepare_target(&directory, OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = directory
        .stage_file_preserving_target(
            OsStr::new("target"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"replacement").unwrap();
    stage.sync(&cancel).unwrap();
    assert!(matches!(
        stage
            .publish_replace(OsStr::new("target"), &cancel)
            .unwrap(),
        TransferPublication::Replaced { .. }
    ));
    let published = File::open(&target).unwrap();
    assert_eq!(published.metadata().unwrap().mode() & 0o7777, 0o640);
    let mut value = [0u8; 32];
    let count = fgetxattr(&published, "user.midnight-test", value.as_mut_slice()).unwrap();
    assert_eq!(&value[..count], b"retained");
    let mut actual_acl = vec![0u8; acl.len()];
    let count = fgetxattr(
        &published,
        "system.posix_acl_access",
        actual_acl.as_mut_slice(),
    )
    .unwrap();
    assert_eq!(&actual_acl[..count], acl);
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
}

#[test]
fn preserving_output_refuses_privileged_mode_without_changing_original() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    rustix::fs::fchmod(File::open(&target).unwrap(), Mode::from_raw_mode(0o4600)).unwrap();
    let directory = Directory::from_file(
        File::from(openat(CWD, temp.path(), DIRECTORY, Mode::empty()).unwrap()),
        temp.path().to_owned(),
    )
    .unwrap();
    let cancel = Cancellation::default();
    let expected = prepare_target(&directory, OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    assert!(
        directory
            .stage_file_preserving_target(
                OsStr::new("target"),
                expected,
                TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel
            )
            .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o4600);
}

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
        preserved: None,
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
