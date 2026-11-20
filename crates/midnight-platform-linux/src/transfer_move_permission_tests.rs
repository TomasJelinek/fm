//! A Move works with the source's permissions: an owned write-protected folder
//! moves as it is, and a permission change never invalidates a prepared Move.
use super::*;
use midnight_core::{TransferNativeMoveOutcome, TransferService};
use std::{fs, os::unix::fs::PermissionsExt};

/// The policy Copy and Move compare revisions under (VIEW-05).
const TRANSFER: TransferRevisionPolicy = TransferRevisionPolicy::AcceptChangeTimeOnly;

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
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
