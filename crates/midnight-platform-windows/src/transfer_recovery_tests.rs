use super::*;
use std::{fs, mem::size_of_val};

#[test]
fn preserving_output_freezes_inherited_acl_without_adding_parent_grants() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    let directory = Directory::from_file(
        native::open_parent(temp.path()).unwrap(),
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
    let descriptor = security_descriptor(&target);
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: initialized native descriptor and writable scalar outputs remain live.
    assert_ne!(
        unsafe {
            GetSecurityDescriptorControl(
                descriptor.as_ptr().cast_mut().cast(),
                &mut control,
                &mut revision,
            )
        },
        0
    );
    assert_ne!(control & SE_DACL_PROTECTED, 0);
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
}

#[test]
fn preserving_output_keeps_private_acl_and_hidden_attribute_before_content() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"private original").unwrap();
    let directory = Directory::from_file(
        native::open_parent(temp.path()).unwrap(),
        temp.path().to_owned(),
    )
    .unwrap();
    let private = create_private_directory(&directory.file, OsStr::new("private"), 0).unwrap();
    let private_path = temp.path().join("private");
    let descriptor = security_descriptor(&private_path);
    let target_wide: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: an initialized descriptor and a terminated fixture path remain live.
    assert_ne!(
        unsafe {
            SetFileSecurityW(
                target_wide.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor.as_ptr().cast_mut().cast(),
            )
        },
        0
    );
    // SAFETY: the owned fixture is a normal file; only its display attribute is changed.
    assert_ne!(
        unsafe { SetFileAttributesW(target_wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) },
        0
    );
    let original_security = security_descriptor(&target);
    let expected = prepare_target(&directory, OsStr::new("target"), &Cancellation::default())
        .unwrap()
        .unwrap();
    let mut stage = directory
        .stage_file_preserving_target(
            OsStr::new("target"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &Cancellation::default(),
        )
        .unwrap();
    let stage_path = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".me-")
        })
        .expect("an exclusive native stage exists");
    assert_eq!(security_descriptor(&stage_path), original_security);
    assert_ne!(
        fs::metadata(&stage_path).unwrap().file_attributes() & FILE_ATTRIBUTE_HIDDEN,
        0
    );
    stage.write_all(b"private replacement").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"private original");
    stage.sync(&Cancellation::default()).unwrap();
    assert!(matches!(
        stage
            .publish_replace(OsStr::new("target"), &Cancellation::default())
            .unwrap(),
        TransferPublication::Replaced { .. }
    ));
    assert_eq!(security_descriptor(&target), original_security);
    assert_ne!(
        fs::metadata(&target).unwrap().file_attributes() & FILE_ATTRIBUTE_HIDDEN,
        0
    );
    assert_eq!(fs::read(&target).unwrap(), b"private replacement");
    drop(private);
}

