#![cfg(target_os = "linux")]
mod support;
use midnight_filesystem_plugin_api::*;
use std::{
    ffi::OsString,
    fs::{self, File, FileTimes},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, symlink},
    },
    time::Duration,
};
use support::*;

fn native_identity(path: &std::path::Path) -> (u64, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

fn name(bytes: &[u8]) -> NativeText {
    NativeText {
        encoding: NameEncoding::UnixBytes,
        bytes: bytes.to_vec(),
    }
}
fn prepare(client: &Client, path: &std::path::Path) -> PreparedRename {
    let (mount, node) = client.resolve(path);
    match client.call(Request::PrepareRename {
        mount,
        node: node.id,
    }) {
        Response::RenamePrepared(value) => value,
        other => panic!("native prepared Rename must succeed: {other:?}"),
    }
}
fn attempt(client: &Client, prepared: u64, attempt: u64, policy: RenamePolicy) -> RenameOutcome {
    let Response::RenameState(state) = client.call(Request::Rename {
        prepared,
        attempt,
        name: name(b"target"),
        policy,
    }) else {
        panic!("native Rename must return its retained state")
    };
    state.outcome
}
fn conflict(outcome: RenameOutcome) -> RenameConflict {
    match outcome {
        RenameOutcome::Conflict(value) => value,
        other => panic!("destination must require explicit consent: {other:?}"),
    }
}
fn recovery_path(value: &RenameRecovery) -> std::path::PathBuf {
    assert_eq!(value.address.encoding, AddressEncoding::UnixBytes);
    std::path::PathBuf::from(std::ffi::OsString::from_vec(value.address.bytes.clone()))
}

#[test]
fn no_replace_conflict_preserves_source_and_target_and_pins_metadata() {
    let temp = Temp::new();
    fs::write(temp.path.join("source"), b"source bytes").unwrap();
    fs::write(temp.path.join("target"), b"target bytes").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &temp.path.join("source"));
    let collision = conflict(attempt(
        &client,
        prepared.handle,
        1,
        RenamePolicy::NoReplace,
    ));
    assert_eq!(collision.target.name, name(b"target"));
    assert_eq!(collision.target.metadata.size, Some(12));
    assert_ne!(collision.target.identity, prepared.source.identity);
    assert_eq!(collision.can_replace, prepared.recoverable_replace);
    assert_eq!(fs::read(temp.path.join("source")).unwrap(), b"source bytes");
    assert_eq!(fs::read(temp.path.join("target")).unwrap(), b"target bytes");
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
}

#[test]
fn qualified_replacement_retains_displaced_target_with_exact_raw_locator() {
    let temp = Temp::new();
    fs::write(temp.path.join("source"), b"source bytes").unwrap();
    fs::write(temp.path.join("target"), b"displaced bytes").unwrap();
    let target_id = native_identity(&temp.path.join("target"));
    let source_id = native_identity(&temp.path.join("source"));
    let client = Client::new();
    let prepared = prepare(&client, &temp.path.join("source"));
    assert!(
        prepared.recoverable_replace,
        "replacement qualification needs an owned local ext/XFS/Btrfs/tmpfs fixture; do not run it on DrvFS/remote storage"
    );
    let collision = conflict(attempt(
        &client,
        prepared.handle,
        1,
        RenamePolicy::NoReplace,
    ));
    assert!(collision.can_replace);
    let outcome = attempt(
        &client,
        prepared.handle,
        2,
        RenamePolicy::RecoverableCurrentEntry {
            conflict: collision.token,
        },
    );
    let RenameOutcome::Renamed {
        recovery,
        same_entry,
        durability_confirmed,
        ..
    } = outcome
    else {
        panic!("qualified replacement must publish and retain the target: {outcome:?}")
    };
    assert!(!same_entry);
    assert_eq!(native_identity(&temp.path.join("target")), source_id);
    assert!(durability_confirmed);
    assert!(!temp.path.join("source").exists());
    assert_eq!(fs::read(temp.path.join("target")).unwrap(), b"source bytes");
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].role, RenameRecoveryRole::DisplacedTarget);
    assert_eq!(recovery[0].identity, collision.target.identity);
    let retained = recovery_path(&recovery[0]);
    assert!(retained.starts_with(&temp.path));
    assert_eq!(fs::read(&retained).unwrap(), b"displaced bytes");
    assert_eq!(native_identity(&retained), target_id);
    assert_eq!(
        client.call(Request::CloseRename {
            prepared: prepared.handle
        }),
        Response::Unit
    );
    assert_eq!(fs::read(retained).unwrap(), b"displaced bytes");
}

