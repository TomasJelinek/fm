//! Actual independently built module, not a statically linked provider test double.
use midnight_core::{Cancellation, DirectoryService, EntryKind, StorageService, VolumeService};
use midnight_native_filesystem_host::{Catalog, NativeProvider};
use std::path::{Path, PathBuf};

fn installation() -> PathBuf {
    PathBuf::from(
        std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION")
            .expect("Set protected native test installation"),
    )
    .canonicalize()
    .unwrap()
}

#[test]
#[ignore = "requires a protected independent native plugin installation"]
fn dynamic_provider_browses_reads_and_rejects_changed_source_without_fallback() {
    let catalog = Catalog::discover(&installation()).unwrap();
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    let host = NativeProvider::open(catalog.native_package().unwrap()).unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("žluťoučký 文档.txt");
    std::fs::write(&source, b"original source\0\xff").unwrap();
    let mut entries = Vec::new();
    host.read_directory(fixture.path(), &|| false, &mut |entry| {
        entries.push(entry);
        true
    })
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, source);
    assert_eq!(entries[0].kind, EntryKind::File);
    assert_eq!(host.entry_metadata(&source).unwrap().size, Some(17));
    let captured = host.capture(&source, &|| false).unwrap();
    assert_eq!(captured.read_at(0, 99).unwrap(), b"original source\0\xff");
    assert_eq!(captured.read_at(99, 1).unwrap(), b"");
    assert!(
        captured.path().as_os_str().is_empty(),
        "Content travels through the provider"
    );
    std::fs::rename(&source, fixture.path().join("old")).unwrap();
    std::fs::write(&source, b"replacement different length").unwrap();
    assert!(captured.revalidate().is_err());
    drop(captured);
    let batch_root = fixture.path().join("batches");
    std::fs::create_dir(&batch_root).unwrap();
    for number in 0..260 {
        std::fs::write(batch_root.join(format!("entry-{number}.txt")), b"entry").unwrap();
    }
    let mut count = 0;
    host.read_directory(&batch_root, &|| false, &mut |_| {
        count += 1;
        true
    })
    .unwrap();
    assert_eq!(
        count, 260,
        "All three directory pages must be consumed exactly once"
    );
    // The native module has only 64 mounts/readers: repeated canceled/early-stop
    // operations prove cleanup rather than succeeding by consuming a finite pool.
    for _ in 0..80 {
        host.read_directory(fixture.path(), &|| false, &mut |_| false)
            .unwrap();
        let captured = host.capture(&source, &|| false).unwrap();
        assert_eq!(captured.length(), 28);
    }
    // Capturing a panel order must not exhaust the plugin's 64-reader limit.
    let cache = midnight_native_filesystem_host::CaptureCache::new(host.clone());
    let snapshots: Vec<_> = (0..80)
        .map(|_| cache.capture(&source, &|| false).unwrap())
        .collect();
    for captured in &snapshots {
        assert_eq!(captured.read_at(0, 11).unwrap(), b"replacement");
    }
    std::fs::rename(&source, fixture.path().join("replacement-old")).unwrap();
    std::fs::write(&source, b"replacement final").unwrap();
    assert!(
        snapshots[0].read_at(0, 11).is_err(),
        "An evicted reader cannot silently capture replacement bytes"
    );
    drop(snapshots);
    std::thread::scope(|scope| {
        for _ in 0..12 {
            let host = &host;
            let source = &source;
            scope.spawn(move || {
                for _ in 0..10 {
                    let captured = host.capture(source, &|| false).unwrap();
                    assert_eq!(captured.read_at(0, 128).unwrap(), b"replacement final");
                }
            });
        }
    });
    // Cleanup admission cannot leak handles under concurrent call pressure.
    for _ in 0..80 {
        drop(host.capture(&source, &|| false).unwrap());
    }
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(
        DirectoryService::read_directory(&host, fixture.path(), &cancelled, &mut |_| panic!(
            "canceled entry"
        ))
        .is_err()
    );
    assert!(!host.list_volumes().unwrap().is_empty());
    let space = host.space_info(fixture.path()).unwrap();
    assert!(space.total > 0 && space.available <= space.total);
    let details = host
        .volume_information(fixture.path(), &Cancellation::default())
        .unwrap();
    assert!(details.root.is_some());
    assert!(details.capacity.total.is_some_and(|total| total > 0));
    assert!(details.capacity.used().is_some());
    let _label = host.volume_label(fixture.path()).unwrap();
    let installed = midnight_native_filesystem_host::InstalledProvider::from_installation(
        installation(),
        Some(fixture.path().to_owned()),
    );
    assert!(
        installed
            .list_volumes()
            .unwrap()
            .iter()
            .any(|volume| volume.id == "home" && volume.root == fixture.path())
    );
    assert!(
        installed
            .volume_information(fixture.path(), &Cancellation::default())
            .is_ok()
    );
    assert!(installed.capture(&source, &|| false).is_ok());
}