#[test]
fn preserving_output_keeps_an_explicit_integrity_label() {
    let temp = fixture();
    let target = temp.path().join("target");
    let directory = Directory::from_file(
        native::open_parent(temp.path()).unwrap(),
        temp.path().to_owned(),
    )
    .unwrap();
    // The test token may inherit only Modify from the workspace. Create the fixture
    // with explicit WRITE_OWNER authority, which setting a mandatory label requires.
    let mut fixture_file = create_private_file(&directory.file, OsStr::new("target")).unwrap();
    fixture_file.write_all(b"original").unwrap();
    let mut sid = [0u64; 16];
    let mut length = size_of_val(&sid) as u32;
    // SAFETY: aligned, writable storage covers the bounded native SID output.
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinLowLabelSid,
                ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut length,
            )
        },
        0
    );
    let mut acl = [0u64; 32];
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: all structures are aligned and live; policy 1 means no-write-up.
    assert_ne!(
        unsafe {
            InitializeAcl(
                acl.as_mut_ptr().cast(),
                size_of_val(&acl) as u32,
                ACL_REVISION,
            )
        },
        0
    );
    assert_ne!(
        unsafe {
            AddMandatoryAce(
                acl.as_mut_ptr().cast(),
                ACL_REVISION,
                0,
                1,
                sid.as_mut_ptr().cast(),
            )
        },
        0
    );
    assert_ne!(
        unsafe { InitializeSecurityDescriptor(descriptor_ptr, 1) },
        0
    );
    assert_ne!(
        unsafe { SetSecurityDescriptorSacl(descriptor_ptr, 1, acl.as_mut_ptr().cast(), 0) },
        0
    );
    // SAFETY: only the owned fixture's mandatory label is modified.
    assert_ne!(
        unsafe {
            SetKernelObjectSecurity(
                fixture_file.as_raw_handle(),
                LABEL_SECURITY_INFORMATION,
                descriptor_ptr,
            )
        },
        0
    );
    drop(fixture_file);
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
    let stored = security_descriptor(&target);
    let mut present = 0;
    let mut defaulted = 0;
    let mut actual = ptr::null_mut();
    // SAFETY: initialized native descriptor owns the borrowed SACL.
    assert_ne!(
        unsafe {
            GetSecurityDescriptorSacl(
                stored.as_ptr().cast_mut().cast(),
                &mut present,
                &mut actual,
                &mut defaulted,
            )
        },
        0
    );
    assert_ne!(present, 0);
    assert!(!actual.is_null());
    let mut ace = ptr::null_mut();
    // SAFETY: successful SACL query returned a valid ACL and output is writable.
    assert_ne!(unsafe { GetAce(actual, 0, &mut ace) }, 0);
    // SAFETY: the label ACE supplied above is a SYSTEM_MANDATORY_LABEL_ACE.
    let label = unsafe { &*ace.cast::<SYSTEM_MANDATORY_LABEL_ACE>() };
    assert_eq!(label.Mask, 1);
    // SAFETY: both initialized SIDs remain owned by live allocations.
    assert_ne!(
        unsafe {
            EqualSid(
                ptr::addr_of!(label.SidStart).cast_mut().cast(),
                sid.as_mut_ptr().cast(),
            )
        },
        0
    );
}

#[test]
fn preserving_output_refuses_alternate_streams_instead_of_losing_them() {
    let temp = fixture();
    let target = temp.path().join("target");
    fs::write(&target, b"original").unwrap();
    fs::write(temp.path().join("target:private"), b"retained stream").unwrap();
    let directory = Directory::from_file(
        native::open_parent(temp.path()).unwrap(),
        temp.path().to_owned(),
    )
    .unwrap();
    let expected = prepare_target(&directory, OsStr::new("target"), &Cancellation::default())
        .unwrap()
        .unwrap();
    assert!(
        directory
            .stage_file_preserving_target(
                OsStr::new("target"),
                expected,
                TransferReplacementPolicy::RecoverableCurrentEntry,
                &Cancellation::default()
            )
            .is_err()
    );
    assert_eq!(
        fs::read(temp.path().join("target:private")).unwrap(),
        b"retained stream"
    );
    assert_eq!(fs::read(&target).unwrap(), b"original");
}

fn security_descriptor(path: &Path) -> Vec<u64> {
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let information = OWNER_SECURITY_INFORMATION
        | GROUP_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | LABEL_SECURITY_INFORMATION;
    let mut needed = 0;
    // SAFETY: terminated fixture path and writable size output; the probe writes no data.
    unsafe {
        GetFileSecurityW(wide.as_ptr(), information, ptr::null_mut(), 0, &mut needed);
    }
    assert!(needed > 0);
    let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: aligned allocation covers the native output size and stays live.
    assert_ne!(
        unsafe {
            GetFileSecurityW(
                wide.as_ptr(),
                information,
                storage.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        },
        0
    );
    storage
}

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
        metadata: false,
        preserved: None,
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
