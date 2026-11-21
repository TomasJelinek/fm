//! A same-volume Move of a folder the user owns works whatever the folder's own
//! write permission: nothing is asked, and the moved folder keeps its mode. A
//! cross-volume Move takes the copied entries out of the owned write-protected folders
//! of the moved tree without asking, and a folder that stays keeps its mode; a child
//! that cannot be moved (a special file) is asked about alone.
use super::moves::child_failures::{Refusing, decide, failure, finished, move_request};
use super::*;
use std::os::unix::fs::PermissionsExt;

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[tokio::test]
async fn moving_an_owned_write_protected_folder_asks_nothing_and_keeps_its_mode() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(source.join("locked")).unwrap();
    fs::create_dir_all(source.join("closed")).unwrap();
    fs::write(source.join("locked/file.txt"), b"inside").unwrap();
    fs::create_dir(&destination).unwrap();
    // Read-only, and without any permission for anyone.
    let folders = [("locked", 0o555), ("closed", 0o000)];
    for (name, protected) in folders {
        chmod(&source.join(name), protected);
    }
    let mut h = Harness::native();
    h.controller
        .enqueue_clipboard_move(ClipboardMoveRequest {
            start_policy: midnight_core::TransferStartPolicy::Immediate,
            verification: TransferVerification::default(),
            id: OperationId(1),
            clipboard: Arc::new(FileClipboard {
                paths: folders.iter().map(|(name, _)| source.join(name)).collect(),
                intent: FileClipboardIntent::Move,
            }),
            destination: destination.clone(),
        })
        .await;
    let done = loop {
        match h.next().await {
            CopyEvent::Failure(failure) => {
                h.controller.cancel(OperationId(1));
                panic!("asked: {failure:?}");
            }
            CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
            _ => {}
        }
    };
    h.close().await;
    let modes: Vec<_> = folders
        .iter()
        .map(|(name, _)| destination.join(name))
        .filter(|moved| moved.exists())
        .map(|moved| mode(&moved))
        .collect();
    // Writable again, so the fixture can be removed.
    for (name, _) in folders {
        for folder in [source.join(name), destination.join(name)] {
            if folder.exists() {
                chmod(&folder, 0o755);
            }
        }
    }
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.native_entries_moved, 2, "{done:?}");
    assert_eq!(modes, [0o555, 0o000]);
    for (name, _) in folders {
        assert!(!source.join(name).exists(), "{name}");
    }
    assert_eq!(
        fs::read(destination.join("locked/file.txt")).unwrap(),
        b"inside"
    );
}

#[tokio::test]
async fn a_cross_volume_move_empties_owned_write_protected_folders_which_keep_their_modes() {
    let temp = fixture();
    let locked = temp.path().join("source").join("locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    fs::write(locked.join("file.txt"), b"file").unwrap();
    fs::write(locked.join("kept.txt"), b"kept").unwrap();
    fs::write(locked.join("inner/deeper.txt"), b"deeper").unwrap();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    for folder in [locked.join("inner"), locked.clone()] {
        chmod(&folder, 0o555);
    }
    // Another volume, where `kept.txt` cannot be moved, so `locked` stays.
    let mut h = Harness::new(Some(Arc::new(Refusing::new(&[("kept.txt", usize::MAX)]))));
    h.controller
        .enqueue_clipboard_move(move_request(&locked, &destination))
        .await;
    let failure = failure(&mut h).await;
    decide(&h, &failure, CopyChoice::Skip).await;
    let done = finished(&mut h).await;
    h.close().await;
    let kept = (mode(&locked), fs::read_dir(&locked).unwrap().count());
    let moved = destination.join("locked");
    // Writable again, so the fixture can be removed.
    for folder in [&locked, &moved, &moved.join("inner")] {
        if folder.exists() {
            chmod(folder, 0o755);
        }
    }
    // The only question was about the child that cannot be moved.
    assert_eq!(failure.source, Some(locked.join("kept.txt")), "{failure:?}");
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.skipped, done.failed), (1, 1), "{done:?}");
    assert_eq!(done.source_files_removed, 2, "{done:?}");
    assert_eq!(kept, (0o555, 1));
    assert_eq!(fs::read(locked.join("kept.txt")).unwrap(), b"kept");
    assert_eq!(fs::read(moved.join("file.txt")).unwrap(), b"file");
    assert_eq!(fs::read(moved.join("inner/deeper.txt")).unwrap(), b"deeper");
}

#[tokio::test]
async fn a_special_file_stays_alone_in_its_write_protected_folder_and_the_rest_moves() {
    let temp = fixture();
    let locked = temp.path().join("source").join("locked");
    fs::create_dir_all(locked.join("inner")).unwrap();
    fs::write(locked.join("file.txt"), b"file").unwrap();
    fs::write(locked.join("inner/deeper.txt"), b"deeper").unwrap();
    // A socket cannot be moved by copying it: only it stays, and so does its folder.
    drop(std::os::unix::net::UnixListener::bind(locked.join("socket")).unwrap());
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    for folder in [locked.join("inner"), locked.clone()] {
        chmod(&folder, 0o555);
    }
    let service = super::moves::CrossDeviceService(midnight_platform::LocalFileSystem::new(None));
    let mut h = Harness::new(Some(Arc::new(service)));
    h.controller
        .enqueue_clipboard_move(move_request(&locked, &destination))
        .await;
    let failure = failure(&mut h).await;
    decide(&h, &failure, CopyChoice::Skip).await;
    let done = finished(&mut h).await;
    h.close().await;
    let kept = (mode(&locked), fs::read_dir(&locked).unwrap().count());
    let moved = destination.join("locked");
    // Writable again, so the fixture can be removed.
    for folder in [&locked, &moved, &moved.join("inner")] {
        if folder.exists() {
            chmod(folder, 0o755);
        }
    }
    assert_eq!(failure.stage, CopyErrorStage::MovingSource, "{failure:?}");
    assert_eq!(failure.source, Some(locked.join("socket")), "{failure:?}");
    assert!(!failure.can_retry && failure.can_skip, "{failure:?}");
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.skipped, done.failed), (1, 1), "{done:?}");
    assert_eq!(kept, (0o555, 1));
    assert_eq!(fs::read(moved.join("file.txt")).unwrap(), b"file");
    assert_eq!(fs::read(moved.join("inner/deeper.txt")).unwrap(), b"deeper");
}
