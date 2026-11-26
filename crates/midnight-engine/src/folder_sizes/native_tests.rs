//! Actual platform resolution and bounded scanner integration on owned fixtures.
use super::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    sync::{Arc, mpsc},
    thread,
};

use crate::native_test_links as links;

fn measured(path: &Path, kind: EntryKind) -> FolderSize {
    let root = Entry::new(EntryId(1), path.into(), kind);
    let cancel = Cancellation::default();
    let worker_cancel = cancel.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let mut last = FolderSize::default();
        compute_batched(
            &LocalFileSystem::new(None),
            &[root],
            &worker_cancel,
            &mut |_, size| {
                last = size;
                true
            },
            // Real platform resolution, without sharing the process-wide gate
            // that other library tests take exclusively.
            &Arc::new(IoBudget::isolated()),
            &crate::ReportArena::testing(),
        );
        sender.send(last).unwrap();
    });
    let result = receiver.recv_timeout(Duration::from_secs(8));
    cancel.cancel();
    worker.join().unwrap();
    result.expect("directory cycles must finish without repeated recursion")
}

#[test]
fn native_size_scans_skip_links_inside_the_tree_and_follow_only_a_selected_root_link() {
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().join("root 資料");
    let sibling = owned.path().join("ancestor sibling");
    let external = owned.path().join("large external tree");
    fs::create_dir_all(root.join("child")).unwrap();
    fs::create_dir(&sibling).unwrap();
    fs::create_dir_all(external.join("nested")).unwrap();
    fs::write(root.join("own"), [1; 7]).unwrap();
    fs::write(root.join("child/own"), [2; 11]).unwrap();
    fs::write(sibling.join("payload"), [3; 5]).unwrap();
    for index in 0..8 {
        fs::write(external.join(format!("bulk-{index}")), [4; 4096]).unwrap();
    }
    fs::write(external.join("nested/deep"), [5; 4096]).unwrap();
    links::create(&root.join("large alias"), &external);
    links::create(&root.join("child/to ancestor sibling"), &sibling);
    links::create(&root.join("self"), &root);
    links::create(&root.join("child/up"), &root);
    let direct = owned.path().join("selected junction");
    links::create(&direct, &root);

    let filesystem = LocalFileSystem::new(None);
    let identity =
        DirectoryService::resolve_size_directory(&filesystem, &root, &Cancellation::default())
            .unwrap()
            .unwrap();
    let alias =
        DirectoryService::resolve_size_directory(&filesystem, &direct, &Cancellation::default())
            .unwrap()
            .unwrap();
    assert_eq!(identity.identity, alias.identity);
    assert_eq!(identity.path, alias.path);
    let mut listed_kind = None;
    DirectoryService::read_directory(
        &filesystem,
        owned.path(),
        &Cancellation::default(),
        &mut |entry| {
            if entry.path == direct {
                listed_kind = Some(entry.kind);
            }
            true
        },
    )
    .unwrap();
    let listed_kind = listed_kind.expect("selected directory link is listed");
    assert_eq!(listed_kind, EntryKind::DirectoryLink);
    // Only the tree's own files count: the external tree, the ancestor's
    // sibling and both cyclic links are skipped, never resolved or double counted.
    for (path, kind) in [(&root, EntryKind::Directory), (&direct, listed_kind)] {
        let size = measured(path, kind);
        assert_eq!(
            (size.bytes, size.files, size.errors, size.skipped_links),
            (18, 2, 0, 4)
        );
        assert!(size.finished);
        assert!(
            size.warnings.is_none(),
            "a skipped link is not an access failure"
        );
    }
}

#[test]
fn native_size_link_unavailable_target_is_skipped_inside_and_reported_as_a_root() {
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().join("root");
    let target = owned.path().join("vanishing target");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(root.join("own"), [1; 7]).unwrap();
    let dangling = root.join("missing junction");
    links::create(&dangling, &target);
    fs::remove_dir(&target).unwrap();
    let size = measured(&root, EntryKind::Directory);
    assert_eq!(
        (size.bytes, size.files, size.errors, size.skipped_links),
        (7, 1, 0, 1)
    );
    assert!(size.finished);
    assert!(size.warnings.is_none());
    let selected = measured(&dangling, EntryKind::DirectoryLink);
    assert_eq!(
        (
            selected.bytes,
            selected.files,
            selected.errors,
            selected.skipped_links
        ),
        (0, 0, 1, 1)
    );
    assert!(selected.finished);
    let warning = selected.warnings.as_ref().unwrap().summary();
    assert_eq!(warning.observed, 1);
    assert!(warning.tooltip.contains("missing junction"));
    assert!(warning.tooltip.contains("resolve directory size target"));
}

