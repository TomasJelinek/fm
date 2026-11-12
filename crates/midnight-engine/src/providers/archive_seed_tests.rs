use super::*;
use crate::providers::captured_archive_root;
use midnight_core::Entry;

fn entries(state: &State, directory: u64) -> Vec<Entry> {
    state.rows.lock().unwrap()[&directory]
        .iter()
        .cloned()
        .map(Entry::from_archive)
        .collect()
}

#[test]
fn archive_seed_keeps_raw_cache_targets_and_metadata_without_relisting() {
    let (location, state) = fixture(51, 1);
    let original = entries(&state, 1);
    let opened = captured_archive_root(&location, &original, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    assert_eq!(state.lists.load(Ordering::Acquire), 0);
    assert_eq!(state.reads.load(Ordering::Acquire), 0);
    assert_eq!(rows[0].raw_name.as_ref(), &[0xff, 2]);
    assert_eq!(rows[0].metadata, original[0].metadata);
    let ProviderTarget::Archive(target) = opened
        .mount
        .resolve_target(&rows[0].token, &Cancellation::default())
        .unwrap()
    else {
        panic!("archive target required")
    };
    assert_eq!(target.location.directory, 1);
    assert_eq!(target.member.unwrap().id.entry, 2);
    let source = opened
        .mount
        .open_content(&rows[0].token, &Cancellation::default())
        .unwrap();
    assert_eq!(source.read_at(0, 5).unwrap(), b"hello");
    let children = listing(opened.mount.as_ref(), &rows[2].token);
    assert_eq!(children.len(), 1);
    assert_eq!(state.lists.load(Ordering::Acquire), 1);
    assert!(matches!(
        opened
            .mount
            .list(&opened.root, &Cancellation::default(), &mut |_| true),
        Err(ProviderError::StaleReference)
    ));
    assert_eq!(state.lists.load(Ordering::Acquire), 1);
}

#[test]
fn archive_seed_captured_subdirectory_preserves_ancestry() {
    let (location, state) = fixture(52, 4);
    let opened =
        captured_archive_root(&location, &entries(&state, 4), &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    let ProviderTarget::Archive(target) = opened
        .mount
        .resolve_target(&rows[0].token, &Cancellation::default())
        .unwrap()
    else {
        panic!("archive target required")
    };
    assert_eq!(target.location.address, location.address);
    assert_eq!(target.location.trail, location.trail);
    assert_eq!(target.member.unwrap().parent, 4);
    assert_eq!(state.lists.load(Ordering::Acquire), 0);
}

#[test]
fn archive_seed_rejects_snapshot_substitution_and_duplicate_members_atomically() {
    for mode in 0..6 {
        let (location, state) = fixture(53, 1);
        let mut bad = entries(&state, 1);
        match mode {
            0 => bad[0].virtual_node.as_mut().unwrap().parent = 99,
            1 => bad[0].virtual_node.as_mut().unwrap().id.mount = 99,
            2 => bad[1] = bad[0].clone(),
            3 => bad[0].virtual_node = None,
            4 => bad[0].metadata.size = Some(999),
            _ => bad[0].kind = EntryKind::Directory,
        }
        assert!(
            matches!(
                captured_archive_root(&location, &bad, &Cancellation::default()),
                Err(ProviderError::InvalidResponse(_))
            ),
            "mode {mode}"
        );
        assert_eq!(state.lists.load(Ordering::Acquire), 0);
        let valid = captured_archive_root(&location, &entries(&state, 1), &Cancellation::default())
            .unwrap();
        assert_eq!(listing(valid.mount.as_ref(), &valid.root).len(), 3);
    }
}

#[test]
fn archive_seed_checks_source_and_cancellation_before_and_after_capture() {
    let (location, state) = fixture(54, 1);
    let original = entries(&state, 1);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        captured_archive_root(&location, &original, &cancel),
        Err(ProviderError::Cancelled)
    ));
    assert_eq!(state.validations.load(Ordering::Acquire), 0);
    state.fail_validate_at.store(2, Ordering::Release);
    assert!(captured_archive_root(&location, &original, &Cancellation::default()).is_err());
    state.fail_validate_at.store(0, Ordering::Release);
    let opened = captured_archive_root(&location, &original, &Cancellation::default()).unwrap();
    state.invalid.store(true, Ordering::Release);
    let mut emitted = 0;
    assert!(
        opened
            .mount
            .list(&opened.root, &Cancellation::default(), &mut |_| {
                emitted += 1;
                true
            })
            .is_err()
    );
    assert_eq!(emitted, 0);
    assert_eq!(state.lists.load(Ordering::Acquire), 0);
}

#[test]
fn archive_seed_final_capture_validation_cancels_without_exposing_an_owner() {
    let (location, state) = fixture(55, 1);
    let original = entries(&state, 1);
    let cancel = Cancellation::default();
    *state.cancel_validate.lock().unwrap() = Some((3, cancel.clone()));
    let owners = Arc::strong_count(location.mount.as_ref().unwrap());
    assert!(matches!(
        captured_archive_root(&location, &original, &cancel),
        Err(ProviderError::Cancelled)
    ));
    assert_eq!(Arc::strong_count(location.mount.as_ref().unwrap()), owners);
    assert_eq!(state.lists.load(Ordering::Acquire), 0);
}

#[test]
fn archive_seed_preserves_known_metadata_and_empty_root_without_io() {
    let (location, state) = fixture(56, 1);
    let mut original = entries(&state, 1);
    original[0].metadata.modified = Some(std::time::UNIX_EPOCH);
    original[0].metadata.localized_modified = Some(midnight_core::LocalizedDateTime {
        date: "captured".into(),
        time: "once".into(),
    });
    let opened = captured_archive_root(&location, &original, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    assert_eq!(rows[0].metadata, original[0].metadata);
    let empty = captured_archive_root(&location, &[], &Cancellation::default()).unwrap();
    assert!(listing(empty.mount.as_ref(), &empty.root).is_empty());
    assert_eq!(state.lists.load(Ordering::Acquire), 0);
    assert_eq!(state.reads.load(Ordering::Acquire), 0);
}
