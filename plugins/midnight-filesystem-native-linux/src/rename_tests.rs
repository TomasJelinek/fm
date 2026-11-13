//! These tests call the exported C ABI; only race timing and one fsync failure
//! are controlled. Namespace operations and independent (dev, ino) checks are real.
#![allow(unsafe_code)]
use crate::{
    rename_test_hooks::{self as hooks, Boundary},
    rename_test_support::{Client, Temp},
};
use midnight_filesystem_plugin_api::{abi::*, *};
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn identity(path: &Path) -> (u64, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}
fn raw(bytes: &[u8]) -> NativeText {
    NativeText {
        encoding: NameEncoding::UnixBytes,
        bytes: bytes.to_vec(),
    }
}
fn prepare(client: &Client, path: &Path) -> PreparedRename {
    let (mount, node) = client.resolve(path);
    match client.call(Request::PrepareRename {
        mount,
        node: node.id,
    }) {
        Response::RenamePrepared(value) => value,
        response => panic!("real ABI Prepare failed: {response:?}"),
    }
}
fn attempt(client: &Client, prepared: u64, number: u64, policy: RenamePolicy) -> RenameOutcome {
    match client.call(Request::Rename {
        prepared,
        attempt: number,
        name: raw(b"target"),
        policy,
    }) {
        Response::RenameState(state) => state.outcome,
        response => panic!("real ABI Rename failed: {response:?}"),
    }
}
fn replacement(client: &Client, prepared: &PreparedRename) -> RenamePolicy {
    assert!(
        prepared.recoverable_replace,
        "requires owned ext/XFS/Btrfs/tmpfs fixtures"
    );
    let RenameOutcome::Conflict(conflict) =
        attempt(client, prepared.handle, 1, RenamePolicy::NoReplace)
    else {
        panic!("expected pinned collision");
    };
    assert!(conflict.can_replace);
    RenamePolicy::RecoverableCurrentEntry {
        conflict: conflict.token,
    }
}
fn locator(receipt: &RenameRecovery) -> PathBuf {
    assert_eq!(receipt.address.encoding, AddressEncoding::UnixBytes);
    PathBuf::from(OsString::from_vec(receipt.address.bytes.clone()))
}
fn residual(values: &[RenameRecovery], role: RenameRecoveryRole) -> &RenameRecovery {
    values.iter().find(|value| value.role == role).unwrap()
}
fn setup() -> (Temp, Client, PreparedRename, (u64, u64)) {
    let temp = Temp::new();
    fs::write(temp.path.join("source"), b"original source").unwrap();
    let original = identity(&temp.path.join("source"));
    let client = Client::new();
    let prepared = prepare(&client, &temp.path.join("source"));
    (temp, client, prepared, original)
}

