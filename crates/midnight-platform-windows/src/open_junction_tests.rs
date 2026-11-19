//! One opt-in, read-only qualification against an OS compatibility junction, plus
//! fixture tests that create native junctions and temporarily deny listing under
//! `_build/qa`.
use super::*;
use crate::test_fixtures::{SavedDacl, deny_read_data, restore_dacl};
use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::{Component, PathBuf, Prefix},
};

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

/// Denies listing to Everyone through a native DACL edit of the entry itself (a
/// junction is changed itself, never its target; no icacls process) and restores the
/// original DACL on drop.
struct DeniedListing(PathBuf, SavedDacl);
impl DeniedListing {
    fn new(path: &Path) -> Self {
        Self(path.to_owned(), deny_read_data(path).unwrap())
    }
}
impl Drop for DeniedListing {
    fn drop(&mut self) {
        if let Err(error) = restore_dacl(&self.0, &self.1)
            && !std::thread::panicking()
        {
            panic!(
                "cannot restore fixture ACL at {}: {error}",
                self.0.display()
            );
        }
    }
}

fn fixture_directory() -> tempfile::TempDir {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/junction-navigation/fixtures");
    fs::create_dir_all(&fixtures).unwrap();
    let root = fixtures.canonicalize().unwrap();
    // Navigation results are compared with the ordinary (non-verbatim) spelling that
    // panels use, so keep the canonical root's raw UTF-16 without the `\\?\` prefix.
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
fn tooltip_reads_the_stored_target_of_a_dangling_junction() {
    let temp = fixture_directory();
    let target = temp.path().join("target_文档 & 🦀");
    let alias = temp.path().join("alias_e\u{301}");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&alias).unwrap();
    crate::test_fixtures::create_junction(&alias, &target).unwrap();
    fs::remove_dir(&target).unwrap();

    let shown = WindowsFileSystem::new(None)
        .read_link_target(&alias)
        .unwrap()
        .unwrap();
    let mut verbatim = OsString::from(r"\\?\");
    verbatim.push(&target);
    assert!(
        shown == target || shown == verbatim,
        "unexpected junction target {shown:?}"
    );
}

#[test]
fn denied_junction_alias_keeps_target_acl_authority_and_never_launches() {
    let temp = fixture_directory();
    let target = temp.path().join("target_文档 & 🦀");
    let alias = temp.path().join("alias_e\u{301}");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("preserved.txt"), b"unchanged").unwrap();
    fs::create_dir(&alias).unwrap();
    crate::test_fixtures::create_junction(&alias, &target).unwrap();
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
