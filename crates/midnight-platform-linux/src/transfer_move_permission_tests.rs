//! A Move works with the source's permissions: an owned write-protected folder
//! moves as it is, entries leave an owned write-protected folder of the moved tree,
//! which keeps its mode, the folder that contains the selection is never changed, and
//! a permission change never invalidates a prepared Move.
use super::*;
use midnight_core::{TransferMoveFinalizeOutcome, TransferNativeMoveOutcome, TransferService};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

/// The policy Copy and Move compare revisions under (VIEW-05).
const TRANSFER: TransferRevisionPolicy = TransferRevisionPolicy::AcceptChangeTimeOnly;

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Folder permissions do not bind the superuser.
fn privileged() -> bool {
    let root = rustix::process::geteuid().is_root();
    if root {
        eprintln!("SKIP: permissions do not apply to the superuser");
    }
    root
}

/// The private stages and recovery folders left in `folder`.
fn private_names(folder: &Path) -> Vec<OsString> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.as_bytes().starts_with(b".midnight-"))
        .collect()
}

/// The selected folder `path`, prepared as a Linux Move prepares it (links move
/// themselves), and its prepared children.
fn prepared_children(
    path: &Path,
) -> Result<(PreparedTransferMove, Vec<PreparedTransferMove>), FsError> {
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let policy = CopyLinkPolicy::Preserve;
    let PreparedTransferMoveEntry::Ordinary(mut root) =
        service.prepare_move_source_with_policy(path, policy, TRANSFER, &cancel)?
    else {
        panic!("{path:?} is an ordinary Move source");
    };
    let mut children = Vec::new();
    while let Some(child) = root.source.next_move_child_with_links(policy, &cancel)? {
        let PreparedTransferMoveEntry::Ordinary(child) = child else {
            panic!("an ordinary Move child");
        };
        children.push(child);
    }
    Ok((root, children))
}

/// A folder `locked` in `source` with a file, a link and a folder, the folders
/// write-protected (mode 0555).
fn write_protected_tree(source: &Path) -> PathBuf {
    let locked = source.join("locked");
    fs::create_dir_all(locked.join("folder")).unwrap();
    fs::write(locked.join("file"), b"file").unwrap();
    fs::write(locked.join("folder/inner"), b"inner").unwrap();
    symlink("file", locked.join("link")).unwrap();
    chmod(&locked.join("folder"), 0o555);
    chmod(&locked, 0o555);
    locked
}

#[test]
fn entries_leave_an_owned_write_protected_folder_of_the_moved_tree_which_keeps_its_mode() {
    if privileged() {
        return;
    }
    let Some(temp) = crate::test_support::ext_temporary("move-protected-", "protected Move") else {
        return;
    };
    let (source, destination) = (temp.path().join("source"), temp.path().join("destination"));
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let locked = write_protected_tree(&source);
    let cancel = Cancellation::default();
    let target = crate::LinuxFileSystem::new(None)
        .prepare_move_destination(&destination, &cancel)
        .unwrap();
    let mut modes = Vec::new();
    let removed = (|| -> Result<(), FsError> {
        let (root, children) = prepared_children(&locked)?;
        for child in children {
            let capture = child.capture;
            let name = capture.entry().item.path.file_name().unwrap().to_owned();
            if name == "file" {
                // How a cross-volume Move removes a file it copied and verified.
                capture.preflight_finalization(&cancel)?;
                let outcome = capture.capture(&cancel)?;
                assert!(
                    matches!(outcome, TransferMoveCaptureOutcome::Captured { .. }),
                    "{outcome:?}"
                );
                modes.push(mode(&locked));
                let mut finalizer = capture.prepare_finalization(&cancel)?;
                let outcome = finalizer.finalize(&cancel);
                assert!(
                    matches!(outcome, TransferMoveFinalizeOutcome::Removed { .. }),
                    "{outcome:?}"
                );
                drop(finalizer);
                let cleanup = capture.take_stage_cleanup().expect("the private stage");
                drop((child.source, capture));
                cleanup.cleanup()?;
            } else {
                // A link, and on one volume a folder, leaves by its own rename.
                let outcome = capture.try_native_move(&*target, &name, None, &cancel)?;
                assert!(
                    matches!(outcome, TransferNativeMoveOutcome::Moved { .. }),
                    "{outcome:?}"
                );
            }
            modes.push(mode(&locked));
        }
        // The emptied folder leaves the folder that contains it, which is writable.
        root.capture.remove_empty_directory(&cancel).map(drop)
    })();
    let moved_folder = destination.join("folder");
    let moved_mode = moved_folder.exists().then(|| mode(&moved_folder));
    // Writable again, so the fixture can be removed.
    for folder in [&locked, &moved_folder] {
        if folder.exists() {
            chmod(folder, 0o755);
        }
    }
    removed.unwrap();
    assert_eq!(modes, [0o555; 4]);
    assert!(!locked.exists());
    assert_eq!(moved_mode, Some(0o555));
    assert_eq!(fs::read(moved_folder.join("inner")).unwrap(), b"inner");
    assert_eq!(
        fs::read_link(destination.join("link")).unwrap(),
        Path::new("file")
    );
    assert!(private_names(&source).is_empty());
}