#[test]
fn source_substitution_in_capture_gap_is_restored_without_adopting_it() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let parent = temp.path.clone();
    let raced = Arc::new(std::sync::Mutex::new(None));
    let seen = raced.clone();
    hooks::at(Boundary::BeforeCapture, move |_| {
        fs::rename(parent.join("source"), parent.join("original-away")).unwrap();
        fs::write(parent.join("source"), b"raced source").unwrap();
        *seen.lock().unwrap() = Some(identity(&parent.join("source")));
    });
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    assert!(
        matches!(
            outcome,
            RenameOutcome::Unchanged(Failure {
                code: ErrorCode::SourceChanged,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert_eq!(identity(&temp.path.join("original-away")), original);
    assert_eq!(
        identity(&temp.path.join("source")),
        raced.lock().unwrap().unwrap()
    );
    assert_eq!(fs::read(temp.path.join("source")).unwrap(), b"raced source");
    assert!(!temp.path.join("target").exists());
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
}

#[test]
fn cancellation_after_capture_restores_independently_of_cancelled_token() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    hooks::at(Boundary::AfterCapture, move |_| {
        flag.store(true, Ordering::Release)
    });
    let outcome = controlled_call(&client, prepared.handle, &cancelled);
    assert!(
        matches!(
            outcome,
            RenameOutcome::Unchanged(Failure {
                code: ErrorCode::Cancelled,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert_eq!(identity(&temp.path.join("source")), original);
    assert!(!temp.path.join("target").exists());
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
}

#[test]
fn restoration_never_replaces_a_new_winner_and_reports_exact_raw_source() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let parent = temp
        .path
        .join(OsString::from_vec(b"raw-parent-\xff".to_vec()));
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("source"), b"original source").unwrap();
    let original = identity(&parent.join("source"));
    let client = Client::new();
    let prepared = prepare(&client, &parent.join("source"));
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let captured_parent = parent.clone();
    hooks::at(Boundary::AfterCapture, move |_| {
        fs::write(captured_parent.join("source"), b"new winner").unwrap();
        flag.store(true, Ordering::Release);
    });
    let outcome = controlled_call(&client, prepared.handle, &cancelled);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = outcome
    else {
        panic!("restoration collision must retain source: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Recovery);
    assert_eq!(destination_name, None);
    assert_eq!(fs::read(parent.join("source")).unwrap(), b"new winner");
    assert_ne!(identity(&parent.join("source")), original);
    let receipt = residual(&recovery, RenameRecoveryRole::Source);
    let retained = locator(receipt);
    let mut prefix = parent.as_os_str().as_bytes().to_vec();
    prefix.push(b'/');
    assert!(receipt.address.bytes.starts_with(&prefix));
    assert!(std::str::from_utf8(&receipt.address.bytes).is_err());
    assert_eq!(retained.file_name().unwrap(), "item");
    assert_eq!(identity(&retained), original);
    assert_eq!(receipt.identity, prepared.source.identity);
    assert_eq!(fs::read(&retained).unwrap(), b"original source");
    assert_eq!(
        client.call(Request::CloseRename {
            prepared: prepared.handle
        }),
        Response::Unit
    );
    assert_eq!(identity(&retained), original);
}

#[test]
fn raced_capture_substitute_and_new_restore_winner_both_survive() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let parent = temp.path.clone();
    let raced = Arc::new(std::sync::Mutex::new(None));
    let seen = raced.clone();
    hooks::at(Boundary::BeforeCapture, move |_| {
        fs::rename(parent.join("source"), parent.join("original-away")).unwrap();
        fs::write(parent.join("source"), b"captured substitute").unwrap();
        *seen.lock().unwrap() = Some(identity(&parent.join("source")));
    });
    let parent = temp.path.clone();
    hooks::at(Boundary::AfterCapture, move |_| {
        fs::write(parent.join("source"), b"restore winner").unwrap();
    });
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = outcome
    else {
        panic!("raced substitute must be retained: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Unknown);
    assert_eq!(identity(&temp.path.join("original-away")), original);
    assert_eq!(
        fs::read(temp.path.join("source")).unwrap(),
        b"restore winner"
    );
    let receipt = residual(&recovery, RenameRecoveryRole::Source);
    assert_eq!(identity(&locator(receipt)), raced.lock().unwrap().unwrap());
    assert_ne!(receipt.identity, prepared.source.identity);
    assert_eq!(fs::read(locator(receipt)).unwrap(), b"captured substitute");
}

#[test]
fn replacement_capture_gap_keeps_unconsented_displaced_inode() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    fs::write(temp.path.join("target"), b"consented target").unwrap();
    let consented = identity(&temp.path.join("target"));
    let policy = replacement(&client, &prepared);
    let parent = temp.path.clone();
    let raced = Arc::new(std::sync::Mutex::new(None));
    let seen = raced.clone();
    hooks::at(Boundary::BeforePublish, move |_| {
        fs::rename(parent.join("target"), parent.join("consented-away")).unwrap();
        fs::write(parent.join("target"), b"unconsented target").unwrap();
        *seen.lock().unwrap() = Some(identity(&parent.join("target")));
    });
    let outcome = attempt(&client, prepared.handle, 2, policy);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = outcome
    else {
        panic!("post-exchange mismatch must be honest: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Destination);
    assert_eq!(destination_name, Some(raw(b"target")));
    assert_eq!(identity(&temp.path.join("target")), original);
    assert_eq!(identity(&temp.path.join("consented-away")), consented);
    let receipt = residual(&recovery, RenameRecoveryRole::DisplacedTarget);
    assert_eq!(identity(&locator(receipt)), raced.lock().unwrap().unwrap());
    assert_eq!(fs::read(locator(receipt)).unwrap(), b"unconsented target");
}

#[test]
fn late_cancel_after_successful_native_commit_keeps_actual_result_and_replay() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    hooks::at(Boundary::AfterCommit, move |_| {
        flag.store(true, Ordering::Release)
    });
    let outcome = controlled_call(&client, prepared.handle, &cancelled);
    assert!(
        matches!(
            outcome,
            RenameOutcome::Renamed {
                durability_confirmed: true,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(identity(&temp.path.join("target")), original);
    assert!(!temp.path.join("source").exists());
    let Response::RenameState(query) = client.cancelled(
        Request::GetRenameState {
            prepared: prepared.handle,
        },
        1,
    ) else {
        panic!("cancel-independent query failed");
    };
    assert_eq!(query.outcome, outcome);
    assert_eq!(
        attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace),
        outcome
    );
    assert_eq!(identity(&temp.path.join("target")), original);
}

#[test]
fn sync_failure_reports_committed_source_and_exact_empty_stage_identity() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    hooks::fail_next_sync();
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = outcome
    else {
        panic!("directory sync failure must not claim success: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Destination);
    assert_eq!(destination_name, Some(raw(b"target")));
    assert_eq!(identity(&temp.path.join("target")), original);
    let receipt = residual(&recovery, RenameRecoveryRole::EmptyStage);
    let stage = locator(receipt);
    assert_eq!(stage.parent(), Some(temp.path.as_path()));
    assert!(fs::symlink_metadata(&stage).unwrap().is_dir());
    let observed = identity(&stage);
    assert_eq!(
        u64::from_le_bytes(receipt.identity[0..8].try_into().unwrap()),
        observed.0
    );
    assert_eq!(
        u64::from_le_bytes(receipt.identity[8..16].try_into().unwrap()),
        observed.1
    );
    assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
    client.call(Request::CloseRename {
        prepared: prepared.handle,
    });
    assert_eq!(identity(&stage), observed);
}

#[test]
fn unexpected_stage_child_is_never_recursively_removed() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let child_id = Arc::new(std::sync::Mutex::new(None));
    let seen = child_id.clone();
    hooks::at(Boundary::BeforeCleanup, move |stage| {
        fs::write(stage.join("unexpected"), b"retain unexpected").unwrap();
        *seen.lock().unwrap() = Some(identity(&stage.join("unexpected")));
    });
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = outcome
    else {
        panic!("nonempty stage must remain explicit: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Destination);
    assert_eq!(identity(&temp.path.join("target")), original);
    let stage = locator(residual(&recovery, RenameRecoveryRole::EmptyStage));
    assert_eq!(
        identity(&stage.join("unexpected")),
        child_id.lock().unwrap().unwrap()
    );
    assert_eq!(
        fs::read(stage.join("unexpected")).unwrap(),
        b"retain unexpected"
    );
}

#[test]
fn replacement_sync_failure_retains_exact_displaced_inode_after_close() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    fs::write(temp.path.join("target"), b"retained target").unwrap();
    let displaced = identity(&temp.path.join("target"));
    let policy = replacement(&client, &prepared);
    hooks::fail_next_sync();
    let outcome = attempt(&client, prepared.handle, 2, policy);
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = outcome
    else {
        panic!("replacement sync failure must retain target: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Destination);
    assert_eq!(identity(&temp.path.join("target")), original);
    let path = locator(residual(&recovery, RenameRecoveryRole::DisplacedTarget));
    assert_eq!(identity(&path), displaced);
    assert_eq!(fs::read(&path).unwrap(), b"retained target");
    client.call(Request::CloseRename {
        prepared: prepared.handle,
    });
    assert_eq!(identity(&path), displaced);
}

fn controlled_call(client: &Client, prepared: u64, cancelled: &AtomicBool) -> RenameOutcome {
    unsafe extern "C" fn callback(value: u64) -> u32 {
        // SAFETY: controlled_call keeps the AtomicBool borrowed through this synchronous ABI call.
        u32::from(unsafe { &*(value as *const AtomicBool) }.load(Ordering::Acquire))
    }
    let request = Request::Rename {
        prepared,
        attempt: 1,
        name: raw(b"target"),
        policy: RenamePolicy::NoReplace,
    };
    let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
    let call = Call {
        struct_size: size_of::<Call>() as u32,
        operation: request.operation() as u32,
        payload: Slice {
            data: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
        cancel: Cancellation {
            context: cancelled as *const AtomicBool as u64,
            cancelled: Some(callback),
        },
    };
    let mut output = Buffer::default();
    // SAFETY: caller-owned bytes, callback state and output remain mapped until synchronous return.
    assert_eq!(
        unsafe { client.api.call.unwrap()(client.instance, &call, &mut output) },
        OK
    );
    output.validate(DEFAULT_REPLY_BYTES).unwrap();
    // SAFETY: the instance retains immutable reply storage until the matching release below.
    let response = decode_response(
        unsafe { std::slice::from_raw_parts(output.bytes.data, output.bytes.len as usize) },
        DEFAULT_REPLY_BYTES as usize,
    );
    // SAFETY: this output belongs to the live producing descriptor and instance.
    assert_eq!(
        unsafe { client.api.release_buffer.unwrap()(client.instance, output.id) },
        OK
    );
    match response.unwrap() {
        Response::RenameState(state) => state.outcome,
        response => panic!("expected retained native state: {response:?}"),
    }
}

#[test]
fn a_late_no_replace_target_is_preserved_and_requires_fresh_conflict() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let parent = temp.path.clone();
    let winner = Arc::new(std::sync::Mutex::new(None));
    let seen = winner.clone();
    hooks::at(Boundary::BeforePublish, move |_| {
        fs::write(parent.join("target"), b"new target winner").unwrap();
        *seen.lock().unwrap() = Some(identity(&parent.join("target")));
    });
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    assert!(matches!(outcome, RenameOutcome::Conflict(_)), "{outcome:?}");
    assert_eq!(identity(&temp.path.join("source")), original);
    assert_eq!(
        identity(&temp.path.join("target")),
        winner.lock().unwrap().unwrap()
    );
    assert_eq!(
        fs::read(temp.path.join("target")).unwrap(),
        b"new target winner"
    );
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
}

#[test]
fn changed_target_after_capture_is_restored_then_demands_fresh_consent() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    fs::write(temp.path.join("target"), b"old target").unwrap();
    let old_target = identity(&temp.path.join("target"));
    let policy = replacement(&client, &prepared);
    let parent = temp.path.clone();
    hooks::at(Boundary::AfterCapture, move |_| {
        fs::rename(parent.join("target"), parent.join("old-away")).unwrap();
        fs::write(parent.join("target"), b"new target").unwrap();
    });
    let outcome = attempt(&client, prepared.handle, 2, policy);
    let RenameOutcome::Conflict(conflict) = outcome else {
        panic!("changed consent must issue fresh collision: {outcome:?}");
    };
    assert!(conflict.can_replace);
    assert_eq!(identity(&temp.path.join("source")), original);
    assert_eq!(identity(&temp.path.join("old-away")), old_target);
    assert_eq!(fs::read(temp.path.join("target")).unwrap(), b"new target");
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 3);
}

#[test]
fn source_data_edits_preserve_prepared_object_authority() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    fs::write(temp.path.join("source"), b"edited source contents").unwrap();
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    assert!(
        matches!(outcome, RenameOutcome::Renamed { .. }),
        "{outcome:?}"
    );
    assert_eq!(identity(&temp.path.join("target")), original);
    assert_eq!(
        fs::read(temp.path.join("target")).unwrap(),
        b"edited source contents"
    );
}

#[test]
fn error_return_after_real_capture_does_not_hide_or_delete_captured_source() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    hooks::fail_after_next_rename();
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    assert!(
        matches!(
            outcome,
            RenameOutcome::Unchanged(Failure {
                code: ErrorCode::Io,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert_eq!(identity(&temp.path.join("source")), original);
    assert_eq!(
        fs::read(temp.path.join("source")).unwrap(),
        b"original source"
    );
    assert!(!temp.path.join("target").exists());
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
}

#[test]
fn error_return_after_real_publication_reports_destination_and_empty_stage() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    hooks::at(Boundary::BeforePublish, |_| hooks::fail_after_next_rename());
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = outcome
    else {
        panic!("ambiguous native publication must report actual committed facts: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Destination);
    assert_eq!(destination_name, Some(raw(b"target")));
    assert_eq!(identity(&temp.path.join("target")), original);
    assert!(!temp.path.join("source").exists());
    let stage = locator(residual(&recovery, RenameRecoveryRole::EmptyStage));
    assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
}

#[test]
fn edited_target_in_exchange_gap_is_retained_without_false_success() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    fs::write(temp.path.join("target"), b"old target").unwrap();
    let displaced = identity(&temp.path.join("target"));
    let policy = replacement(&client, &prepared);
    let parent = temp.path.clone();
    hooks::at(Boundary::BeforePublish, move |_| {
        use std::{
            fs::{File, FileTimes},
            time::Duration,
        };
        let path = parent.join("target");
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"new target").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(before + Duration::from_secs(2)))
            .unwrap();
    });
    let outcome = attempt(&client, prepared.handle, 2, policy);
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = outcome
    else {
        panic!("changed displaced revision must not claim replacement success: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Destination);
    assert_eq!(identity(&temp.path.join("target")), original);
    let retained = locator(residual(&recovery, RenameRecoveryRole::DisplacedTarget));
    assert_eq!(identity(&retained), displaced);
    assert_eq!(fs::read(retained).unwrap(), b"new target");
}

#[test]
fn staged_source_substitution_is_retained_without_restoring_or_publishing_it() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let parent = temp.path.clone();
    let substitute = Arc::new(std::sync::Mutex::new(None));
    let seen = substitute.clone();
    hooks::at(Boundary::AfterCapture, move |stage| {
        fs::rename(stage.join("item"), parent.join("original-away")).unwrap();
        fs::write(stage.join("item"), b"staged substitute").unwrap();
        *seen.lock().unwrap() = Some(identity(&stage.join("item")));
    });
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = outcome
    else {
        panic!("staged substitution must remain explicit: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Unknown);
    assert_eq!(identity(&temp.path.join("original-away")), original);
    let path = locator(residual(&recovery, RenameRecoveryRole::Source));
    assert_eq!(identity(&path), substitute.lock().unwrap().unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"staged substitute");
    assert!(!temp.path.join("source").exists());
    assert!(!temp.path.join("target").exists());
}

#[test]
fn error_return_after_real_exchange_reports_displaced_target_role_and_inode() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    fs::write(temp.path.join("target"), b"retained target").unwrap();
    let displaced = identity(&temp.path.join("target"));
    let policy = replacement(&client, &prepared);
    hooks::at(Boundary::BeforePublish, |_| hooks::fail_after_next_rename());
    let outcome = attempt(&client, prepared.handle, 2, policy);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = outcome
    else {
        panic!("ambiguous exchange must preserve actual displaced object: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::Destination);
    assert_eq!(destination_name, Some(raw(b"target")));
    assert_eq!(identity(&temp.path.join("target")), original);
    let path = locator(residual(&recovery, RenameRecoveryRole::DisplacedTarget));
    assert_eq!(identity(&path), displaced);
    assert_eq!(fs::read(path).unwrap(), b"retained target");
    assert!(!temp.path.join("source").exists());
}

#[test]
fn restoration_sync_failure_reports_original_source_and_retained_empty_stage() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    hooks::at(Boundary::AfterCapture, move |_| {
        flag.store(true, Ordering::Release)
    });
    hooks::fail_next_sync();
    let outcome = controlled_call(&client, prepared.handle, &cancelled);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = outcome
    else {
        panic!("restored source durability failure must retain truthful facts: {outcome:?}");
    };
    assert_eq!(source_location, RenameSourceLocation::OriginalName);
    assert_eq!(destination_name, None);
    assert_eq!(identity(&temp.path.join("source")), original);
    assert!(!temp.path.join("target").exists());
    let receipt = residual(&recovery, RenameRecoveryRole::EmptyStage);
    let stage = locator(receipt);
    let observed = identity(&stage);
    assert_eq!(
        u64::from_le_bytes(receipt.identity[0..8].try_into().unwrap()),
        observed.0
    );
    assert_eq!(
        u64::from_le_bytes(receipt.identity[8..16].try_into().unwrap()),
        observed.1
    );
    assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
    assert_eq!(
        client.cancelled(
            Request::CloseRename {
                prepared: prepared.handle
            },
            1
        ),
        Response::Unit
    );
    assert_eq!(identity(&stage), observed);
    assert_eq!(identity(&temp.path.join("source")), original);
}

#[test]
fn substitution_before_restore_validation_never_reports_cached_source_at_substitute() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    hooks::at(Boundary::AfterCapture, move |_| {
        flag.store(true, Ordering::Release)
    });
    let parent = temp.path.clone();
    let substitute = Arc::new(std::sync::Mutex::new(None));
    let seen = substitute.clone();
    hooks::at(Boundary::BeforeRestoreValidation, move |stage| {
        fs::rename(stage.join("item"), parent.join("original-away")).unwrap();
        fs::write(stage.join("item"), b"restoration substitute").unwrap();
        *seen.lock().unwrap() = Some(identity(&stage.join("item")));
    });
    let outcome = controlled_call(&client, prepared.handle, &cancelled);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = &outcome
    else {
        panic!("restoration race must remain explicit: {outcome:?}");
    };
    assert_eq!(*source_location, RenameSourceLocation::Unknown);
    assert_eq!(*destination_name, None);
    assert!(!temp.path.join("source").exists());
    assert!(!temp.path.join("target").exists());
    assert_eq!(identity(&temp.path.join("original-away")), original);
    let receipt = residual(recovery, RenameRecoveryRole::Source);
    let path = locator(receipt);
    let observed = identity(&path);
    assert_eq!(observed, substitute.lock().unwrap().unwrap());
    assert_eq!(path.file_name().unwrap(), "item");
    assert_eq!(fs::read(&path).unwrap(), b"restoration substitute");
    assert_ne!(receipt.identity, prepared.source.identity);
    if !receipt.identity.is_empty() {
        assert_eq!(
            u64::from_le_bytes(receipt.identity[0..8].try_into().unwrap()),
            observed.0
        );
        assert_eq!(
            u64::from_le_bytes(receipt.identity[8..16].try_into().unwrap()),
            observed.1
        );
    }
    let Response::RenameState(query) = client.cancelled(
        Request::GetRenameState {
            prepared: prepared.handle,
        },
        1,
    ) else {
        panic!("query failed");
    };
    assert_eq!(query.outcome, outcome);
    assert_eq!(
        attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace),
        outcome
    );
    assert_eq!(
        client.cancelled(
            Request::CloseRename {
                prepared: prepared.handle
            },
            1
        ),
        Response::Unit
    );
    assert_eq!(identity(&temp.path.join("original-away")), original);
    assert_eq!(identity(&path), observed);
    assert_eq!(fs::read(&path).unwrap(), b"restoration substitute");
}

#[test]
fn failed_exchange_with_original_source_alias_still_reports_displaced_target() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    fs::write(temp.path.join("target"), b"consented target").unwrap();
    let displaced = identity(&temp.path.join("target"));
    let policy = replacement(&client, &prepared);
    let parent = temp.path.clone();
    hooks::at(Boundary::BeforePublish, move |stage| {
        fs::hard_link(stage.join("item"), parent.join("source")).unwrap();
        hooks::fail_after_next_rename();
    });
    let outcome = attempt(&client, prepared.handle, 2, policy.clone());
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = &outcome
    else {
        panic!("post-exchange error must retain displaced target: {outcome:?}");
    };
    assert_eq!(
        *source_location,
        RenameSourceLocation::Unknown,
        "two source aliases must not become a forced destination claim"
    );
    assert_eq!(identity(&temp.path.join("source")), original);
    assert_eq!(identity(&temp.path.join("target")), original);
    assert_eq!(
        fs::read(temp.path.join("source")).unwrap(),
        b"original source"
    );
    assert_eq!(
        fs::read(temp.path.join("target")).unwrap(),
        b"original source"
    );
    let receipt = residual(recovery, RenameRecoveryRole::DisplacedTarget);
    let path = locator(receipt);
    assert_eq!(identity(&path), displaced);
    assert_eq!(fs::read(&path).unwrap(), b"consented target");
    assert!(
        !recovery
            .iter()
            .any(|record| record.role == RenameRecoveryRole::Source
                && identity(&locator(record)) == displaced)
    );
    let Response::RenameState(query) = client.call(Request::GetRenameState {
        prepared: prepared.handle,
    }) else {
        panic!("query failed");
    };
    assert_eq!(query.outcome, outcome);
    assert_eq!(attempt(&client, prepared.handle, 2, policy), outcome);
    assert_eq!(
        client.call(Request::CloseRename {
            prepared: prepared.handle
        }),
        Response::Unit
    );
    assert_eq!(identity(&temp.path.join("source")), original);
    assert_eq!(identity(&temp.path.join("target")), original);
    assert_eq!(identity(&path), displaced);
    assert_eq!(fs::read(path).unwrap(), b"consented target");
}

fn assert_recovery_receipt_identity(receipt: &RenameRecovery, expected: (u64, u64)) {
    assert!(
        receipt.identity.len() >= 16,
        "recovery locator must retain its observed identity"
    );
    assert_eq!(
        u64::from_le_bytes(receipt.identity[0..8].try_into().unwrap()),
        expected.0
    );
    assert_eq!(
        u64::from_le_bytes(receipt.identity[8..16].try_into().unwrap()),
        expected.1
    );
    assert_eq!(identity(&locator(receipt)), expected);
}

fn assert_recovery_query_replay_and_close(
    client: &Client,
    prepared: &PreparedRename,
    outcome: &RenameOutcome,
) {
    let Response::RenameState(query) = client.cancelled(
        Request::GetRenameState {
            prepared: prepared.handle,
        },
        1,
    ) else {
        panic!("query failed");
    };
    assert_eq!(&query.outcome, outcome);
    assert_eq!(
        &attempt(client, prepared.handle, 1, RenamePolicy::NoReplace),
        outcome
    );
    assert_eq!(
        client.cancelled(
            Request::CloseRename {
                prepared: prepared.handle,
            },
            1,
        ),
        Response::Unit
    );
}

#[test]
fn recovery_locator_tracks_relocated_parent_without_adopting_old_path_decoy() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let parent = temp
        .path
        .join(OsString::from_vec(b"old-parent-\xff".to_vec()));
    let moved_parent = temp
        .path
        .join(OsString::from_vec(b"moved-parent-\xfe".to_vec()));
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("source"), b"retained original source").unwrap();
    let original = identity(&parent.join("source"));
    let client = Client::new();
    let prepared = prepare(&client, &parent.join("source"));
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let old_parent = parent.clone();
    let relocated_parent = moved_parent.clone();
    let observed = Arc::new(std::sync::Mutex::new(None));
    let seen = observed.clone();
    hooks::at(Boundary::AfterCapture, move |stage| {
        let basename = stage.file_name().unwrap().to_owned();
        fs::rename(&old_parent, &relocated_parent).unwrap();
        fs::create_dir(&old_parent).unwrap();
        fs::write(old_parent.join("source"), b"old source path decoy").unwrap();
        let decoy_stage = old_parent.join(&basename);
        fs::create_dir(&decoy_stage).unwrap();
        fs::write(decoy_stage.join("item"), b"old recovery path decoy").unwrap();
        *seen.lock().unwrap() = Some((
            relocated_parent.join(basename).join("item"),
            decoy_stage.join("item"),
        ));
        flag.store(true, Ordering::Release);
    });
    let outcome = controlled_call(&client, prepared.handle, &cancelled);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = &outcome
    else {
        panic!("relocated parent must retain explicit recovery facts: {outcome:?}");
    };
    assert!(matches!(
        source_location,
        RenameSourceLocation::Recovery | RenameSourceLocation::Unknown
    ));
    assert_eq!(*destination_name, None);
    let (retained, decoy) = observed.lock().unwrap().clone().unwrap();
    assert_eq!(identity(&retained), original);
    assert_ne!(identity(&decoy), original);
    assert_eq!(fs::read(&retained).unwrap(), b"retained original source");
    assert_eq!(fs::read(&decoy).unwrap(), b"old recovery path decoy");
    let receipt = residual(recovery, RenameRecoveryRole::Source);
    assert_eq!(
        locator(receipt),
        retained,
        "a retained FD identity must not be paired with its former pathname"
    );
    assert!(std::str::from_utf8(&receipt.address.bytes).is_err());
    assert_recovery_receipt_identity(receipt, original);
    assert_recovery_query_replay_and_close(&client, &prepared, &outcome);
    assert_eq!(identity(&retained), original);
    assert_eq!(fs::read(&decoy).unwrap(), b"old recovery path decoy");
    assert_eq!(
        fs::read(parent.join("source")).unwrap(),
        b"old source path decoy"
    );
    assert!(!parent.join("target").exists());
    assert!(!moved_parent.join("target").exists());
}

#[test]
fn recovery_locator_tracks_relocated_stage_without_adopting_old_path_decoy() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let retained_stage = temp
        .path
        .join(OsString::from_vec(b"moved-stage-\xff".to_vec()));
    let moved_stage = retained_stage.clone();
    let observed = Arc::new(std::sync::Mutex::new(None));
    let seen = observed.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    hooks::at(Boundary::AfterCapture, move |stage| {
        fs::rename(stage, &moved_stage).unwrap();
        fs::create_dir(stage).unwrap();
        fs::write(stage.join("item"), b"old stage path decoy").unwrap();
        *seen.lock().unwrap() = Some(stage.join("item"));
        flag.store(true, Ordering::Release);
    });
    let outcome = controlled_call(&client, prepared.handle, &cancelled);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = &outcome
    else {
        panic!("relocated stage must retain explicit recovery facts: {outcome:?}");
    };
    assert!(matches!(
        source_location,
        RenameSourceLocation::Recovery | RenameSourceLocation::Unknown
    ));
    assert_eq!(*destination_name, None);
    let retained = retained_stage.join("item");
    let decoy = observed.lock().unwrap().clone().unwrap();
    assert_eq!(identity(&retained), original);
    assert_ne!(identity(&decoy), original);
    assert_eq!(fs::read(&retained).unwrap(), b"original source");
    assert_eq!(fs::read(&decoy).unwrap(), b"old stage path decoy");
    let receipt = residual(recovery, RenameRecoveryRole::Source);
    assert_eq!(locator(receipt), retained);
    assert!(std::str::from_utf8(&receipt.address.bytes).is_err());
    assert_recovery_receipt_identity(receipt, original);
    assert_recovery_query_replay_and_close(&client, &prepared, &outcome);
    assert_eq!(identity(&retained), original);
    assert_eq!(fs::read(&decoy).unwrap(), b"old stage path decoy");
    assert!(!temp.path.join("source").exists());
    assert!(!temp.path.join("target").exists());
}

#[test]
fn recovery_locator_tracks_relocated_empty_stage_and_preserves_old_path_decoy() {
    let _reset = hooks::reset();
    let (temp, client, prepared, original) = setup();
    let retained_stage = temp
        .path
        .join(OsString::from_vec(b"moved-empty-stage-\xff".to_vec()));
    let moved_stage = retained_stage.clone();
    let observed = Arc::new(std::sync::Mutex::new(None));
    let seen = observed.clone();
    hooks::at(Boundary::BeforeCleanup, move |stage| {
        let retained_identity = identity(stage);
        fs::rename(stage, &moved_stage).unwrap();
        fs::create_dir(stage).unwrap();
        fs::write(stage.join("decoy-child"), b"preserve decoy child").unwrap();
        *seen.lock().unwrap() = Some((stage.to_owned(), retained_identity));
    });
    let outcome = attempt(&client, prepared.handle, 1, RenamePolicy::NoReplace);
    let RenameOutcome::RecoveryRequired {
        source_location,
        destination_name,
        recovery,
        ..
    } = &outcome
    else {
        panic!("relocated empty stage must retain explicit recovery facts: {outcome:?}");
    };
    assert_eq!(*source_location, RenameSourceLocation::Destination);
    assert_eq!(*destination_name, Some(raw(b"target")));
    assert_eq!(identity(&temp.path.join("target")), original);
    let (decoy, retained_identity) = observed.lock().unwrap().clone().unwrap();
    assert_eq!(identity(&retained_stage), retained_identity);
    assert_ne!(identity(&decoy), retained_identity);
    assert_eq!(fs::read_dir(&retained_stage).unwrap().count(), 0);
    let receipt = residual(recovery, RenameRecoveryRole::EmptyStage);
    assert_eq!(locator(receipt), retained_stage);
    assert!(std::str::from_utf8(&receipt.address.bytes).is_err());
    assert_recovery_receipt_identity(receipt, retained_identity);
    assert_recovery_query_replay_and_close(&client, &prepared, &outcome);
    assert_eq!(identity(&retained_stage), retained_identity);
    assert_eq!(fs::read_dir(&retained_stage).unwrap().count(), 0);
    assert_eq!(
        fs::read(decoy.join("decoy-child")).unwrap(),
        b"preserve decoy child"
    );
    assert_eq!(identity(&temp.path.join("target")), original);
}
