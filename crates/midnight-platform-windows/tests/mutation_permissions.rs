#![cfg(windows)]
#[path = "../src/test_fixtures.rs"]
mod test_fixtures;

use midnight_core::{Cancellation, DeleteMode, MutationService};
use midnight_platform_windows::WindowsFileSystem;
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};
use test_fixtures::{SavedDacl, deny_read_data, restore_dacl};

fn fixture() -> tempfile::TempDir {
    test_fixtures::build_temp("midnight-mutation-acl-")
}

/// Denies content read and directory listing to Everyone through a native DACL edit
/// (no PowerShell), restoring the exact original DACL on drop.
struct DeniedRead {
    paths: Vec<PathBuf>,
    original: SavedDacl,
}

impl DeniedRead {
    fn new(path: &Path) -> Self {
        Self {
            paths: vec![path.to_owned()],
            original: deny_read_data(path).unwrap(),
        }
    }

    fn renamed_to(&mut self, path: &Path) {
        self.paths.push(path.to_owned());
    }
}

impl Drop for DeniedRead {
    fn drop(&mut self) {
        for path in &self.paths {
            if path.exists() {
                // Restore the exact original DACL even when the tested operation panics.
                // Removed fixtures have no remaining namespace entry to restore.
                if let Err(error) = restore_dacl(path, &self.original) {
                    eprintln!(
                        "cannot restore owned fixture ACL at {}: {error}",
                        path.display()
                    );
                    if !std::thread::panicking() {
                        panic!("fixture ACL cleanup failed");
                    }
                }
            }
        }
    }
}

#[test]
fn rename_file_with_delete_permission_does_not_require_content_read() {
    let temp = fixture();
    let source = temp.path().join("unreadable.txt");
    let destination = temp.path().join("renamed.txt");
    fs::write(&source, b"preserved without reading").unwrap();
    let mut acl = DeniedRead::new(&source);
    acl.renamed_to(&destination);
    assert_eq!(
        fs::read(&source).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let prepared = WindowsFileSystem::new(None)
        .prepare(&source, &Cancellation::default())
        .unwrap();
    let completion = prepared
        .rename(OsStr::new("renamed.txt"), &Cancellation::default())
        .unwrap();
    assert!(completion.residue.is_none());
    assert_eq!(completion.destination, destination);
    drop(acl);
    assert_eq!(fs::read(destination).unwrap(), b"preserved without reading");
}

#[test]
fn rename_directory_with_delete_permission_does_not_require_listing() {
    let temp = fixture();
    let source = temp.path().join("unlistable");
    let destination = temp.path().join("renamed");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("sentinel"), b"preserved").unwrap();
    let mut acl = DeniedRead::new(&source);
    acl.renamed_to(&destination);
    assert_eq!(
        fs::read_dir(&source).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let prepared = WindowsFileSystem::new(None)
        .prepare(&source, &Cancellation::default())
        .unwrap();
    let completion = prepared
        .rename(OsStr::new("renamed"), &Cancellation::default())
        .unwrap();
    assert!(completion.residue.is_none());
    drop(acl);
    assert_eq!(
        fs::read(destination.join("sentinel")).unwrap(),
        b"preserved"
    );
}

#[test]
fn native_move_does_not_require_file_content_or_directory_listing_permissions() {
    use midnight_core::{TransferNativeMoveOutcome, TransferService};
    for directory in [false, true] {
        let temp = fixture();
        let source = temp.path().join("source");
        if directory {
            fs::create_dir(&source).unwrap();
            fs::write(source.join("sentinel"), b"retained").unwrap();
        } else {
            fs::write(&source, b"retained").unwrap();
        }
        let target_parent = temp.path().join("target");
        fs::create_dir(&target_parent).unwrap();
        let target = target_parent.join("moved");
        let mut acl = DeniedRead::new(&source);
        acl.renamed_to(&target);
        let denied = if directory {
            fs::read_dir(&source).map(|_| ())
        } else {
            fs::read(&source).map(|_| ())
        };
        assert_eq!(denied.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let cancel = Cancellation::default();
        let filesystem = WindowsFileSystem::new(None);
        let prepared = filesystem
            .prepare_move_source(&source, &cancel)
            .expect("rename preparation must not request content/list access");
        let destination = filesystem
            .prepare_move_destination(&target_parent, &cancel)
            .unwrap();
        assert!(matches!(
            prepared
                .capture
                .try_native_move(destination.as_ref(), OsStr::new("moved"), None, &cancel)
                .unwrap(),
            TransferNativeMoveOutcome::Moved { .. }
        ));
        assert!(!source.exists());
        drop(prepared);
        drop(destination);
        drop(acl);
        assert_eq!(
            fs::read(if directory {
                target.join("sentinel")
            } else {
                target
            })
            .unwrap(),
            b"retained"
        );
    }
}

#[test]
fn permanent_leaf_and_recursive_delete_do_not_require_file_content_read() {
    for recursive in [false, true] {
        let temp = fixture();
        let tree = temp.path().join("tree");
        fs::create_dir(&tree).unwrap();
        let source = tree.join("unreadable.txt");
        fs::write(&source, b"owned deletion fixture").unwrap();
        let acl = DeniedRead::new(&source);
        assert_eq!(
            fs::read(&source).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let target = if recursive { &tree } else { &source };
        let prepared = WindowsFileSystem::new(None)
            .prepare(target, &Cancellation::default())
            .unwrap();
        let completion = prepared
            .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .unwrap();
        assert!(completion.residue.is_none());
        drop(acl);
        assert!(!target.exists());
    }
}

#[test]
fn recursive_delete_reports_missing_directory_list_permission_without_removing_contents() {
    let temp = fixture();
    let source = temp.path().join("unlistable");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("sentinel"), b"preserved").unwrap();
    let acl = DeniedRead::new(&source);
    assert_eq!(
        fs::read_dir(&source).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let prepared = WindowsFileSystem::new(None)
        .prepare(&source, &Cancellation::default())
        .unwrap();
    assert_eq!(
        prepared
            .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .unwrap_err()
            .kind,
        io::ErrorKind::PermissionDenied
    );
    drop(acl);
    assert_eq!(fs::read(source.join("sentinel")).unwrap(), b"preserved");
}