#[test]
fn a_restored_source_returns_to_its_write_protected_folder_which_keeps_its_mode() {
    if privileged() {
        return;
    }
    let temp = native_tests::fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let locked = write_protected_tree(&source);
    let cancel = Cancellation::default();
    let restored = (|| -> Result<(), FsError> {
        let (_root, children) = prepared_children(&locked)?;
        let file = children
            .into_iter()
            .find(|child| child.capture.entry().item.path == locked.join("file"))
            .expect("the file");
        // A Skip or a cancelled Move puts a captured source back.
        file.capture.capture(&cancel)?;
        file.capture.restore()?;
        let cleanup = file
            .capture
            .take_stage_cleanup()
            .expect("the private stage");
        drop(file);
        cleanup.cleanup()
    })();
    let left = (mode(&locked), private_names(&locked));
    chmod(&locked, 0o755);
    chmod(&locked.join("folder"), 0o755);
    restored.unwrap();
    assert_eq!(left, (0o555, Vec::<OsString>::new()));
    assert_eq!(fs::read(locked.join("file")).unwrap(), b"file");
}

#[test]
fn the_folder_that_contains_the_selection_is_never_made_writable() {
    if privileged() {
        return;
    }
    let temp = native_tests::fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"file").unwrap();
    chmod(&source, 0o555);
    let service = crate::LinuxFileSystem::new(None);
    let refused = service.prepare_move_source_with_policy(
        &source.join("file"),
        CopyLinkPolicy::Preserve,
        TRANSFER,
        &Cancellation::default(),
    );
    let left = mode(&source);
    chmod(&source, 0o755);
    let Err(failure) = refused else {
        panic!("a write-protected containing folder refuses the Move");
    };
    assert_eq!(failure.path, source.join("file"));
    assert!(failure.message.contains("temporary staging"), "{failure:?}");
    assert_eq!(left, 0o555);
    assert_eq!(fs::read(source.join("file")).unwrap(), b"file");
}

/// Move `name` from `source` into `destination` natively, its source pinned under
/// `policy`; `before` runs between the preparation and the Move.
fn native_move(
    source: &Path,
    destination: &Path,
    name: &str,
    policy: TransferRevisionPolicy,
    before: impl FnOnce(),
) -> Result<TransferNativeMoveOutcome, FsError> {
    let service = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let PreparedTransferMoveEntry::Ordinary(prepared) = service
        .prepare_move_source_with_policy(
            &source.join(name),
            CopyLinkPolicy::Reject,
            policy,
            &cancel,
        )
        .unwrap()
    else {
        panic!("{name} is an ordinary Move source");
    };
    let target = service
        .prepare_move_destination(destination, &cancel)
        .unwrap();
    before();
    prepared
        .capture
        .try_native_move(&*target, OsStr::new(name), None, &cancel)
}

