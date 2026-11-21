//! Cancelling a copy never terminates the archive session other owners share.
use super::*;
use crate::archive_shared::testing::SharedMount;

fn member(id: u64, parent: u64, kind: EntryKind, size: Option<u64>) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId {
            mount: 61,
            entry: id,
        },
        parent,
        raw_name: Arc::from(format!("member-{id}").as_bytes()),
        display_name: format!("member-{id}"),
        kind,
        size,
        modified: None,
        readable: kind == EntryKind::File,
        access: Default::default(),
    }
}

fn shared() -> Arc<SharedMount> {
    let mount = SharedMount::new(61);
    mount.insert(2, vec![member(3, 2, EntryKind::File, Some(7))]);
    mount.insert_content(3, b"payload");
    Arc::new(mount)
}

/// Run `step` on a worker while the copy is cancelled during its provider call.
fn cancel_during<T: Send>(mount: &SharedMount, step: impl FnOnce(&Cancellation) -> T + Send) -> T {
    let cancel = Cancellation::default();
    let held = mount.hold_next();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| step(&cancel));
        held.wait_entered();
        cancel.cancel();
        held.release();
        worker.join().unwrap()
    })
}

#[test]
fn cancelled_member_read_is_discarded_without_terminating_the_shared_mount() {
    let mount = shared();
    let mut source = Source::new(
        MountReference::new(mount.clone()),
        member(3, 2, EntryKind::File, Some(7)),
        0,
        traversal(),
    )
    .unwrap();
    let result = cancel_during(&mount, |cancel| {
        source.read_chunk(&mut [0; 7], cancel).map(|_| ())
    });
    assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::Interrupted));
    assert!(
        !mount.terminated(),
        "copy cancellation reached the shared archive exchange"
    );
    let mut buffer = [0; 7];
    assert_eq!(
        source
            .read_chunk(&mut buffer, &Cancellation::default())
            .unwrap(),
        7
    );
    assert_eq!(&buffer, b"payload");
}

#[test]
fn cancelled_directory_listing_is_discarded_and_the_source_lists_again() {
    let mount = shared();
    let mut source = Source::new(
        MountReference::new(mount.clone()),
        member(2, 1, EntryKind::Directory, None),
        0,
        traversal(),
    )
    .unwrap();
    let result = cancel_during(&mount, |cancel| source.next_child(cancel).map(|_| ()));
    assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::Interrupted));
    let child = source.next_child(&Cancellation::default()).unwrap();
    assert_eq!(child.unwrap().virtual_identity().unwrap().entry, 3);
}
