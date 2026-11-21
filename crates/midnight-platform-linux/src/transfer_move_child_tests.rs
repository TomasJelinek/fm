//! A folder of the moved tree is checked once, before its entries: what every entry
//! would fail alike is the folder's own failure, which names the folder, and a child
//! that cannot be moved fails alone, so the folder goes on with its next entry.
use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

/// The policy Copy and Move compare revisions under (VIEW-05).
const TRANSFER: TransferRevisionPolicy = TransferRevisionPolicy::AcceptChangeTimeOnly;

/// The child `name` of `folder`, prepared as a Linux Move prepares it (links move
/// themselves): `Err` for the folder's own failure, `Ok(Err)` for the child's.
fn child(folder: &Path, name: &str) -> Result<Result<PreparedTransferMoveEntry, FsError>, FsError> {
    prepare_child_or_failure(
        Arc::new(File::open(folder).unwrap()),
        folder.to_owned(),
        OsStr::new(name),
        None,
        directory_links::Traversal::for_policy(CopyLinkPolicy::Preserve),
        TRANSFER,
        &Cancellation::default(),
    )
}

#[test]
fn a_child_that_cannot_be_moved_fails_alone() {
    let temp = native_tests::fixture();
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file"), b"file").unwrap();
    rustix::fs::mkfifoat(
        CWD,
        folder.join("pipe"),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let Ok(Err(failure)) = child(&folder, "pipe") else {
        panic!("a pipe is a failure of its own");
    };
    assert_eq!(failure.path, folder.join("pipe"));
    assert_eq!(failure.kind, io::ErrorKind::Unsupported, "{failure:?}");
    assert!(matches!(child(&folder, "file"), Ok(Ok(_))));
}

#[test]
fn a_folder_whose_entries_cannot_leave_it_fails_once_naming_the_folder() {
    if rustix::process::geteuid().is_root() {
        eprintln!("SKIP: permissions do not apply to the superuser");
        return;
    }
    let temp = native_tests::fixture();
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file"), b"file").unwrap();
    // Its names can be read, but no entry can be opened, so none can leave it.
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o600)).unwrap();
    let refused = child(&folder, "file");
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    let Err(failure) = refused else {
        panic!("the folder's own failure");
    };
    assert_eq!(failure.path, folder);
    assert_eq!(failure.kind, io::ErrorKind::PermissionDenied, "{failure:?}");
    assert!(failure.message.contains("search permission"), "{failure:?}");
}

#[test]
fn an_owned_write_protected_folder_lets_its_entries_be_prepared() {
    if rustix::process::geteuid().is_root() {
        eprintln!("SKIP: permissions do not apply to the superuser");
        return;
    }
    let temp = native_tests::fixture();
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("file"), b"file").unwrap();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
    let prepared = child(&folder, "file");
    let mode = fs::metadata(&folder).unwrap().permissions().mode() & 0o7777;
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(prepared, Ok(Ok(_))));
    // Preparing changes nothing: owner write comes only with each change that needs it.
    assert_eq!(mode, 0o555);
}
