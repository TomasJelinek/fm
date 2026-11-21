//! A child that cannot be opened is that child's failure, not its folder's: the
//! folder's enumeration continues with the next child.
use super::*;
use midnight_core::TransferService;
use std::{fs, os::unix::fs::PermissionsExt};

/// Whether permissions bind this process; they do not bind the superuser.
fn privileged() -> bool {
    let root = rustix::process::geteuid().is_root();
    if root {
        eprintln!("SKIP: permissions do not apply to the superuser");
    }
    root
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_child_that_cannot_be_read_fails_alone_and_can_be_opened_again_by_name() {
    if privileged() {
        return;
    }
    let temp = crate::test_support::temporary("child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir_all(folder.join("locked")).unwrap();
    for file in ["first", "unreadable", "last"] {
        fs::write(folder.join(file), file).unwrap();
    }
    chmod(&folder.join("unreadable"), 0o000);
    chmod(&folder.join("locked"), 0o000);
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&folder, &cancel).unwrap();
    let (mut opened, mut failed) = (Vec::new(), Vec::new());
    while let Some(child) = source.next_child_or_failure(&cancel).unwrap() {
        match child {
            Ok(child) => opened.push(child.entry().item.path.clone()),
            Err(failure) => {
                assert_eq!(
                    failure.error.kind,
                    io::ErrorKind::PermissionDenied,
                    "{:?}",
                    failure.error
                );
                assert_eq!(failure.error.path, folder.join(&failure.name));
                failed.push(failure.name);
            }
        }
    }
    opened.sort();
    failed.sort();
    assert_eq!(opened, [folder.join("first"), folder.join("last")]);
    assert_eq!(failed, ["locked", "unreadable"]);
    // Retry: once readable, each child opens by its name.
    chmod(&folder.join("unreadable"), 0o644);
    chmod(&folder.join("locked"), 0o755);
    for name in ["locked", "unreadable"] {
        let child = source.open_child(OsStr::new(name), &cancel).unwrap();
        assert_eq!(child.entry().item.path, folder.join(name));
    }
}

#[test]
fn a_folder_whose_children_cannot_be_opened_fails_as_a_whole() {
    if privileged() {
        return;
    }
    let temp = crate::test_support::temporary("child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    for file in ["first", "second"] {
        fs::write(folder.join(file), file).unwrap();
    }
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&folder, &cancel).unwrap();
    // Names can be read but not opened: one failure for the folder, not one per child.
    chmod(&folder, 0o644);
    let failure = source.next_child_or_failure(&cancel).err();
    chmod(&folder, 0o755);
    let failure = failure.expect("the folder itself fails");
    assert_eq!(failure.kind, io::ErrorKind::PermissionDenied, "{failure:?}");
    assert_eq!(failure.path, folder);
}

#[test]
fn a_replaced_folder_is_the_folder_s_failure_not_a_child_s() {
    let temp = crate::test_support::temporary("child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file"), b"file").unwrap();
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&folder, &cancel).unwrap();
    fs::rename(&folder, temp.path().join("moved")).unwrap();
    fs::create_dir(&folder).unwrap();
    let failure = source
        .next_child_or_failure(&cancel)
        .err()
        .expect("the folder itself changed");
    assert_eq!(failure.path, folder);
}

/// A folder selected for Move as the Linux Move dialog prepares it (links move
/// themselves), with the policy it enumerates its children under.
fn prepared_move_folder(folder: &Path) -> (Box<dyn TransferSource>, CopyLinkPolicy) {
    let policy = CopyLinkPolicy::Preserve;
    let midnight_core::PreparedTransferMoveEntry::Ordinary(prepared) =
        crate::LinuxFileSystem::new(None)
            .prepare_move_source_with_policy(
                folder,
                policy,
                TransferRevisionPolicy::AcceptChangeTimeOnly,
                &Cancellation::default(),
            )
            .unwrap()
    else {
        panic!("an ordinary Move folder");
    };
    (prepared.source, policy)
}

#[test]
fn a_move_child_that_cannot_be_prepared_fails_alone_and_can_be_prepared_again_by_name() {
    let temp = crate::test_support::temporary("move-child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    for file in ["first", "last"] {
        fs::write(folder.join(file), file).unwrap();
    }
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        folder.join("pipe"),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let cancel = Cancellation::default();
    let (mut source, policy) = prepared_move_folder(&folder);
    let (mut prepared, mut failed) = (Vec::new(), Vec::new());
    while let Some(child) = source.next_move_child_or_failure(policy, &cancel).unwrap() {
        match child {
            Ok(entry) => prepared.push(entry.source().entry().item.path.clone()),
            Err(failure) => {
                // A special file cannot be moved by copying: no Retry can change that.
                assert_eq!(
                    failure.error.kind,
                    io::ErrorKind::Unsupported,
                    "{failure:?}"
                );
                assert_eq!(failure.error.path, folder.join(&failure.name));
                failed.push(failure.name);
            }
        }
    }
    prepared.sort();
    assert_eq!(prepared, [folder.join("first"), folder.join("last")]);
    assert_eq!(failed, ["pipe"]);
    // Retry prepares the child by its name: refused while it is a pipe, prepared once
    // it is an ordinary file.
    let pipe = OsStr::new("pipe");
    assert!(source.prepare_move_child(pipe, policy, &cancel).is_err());
    fs::remove_file(folder.join("pipe")).unwrap();
    fs::write(folder.join("pipe"), b"now a file").unwrap();
    let entry = source.prepare_move_child(pipe, policy, &cancel).unwrap();
    assert_eq!(entry.source().entry().item.path, folder.join("pipe"));
}

#[test]
fn a_move_folder_whose_entries_cannot_leave_it_fails_once_as_itself() {
    let temp = crate::test_support::temporary("move-child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    for file in ["first", "second"] {
        fs::write(folder.join(file), file).unwrap();
    }
    let cancel = Cancellation::default();
    let (mut source, policy) = prepared_move_folder(&folder);
    // Other users may change names in it, so it cannot hold an entry's private stage:
    // its entries can be listed and opened, yet the folder fails once, as itself.
    chmod(&folder, 0o777);
    let failure = source.next_move_child_or_failure(policy, &cancel).err();
    chmod(&folder, 0o755);
    let failure = failure.expect("the folder itself fails");
    assert_eq!(failure.kind, io::ErrorKind::Unsupported, "{failure:?}");
    assert_eq!(failure.path, folder);
}

#[test]
fn a_move_folder_is_checked_once_for_all_of_its_children() {
    let temp = crate::test_support::temporary("move-child-check-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    for file in ["first", "second", "third"] {
        fs::write(folder.join(file), file).unwrap();
    }
    let cancel = Cancellation::default();
    let (mut source, policy) = prepared_move_folder(&folder);
    let checks = || move_source::MOVED_FOLDER_CHECKS.get();
    let before = checks();
    let mut children = 0;
    while let Some(child) = source.next_move_child_or_failure(policy, &cancel).unwrap() {
        assert!(child.is_ok(), "{:?}", child.err());
        children += 1;
    }
    assert_eq!(children, 3);
    assert_eq!(
        checks() - before,
        1,
        "the folder is checked once, before its first child"
    );
}