/// A Windows compatibility junction denies listing on itself only: sizing it measures
/// its readable target without an error, and one to a denied folder reports that folder.
#[cfg(windows)]
#[test]
fn native_size_of_a_denied_compatibility_junction_measures_its_target() {
    let fixture = crate::providers::browse_listing::tests::denied_junctions();
    let size = measured(&fixture.alias, EntryKind::DirectoryLink);
    assert_eq!(
        (size.bytes, size.files, size.errors, size.skipped_links),
        (12, 2, 0, 0)
    );
    assert!(size.finished);
    assert!(size.warnings.is_none());
    let denied = measured(&fixture.private_alias, EntryKind::DirectoryLink);
    assert_eq!((denied.bytes, denied.files, denied.errors), (0, 0, 1));
    assert!(denied.finished);
}

/// Manual throughput probe:`cargo test -p midnight-engine --lib
/// folder_sizes::native_tests::native_size_scan_throughput_probe -- --ignored --nocapture`.
#[test]
#[ignore = "timing probe for manual before/after measurements"]
fn native_size_scan_throughput_probe() {
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().join("probe");
    for directory in 0..100 {
        let directory = root.join(format!("directory-{directory}"));
        fs::create_dir_all(&directory).unwrap();
        for file in 0..50 {
            fs::write(directory.join(format!("file-{file}")), [7; 3]).unwrap();
        }
    }
    let mut samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let size = measured(&root, EntryKind::Directory);
        samples.push(started.elapsed());
        assert_eq!((size.bytes, size.files, size.errors), (15_000, 5_000, 0));
    }
    samples.sort();
    println!(
        "native size scan of 100 directories x 50 files: median {:?}, samples {samples:?}",
        samples[samples.len() / 2]
    );
}

#[test]
fn native_size_resolution_cancellation_does_not_touch_an_unavailable_path() {
    let cancel = Cancellation::default();
    cancel.cancel();
    let error = DirectoryService::resolve_size_directory(
        &LocalFileSystem::new(None),
        Path::new("unavailable-size-path"),
        &cancel,
    )
    .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
}

/// A real folder whose listing is denied (a deny ACE for Everyone on Windows, mode 000
/// on Linux) is skipped as one error: its siblings before and after it, and their
/// subtrees, are still summed and the scan finishes.
#[test]
fn native_size_scan_skips_a_denied_folder_and_counts_its_siblings() {
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().join("root");
    fs::create_dir_all(root.join("a/nested")).unwrap();
    fs::create_dir_all(root.join("denied")).unwrap();
    fs::create_dir_all(root.join("z")).unwrap();
    fs::write(root.join("a/nested/one"), [1; 3]).unwrap();
    fs::write(root.join("denied/hidden"), [2; 4096]).unwrap();
    fs::write(root.join("z/two"), [3; 5]).unwrap();
    fs::write(root.join("top"), [4; 7]).unwrap();
    let denied = root.join("denied");
    let _guard = DeniedFolder::new(&denied);
    if fs::read_dir(&denied).is_ok() {
        // Only Linux root reads a mode-000 folder anyway: nothing to prove there. A
        // deny ACE applies to every Windows account.
        #[cfg(windows)]
        panic!("the deny ACE must refuse listing");
        #[cfg(unix)]
        return;
    }
    let size = measured(&root, EntryKind::Directory);
    assert_eq!((size.bytes, size.files, size.errors), (15, 3, 1));
    assert!(size.finished);
    let warning = size.warnings.as_ref().unwrap().summary();
    assert_eq!(warning.observed, 1);
    assert!(warning.tooltip.contains("denied"), "{}", warning.tooltip);
}

/// Denies listing one fixture folder and restores it on drop, so the temporary
/// fixture can be removed.
struct DeniedFolder(PathBuf, Restore);
#[cfg(windows)]
type Restore = midnight_platform_windows::SavedDacl;
#[cfg(unix)]
type Restore = fs::Permissions;
impl DeniedFolder {
    fn new(path: &Path) -> Self {
        #[cfg(windows)]
        let restore = midnight_platform_windows::deny_read_data(path).unwrap();
        #[cfg(unix)]
        let restore = {
            use std::os::unix::fs::PermissionsExt;
            let original = fs::metadata(path).unwrap().permissions();
            fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
            original
        };
        Self(path.to_owned(), restore)
    }
}
impl Drop for DeniedFolder {
    fn drop(&mut self) {
        #[cfg(windows)]
        let restored = midnight_platform_windows::restore_dacl(&self.0, &self.1);
        #[cfg(unix)]
        let restored = fs::set_permissions(&self.0, self.1.clone());
        if let Err(error) = restored
            && !thread::panicking()
        {
            panic!("cannot restore fixture access at {:?}: {error}", self.0);
        }
    }
}
