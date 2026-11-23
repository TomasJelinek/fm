//! Exercise the real Windows ACL boundary, including mutation between package validations.
use super::*;

fn fixture() -> tempfile::TempDir {
    let base = std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("secure HOST_TEST_INSTALL_ROOT");
    let root = tempfile::tempdir_in(base).unwrap();
    protect_fixture(root.path()).unwrap();
    root
}

/// Shared ACLs no longer refuse a package; each validation still reads every object's
/// current owner and ACL, so a later inspection reports the change.
#[test]
fn another_package_validation_rereads_changed_file_or_ancestor_permissions() {
    let root = fixture();
    let directory = root.path().join("package");
    std::fs::create_dir(&directory).unwrap();
    let file = directory.join("helper.dll");
    std::fs::write(&file, b"fixture").unwrap();
    let directory = directory.canonicalize().unwrap();
    crate::validate_plugin_package(&directory).unwrap();
    fixture_acl(&file, "D:P(A;;FA;;;WD)").unwrap();
    crate::validate_plugin_package(&directory).unwrap();
    assert_eq!(
        crate::installation_sharing(&directory).unwrap().shared,
        [file.canonicalize().unwrap()]
    );
    protect_fixture(&file).unwrap();
    crate::validate_plugin_package(&directory).unwrap();
    fixture_acl(root.path(), "D:P(A;OICI;FA;;;WD)").unwrap();
    let changed = crate::validate_plugin_package(&directory);
    let sharing = crate::installation_sharing(&directory).unwrap();
    protect_fixture(root.path()).unwrap();
    changed.unwrap();
    assert_eq!(
        sharing.shared_ancestor,
        Some(root.path().canonicalize().unwrap()),
        "{sharing:?}"
    );
    assert!(
        crate::installation_sharing(&directory)
            .unwrap()
            .is_private()
    );

    // Objects in a nested folder are checked on their own after their folder, which is
    // checked as an object of the package: a writable notice or folder is still refused.
    let notices = directory.join("notices");
    std::fs::create_dir(&notices).unwrap();
    let notice = notices.join("LICENSE.txt");
    std::fs::write(&notice, b"fixture").unwrap();
    crate::validate_plugin_package(&directory).unwrap();
    for path in [&notice, &notices] {
        fixture_acl(path, "D:P(A;;FA;;;WD)").unwrap();
        let changed = crate::validate_plugin_package(&directory);
        let sharing = crate::installation_sharing(&directory).unwrap();
        protect_fixture(path).unwrap();
        changed.unwrap();
        assert!(sharing.shared.contains(path), "{sharing:?}");
    }
    crate::validate_plugin_package(&directory).unwrap();
}
