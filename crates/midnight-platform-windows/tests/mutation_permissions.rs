#![cfg(windows)]

use midnight_core::{Cancellation, DeleteMode, MutationService};
use midnight_platform_windows::WindowsFileSystem;
use std::{
    ffi::OsStr,
    fs, io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("midnight-mutation-acl-")
        .tempdir_in(root)
        .unwrap()
}

struct DeniedRead {
    paths: Vec<PathBuf>,
    original: String,
}

impl DeniedRead {
    fn new(path: &Path) -> Self {
        Self {
            paths: vec![path.to_owned()],
            original: acl_script("Deny", path, None).unwrap().trim().to_owned(),
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
                if let Err(error) = acl_script("Restore", path, Some(&self.original)) {
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

fn acl_script(action: &str, path: &Path, original: Option<&str>) -> io::Result<String> {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-read-acl.ps1"))
        .args(["-Action", action])
        .arg("-FixturePath")
        .arg(path)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    if let Some(original) = original {
        command.arg("-OriginalDacl").arg(original);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
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
    assert_eq!(
        prepared
            .rename(OsStr::new("renamed.txt"), &Cancellation::default())
            .unwrap(),
        destination
    );
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
    prepared
        .rename(OsStr::new("renamed"), &Cancellation::default())
        .unwrap();
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
        prepared
            .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .unwrap();
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
