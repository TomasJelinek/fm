//! Stage admission, refused-stage cleanup, parent protection and case aliases
//! through the exported C ABI. Mode, ownership and lookup behavior of other
//! filesystems is simulated only where an owned local fixture cannot show it.
use crate::{
    rename_test_hooks::{self as hooks, Boundary},
    rename_test_support::{Client, Temp},
};
use midnight_filesystem_plugin_api::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

fn identity(path: &Path) -> (u64, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}
fn name(bytes: &[u8]) -> NativeText {
    NativeText {
        encoding: NameEncoding::UnixBytes,
        bytes: bytes.to_vec(),
    }
}
fn prepare_response(client: &Client, path: &Path) -> Response {
    let (mount, node) = client.resolve(path);
    client.call(Request::PrepareRename {
        mount,
        node: node.id,
    })
}
fn prepare(client: &Client, path: &Path) -> PreparedRename {
    match prepare_response(client, path) {
        Response::RenamePrepared(value) => value,
        response => panic!("real ABI Prepare failed: {response:?}"),
    }
}
fn rename(client: &Client, prepared: &PreparedRename, destination: &[u8]) -> RenameOutcome {
    match client.call(Request::Rename {
        prepared: prepared.handle,
        attempt: 1,
        name: name(destination),
        policy: RenamePolicy::NoReplace,
    }) {
        Response::RenameState(state) => state.outcome,
        response => panic!("real ABI Rename failed: {response:?}"),
    }
}
fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}
fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
fn renamed(outcome: &RenameOutcome) -> bool {
    matches!(outcome, RenameOutcome::Renamed {
        same_entry: false,
        recovery,
        durability_confirmed: true,
        ..
    } if recovery.is_empty())
}
fn unchanged(outcome: &RenameOutcome, code: ErrorCode) -> bool {
    matches!(outcome, RenameOutcome::Unchanged(failure) if failure.code == code)
}

#[test]
fn private_mount_provided_stage_mode_is_accepted_and_the_stage_removed() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let source = temp.path.join("source");
    fs::write(&source, b"source bytes").unwrap();
    let original = identity(&source);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    // vfat/exfat with a 022 mask: every directory reports 0755 and keeps it.
    hooks::ignore_private_mode();
    hooks::at(Boundary::AfterStageCreate, |stage| chmod(stage, 0o755));
    let outcome = rename(&client, &prepared, b"target");
    assert!(renamed(&outcome), "{outcome:?}");
    assert_eq!(identity(&temp.path.join("target")), original);
    assert_eq!(names(&temp.path), ["target"]);
}

#[test]
fn stage_other_users_could_change_is_refused_and_removed() {
    for mode in [0o777, 0o757] {
        let _reset = hooks::reset();
        let temp = Temp::new();
        let source = temp.path.join("source");
        fs::write(&source, b"source bytes").unwrap();
        let original = identity(&source);
        let client = Client::new();
        let prepared = prepare(&client, &source);
        hooks::ignore_private_mode();
        hooks::at(Boundary::AfterStageCreate, move |stage| chmod(stage, mode));
        let outcome = rename(&client, &prepared, b"target");
        assert!(unchanged(&outcome, ErrorCode::Unsupported), "{outcome:?}");
        assert_eq!(identity(&source), original);
        assert_eq!(names(&temp.path), ["source"], "mode {mode:o}");
    }
}

#[test]
fn restrictive_stage_mode_is_corrected_to_owner_only() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let source = temp.path.join("source");
    fs::write(&source, b"source bytes").unwrap();
    let original = identity(&source);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    // A umask or default ACL without owner write would block the capture.
    hooks::at(Boundary::AfterStageCreate, |stage| chmod(stage, 0o500));
    let outcome = rename(&client, &prepared, b"target");
    assert!(renamed(&outcome), "{outcome:?}");
    assert_eq!(identity(&temp.path.join("target")), original);
    assert_eq!(names(&temp.path), ["target"]);
}

