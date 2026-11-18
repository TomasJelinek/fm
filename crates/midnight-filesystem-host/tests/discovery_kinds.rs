//! Filesystem workers and native filesystem packages share an installation root.
use midnight_core::Cancellation;
use midnight_filesystem_host::{Catalog, Mount};
use std::path::Path;

/// Grant another account write access to one package object.
fn make_shared(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).unwrap();
    }
    #[cfg(windows)]
    {
        // S-1-1-0 is Everyone; icacls applies one explicit Modify ACE directly. Mounts
        // in this binary launch workers, so the probe spawns under their creation lock
        // and is waited for only after that lock is released.
        let status = midnight_worker_platform::spawn_command(
            std::process::Command::new("icacls")
                .arg(path)
                .args(["/grant", "*S-1-1-0:(M)", "/q"])
                .stdout(std::process::Stdio::null()),
        )
        .unwrap()
        .wait()
        .unwrap();
        assert!(status.success());
    }
}

fn install_worker(root: &Path, directory: &str, id: &str) {
    let package = root.join("filesystems").join(directory);
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("plugin.toml"),
        format!(
            "schema_version = 1\nid = \"{id}\"\nkind = \"filesystem-worker\"\n\
             version = \"1\"\nentry_executable = \"worker.exe\"\ntarget = \"{}\"\n\
             protocol = 3\nformats = [\"zip\"]\n",
            midnight_plugin_host::host_target()
        ),
    )
    .unwrap();
    std::fs::write(package.join("worker.exe"), b"never launched").unwrap();
}

#[test]
#[ignore = "requires a protected HOST_TEST_INSTALL_ROOT"]
fn default_worker_for_a_format_is_chosen_by_id_not_enumeration_order() {
    let parent = std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("protected test installation");
    let root = tempfile::tempdir_in(parent).unwrap();
    // Directory names sort opposite to the IDs they contain.
    install_worker(root.path(), "a-directory", "z.worker");
    install_worker(root.path(), "b-directory", "a.worker");
    let catalog = Catalog::discover(root.path()).unwrap();
    let ids: Vec<_> = catalog.packages.iter().map(|package| &package.id).collect();
    assert_eq!(ids, ["a.worker", "z.worker"], "{:?}", catalog.diagnostics);
    assert_eq!(
        catalog.choose(Path::new("archive.zip")).unwrap().0.id,
        "a.worker"
    );
}

#[test]
#[ignore = "requires a protected HOST_TEST_INSTALL_ROOT"]
fn archive_worker_package_objects_are_validated_at_discovery_and_before_launch() {
    let parent = std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("protected test installation");
    let root = tempfile::tempdir_in(parent).unwrap();
    install_worker(root.path(), "archive", "archive.fixture");
    let dependency = root.path().join("filesystems/archive/codec.dll");
    std::fs::write(&dependency, b"dependency the OS loader may resolve").unwrap();
    let catalog = Catalog::discover(root.path()).unwrap();
    assert_eq!(catalog.packages.len(), 1, "{:?}", catalog.diagnostics);
    // A later replaceable dependency is refused before any package code runs.
    make_shared(&dependency);
    let archive = root.path().join("archive.zip");
    std::fs::write(&archive, b"PK").unwrap();
    let source = midnight_plugin_host::CapturedFile::capture(&archive).unwrap();
    let (worker, format) = catalog.choose(&archive).unwrap();
    let error = Mount::open_captured(worker, format, &archive, source, &Cancellation::default())
        .unwrap_err();
    assert!(error.message.contains("writable"), "{error}");
    let catalog = Catalog::discover(root.path()).unwrap();
    assert!(catalog.packages.is_empty());
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("writable")),
        "{:?}",
        catalog.diagnostics
    );
}

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
