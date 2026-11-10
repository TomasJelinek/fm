//! Exact archive action context is derived from captured raw ancestry, never captions.
use super::*;
use crate::providers::{ArchiveTarget, ProviderTarget};

fn target(mount: &dyn MountedFilesystem, node: &NodeToken) -> ArchiveTarget {
    match mount
        .resolve_target(node, &Cancellation::default())
        .unwrap()
    {
        ProviderTarget::Archive(target) => *target,
        ProviderTarget::Native(path) => panic!("archive target became a native path: {path:?}"),
    }
}

fn assert_location(actual: &ArchiveLocation, base: &ArchiveLocation, ids: &[u64], raw: &[&[u8]]) {
    assert_eq!(actual.directory, *ids.last().unwrap());
    assert_eq!(
        actual.trail.iter().map(|row| row.0).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        actual.address.components,
        raw.iter().map(|part| part.to_vec()).collect::<Vec<_>>()
    );
    assert_eq!(actual.address.provider, base.address.provider);
    assert_eq!(actual.address.format, base.address.format);
    assert_eq!(actual.address.container, base.address.container);
    assert_eq!(actual.address.source_version, base.address.source_version);
    assert!(Arc::ptr_eq(
        actual.mount.as_ref().unwrap(),
        base.mount.as_ref().unwrap()
    ));
    actual.address.validate().unwrap();
}