/// The mkdir (`true`) or capture renameat2 on unqualified storage fails with
/// `errno` without performing it; nothing may remain.
fn refused_on_unqualified_storage(mkdir: bool, errno: i32) -> RenameOutcome {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let source = temp.path.join("source");
    fs::write(&source, b"source bytes").unwrap();
    let original = identity(&source);
    let client = Client::new();
    hooks::treat_filesystem_as_unqualified();
    let prepared = prepare(&client, &source);
    assert!(!prepared.recoverable_replace);
    if mkdir {
        hooks::refuse_next_mkdir(errno);
    } else {
        hooks::refuse_next_rename(errno);
    }
    let outcome = rename(&client, &prepared, b"target");
    assert_eq!(identity(&source), original, "errno {errno}: {outcome:?}");
    assert_eq!(names(&temp.path), ["source"], "errno {errno}: {outcome:?}");
    outcome
}
fn unchanged_with(outcome: &RenameOutcome, code: ErrorCode, errno: i32) -> bool {
    matches!(outcome, RenameOutcome::Unchanged(failure)
        if failure.code == code && failure.os_code == Some(errno))
}

#[test]
fn definite_refusals_on_unqualified_storage_leave_nothing_behind() {
    for (mkdir, errno, code) in [
        (true, libc::EACCES, ErrorCode::PermissionDenied),
        (true, libc::EROFS, ErrorCode::Io),
        (false, libc::EXDEV, ErrorCode::Io),
        // CIFS sharing violations: an item another SMB client holds open.
        (false, libc::EBUSY, ErrorCode::Io),
        (false, libc::ETXTBSY, ErrorCode::Io),
    ] {
        let outcome = refused_on_unqualified_storage(mkdir, errno);
        assert!(
            unchanged_with(&outcome, code, errno),
            "errno {errno}: {outcome:?}"
        );
    }
}

#[test]
fn refused_rename_flags_are_unsupported_and_leave_nothing_behind() {
    // A FUSE server without rename flags refuses RENAME_NOREPLACE.
    for errno in [libc::EINVAL, libc::ENOSYS, libc::EOPNOTSUPP] {
        let outcome = refused_on_unqualified_storage(false, errno);
        assert!(
            unchanged_with(&outcome, ErrorCode::Unsupported, errno)
                && matches!(&outcome, RenameOutcome::Unchanged(failure)
                    if failure.message.starts_with("This filesystem cannot rename without")),
            "errno {errno}: {outcome:?}"
        );
    }
}

#[test]
fn failures_before_the_capture_request_remove_the_stage_on_every_filesystem() {
    for unqualified in [false, true] {
        let _reset = hooks::reset();
        let temp = Temp::new();
        let parent = temp.path.join("parent");
        fs::create_dir(&parent).unwrap();
        let source = parent.join("source");
        fs::write(&source, b"source bytes").unwrap();
        let original = identity(&source);
        let client = Client::new();
        if unqualified {
            hooks::treat_filesystem_as_unqualified();
        }
        let prepared = prepare(&client, &source);
        // Other users may change the parent after the stage exists: the capture is never requested.
        let shared = parent.clone();
        hooks::at(Boundary::AfterStageCreate, move |_| chmod(&shared, 0o777));
        let outcome = rename(&client, &prepared, b"target");
        chmod(&parent, 0o755);
        assert!(
            unchanged(&outcome, ErrorCode::Unsupported),
            "unqualified {unqualified}: {outcome:?}"
        );
        assert_eq!(identity(&source), original);
        assert_eq!(names(&parent), ["source"], "unqualified {unqualified}");
    }
}

#[test]
fn nfs_mounts_are_refused_before_a_name_is_requested() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let source = temp.path.join("source");
    fs::write(&source, b"source bytes").unwrap();
    let client = Client::new();
    hooks::treat_filesystem_as_nfs();
    let response = prepare_response(&client, &source);
    assert!(
        matches!(&response, Response::Failed(failure)
            if failure.code == ErrorCode::Unsupported && failure.message.contains("NFS")),
        "{response:?}"
    );
    assert_eq!(names(&temp.path), ["source"]);
}

