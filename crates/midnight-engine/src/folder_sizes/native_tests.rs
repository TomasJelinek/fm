//! Actual platform resolution and bounded scanner integration on owned fixtures.
use super::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    sync::{Arc, mpsc},
    thread,
};

#[cfg(windows)]
#[path = "../../tests/support/checksum_verify_links_windows.rs"]
mod links;
#[cfg(target_os = "linux")]
#[path = "../../tests/support/checksum_verify_links_linux.rs"]
mod links;

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
            &Arc::new(IoBudget::new()),
        );
        sender.send(last).unwrap();
    });
    let result = receiver.recv_timeout(Duration::from_secs(8));
    cancel.cancel();
    worker.join().unwrap();
    result.expect("directory cycles must finish without repeated recursion")
}

#[test]
fn native_size_links_follow_targets_and_stop_ancestor_and_self_cycles() {
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().join("root 資料");
    let target = owned.path().join("external target");
    fs::create_dir_all(root.join("child")).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(root.join("own"), [1; 7]).unwrap();
    fs::write(root.join("child/own"), [2; 11]).unwrap();
    fs::write(target.join("payload"), [3; 5]).unwrap();
    links::create(&root.join("alias"), &target);
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
    for (path, kind) in [(&root, EntryKind::Directory), (&direct, listed_kind)] {
        let size = measured(path, kind);
        assert_eq!(
            (size.bytes, size.files, size.errors, size.skipped_links),
            (23, 3, 0, 2)
        );
        assert!(size.finished);
        assert!(
            size.warnings.is_none(),
            "cycle omission is not an access failure"
        );
    }
}

#[test]
fn native_size_link_unavailable_target_preserves_partial_bytes_and_warning() {
    let owned = tempfile::tempdir().unwrap();
    let root = owned.path().join("root");
    let target = owned.path().join("vanishing target");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(root.join("own"), [1; 7]).unwrap();
    links::create(&root.join("missing junction"), &target);
    fs::remove_dir(&target).unwrap();
    let size = measured(&root, EntryKind::Directory);
    assert_eq!(
        (size.bytes, size.files, size.errors, size.skipped_links),
        (7, 1, 1, 1)
    );
    assert!(size.finished);
    let warning = size.warnings.as_ref().unwrap().summary();
    assert_eq!(warning.observed, 1);
    assert!(warning.tooltip.contains("missing junction"));
    assert!(warning.tooltip.contains("resolve directory size target"));
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
