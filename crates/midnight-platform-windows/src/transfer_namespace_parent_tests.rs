use super::*;

#[test]
fn volume_query_handle_retains_pinned_root_directory_identity() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    std::fs::create_dir_all(&base).unwrap();
    let temp = tempfile::tempdir_in(base).unwrap();
    let directory = metadata_directory(temp.path()).unwrap();
    let physical = physical_path(&directory).unwrap();
    let root_path = physical
        .ancestors()
        .filter(|path| path.has_root())
        .last()
        .unwrap();
    let root = anchor(root_path).unwrap();
    let query = open_volume(&root).unwrap();
    assert!(Arc::ptr_eq(&query.root, &root));
    assert_eq!(
        identity(&query.file, &root.physical).unwrap(),
        root.identity
    );
    drop(root);
    assert_eq!(
        identity(&query.file, &query.root.physical).unwrap(),
        query.root.identity
    );
}

#[test]
fn successful_complete_query_accepts_flags_without_echoed_request_fields() {
    // The normal-user root-directory probe succeeded with this exact output:
    // sixteen returned bytes, disabled flag set, no Version or FlagMask echo.
    let response = FILE_FS_PERSISTENT_VOLUME_INFORMATION {
        VolumeFlags: PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
        ..Default::default()
    };
    assert!(persistent_short_names_disabled(1, 16, &response));
}

#[test]
fn failed_query_never_accepts_preinitialized_disabled_flags() {
    let response = FILE_FS_PERSISTENT_VOLUME_INFORMATION {
        VolumeFlags: PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
        FlagMask: PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
        Version: 1,
        ..Default::default()
    };
    for returned in [0, 16] {
        assert!(!persistent_short_names_disabled(0, returned, &response));
    }
}

#[test]
fn short_query_reply_never_qualifies_even_when_disabled_flag_is_present() {
    let response = FILE_FS_PERSISTENT_VOLUME_INFORMATION {
        VolumeFlags: PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
        ..Default::default()
    };
    for returned in [0, 1, 4, 8, 12, 15] {
        assert!(!persistent_short_names_disabled(1, returned, &response));
    }
}

#[test]
fn successful_query_without_disabled_flag_remains_unqualified() {
    for flags in [0, 2, 0xffff_fffe] {
        let response = FILE_FS_PERSISTENT_VOLUME_INFORMATION {
            VolumeFlags: flags,
            FlagMask: PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
            Version: 1,
            ..Default::default()
        };
        assert!(!persistent_short_names_disabled(1, 16, &response));
    }
}