#[test]
fn an_owned_write_protected_folder_moves_and_keeps_its_own_mode() {
    let temp = native_tests::fixture();
    let (source, destination) = (temp.path().join("source"), temp.path().join("destination"));
    fs::create_dir_all(source.join("locked/inner")).unwrap();
    fs::write(source.join("locked/inner/file"), b"inside").unwrap();
    fs::create_dir(&destination).unwrap();
    for (folder, protected) in [("locked/inner", 0o555), ("locked", 0o555)] {
        chmod(&source.join(folder), protected);
    }
    let outcome = native_move(&source, &destination, "locked", TRANSFER, || {});
    let moved = destination.join("locked");
    let modes = moved
        .exists()
        .then(|| (mode(&moved), mode(&moved.join("inner"))));
    // Writable again, so the fixture can be removed.
    for folder in [&moved, &source.join("locked")] {
        if folder.exists() {
            chmod(folder, 0o755);
            chmod(&folder.join("inner"), 0o755);
        }
    }
    let outcome = outcome.unwrap();
    assert!(
        matches!(outcome, TransferNativeMoveOutcome::Moved { .. }),
        "{outcome:?}"
    );
    assert_eq!(modes, Some((0o555, 0o555)));
    assert!(!source.join("locked").exists());
    assert_eq!(fs::read(moved.join("inner/file")).unwrap(), b"inside");
}

#[test]
fn a_permission_change_after_preparation_does_not_invalidate_the_move() {
    let temp = native_tests::fixture();
    let (source, destination) = (temp.path().join("source"), temp.path().join("destination"));
    fs::create_dir_all(source.join("folder")).unwrap();
    fs::write(source.join("file"), b"file").unwrap();
    fs::write(source.join("exact"), b"exact").unwrap();
    fs::create_dir(&destination).unwrap();
    for name in ["folder", "file"] {
        // The user fixed the source's permissions after the first attempt.
        let changed = source.join(name);
        let outcome = native_move(&source, &destination, name, TRANSFER, || {
            chmod(&changed, 0o700)
        })
        .unwrap_or_else(|failure| panic!("{name}: {failure}"));
        assert!(
            matches!(outcome, TransferNativeMoveOutcome::Moved { .. }),
            "{name}: {outcome:?}"
        );
        assert_eq!(mode(&destination.join(name)), 0o700);
    }
    // An exact pin, as Rename's capture uses, still refuses the change.
    let exact = source.join("exact");
    let failure = native_move(
        &source,
        &destination,
        "exact",
        TransferRevisionPolicy::Exact,
        || chmod(&exact, 0o600),
    )
    .unwrap_err();
    assert_eq!(failure.message, "the source changed after it was chosen");
    assert!(exact.exists() && !destination.join("exact").exists());
}

#[test]
fn a_source_changed_after_preparation_is_named_as_the_source() {
    let temp = native_tests::fixture();
    let (source, destination) = (temp.path().join("source"), temp.path().join("destination"));
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"file").unwrap();
    fs::create_dir(&destination).unwrap();
    let failure = native_move(&source, &destination, "file", TRANSFER, || {
        fs::write(source.join("file"), b"changed content").unwrap();
    })
    .unwrap_err();
    assert_eq!(failure.message, "the source changed after it was chosen");
    assert_eq!(failure.path, source.join("file"));
    assert_eq!(fs::read(source.join("file")).unwrap(), b"changed content");
    assert!(!destination.join("file").exists());
}

