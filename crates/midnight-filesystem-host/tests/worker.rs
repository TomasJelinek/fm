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
        "schema_version = 1\nid = \"midnight.fs.archive\"\nkind = \"filesystem-worker\"\nversion = \"0.1.0\"\nentry_executable = \"{filename}\"\ntarget = \"{}\"\nprotocol = 2\nstable_entry_ids = true\nformats = [\"gzip\", \"zip\", \"tar\", \"tar-gzip\", \"7z\"]\n",
        midnight_plugin_host::host_target())).unwrap();
}

#[test]
fn missing_packages_leave_archive_capability_absent() {
    let root = tempfile::tempdir().unwrap();
    let catalog = Catalog::discover(root.path()).unwrap();
    assert!(catalog.choose(Path::new("file.zip")).is_none());
}

fn archive_fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.gz")
}

#[test]
fn captured_archive_source_rejects_mutation() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.gz");
    fs::copy(archive_fixture(), &source).unwrap();
    let captured = midnight_plugin_host::CapturedFile::capture(&source).unwrap();
    fs::write(&source, b"changed").unwrap();
    assert!(captured.revalidate().is_err());
    assert!(captured.read_at(0, 5).is_err());
}

#[test]
#[ignore = "requires ARCHIVE_WORKER executable built by the independent plugin"]
fn real_sandboxed_worker_lists_reads_and_rejects_cancellation() {
    let root = tempfile::tempdir_in(
        std::env::var_os("ARCHIVE_TEST_INSTALL_ROOT").expect("private installation root"),
    )
    .unwrap();
    let worker = std::env::var_os("ARCHIVE_WORKER").unwrap();
    install(root.path(), Path::new(&worker), "archives");
    let catalog = Catalog::discover(root.path()).unwrap();
    assert_eq!(catalog.packages.len(), 1, "{:?}", catalog.diagnostics);
    // Use settled input: endpoint security can asynchronously update fresh test
    // files' metadata. The separate test above covers changed native revisions.
    let source = archive_fixture();
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
    let revision = mount.address().source_version;
    let identity = mount
        .persistent_member_identity(entries[0].id.entry, &revision)
        .unwrap()
        .unwrap();
    assert!(
        mount
            .persistent_member_identity(entries[0].id.entry, b"stale")
            .is_err()
    );
    let reopened = Mount::open(package, format, &source, &cancel).unwrap();
    assert_ne!(mount.id(), reopened.id());
    assert_eq!(
        reopened
            .persistent_member_identity(entries[0].id.entry, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
    assert!(reopened.persistent_member_identity(0, &revision).is_err());
    drop(reopened);
    drop(mount);
    install(root.path(), Path::new(&worker), "duplicate");
    assert!(Catalog::discover(root.path()).unwrap().packages.is_empty());
}
