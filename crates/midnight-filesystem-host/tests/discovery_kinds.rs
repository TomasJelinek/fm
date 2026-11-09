//! Filesystem workers and native filesystem packages share an installation root.
use midnight_filesystem_host::Catalog;

#[test]
#[ignore = "requires a protected HOST_TEST_INSTALL_ROOT"]
fn native_packages_are_left_to_native_host_and_overflow_never_selects_partial_catalog() {
    let parent = std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("protected test installation");
    let root = tempfile::tempdir_in(parent).unwrap();
    let package = root.path().join("filesystems/native");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("plugin.toml"), "kind = 'filesystem-native'\n").unwrap();
    let catalog = Catalog::discover(root.path()).unwrap();
    assert!(catalog.packages.is_empty());
    assert!(
        catalog.diagnostics.is_empty(),
        "Native package validation belongs to its host: {:?}",
        catalog.diagnostics
    );
    for number in 0..64 {
        let package = root
            .path()
            .join("filesystems")
            .join(format!("other-{number}"));
        std::fs::create_dir(&package).unwrap();
        std::fs::write(package.join("plugin.toml"), "kind = 'filesystem-native'\n").unwrap();
    }
    assert!(
        Catalog::discover(root.path()).is_err(),
        "Skipped native packages still count toward the shared discovery budget"
    );
}