#[test]
fn a_moved_folder_that_cannot_get_its_mode_back_is_reported_with_the_move() {
    if privileged() {
        return;
    }
    let temp = native_tests::fixture();
    let (source, destination) = (temp.path().join("source"), temp.path().join("destination"));
    fs::create_dir_all(source.join("locked")).unwrap();
    fs::create_dir(&destination).unwrap();
    chmod(&source.join("locked"), 0o555);
    // The filesystem turns read-only before the folder gets its own mode back.
    let outcome = native_move(&source, &destination, "locked", TRANSFER, || {
        writable::FAIL_RESTORE.set(Some(rustix::io::Errno::ROFS));
    });
    let injected = writable::FAIL_RESTORE.take();
    let moved = destination.join("locked");
    let left = moved.exists().then(|| mode(&moved));
    for folder in [&moved, &source.join("locked")] {
        if folder.exists() {
            chmod(folder, 0o755);
        }
    }
    assert_eq!(injected, None, "the change gave the folder its mode back");
    // The folder moved, and the Move says that it keeps the owner write it needed.
    let outcome = outcome.unwrap();
    let TransferNativeMoveOutcome::RecoveryRequired {
        destination: Some(reached),
        recovery,
        error,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(reached, moved);
    assert!(recovery.is_empty(), "{recovery:?}");
    assert_eq!(error.path, source.join("locked"));
    assert!(
        error.message.contains("0555") && error.message.contains("keeps owner write"),
        "{error:?}"
    );
    assert_eq!(left, Some(0o755));
}

#[test]
fn a_capture_whose_folder_cannot_get_its_mode_back_keeps_the_source_for_recovery() {
    if privileged() {
        return;
    }
    let temp = native_tests::fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let locked = write_protected_tree(&source);
    let cancel = Cancellation::default();
    let restored = (|| -> Result<(), FsError> {
        let (_root, children) = prepared_children(&locked)?;
        let file = children
            .into_iter()
            .find(|child| child.capture.entry().item.path == locked.join("file"))
            .expect("the file");
        writable::FAIL_RESTORE.set(Some(rustix::io::Errno::IO));
        let outcome = file.capture.capture(&cancel)?;
        let TransferMoveCaptureOutcome::RecoveryRequired {
            recovery, error, ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(recovery.len(), 1);
        assert_eq!(error.path, locked);
        assert!(error.message.contains("keeps owner write"), "{error:?}");
        // The folder kept owner write, so the source goes back without another change.
        file.capture.restore()?;
        let cleanup = file
            .capture
            .take_stage_cleanup()
            .expect("the private stage");
        drop(file);
        cleanup.cleanup()
    })();
    writable::FAIL_RESTORE.set(None);
    let left = (mode(&locked), private_names(&locked));
    chmod(&locked, 0o755);
    chmod(&locked.join("folder"), 0o755);
    restored.unwrap();
    assert_eq!(left, (0o755, Vec::<OsString>::new()));
    assert_eq!(fs::read(locked.join("file")).unwrap(), b"file");
}

#[test]
fn a_set_group_id_folder_gets_owner_write_only_where_it_keeps_the_bit() {
    // Outside the folder's group the kernel would clear the bit, both when owner write is
    // added and when the mode is set back (`crate::ownership::keeps_set_group_id`).
    assert!(!crate::ownership::keeps_set_group_id(0o2555, 4242, |_| {
        false
    }));
    if privileged() {
        return;
    }
    // In the caller's own group the bit stays through the owner write and back.
    let temp = native_tests::fixture();
    let folder = temp.path().join("shared");
    fs::create_dir(&folder).unwrap();
    chmod(&folder, 0o2555);
    if mode(&folder) != 0o2555 {
        chmod(&folder, 0o755);
        eprintln!("SKIP: the folder's group is not the caller's, or the filesystem drops the bit");
        return;
    }
    let file = File::open(&folder).unwrap();
    let grant = writable::OwnerWrite::grant(&file, &folder).expect("an owned folder");
    let writable = mode(&folder);
    let restored = grant.restore();
    let left = mode(&folder);
    chmod(&folder, 0o755);
    restored.unwrap();
    assert_eq!((writable, left), (0o2755, 0o2555));
}
