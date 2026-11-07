use midnight_core::{ArchiveMount, Cancellation};
use midnight_filesystem_host::{Catalog, Mount};
use std::{fs, path::Path};
#[cfg(target_os = "linux")]
#[path = "support/linux.rs"]
mod native;

fn install(root: &Path, worker: &Path, directory: &str) {
    let package = root.join("filesystems").join(directory);
    fs::create_dir_all(&package).unwrap();
    let filename = worker.file_name().unwrap().to_str().unwrap();
    fs::copy(worker, package.join(filename)).unwrap();
    #[cfg(target_os = "linux")]
    native::secure_test_executable(&package.join(filename));
    fs::write(package.join("plugin.toml"), format!(
        "schema_version = 1\nid = \"midnight.fs.archive\"\nkind = \"filesystem-worker\"\nversion = \"0.1.0\"\nentry_executable = \"{filename}\"\ntarget = \"{}\"\nprotocol = 1\nformats = [\"gzip\", \"zip\", \"tar\", \"tar-gzip\", \"7z\"]\n",
        midnight_plugin_host::host_target())).unwrap();
}

#[test]
fn missing_packages_leave_archive_capability_absent() {
    let root = tempfile::tempdir().unwrap();
    let catalog = Catalog::discover(root.path()).unwrap();
    assert!(catalog.choose(Path::new("file.zip")).is_none());
}

#[test]
#[ignore = "requires ARCHIVE_WORKER executable built by the independent plugin"]
fn real_sandboxed_worker_lists_reads_and_rejects_changed_source() {
    let root = tempfile::tempdir_in(
        std::env::var_os("ARCHIVE_TEST_INSTALL_ROOT").expect("private installation root"),
    )
    .unwrap();
    let worker = std::env::var_os("ARCHIVE_WORKER").unwrap();
    install(root.path(), Path::new(&worker), "archives");
    let catalog = Catalog::discover(root.path()).unwrap();
    assert_eq!(catalog.packages.len(), 1, "{:?}", catalog.diagnostics);
    let source = root.path().join("source.gz");
    fs::write(
        &source,
        [
            31, 139, 8, 0, 0, 0, 0, 0, 2, 3, 203, 72, 205, 201, 201, 7, 0, 134, 166, 16, 54, 5, 0,
            0, 0,
        ],
    )
    .unwrap();
    let (package, format) = catalog.choose(&source).unwrap();
    let cancel = Cancellation::default();
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert_eq!(
        Mount::open(package, format, &source, &cancelled)
            .unwrap_err()
            .kind,
        std::io::ErrorKind::Interrupted
    );
    let mount = Mount::open(package, format, &source, &cancel).unwrap();
    let entries = mount.list(1, &cancel).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        mount.read(entries[0].id.entry, 1, 3, &cancel).unwrap(),
        b"ell"
    );
    fs::write(&source, b"changed").unwrap();
    assert!(mount.read(entries[0].id.entry, 0, 5, &cancel).is_err());
    drop(mount);
    install(root.path(), Path::new(&worker), "duplicate");
    assert!(Catalog::discover(root.path()).unwrap().packages.is_empty());
}
