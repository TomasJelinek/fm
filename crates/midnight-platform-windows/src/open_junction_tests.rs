//! Opt-in, read-only qualification against an OS compatibility junction.
use super::*;
use std::{
    ffi::OsString,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::{Component, PathBuf, Prefix},
    process::Command,
};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

#[test]
fn denied_compatibility_junction_explicitly_opens_its_native_target() {
    let Some(path) = std::env::var_os("MIDNIGHT_TEST_COMPATIBILITY_JUNCTION") else {
        return;
    };
    let path = PathBuf::from(path);
    assert!(fs::symlink_metadata(&path).unwrap().is_symlink());
    assert_eq!(
        fs::read_dir(&path).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let target = fs::read_link(&path).unwrap();
    assert!(fs::metadata(&target).unwrap().is_dir());
    let mut expected: Vec<_> = fs::read_dir(&target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let outcome = open_with(&path, |_| panic!("junction must never use an association")).unwrap();
    let destination = match outcome {
        OpenEntryResult::DirectoryTarget { path } => path,
        _ => panic!("expected an internal directory destination"),
    };
    assert_eq!(
        destination, target,
        "denied junction must navigate to its OS target"
    );
    assert_eq!(
        WindowsFileSystem::new(None)
            .directory_destination(&path)
            .unwrap(),
        target
    );
    let mut actual: Vec<_> = fs::read_dir(destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

struct DeniedListing(PathBuf);
impl DeniedListing {
    fn new(path: &Path) -> Self {
        Self::acl(path, "/deny", "*S-1-1-0:(RD)");
        Self(path.to_owned())
    }

    fn acl(path: &Path, operation: &str, permission: &str) {
        let executable = crate::desktop::system_directory()
            .unwrap()
            .join("icacls.exe");
        let output = Command::new(executable)
            .arg(path)
            .args([operation, permission, "/L"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(output.status.success(), "icacls: {output:?}");
    }
}
impl Drop for DeniedListing {
    fn drop(&mut self) {
        Self::acl(&self.0, "/remove:d", "*S-1-1-0");
    }
}

fn fixture_directory() -> tempfile::TempDir {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/junction-navigation/fixtures");
    fs::create_dir_all(&fixtures).unwrap();
    let root = fixtures.canonicalize().unwrap();
    // PowerShell's junction constructor misinterprets a verbatim target. Keep
    // its ordinary raw UTF-16 spelling, matching the existing junction fixtures.
    let root = if matches!(root.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_)))
    {
        let units: Vec<_> = root.as_os_str().encode_wide().collect();
        PathBuf::from(OsString::from_wide(&units[4..]))
    } else {
        root
    };
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn denied_junction_alias_keeps_target_acl_authority_and_never_launches() {
    let temp = fixture_directory();
    let target = temp.path().join("target_文档 & 🦀");
    let alias = temp.path().join("alias_e\u{301}");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("preserved.txt"), b"unchanged").unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../midnight-platform/tests/create-junction.ps1");
    let output = Command::new(crate::desktop::powershell_path().unwrap())
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-JunctionPath")
        .arg(&alias)
        .arg("-TargetPath")
        .arg(&target)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(output.status.success(), "junction fixture: {output:?}");
    let denied_alias = DeniedListing::new(&alias);
    assert_eq!(
        fs::read_dir(&alias).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(
        fs::read_dir(&target).is_ok(),
        "denying alias must not deny target"
    );
    assert!(matches!(
        open_with(&alias, |_| panic!("must not launch")),
        Ok(OpenEntryResult::DirectoryTarget { path }) if path == target
    ));
    let filesystem = WindowsFileSystem::new(None);
    assert_eq!(filesystem.directory_destination(&alias).unwrap(), target);
    {
        let _denied_target = DeniedListing::new(&target);
        assert_eq!(
            fs::read_dir(&target).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let error = open_with(&alias, |_| panic!("must not launch")).unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
        assert_eq!(error.path, alias);
        assert_eq!(
            filesystem.directory_destination(&alias).unwrap_err().kind,
            io::ErrorKind::PermissionDenied
        );
    }
    drop(denied_alias);
    assert!(matches!(
        open_with(&alias, |_| panic!("must not launch")),
        Ok(OpenEntryResult::Directory)
    ));
    fs::remove_file(target.join("preserved.txt")).unwrap();
    fs::remove_dir(&target).unwrap();
    assert!(open_with(&alias, |_| panic!("missing target must not launch")).is_err());
    fs::remove_dir(&alias).unwrap();
    fs::write(&alias, b"replacement is never opened").unwrap();
    assert_eq!(
        filesystem.directory_destination(&alias).unwrap_err().kind,
        io::ErrorKind::Unsupported
    );
}

#[test]
fn directory_only_activation_rejects_a_regular_file_without_opening_it() {
    let temp = fixture_directory();
    let path = temp.path().join("replaced_folder.exe");
    fs::write(&path, b"never execute").unwrap();
    let error = WindowsFileSystem::new(None)
        .directory_destination(&path)
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert_eq!(error.path, path);
    assert_eq!(fs::read(path).unwrap(), b"never execute");
}