/// VIEW-05 through the actual native plugin, whose reader checks every read against its
/// own revision (change time included): after only the change time moved, a viewer capture
/// reads on and an exact capture fails; a write still fails both.
#[test]
#[ignore = "requires a protected independent native plugin installation"]
fn installed_viewer_capture_reads_on_after_only_the_change_time_moved() {
    use midnight_plugin_host::RevisionPolicy;
    let installed =
        midnight_native_filesystem_host::InstalledProvider::from_installation(installation(), None);
    let fixture = tempfile::tempdir().unwrap();
    // No extension: endpoint protection on some machines rescans fresh `.txt` files.
    let source = fixture.path().join("fresh 文档");
    std::fs::write(&source, b"original content").unwrap();
    let viewer = installed
        .capture_with(&source, RevisionPolicy::AcceptChangeTimeOnly, &|| false)
        .unwrap();
    let unread = installed
        .capture_with(&source, RevisionPolicy::AcceptChangeTimeOnly, &|| false)
        .unwrap();
    let exact = installed.capture(&source, &|| false).unwrap();
    assert_eq!(viewer.read_at(0, 8).unwrap(), b"original");
    assert_eq!(exact.read_at(0, 8).unwrap(), b"original");
    // The read-only attribute and NTFS ChangeTime on Windows, the mode and ctime on Linux:
    // the native revision includes neither attributes nor mode.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let writable = std::fs::metadata(&source).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&source, read_only).unwrap();
    std::fs::set_permissions(&source, writable).unwrap();
    assert!(exact.read_at(0, 8).is_err(), "the change time did not move");
    assert!(exact.revalidate().is_err());
    assert_eq!(viewer.read_at(9, 7).unwrap(), b"content");
    viewer.revalidate().unwrap();
    assert_eq!(unread.read_at(0, 8).unwrap(), b"original");
    // Same length, new write time.
    std::fs::write(&source, b"changed  content").unwrap();
    for capture in [&viewer, &unread] {
        assert!(capture.read_at(0, 8).is_err());
        assert!(capture.revalidate().is_err());
    }
}

#[cfg(windows)]
#[path = "support/archive_tag_windows.rs"]
mod archive_tag;
#[cfg(target_os = "linux")]
#[path = "support/archive_tag_linux.rs"]
mod archive_tag;

/// Tag the file (`archive_tag::tag`) until its change time has moved, at the file
/// system's clock tick at the latest; each pass is a real metadata change, without
/// sleeping.
#[cfg(any(windows, target_os = "linux"))]
fn tag_until_the_change_time_moved(path: &Path) {
    let revision = || {
        midnight_plugin_host::CapturedFile::capture(path)
            .unwrap()
            .stable_version()
    };
    let before = revision();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while revision() == before {
        assert!(
            std::time::Instant::now() < deadline,
            "the change time did not move"
        );
        archive_tag::tag(path);
    }
}

