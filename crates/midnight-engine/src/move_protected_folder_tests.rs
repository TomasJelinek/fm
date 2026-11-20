//! A same-volume Move of a folder the user owns works whatever the folder's own
//! write permission: nothing is asked, and the moved folder keeps its mode.
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