#[test]
fn ambiguous_capture_failure_on_unqualified_storage_retains_the_empty_stage() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let source = temp.path.join("source");
    fs::write(&source, b"source bytes").unwrap();
    let original = identity(&source);
    let client = Client::new();
    hooks::treat_filesystem_as_unqualified();
    let prepared = prepare(&client, &source);
    // A remote request that failed with EIO may still complete later.
    hooks::refuse_next_rename(libc::EIO);
    let outcome = rename(&client, &prepared, b"target");
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = outcome
    else {
        panic!("an ambiguous remote failure must stay explicit: {outcome:?}")
    };
    assert_eq!(source_location, RenameSourceLocation::OriginalName);
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].role, RenameRecoveryRole::EmptyStage);
    assert_eq!(identity(&source), original);
    let stage = names(&temp.path);
    assert_eq!(stage.len(), 2);
    assert!(stage[0].starts_with(".midnight-rename-"));
    assert_eq!(fs::read_dir(temp.path.join(&stage[0])).unwrap().count(), 0);
}

#[test]
fn case_only_rename_on_a_case_insensitive_lookup_publishes_the_new_name() {
    for directory in [false, true] {
        let _reset = hooks::reset();
        let temp = Temp::new();
        let source = temp.path.join("photo.JPG");
        if directory {
            fs::create_dir(&source).unwrap();
        } else {
            fs::write(&source, b"photo bytes").unwrap();
        }
        let original = identity(&source);
        let client = Client::new();
        let prepared = prepare(&client, &source);
        // vfat, exfat, CIFS and casefolded directories resolve the new name to the source.
        hooks::fold_lookup_case();
        let outcome = rename(&client, &prepared, b"photo.jpg");
        assert!(renamed(&outcome), "{outcome:?}");
        assert_eq!(names(&temp.path), ["photo.jpg"]);
        assert_eq!(identity(&temp.path.join("photo.jpg")), original);
    }
}

#[test]
fn case_alias_of_another_hard_link_remains_a_conflict() {
    let _reset = hooks::reset();
    let temp = Temp::new();
    let source = temp.path.join("photo.JPG");
    fs::write(&source, b"photo bytes").unwrap();
    fs::hard_link(&source, temp.path.join("PHOTO.jpg")).unwrap();
    let original = identity(&source);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    hooks::fold_lookup_case();
    let outcome = rename(&client, &prepared, b"photo.jpg");
    let RenameOutcome::Conflict(conflict) = outcome else {
        panic!("another name of the same file must not be claimed: {outcome:?}")
    };
    assert!(!conflict.can_replace);
    assert_eq!(conflict.target.identity, prepared.source.identity);
    assert_eq!(names(&temp.path), ["PHOTO.jpg", "photo.JPG"]);
    assert_eq!(identity(&source), original);
}

#[test]
fn shared_writable_parents_are_refused_before_a_name_is_requested() {
    for (mode, private_group, accepted) in [
        (0o775, true, true),
        (0o775, false, false),
        (0o777, true, false),
        (0o1777, false, true),
    ] {
        let _reset = hooks::reset();
        let temp = Temp::new();
        let parent = temp.path.join("parent");
        fs::create_dir(&parent).unwrap();
        let source = parent.join("source");
        fs::write(&source, b"source bytes").unwrap();
        chmod(&parent, mode);
        hooks::assume_private_group(private_group);
        let client = Client::new();
        let response = prepare_response(&client, &source);
        if accepted {
            let Response::RenamePrepared(prepared) = response else {
                panic!("mode {mode:o} must be accepted: {response:?}")
            };
            let outcome = rename(&client, &prepared, b"target");
            assert!(renamed(&outcome), "mode {mode:o}: {outcome:?}");
            assert_eq!(names(&parent), ["target"]);
        } else {
            assert!(
                matches!(
                    response,
                    Response::Failed(Failure {
                        code: ErrorCode::Unsupported,
                        ..
                    })
                ),
                "mode {mode:o}: {response:?}"
            );
            assert_eq!(names(&parent), ["source"]);
        }
        chmod(&parent, 0o755);
    }
}