/// Archive containers through the actual native plugin, whose reader checks every read
/// against its own revision (change time included): after the archive was tagged, the
/// archive capture reads on and its saved revision still restores it, while an exact
/// capture fails. A rewrite of the same length fails both, for good.
#[test]
#[cfg(any(windows, target_os = "linux"))]
#[ignore = "requires a protected independent native plugin installation"]
fn installed_archive_capture_reads_on_after_the_archive_was_tagged() {
    let installed =
        midnight_native_filesystem_host::InstalledProvider::from_installation(installation(), None);
    let fixture = tempfile::tempdir().unwrap();
    // No extension: endpoint protection tags some fresh file types on its own.
    let container = fixture.path().join("tagged 文档");
    std::fs::write(&container, b"archive bytes").unwrap();
    let archive = installed
        .capture_matching(&container, None, &|| false)
        .unwrap();
    let saved = archive.stable_version();
    let exact = installed.capture(&container, &|| false).unwrap();
    assert_eq!(archive.read_at(0, 7).unwrap(), b"archive");
    assert_eq!(exact.read_at(0, 7).unwrap(), b"archive");
    tag_until_the_change_time_moved(&container);
    assert!(exact.read_at(0, 7).is_err());
    assert!(exact.revalidate().is_err());
    assert_eq!(archive.read_at(8, 5).unwrap(), b"bytes");
    archive.revalidate().unwrap();
    let restored = installed
        .capture_matching(&container, Some(&saved), &|| false)
        .unwrap();
    assert_eq!(restored.read_at(0, 7).unwrap(), b"archive");
    std::fs::write(&container, b"changed bytes").unwrap();
    for capture in [&archive, &restored] {
        assert!(capture.read_at(0, 7).is_err());
        assert!(capture.revalidate().is_err());
        assert!(capture.changed_since_capture());
    }
    assert!(
        installed
            .capture_matching(&container, Some(&saved), &|| false)
            .is_err()
    );
}

#[test]
#[ignore = "requires a protected independent native plugin installation"]
fn missing_duplicate_incompatible_and_removed_native_packages_fail_closed() {
    let installation = installation();
    let original = Catalog::discover(&installation).unwrap();
    let package = original.native_package().unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("native-discovery-")
        .tempdir_in(&installation)
        .unwrap();
    assert!(
        Catalog::discover(fixture.path())
            .unwrap()
            .native_package()
            .is_err()
    );
    let first = fixture.path().join("filesystems/one");
    std::fs::create_dir_all(&first).unwrap();
    copy_package(package, &first);
    let catalog = Catalog::discover(fixture.path()).unwrap();
    assert_eq!(catalog.packages.len(), 1);
    let selected = catalog.native_package().unwrap().clone();
    let second = fixture.path().join("filesystems/two");
    std::fs::create_dir_all(&second).unwrap();
    copy_package(package, &second);
    let duplicates = Catalog::discover(fixture.path()).unwrap();
    assert!(duplicates.packages.is_empty());
    assert!(duplicates.native_package().is_err());
    assert!(
        duplicates
            .diagnostics
            .iter()
            .any(|line| line.contains("Duplicate"))
    );
    std::fs::remove_file(&selected.library).unwrap();
    assert!(
        NativeProvider::open(&selected).is_err(),
        "Removed module must never become a built-in provider"
    );
    let manifest = second.join("plugin.toml");
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace("abi_major = 1", "abi_major = 99");
    std::fs::write(manifest, text).unwrap();
    assert!(
        Catalog::discover(fixture.path())
            .unwrap()
            .native_package()
            .is_err()
    );
    let overflow = tempfile::Builder::new()
        .prefix("native-overflow-")
        .tempdir_in(&installation)
        .unwrap();
    for number in 0..65 {
        let directory = overflow
            .path()
            .join("filesystems")
            .join(format!("package-{number}"));
        std::fs::create_dir_all(&directory).unwrap();
        if number == 0 {
            copy_package(package, &directory);
        } else {
            std::fs::write(
                directory.join("plugin.toml"),
                "kind = 'filesystem-worker'\n",
            )
            .unwrap();
        }
    }
    assert!(
        Catalog::discover(overflow.path()).is_err(),
        "A partial scan cannot exclude a hidden duplicate or competing provider"
    );
}

fn copy_package(package: &midnight_native_filesystem_host::Package, destination: &Path) {
    std::fs::copy(
        package.directory.join("plugin.toml"),
        destination.join("plugin.toml"),
    )
    .unwrap();
    std::fs::copy(
        &package.library,
        destination.join(package.library.file_name().unwrap()),
    )
    .unwrap();
}