#[test]
fn nested_directory_and_file_targets_retain_complete_raw_ancestry_from_captured_subdirectory() {
    let (mut location, state) = fixture(31, 4);
    location.address.format = Some(midnight_core::ArchiveFormat::Zip);
    let base = location.clone();
    {
        let mut rows = state.rows.lock().unwrap();
        rows.get_mut(&4)
            .unwrap()
            .push(node(31, 6, 4, EntryKind::Directory));
        rows.insert(6, vec![node(31, 7, 6, EntryKind::Directory)]);
        rows.insert(7, vec![node(31, 8, 7, EntryKind::File)]);
    }
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    location.address.components[0] = b"mutated caller state".to_vec();
    location.trail[1].1 = "mutated caption".into();
    let base_target = target(opened.mount.as_ref(), &opened.root);
    assert_location(&base_target.location, &base, &[1, 4], &[&[0xff, 4]]);
    assert_eq!(base_target.location.trail, base.trail);
    assert!(base_target.member.is_none());

    let first = listing(opened.mount.as_ref(), &opened.root);
    let second = listing(opened.mount.as_ref(), &first[1].token);
    let third = listing(opened.mount.as_ref(), &second[0].token);
    let before = state.lists.load(Ordering::Acquire);
    let directory = target(opened.mount.as_ref(), &second[0].token);
    assert_location(
        &directory.location,
        &base,
        &[1, 4, 6, 7],
        &[&[0xff, 4], &[0xff, 6], &[0xff, 7]],
    );
    assert!(directory.member.is_none());
    let original = directory
        .original
        .as_ref()
        .expect("listed directory record");
    assert_eq!(
        (original.id.mount, original.id.entry, original.parent),
        (31, 7, 6)
    );
    assert_eq!(original.raw_name.as_ref(), &[0xff, 7]);
    assert_eq!(original.kind, EntryKind::Directory);
    let file = target(opened.mount.as_ref(), &third[0].token);
    assert_location(
        &file.location,
        &base,
        &[1, 4, 6, 7],
        &[&[0xff, 4], &[0xff, 6], &[0xff, 7]],
    );
    let original = file.original.as_ref().expect("listed file record");
    assert_eq!(original.id, file.member.as_ref().unwrap().id);
    assert_eq!(original.raw_name, file.member.as_ref().unwrap().raw_name);
    let member = file.member.unwrap();
    assert_eq!(
        (member.id.mount, member.id.entry, member.parent),
        (31, 8, 7)
    );
    assert_eq!(member.raw_name.as_ref(), &[0xff, 8]);
    assert_eq!(member.kind, EntryKind::File);
    assert_eq!(
        state.lists.load(Ordering::Acquire),
        before,
        "resolution must not re-list"
    );
    assert_eq!(
        state.reads.load(Ordering::Acquire),
        0,
        "resolution must not extract"
    );

    let parent = opened
        .mount
        .parent(&second[0].token, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_location(
        &target(opened.mount.as_ref(), &parent).location,
        &base,
        &[1, 4, 6],
        &[&[0xff, 4], &[0xff, 6]],
    );
    let root = opened
        .mount
        .parent(&opened.root, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_location(
        &target(opened.mount.as_ref(), &root).location,
        &base,
        &[1],
        &[],
    );
    assert!(
        opened
            .mount
            .parent(&root, &Cancellation::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn identical_captions_preserve_distinct_raw_file_targets_and_unreadable_metadata() {
    let (location, state) = fixture(32, 1);
    {
        let mut rows = state.rows.lock().unwrap();
        let rows = rows.get_mut(&1).unwrap();
        rows[0].readable = false;
        rows[0].size = None;
    }
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    let first = target(opened.mount.as_ref(), &rows[0].token);
    let second = target(opened.mount.as_ref(), &rows[1].token);
    assert_location(&first.location, &location, &[1], &[]);
    let first = first.member.unwrap();
    let second = second.member.unwrap();
    assert_eq!(first.display_name, second.display_name);
    assert_ne!(first.id, second.id);
    assert_ne!(first.raw_name, second.raw_name);
    assert!(!first.readable);
    assert_eq!(first.size, None);
    assert!(matches!(
        opened
            .mount
            .open_content(&rows[0].token, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn link_and_special_members_do_not_gain_file_or_directory_target_semantics() {
    let (location, state) = fixture(33, 1);
    state.rows.lock().unwrap().insert(
        1,
        [
            EntryKind::DirectoryLink,
            EntryKind::Symlink,
            EntryKind::Other,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| node(33, index as u64 + 2, 1, kind))
        .collect(),
    );
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    for row in listing(opened.mount.as_ref(), &opened.root) {
        assert!(matches!(
            opened
                .mount
                .resolve_target(&row.token, &Cancellation::default()),
            Err(ProviderError::Unsupported)
        ));
        assert!(matches!(
            opened
                .mount
                .open_content(&row.token, &Cancellation::default()),
            Err(ProviderError::Unsupported)
        ));
    }
    assert_eq!(state.reads.load(Ordering::Acquire), 0);
}

#[test]
fn resolution_checks_cancellation_and_source_before_and_after_without_mutating_captured_state() {
    let (location, state) = fixture(34, 4);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let expected = target(opened.mount.as_ref(), &opened.root).location;
    let before = state.validations.load(Ordering::Acquire);
    state.fail_validate_at.store(before + 2, Ordering::Release);
    assert!(matches!(
        opened
            .mount
            .resolve_target(&opened.root, &Cancellation::default()),
        Err(ProviderError::Failed(_))
    ));
    assert_eq!(state.validations.load(Ordering::Acquire), before + 2);
    state.fail_validate_at.store(0, Ordering::Release);
    assert_eq!(
        target(opened.mount.as_ref(), &opened.root).location.address,
        expected.address
    );

    let cancel = Cancellation::default();
    let before = state.validations.load(Ordering::Acquire);
    *state.cancel_validate.lock().unwrap() = Some((before + 2, cancel.clone()));
    assert!(matches!(
        opened.mount.resolve_target(&opened.root, &cancel),
        Err(ProviderError::Cancelled)
    ));
    let before = state.validations.load(Ordering::Acquire);
    assert!(matches!(
        opened.mount.resolve_target(&opened.root, &cancel),
        Err(ProviderError::Cancelled)
    ));
    assert_eq!(state.validations.load(Ordering::Acquire), before);
    *state.cancel_validate.lock().unwrap() = None;
    state.invalid.store(true, Ordering::Release);
    assert!(
        opened
            .mount
            .resolve_target(&opened.root, &Cancellation::default())
            .is_err()
    );
    state.invalid.store(false, Ordering::Release);
    assert_eq!(
        target(opened.mount.as_ref(), &opened.root).location.address,
        expected.address
    );
    for bad in [
        forged(99, 4),
        forged(34, 99),
        NodeToken::new(b"bad").unwrap(),
    ] {
        assert!(matches!(
            opened.mount.resolve_target(&bad, &Cancellation::default()),
            Err(ProviderError::StaleReference)
        ));
    }
}

#[test]
fn capture_rejects_inconsistent_or_invalid_raw_base_and_oversized_or_cyclic_trails() {
    for case in 0..7 {
        let (mut location, _) = fixture(35, 4);
        match case {
            0 => location.address.components.clear(),
            1 => location.address.components[0] = b"..".to_vec(),
            2 => location.address.components[0] = b"wrong/part".to_vec(),
            3 => location.address.provider = "invalid provider".into(),
            4 => location.trail.push((4, "cycle".into())),
            5 => location.trail[1].1 = "x".repeat(64 * 1024 + 1),
            _ => location.trail = (1..=258).map(|id| (id, "d".into())).collect(),
        }
        assert!(
            captured_archive(&location, &Cancellation::default()).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn parent_listing_cannot_replace_captured_raw_ancestry_or_publish_partial_targets() {
    let (location, state) = fixture(36, 4);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let root = opened
        .mount
        .parent(&opened.root, &Cancellation::default())
        .unwrap()
        .unwrap();
    state.rows.lock().unwrap().get_mut(&1).unwrap()[2].raw_name = Arc::from(&b"different"[..]);
    let mut emitted = 0;
    assert!(
        opened
            .mount
            .list(&root, &Cancellation::default(), &mut |_| {
                emitted += 1;
                true
            })
            .is_err()
    );
    assert_eq!(emitted, 0);
    assert!(matches!(
        opened
            .mount
            .resolve_target(&forged(36, 2), &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    assert_location(
        &target(opened.mount.as_ref(), &opened.root).location,
        &location,
        &[1, 4],
        &[&[0xff, 4]],
    );
}

#[test]
fn invalid_raw_targets_and_ancestor_cycles_never_replace_previously_captured_targets() {
    for raw in [
        vec![],
        b".".to_vec(),
        b"..".to_vec(),
        b"a/b".to_vec(),
        b"a\\b".to_vec(),
        b"a:b".to_vec(),
        b"a\0b".to_vec(),
    ] {
        let (location, state) = fixture(37, 1);
        state.rows.lock().unwrap().get_mut(&1).unwrap()[0].raw_name = raw.into();
        let opened = captured_archive(&location, &Cancellation::default()).unwrap();
        let rows = listing(opened.mount.as_ref(), &opened.root);
        // Unsafe archive names remain visible as provider data; they never become action locators.
        assert!(matches!(
            opened
                .mount
                .resolve_target(&rows[0].token, &Cancellation::default()),
            Err(ProviderError::InvalidResponse(_))
        ));
        assert!(matches!(
            opened
                .mount
                .resolve_target(&forged(37, 99), &Cancellation::default()),
            Err(ProviderError::StaleReference)
        ));
    }
    let (location, state) = fixture(38, 4);
    state
        .rows
        .lock()
        .unwrap()
        .get_mut(&4)
        .unwrap()
        .push(node(38, 6, 4, EntryKind::Directory));
    state
        .rows
        .lock()
        .unwrap()
        .insert(6, vec![node(38, 4, 6, EntryKind::Directory)]);
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    assert!(
        opened
            .mount
            .list(&rows[1].token, &Cancellation::default(), &mut |_| true)
            .is_err()
    );
    assert_location(
        &target(opened.mount.as_ref(), &opened.root).location,
        &location,
        &[1, 4],
        &[&[0xff, 4]],
    );
}

#[test]
fn target_depth_and_raw_locator_budget_fail_without_damaging_valid_base() {
    for depth_limit in [true, false] {
        let (mut location, state) = fixture(39, 4);
        if depth_limit {
            location.directory = 257;
            location.trail = (1..=257).map(|id| (id, "same caption".into())).collect();
            location.address.components = vec![b"d".to_vec(); 256];
        } else {
            location.address.components[0] = vec![b'd'; 64 * 1024 - 1];
        }
        let mut child = node(39, 300, location.directory, EntryKind::Directory);
        child.raw_name = Arc::from(&b"dd"[..]);
        state
            .rows
            .lock()
            .unwrap()
            .insert(location.directory, vec![child]);
        let opened = captured_archive(&location, &Cancellation::default()).unwrap();
        let rows = listing(opened.mount.as_ref(), &opened.root);
        assert!(matches!(
            opened
                .mount
                .resolve_target(&rows[0].token, &Cancellation::default()),
            Err(ProviderError::ResourceLimited)
        ));
        let base = target(opened.mount.as_ref(), &opened.root);
        assert_eq!(base.location.address, location.address);
        assert_eq!(base.location.trail, location.trail);
        assert_eq!(state.reads.load(Ordering::Acquire), 0);
    }
}

#[test]
fn resolved_target_retains_original_mount_until_backend_owner_drops_it() {
    let (location, state) = fixture(40, 1);
    let weak = Arc::downgrade(location.mount.as_ref().unwrap());
    let opened = captured_archive(&location, &Cancellation::default()).unwrap();
    let rows = listing(opened.mount.as_ref(), &opened.root);
    let resolved = target(opened.mount.as_ref(), &rows[0].token);
    drop(opened);
    drop(location);
    assert!(weak.upgrade().is_some());
    assert_eq!(resolved.member.as_ref().unwrap().id.entry, 2);
    std::thread::spawn(move || drop(resolved)).join().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(state.reads.load(Ordering::Acquire), 0);
}

#[test]
fn cloned_archive_target_mount_retains_registration_and_refuses_revoked_operations() {
    use crate::providers::{FilesystemProvider, ProviderRegistry};
    use midnight_core::{ProviderId, StoredLocation};

    struct Owner {
        initial: Mutex<Option<ArchiveLocation>>,
        events: DropEvents,
    }
    impl FilesystemProvider for Owner {
        fn resolve(
            &self,
            _: &StoredLocation,
            cancel: &Cancellation,
        ) -> Result<OpenedLocation, ProviderError> {
            let location = self
                .initial
                .lock()
                .unwrap()
                .take()
                .ok_or(ProviderError::Unavailable)?;
            captured_archive(&location, cancel)
        }
    }
    impl Drop for Owner {
        fn drop(&mut self) {
            self.events.lock().unwrap().push("provider");
        }
    }

    let (initial, state) = fixture(41, 1);
    let events = Arc::new(Mutex::new(Vec::new()));
    *state.drop_events.lock().unwrap() = Some(events.clone());
    let weak = Arc::downgrade(initial.mount.as_ref().unwrap());
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new("archive.target.lease").unwrap(),
        std::num::NonZeroU32::MIN,
        Arc::from([]),
    );
    let instance = registry
        .register(
            address.provider().clone(),
            Arc::new(Owner {
                initial: Mutex::new(Some(initial)),
                events: events.clone(),
            }),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let rows = location
        .list(location.root(), &Cancellation::default())
        .unwrap();
    let resolved = location
        .resolve_target(&rows[0].node, &Cancellation::default())
        .unwrap();
    let escaped = match resolved.value().unwrap() {
        ProviderTarget::Archive(target) => target.location.mount.as_ref().unwrap().clone(),
        ProviderTarget::Native(_) => panic!("archive target became native"),
    };
    drop(resolved);
    drop(location);
    escaped.validate().unwrap();
    assert_eq!(escaped.list(1, &Cancellation::default()).unwrap().len(), 3);
    assert_eq!(
        escaped.read(2, 0, 5, &Cancellation::default()).unwrap(),
        b"hello"
    );
    assert!(registry.unregister(address.provider(), instance).unwrap());
    assert!(
        events.lock().unwrap().is_empty(),
        "escaped target mount must retain provider registration"
    );
    assert!(weak.upgrade().is_some());

    let calls = (
        state.validations.load(Ordering::Acquire),
        state.lists.load(Ordering::Acquire),
        state.reads.load(Ordering::Acquire),
    );
    assert!(escaped.validate().is_err());
    assert!(escaped.list(1, &Cancellation::default()).is_err());
    assert!(escaped.read(2, 0, 5, &Cancellation::default()).is_err());
    assert_eq!(
        calls,
        (
            state.validations.load(Ordering::Acquire),
            state.lists.load(Ordering::Acquire),
            state.reads.load(Ordering::Acquire),
        ),
        "revoked operations must not call the underlying archive"
    );
    drop(registry);
    std::thread::spawn(move || drop(escaped)).join().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(*events.lock().unwrap(), ["archive", "provider"]);
}
