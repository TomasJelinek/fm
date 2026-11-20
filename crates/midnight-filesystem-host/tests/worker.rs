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
        "schema_version = 1\nid = \"midnight.fs.archive\"\nkind = \"filesystem-worker\"\nversion = \"0.1.0\"\nentry_executable = \"{filename}\"\ntarget = \"{}\"\nprotocol = 3\nstable_entry_ids = true\nformats = [\"gzip\", \"zip\", \"tar\", \"tar-gzip\", \"7z\"]\n",
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

fn open(
    package: &midnight_filesystem_host::Package,
    format: midnight_filesystem_api::Format,
    source: &Path,
    cancel: &Cancellation,
) -> Result<std::sync::Arc<Mount>, midnight_core::FsError> {
    let captured = midnight_plugin_host::CapturedFile::capture(source).unwrap();
    Mount::open_captured(package, format, source, captured, cancel)
}

#[test]
#[ignore = "requires ARCHIVE_WORKER executable built by the independent plugin"]
fn real_sandboxed_worker_lists_reads_and_refuses_a_cancelled_open() {
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
        open(package, format, &source, &cancelled).unwrap_err().kind,
        std::io::ErrorKind::Interrupted
    );
    let mount = open(package, format, &source, &cancel).unwrap();
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
    let reopened = open(package, format, &source, &cancel).unwrap();
    assert_ne!(mount.id(), reopened.id());
    assert_eq!(
        reopened
            .persistent_member_identity(entries[0].id.entry, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
    assert!(reopened.persistent_member_identity(0, &revision).is_err());
    reopened.release_on_worker().unwrap();
    mount.release_on_worker().unwrap();
    install(root.path(), Path::new(&worker), "duplicate");
    assert!(Catalog::discover(root.path()).unwrap().packages.is_empty());
}

/// Archives far beyond the former 100,000-member limits open and list completely
/// in the sandboxed worker under its memory policy, and Stop still ends an opening
/// promptly. `MIDNIGHT_ARCHIVE_LARGE` lists archives that the archive plugin's
/// `measure_index_memory` generated and kept (`MIDNIGHT_ARCHIVE_MEMORY_KEEP`), with
/// `MIDNIGHT_ARCHIVE_LARGE_MEMBERS` members each, 1,000 to a folder.
#[test]
#[ignore = "requires ARCHIVE_WORKER and generated archives in MIDNIGHT_ARCHIVE_LARGE"]
fn real_sandboxed_worker_lists_archives_of_millions_of_members() {
    use midnight_core::EntryKind;
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir_in(
        std::env::var_os("ARCHIVE_TEST_INSTALL_ROOT").expect("private installation root"),
    )
    .unwrap();
    let worker = std::env::var_os("ARCHIVE_WORKER").unwrap();
    install(root.path(), Path::new(&worker), "archives");
    let catalog = Catalog::discover(root.path()).unwrap();
    let members: usize = std::env::var("MIDNIGHT_ARCHIVE_LARGE_MEMBERS")
        .expect("member count")
        .parse()
        .unwrap();
    let cancel = Cancellation::default();
    for source in std::env::split_paths(&std::env::var_os("MIDNIGHT_ARCHIVE_LARGE").unwrap()) {
        let name = source.display().to_string();
        let (package, format) = catalog.choose(&source).unwrap();
        let start = Instant::now();
        let mount = open(package, format, &source, &cancel)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let opened = start.elapsed();
        let start = Instant::now();
        let (mut files, mut folders, mut pending) = (0, 0, vec![1]);
        while let Some(directory) = pending.pop() {
            for node in mount.list(directory, &cancel).unwrap() {
                if node.kind == EntryKind::Directory {
                    folders += 1;
                    pending.push(node.id.entry);
                } else {
                    files += 1;
                }
            }
        }
        let listed = start.elapsed();
        assert_eq!(
            (files, folders),
            (members, members.div_ceil(1000)),
            "{name}"
        );
        mount.release_on_worker().unwrap();
        // Stop 100 ms into an opening, while the worker indexes the archive.
        let stop = Cancellation::default();
        let stopper = stop.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            stopper.cancel();
            Instant::now()
        });
        let outcome = open(package, format, &source, &stop);
        let returned = Instant::now();
        let stopped = canceller.join().unwrap();
        let stop = match outcome {
            Err(error) => {
                assert_eq!(error.kind, std::io::ErrorKind::Interrupted, "{name}");
                format!("Stop ended the opening after {:.2?}", returned - stopped)
            }
            Ok(mount) => {
                mount.release_on_worker().unwrap();
                "opened before Stop".into()
            }
        };
        eprintln!("{name}: opened in {opened:.2?}, listed in {listed:.2?}; {stop}");
    }
}
