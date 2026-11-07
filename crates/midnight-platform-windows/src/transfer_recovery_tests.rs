use super::*;
use std::fs;

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn stage(path: &Path) -> Stage {
    let cancel = Cancellation::default();
    let directory =
        Directory::from_file(native::open_parent(path).unwrap(), path.to_owned()).unwrap();
    let expected = prepare_target(&directory, OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let file = native::open_relative(
        &directory.file,
        OsStr::new("private-stage"),
        FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
        FILE_SHARE_READ,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE,
    )
    .unwrap();
    let mut stage = Stage {
        file,
        directory,
        published: false,
        published_name: None,
        cleaned: false,
        path: path.join("private-stage"),
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
fn a_new_entry_in_the_capture_gap_wins_and_both_prior_objects_are_retained() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"consented original").unwrap();
    let mut stage = stage(temp.path());
    let outcome = stage
        .replace(OsStr::new("target"), &Cancellation::default(), || {
            fs::write(&target, b"gap winner").unwrap();
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
    assert_eq!(fs::read(&target).unwrap(), b"gap winner");
    assert_eq!(recovery.len(), 2);
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"consented original");
    assert_eq!(fs::read(&recovery[1].path).unwrap(), b"verified bytes");
}

#[test]
fn cancellation_after_capture_retains_original_and_verified_replacement() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    let mut stage = stage(temp.path());
    let cancel = Cancellation::default();
    let outcome = stage
        .replace(OsStr::new("target"), &cancel, || cancel.cancel())
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
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"original");
    assert_eq!(fs::read(&recovery[1].path).unwrap(), b"verified bytes");
}

#[test]
fn recovery_directory_has_a_protected_single_user_dacl() {
    let temp = fixture();
    let directory = Directory::from_file(
        native::open_parent(temp.path()).unwrap(),
        temp.path().to_owned(),
    )
    .unwrap();
    let recovery = create_recovery(&directory, &Cancellation::default()).unwrap();
    let wide: Vec<_> = recovery
        .path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut needed = 0;
    // SAFETY: terminated owned path, zero-size probe and writable output length.
    let information = DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION;
    unsafe {
        GetFileSecurityW(wide.as_ptr(), information, ptr::null_mut(), 0, &mut needed);
    }
    assert!(needed > 0);
    let mut descriptor_storage = vec![0u64; (needed as usize).div_ceil(8)];
    let descriptor = descriptor_storage.as_mut_ptr().cast();
    // SAFETY: descriptor has the probed native length and remains live through the checks.
    assert_ne!(
        unsafe { GetFileSecurityW(wide.as_ptr(), information, descriptor, needed, &mut needed,) },
        0
    );
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: successful security read initialized the descriptor; both outputs are writable.
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0
    );
    assert_ne!(control & SE_DACL_PROTECTED, 0);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    // SAFETY: ACL output is borrowed from the live security descriptor.
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) },
        0
    );
    assert_ne!(present, 0);
    assert!(!acl.is_null());
    // SAFETY: the successful security descriptor query returned this valid ACL.
    assert_eq!(unsafe { (*acl).AceCount }, 1);
    let mut owner = ptr::null_mut();
    // SAFETY: owner is borrowed from the initialized live descriptor.
    assert_ne!(
        unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) },
        0
    );
    let mut ace = ptr::null_mut();
    // SAFETY: the valid ACL contains the one ACE asserted above.
    assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
    // SAFETY: AddAccessAllowedAce creates an ACCESS_ALLOWED_ACE; validate its header
    // before interpreting its SID field, all within the native descriptor's allocation.
    let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
    // ACCESS_ALLOWED_ACE_TYPE is the documented ACE header value zero.
    assert_eq!(ace.Header.AceType, 0);
    assert_eq!(u32::from(ace.Header.AceFlags) & INHERITED_ACE, 0);
    assert_eq!(ace.Mask, FILE_ALL_ACCESS);
    let sid = (&ace.SidStart as *const u32).cast_mut().cast();
    // SAFETY: both SID pointers refer into the valid native security descriptor.
    assert_ne!(unsafe { EqualSid(owner, sid) }, 0);
}

#[test]
fn displayed_target_metadata_and_revision_cannot_straddle_a_concurrent_edit() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"old").unwrap();
    let parent = native::open_parent(temp.path()).unwrap();
    let file = native::open_relative(
        &parent,
        OsStr::new("target"),
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
    )
    .unwrap();
    let result = target_snapshot(&file, &target, || {
        fs::write(&target, b"updated longer contents").unwrap()
    });
    assert!(
        result.is_err(),
        "conflict display and consent revision describe different versions"
    );
}

#[test]
fn native_change_time_rejects_same_length_edit_with_restored_last_write_time() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"old bytes").unwrap();
    let directory = Directory::from_file(
        native::open_parent(temp.path()).unwrap(),
        temp.path().to_owned(),
    )
    .unwrap();
    let cancel = Cancellation::default();
    let expected = prepare_target(&directory, OsStr::new("target"), &cancel)
        .unwrap()
        .unwrap();
    let file = native::open_relative(
        &directory.file,
        OsStr::new("target"),
        FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
    )
    .unwrap();
    let mut original = FILE_BASIC_INFO::default();
    // SAFETY: a live metadata handle and writable fixed-size native structure.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut original as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
    fs::write(&target, b"new bytes").unwrap();
    let changed = FILE_BASIC_INFO {
        LastWriteTime: original.LastWriteTime,
        // Ensure a deterministic observed change independently of timestamp quantum.
        ChangeTime: original.ChangeTime + 10_000_000,
        ..Default::default()
    };
    // SAFETY: the owned fixture metadata handle has FILE_WRITE_ATTRIBUTES and the
    // fixed-size input restores only LastWriteTime while retaining a distinct ChangeTime.
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileBasicInfo,
                (&changed as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
    assert_eq!(
        fs::metadata(&target).unwrap().len(),
        expected.entry().item.metadata.size.unwrap()
    );
    assert_eq!(
        fs::metadata(&target).unwrap().modified().unwrap(),
        expected.entry().item.metadata.modified.unwrap()
    );
    assert!(
        expected.revalidate(&cancel).is_err(),
        "native ChangeTime was omitted from the consent revision"
    );
    assert_eq!(fs::read(&target).unwrap(), b"new bytes");
}