#[test]
fn changed_target_requires_fresh_consent_and_preserves_every_object() {
    let temp = Temp::new();
    fs::write(temp.path.join("source"), b"source bytes").unwrap();
    fs::write(temp.path.join("target"), b"old target").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &temp.path.join("source"));
    assert!(
        prepared.recoverable_replace,
        "replacement test requires qualified local fixture storage"
    );
    let old = conflict(attempt(
        &client,
        prepared.handle,
        1,
        RenamePolicy::NoReplace,
    ));
    fs::rename(temp.path.join("target"), temp.path.join("old-target")).unwrap();
    fs::write(temp.path.join("target"), b"new target").unwrap();
    let fresh = conflict(attempt(
        &client,
        prepared.handle,
        2,
        RenamePolicy::RecoverableCurrentEntry {
            conflict: old.token,
        },
    ));
    assert_ne!(fresh.token, old.token);
    assert_ne!(fresh.target.identity, old.target.identity);
    assert!(fresh.can_replace);
    assert_eq!(fs::read(temp.path.join("source")).unwrap(), b"source bytes");
    assert_eq!(fs::read(temp.path.join("target")).unwrap(), b"new target");
    assert_eq!(
        fs::read(temp.path.join("old-target")).unwrap(),
        b"old target"
    );
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 3);
}

#[test]
fn distinct_hardlink_and_link_or_directory_targets_never_enable_replace() {
    for target_kind in 0..3 {
        let temp = Temp::new();
        fs::write(temp.path.join("source"), b"source bytes").unwrap();
        match target_kind {
            0 => fs::hard_link(temp.path.join("source"), temp.path.join("target")).unwrap(),
            1 => symlink(temp.path.join("source"), temp.path.join("target")).unwrap(),
            _ => fs::create_dir(temp.path.join("target")).unwrap(),
        }
        let client = Client::new();
        let prepared = prepare(&client, &temp.path.join("source"));
        let collision = conflict(attempt(
            &client,
            prepared.handle,
            1,
            RenamePolicy::NoReplace,
        ));
        assert!(!collision.can_replace, "{collision:?}");
        assert_eq!(fs::read(temp.path.join("source")).unwrap(), b"source bytes");
        assert!(fs::symlink_metadata(temp.path.join("target")).is_ok());
        assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
    }
}

