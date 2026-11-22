use super::*;

/// Every default run covers both branches of the namespace tests: in-process in the
/// build volume's native lookup mode, and in a child process forced into the other
/// mode. The forced mode is process-wide because concurrently running tests share
/// pooled parent bindings, whose lookup mode is part of their validation; a per-thread
/// seam would make those tests disagree about one binding.
#[test]
fn namespace_tests_pass_in_the_other_lookup_mode_too() {
    if std::env::var_os(TEST_LOOKUP_ENV).is_some() {
        // Already a forced run: an explicit second pass or the child below.
        return;
    }
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    std::fs::create_dir_all(&base).unwrap();
    let temp = tempfile::tempdir_in(base).unwrap();
    let directory = metadata_directory(temp.path()).unwrap();
    let native = capture(&directory, temp.path(), &Cancellation::default())
        .unwrap()
        .mode;
    let other = match native {
        LookupMode::ShortNamesDisabled { .. } => "unqualified",
        LookupMode::Unqualified => "short-names-disabled",
    };
    let stdout = crate::test_child::run_test_child(
        &["transfer::namespace::tests::", "--test-threads", "4"],
        (TEST_LOOKUP_ENV, other),
    );
    assert!(
        stdout.contains("test result: ok") && !stdout.contains("ok. 0 passed"),
        "namespace tests did not run in the {other} lookup mode:\n{stdout}"
    );
}

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
    let query = match open_volume(&root) {
        Ok(query) => query,
        // A share or other volume without a GUID path has no volume rule query.
        Err(error) if error.kind == io::ErrorKind::Unsupported => return,
        Err(error) => panic!("{error:?}"),
    };
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
fn a_renamed_directory_is_never_bound_to_its_pooled_old_path() {
    let temp = crate::test_fixtures::build_temp("namespace-pool-");
    let cancel = Cancellation::default();
    let old = temp.path().join("old 文档");
    let new = temp.path().join("new 文档");
    std::fs::create_dir(&old).unwrap();
    // Another job (or a paused one awaiting a decision) still holds the pooled Parent.
    let held = capture(&native::open_parent(&old).unwrap(), &old, &cancel).unwrap();
    std::fs::rename(&old, &new).unwrap();
    let fresh = capture(&native::open_parent(&new).unwrap(), &new, &cancel).unwrap();
    assert!(
        !Arc::ptr_eq(&held, &fresh),
        "a stale pooled path is not reused"
    );
    assert_eq!(fresh.physical().file_name(), new.file_name());
    fresh.validate(&cancel).unwrap();
    assert!(
        held.validate(&cancel).is_err(),
        "the old holder sees its own change"
    );
    // The fresh entry now owns the pool slot.
    let again = capture(&native::open_parent(&new).unwrap(), &new, &cancel).unwrap();
    assert!(Arc::ptr_eq(&fresh, &again));
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

/// Manual measurement for the WIN-8 namespace design note: one check of a pinned folder's
/// ancestry by opening each level, against the handle proof of a qualified folder and its
/// parts, each outside a refresh batch.
#[test]
#[ignore = "manual measurement"]
fn measure_final_path_against_ancestry_opens() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_NAME_OPENED, VOLUME_NAME_NT};
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    std::fs::create_dir_all(&base).unwrap();
    let temp = tempfile::tempdir_in(base).unwrap();
    let path = temp.path().join("level0").join("level1").join("level2");
    std::fs::create_dir_all(&path).unwrap();
    let directory = metadata_directory(&path).unwrap();
    let cancel = Cancellation::default();
    let parent = capture(&directory, &path, &cancel).unwrap();
    let levels = parent.chain.len();
    println!(
        "PERF lookup rules {:?}, handle proof {}",
        parent.mode,
        if parent.pinned.is_some() { "on" } else { "off" }
    );
    const N: u32 = 4000;
    let time = |name: &str, check: &dyn Fn()| {
        for _ in 0..100 {
            check();
        }
        let started = std::time::Instant::now();
        for _ in 0..N {
            check();
        }
        let micros = started.elapsed().as_secs_f64() * 1e6 / f64::from(N);
        println!("PERF {name}: {micros:.1} us per call, {levels} levels ({path:?})");
    };
    time(
        "validate (the handle proof when on, and the lookup rules)",
        &|| parent.validate(&cancel).unwrap(),
    );
    time(
        "per-level check, each level opened (validate_levels)",
        &|| parent.validate_levels(&cancel).unwrap(),
    );
    if parent.pinned.is_some() {
        time("handle proof alone (proven)", &|| {
            assert!(parent.proven(&cancel).unwrap())
        });
    }
    time("lookup rules alone (check_rules)", &|| {
        parent.check_rules().unwrap()
    });
    time("link of one level (FileHardLinkInformation)", &|| {
        assert!(linked(&parent.chain[0], &parent.chain[1]))
    });
    time(
        "delete state of the held folder (FileStandardInfo)",
        &|| assert_eq!(delete_pending(&directory), Some(false)),
    );
    time(
        "capture of a pooled parent (identity + final path, no check)",
        &|| {
            capture(&directory, &path, &cancel).unwrap();
        },
    );
    time("GetFinalPathNameByHandleW normalized, volume GUID", &|| {
        physical_path(&directory).unwrap();
    });
    time("GetFinalPathNameByHandleW normalized, volume NT", &|| {
        crate::transfer::destination::final_path(&directory, VOLUME_NAME_NT).unwrap();
    });
    time("GetFinalPathNameByHandleW opened name, volume NT", &|| {
        crate::transfer::destination::final_path(&directory, VOLUME_NAME_NT | FILE_NAME_OPENED)
            .unwrap();
    });
    time("identity of the held folder (one handle query)", &|| {
        identity(&directory, &path).unwrap();
    });
}
