//! A child that cannot be opened is that child's failure, not its folder's: the
//! folder's enumeration continues with the next child, and Retry opens the child again
//! by its name.
use super::*;
use crate::test_fixtures::{SavedDacl, build_temp, deny_read_data, restore_dacl};
use midnight_core::TransferService;
use std::{fs, os::windows::fs::OpenOptionsExt};

/// Denies reading (a file's content, a folder's listing) to Everyone until restored.
struct Unreadable {
    path: PathBuf,
    saved: Option<SavedDacl>,
}

impl Unreadable {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            saved: Some(deny_read_data(path).unwrap()),
        }
    }

    fn restore(&mut self) {
        if let Some(saved) = self.saved.take() {
            restore_dacl(&self.path, &saved).unwrap();
        }
    }
}

impl Drop for Unreadable {
    fn drop(&mut self) {
        // The fixture folder can be removed only once it can be listed again.
        if let Some(saved) = self.saved.take() {
            let _ = restore_dacl(&self.path, &saved);
        }
    }
}

/// Every child of `source`: the paths that opened, and the names that failed with
/// their errors, both sorted.
fn children(
    source: &mut dyn TransferSource,
    cancel: &Cancellation,
) -> (Vec<PathBuf>, Vec<(OsString, FsError)>) {
    let (mut opened, mut failed) = (Vec::new(), Vec::new());
    while let Some(child) = source.next_child_or_failure(cancel).unwrap() {
        match child {
            Ok(child) => opened.push(child.entry().item.path.clone()),
            Err(failure) => failed.push((failure.name, failure.error)),
        }
    }
    opened.sort();
    failed.sort_by(|a, b| a.0.cmp(&b.0));
    (opened, failed)
}

#[test]
fn a_child_that_cannot_be_opened_fails_alone_and_can_be_opened_again_by_name() {
    let temp = build_temp("child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir_all(folder.join("locked")).unwrap();
    fs::write(folder.join("locked").join("inside.txt"), b"inside").unwrap();
    for name in ["first", "unreadable", "written", "last"] {
        fs::write(folder.join(name), name).unwrap();
    }
    let mut unreadable = [
        Unreadable::new(&folder.join("locked")),
        Unreadable::new(&folder.join("unreadable")),
    ];
    // Another program writes this file: Copy reads a file only while nobody writes it.
    let writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(folder.join("written"))
        .unwrap();
    let cancel = Cancellation::default();
    let mut source = crate::WindowsFileSystem::new(None)
        .prepare_transfer_source(&folder, &cancel)
        .unwrap();
    let (opened, failed) = children(&mut *source, &cancel);
    assert_eq!(opened, [folder.join("first"), folder.join("last")]);
    let names: Vec<_> = failed.iter().map(|(name, _)| name.clone()).collect();
    assert_eq!(names, ["locked", "unreadable", "written"]);
    for (name, error) in &failed {
        assert_eq!(error.path, folder.join(name), "{error:?}");
    }
    for (_, error) in &failed[..2] {
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied, "{error:?}");
    }
    let (_, sharing) = &failed[2];
    assert!(sharing.message.contains("(os error 32)"), "{sharing:?}");
    // Retry: once each can be read, it opens by its name.
    for denied in &mut unreadable {
        denied.restore();
    }
    drop(writer);
    for name in ["locked", "unreadable", "written"] {
        let child = source.open_child(OsStr::new(name), &cancel).unwrap();
        assert_eq!(child.entry().item.path, folder.join(name));
    }
    let mut locked = source.open_child(OsStr::new("locked"), &cancel).unwrap();
    let (inside, failed) = children(&mut *locked, &cancel);
    assert_eq!(inside, [folder.join("locked").join("inside.txt")]);
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
fn failures_of_the_folder_itself_end_its_enumeration() {
    let temp = build_temp("child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    for name in ["first", "second"] {
        fs::write(folder.join(name), name).unwrap();
    }
    let service = crate::WindowsFileSystem::new(None);
    // A file has no children to enumerate.
    let mut file = service
        .prepare_transfer_source(&folder.join("first"), &Cancellation::default())
        .unwrap();
    let failure = file
        .next_child_or_failure(&Cancellation::default())
        .err()
        .expect("a file is not a folder");
    assert_eq!(failure.kind, io::ErrorKind::InvalidInput, "{failure:?}");
    assert_eq!(failure.path, folder.join("first"));
    drop(file);
    // A cancellation after the folder's names were read is the folder's, not the next
    // child's.
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&folder, &cancel).unwrap();
    assert!(matches!(
        source.next_child_or_failure(&cancel).unwrap(),
        Some(Ok(_))
    ));
    cancel.cancel();
    let failure = source
        .next_child_or_failure(&cancel)
        .err()
        .expect("the cancelled folder fails itself");
    assert_eq!(failure.kind, io::ErrorKind::Interrupted, "{failure:?}");
    assert_eq!(failure.path, folder);
}

#[test]
fn a_move_child_that_cannot_be_prepared_fails_alone_and_can_be_prepared_again_by_name() {
    let temp = build_temp("move-child-failure-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    for name in ["first", "written", "last"] {
        fs::write(folder.join(name), name).unwrap();
    }
    // Another program writes this file, so a Move cannot hold it for its removal.
    let writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(folder.join("written"))
        .unwrap();
    let cancel = Cancellation::default();
    let policy = CopyLinkPolicy::Reject;
    let midnight_core::PreparedTransferMoveEntry::Ordinary(prepared) =
        crate::WindowsFileSystem::new(None)
            .prepare_move_source_with_policy(
                &folder,
                policy,
                TransferRevisionPolicy::AcceptChangeTimeOnly,
                &cancel,
            )
            .unwrap()
    else {
        panic!("an ordinary Move folder");
    };
    let mut source = prepared.source;
    let (mut moved, mut failed) = (Vec::new(), Vec::new());
    while let Some(child) = source.next_move_child_or_failure(policy, &cancel).unwrap() {
        match child {
            Ok(entry) => moved.push(entry.source().entry().item.path.clone()),
            Err(failure) => {
                assert!(
                    failure.error.message.contains("(os error 32)"),
                    "{failure:?}"
                );
                assert_eq!(failure.error.path, folder.join(&failure.name));
                failed.push(failure.name);
            }
        }
    }
    moved.sort();
    assert_eq!(moved, [folder.join("first"), folder.join("last")]);
    assert_eq!(failed, ["written"]);
    // Retry prepares the child by its name once nothing else holds it.
    let written = OsStr::new("written");
    assert!(source.prepare_move_child(written, policy, &cancel).is_err());
    drop(writer);
    let entry = source.prepare_move_child(written, policy, &cancel).unwrap();
    assert_eq!(entry.source().entry().item.path, folder.join("written"));
}