#[test]
fn same_inode_target_revision_change_requires_newest_consent_without_writes() {
    let temp = Temp::new();
    let source = temp.path.join("source");
    let target = temp.path.join("target");
    fs::write(&source, b"source bytes").unwrap();
    fs::write(&target, b"old target").unwrap();
    let source_id = native_identity(&source);
    let target_id = native_identity(&target);
    let before = fs::symlink_metadata(&target).unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert!(
        prepared.recoverable_replace,
        "replacement test requires qualified local fixture storage"
    );
    let old = conflict(attempt(
        &client,
        prepared.handle,
        1,
        RenamePolicy::NoReplace,
    ));
    assert!(old.can_replace);
    // Edit in place and force a distinct mtime, even on coarse timestamp filesystems.
    fs::write(&target, b"new target").unwrap();
    let modified = before
        .modified()
        .unwrap()
        .checked_add(Duration::from_secs(2))
        .unwrap();
    File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(native_identity(&target), target_id);
    assert_ne!(
        fs::symlink_metadata(&target).unwrap().modified().unwrap(),
        before.modified().unwrap()
    );
    let fresh = conflict(attempt(
        &client,
        prepared.handle,
        2,
        RenamePolicy::RecoverableCurrentEntry {
            conflict: old.token,
        },
    ));
    assert_ne!(fresh.token, old.token);
    assert_eq!(fresh.target.identity, old.target.identity);
    assert_ne!(fresh.target.revision, old.target.revision);
    assert_eq!(fresh.target.metadata.size, old.target.metadata.size);
    assert!(fresh.can_replace);
    assert_eq!(native_identity(&source), source_id);
    assert_eq!(native_identity(&target), target_id);
    assert_eq!(fs::read(&source).unwrap(), b"source bytes");
    assert_eq!(fs::read(&target).unwrap(), b"new target");
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
    assert!(matches!(
        client.call(Request::Rename {
            prepared: prepared.handle,
            attempt: 3,
            name: name(b"target"),
            policy: RenamePolicy::RecoverableCurrentEntry {
                conflict: old.token
            },
        }),
        Response::Failed(Failure {
            code: ErrorCode::Invalid,
            ..
        })
    ));
    assert_eq!(native_identity(&source), source_id);
    assert_eq!(native_identity(&target), target_id);
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
    let outcome = attempt(
        &client,
        prepared.handle,
        3,
        RenamePolicy::RecoverableCurrentEntry {
            conflict: fresh.token,
        },
    );
    let RenameOutcome::Renamed { recovery, .. } = outcome else {
        panic!("only newest revision consent may replace: {outcome:?}")
    };
    assert_eq!(native_identity(&target), source_id);
    assert!(!source.exists());
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].role, RenameRecoveryRole::DisplacedTarget);
    assert_eq!(recovery[0].identity, fresh.target.identity);
    let retained = recovery_path(&recovery[0]);
    assert_eq!(native_identity(&retained), target_id);
    assert_eq!(fs::read(retained).unwrap(), b"new target");
}

#[test]
fn replacement_beneath_raw_parent_reports_lossless_recovery_and_retained_inode() {
    let temp = Temp::new();
    let parent = temp.path.join(OsString::from_vec(b"parent-\xff".to_vec()));
    fs::create_dir(&parent).unwrap();
    let source = parent.join("source");
    let target = parent.join("target");
    fs::write(&source, b"source bytes").unwrap();
    fs::write(&target, b"retained target").unwrap();
    let source_id = native_identity(&source);
    let target_id = native_identity(&target);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert!(
        prepared.recoverable_replace,
        "replacement test requires qualified local fixture storage"
    );
    let collision = conflict(attempt(
        &client,
        prepared.handle,
        1,
        RenamePolicy::NoReplace,
    ));
    assert!(collision.can_replace);
    let outcome = attempt(
        &client,
        prepared.handle,
        2,
        RenamePolicy::RecoverableCurrentEntry {
            conflict: collision.token,
        },
    );
    let RenameOutcome::Renamed { recovery, .. } = outcome else {
        panic!("raw parent must retain lossless replacement recovery: {outcome:?}")
    };
    assert_eq!(native_identity(&target), source_id);
    assert!(!source.exists());
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].role, RenameRecoveryRole::DisplacedTarget);
    assert_eq!(recovery[0].identity, collision.target.identity);
    assert_eq!(recovery[0].address.encoding, AddressEncoding::UnixBytes);
    let mut raw_prefix = parent.as_os_str().as_bytes().to_vec();
    raw_prefix.push(b'/');
    assert!(recovery[0].address.bytes.starts_with(&raw_prefix));
    assert!(std::str::from_utf8(&recovery[0].address.bytes).is_err());
    let retained = recovery_path(&recovery[0]);
    assert_eq!(retained.parent().unwrap().parent(), Some(parent.as_path()));
    assert_eq!(native_identity(&retained), target_id);
    assert_eq!(fs::read(&retained).unwrap(), b"retained target");
    assert_eq!(
        client.call(Request::CloseRename {
            prepared: prepared.handle
        }),
        Response::Unit
    );
    assert_eq!(native_identity(&retained), target_id);
    assert_eq!(fs::read(retained).unwrap(), b"retained target");
}
