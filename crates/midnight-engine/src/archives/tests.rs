use super::*;
use std::sync::Mutex;

#[test]
fn injected_archive_capture_has_no_fallback_and_receives_restore_revision() {
    let called = Arc::new(Mutex::new(None));
    let record = called.clone();
    let capture: ArchiveCapture = Arc::new(move |path, expected, _| {
        *record.lock().unwrap() = Some((path.to_owned(), expected.map(<[u8]>::to_vec)));
        Err(error("provider refused capture"))
    });
    // An existing readable file must not be opened when its provider rejects it.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert!(path.is_file());
    let expected = [1, 2, 3];
    let result = capture_source(
        Some(&capture),
        &path,
        Some(&expected),
        &Cancellation::default(),
    );
    assert!(matches!(result, Err(ref e) if e.to_string().contains("provider refused")));
    assert_eq!(
        *called.lock().unwrap(),
        Some((path.clone(), Some(expected.to_vec())))
    );
    assert!(capture_source(None, &path, None, &Cancellation::default()).is_err());
}

#[test]
fn archive_capture_cancellation_is_checked_before_and_after_callback() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let cancel = Cancellation::default();
    cancel.cancel();
    let capture: ArchiveCapture = Arc::new(|_, _, _| panic!("cancelled callback ran"));
    let result = capture_source(Some(&capture), &path, None, &cancel);
    assert!(matches!(result, Err(ref e) if e.kind == std::io::ErrorKind::Interrupted));

    let capture: ArchiveCapture = Arc::new(|path, _, cancel| {
        let source = midnight_plugin_host::CapturedFile::capture(path).unwrap();
        cancel.cancel();
        Ok(source)
    });
    let result = capture_source(Some(&capture), &path, None, &Cancellation::default());
    assert!(matches!(result, Err(ref e) if e.kind == std::io::ErrorKind::Interrupted));
}

async fn failed_navigation(action: ArchiveAction) -> crate::SnapshotState {
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    snapshots.lock().unwrap()[0].committed = Some(crate::CommittedSnapshot {
        navigation: NavigationId(1),
        path: std::env::temp_dir(),
        entries: Arc::from([]),
        sizes: Arc::default(),
    });
    let (sender, mut events) = tokio::sync::mpsc::channel(4);
    let mut controller = Controller::testing(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        snapshots.clone(),
        Arc::new(Semaphore::new(2)),
    );
    let mut tasks = JoinSet::new();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(2),
            action,
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    assert!(matches!(
        events.recv().await,
        Some(Event::ArchiveListed { result: Err(_), .. })
    ));
    tasks.join_next().await.unwrap().unwrap();
    std::mem::take(&mut snapshots.lock().unwrap()[0])
}

#[tokio::test]
async fn failed_archive_open_restores_native_snapshot_capabilities() {
    let state = failed_navigation(ArchiveAction::Open {
        path: std::env::temp_dir().join("missing.zip"),
        format: ArchiveFormat::Zip,
    })
    .await;
    assert!(state.archive_navigation.is_none());
    assert!(state.archive.is_none());
    assert_eq!(state.committed.unwrap().navigation, NavigationId(1));
}

#[tokio::test]
async fn unavailable_saved_archive_keeps_native_capabilities_disabled() {
    let state = failed_navigation(ArchiveAction::List(ArchiveLocation::unavailable(
        ArchiveAddress {
            format: None,
            provider: "missing.provider".into(),
            container: std::env::temp_dir().join("missing.zip"),
            source_version: vec![1],
            components: vec![],
        },
    )))
    .await;
    assert_eq!(state.archive_navigation, Some(NavigationId(2)));
}

fn shared_mount(id: u64) -> Arc<crate::archive_shared::testing::SharedMount> {
    let mount = crate::archive_shared::testing::SharedMount::new(id);
    mount.insert(
        1,
        vec![ArchiveNode {
            id: VirtualEntryId {
                mount: id,
                entry: 2,
            },
            parent: 1,
            raw_name: Arc::from(b"folder".as_slice()),
            display_name: "folder".into(),
            kind: EntryKind::Directory,
            size: None,
            readable: false,
            modified: None,
            access: Default::default(),
        }],
    );
    Arc::new(mount)
}

/// A location whose mount is already shared with another tab, tree or viewer.
fn shared_listing(mount: Arc<crate::archive_shared::testing::SharedMount>) -> ArchiveAction {
    ArchiveAction::List(ArchiveLocation {
        mount: Some(mount),
        address: ArchiveAddress {
            format: Some(ArchiveFormat::Zip),
            provider: "test.shared".into(),
            container: std::env::temp_dir().join("shared.zip"),
            source_version: vec![1],
            components: vec![],
        },
        directory: 1,
        trail: vec![(1, "shared.zip".into())],
    })
}

fn shared_controller(workers: usize) -> (Controller, tokio::sync::mpsc::Receiver<Event>) {
    let (sender, events) = tokio::sync::mpsc::channel(4);
    let controller = Controller::testing(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        crate::test_snapshots(Default::default()),
        Arc::new(Semaphore::new(workers)),
    );
    (controller, events)
}

async fn listed(
    events: &mut tokio::sync::mpsc::Receiver<Event>,
) -> (NavigationId, Result<(), FsError>) {
    match tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Event::ArchiveListed {
            navigation, result, ..
        } => (navigation, result.map(|_| ())),
        _ => panic!("unexpected event"),
    }
}

#[tokio::test]
async fn superseded_shared_listing_reports_interruption_and_keeps_the_archive_usable() {
    let mount = shared_mount(43);
    let (mut controller, mut events) = shared_controller(2);
    let mut tasks = JoinSet::new();
    let held = mount.hold_next();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(1),
            shared_listing(mount.clone()),
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    let held = tokio::task::spawn_blocking(move || {
        held.wait_entered();
        held
    })
    .await
    .unwrap();
    // Backspace before the listing returns: quick re-navigation cancels it.
    controller.cancel(PaneId::Left);
    held.release();
    let (_, result) = listed(&mut events).await;
    assert!(matches!(result, Err(error) if error.kind == std::io::ErrorKind::Interrupted));
    tasks.join_next().await.unwrap().unwrap();
    assert!(mount.list(1, &Cancellation::default()).is_ok());
}

#[tokio::test]
async fn newest_navigation_waits_for_a_worker_held_by_a_superseded_listing() {
    let mount = shared_mount(44);
    let (mut controller, mut events) = shared_controller(1);
    let mut tasks = JoinSet::new();
    let held = mount.hold_next();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(1),
            shared_listing(mount.clone()),
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    let held = tokio::task::spawn_blocking(move || {
        held.wait_entered();
        held
    })
    .await
    .unwrap();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(2),
            shared_listing(mount.clone()),
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    tokio::task::yield_now().await;
    assert!(
        events.try_recv().is_err(),
        "the newest navigation must wait for the busy worker, not be refused"
    );
    held.release();
    let mut outcomes = std::collections::HashMap::new();
    for _ in 0..2 {
        let (navigation, result) = listed(&mut events).await;
        outcomes.insert(navigation, result.is_ok());
    }
    assert!(
        !outcomes[&NavigationId(1)],
        "superseded result is discarded"
    );
    assert!(outcomes[&NavigationId(2)], "newest navigation is listed");
    while tasks.join_next().await.is_some() {}
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_and_superseded_navigations_stay_within_the_archive_cleanup_share() {
    // Both workers list archives shared with other tabs until the end of the test.
    let busy = [shared_mount(46), shared_mount(47)];
    let waiting = shared_mount(48);
    let (mut controller, events) = shared_controller(MAX_WORKERS);
    let mut tasks = JoinSet::new();
    let mut held = Vec::new();
    for (pane, mount) in [PaneId::Left, PaneId::Right].into_iter().zip(&busy) {
        let hold = mount.hold_next();
        controller
            .navigate(
                pane,
                NavigationId(1),
                shared_listing(mount.clone()),
                SortSpec::default(),
                &mut tasks,
            )
            .await;
        held.push(
            tokio::task::spawn_blocking(move || {
                hold.wait_entered();
                hold
            })
            .await
            .unwrap(),
        );
    }
    // The running listings adopted one retained mount each; keep one token free.
    let retained: Vec<_> = (3..ownership::RETAINED_MOUNTS)
        .map(|_| controller.mounts.reserve().unwrap())
        .collect();
    // Each pane's newest navigation waits for a worker, then the left pane keeps
    // superseding its waiter. This runtime polls superseded waiters only while
    // the intake itself waits, so only the navigation shares bound their slots.
    let panes = std::iter::once(PaneId::Right)
        .chain(std::iter::repeat_n(PaneId::Left, 2 * MAX_REQUESTS))
        .enumerate();
    for (index, pane) in panes {
        let navigation = NavigationId(index as u64 + 2);
        controller
            .navigate(
                pane,
                navigation,
                shared_listing(waiting.clone()),
                SortSpec::default(),
                &mut tasks,
            )
            .await;
        // A retained mount still finds its cleanup slot in the archive share.
        let reserved = controller.mounts.reserve();
        assert!(reserved.is_ok(), "{navigation:?}: {:?}", reserved.err());
    }
    drop(retained);
    drop(events);
    for hold in held {
        hold.release();
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}

#[tokio::test]
async fn archive_listings_sort_names_with_backend_locale_keys() {
    struct Locale;
    impl LocaleService for Locale {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn collation_key(&self, value: &std::ffi::OsStr) -> Result<Option<Vec<u8>>, FsError> {
            Ok(Some(value.to_string_lossy().to_lowercase().into_bytes()))
        }
    }
    let mount = crate::archive_shared::testing::SharedMount::new(45);
    let file = |entry: u64, name: &str| ArchiveNode {
        id: VirtualEntryId { mount: 45, entry },
        parent: 1,
        raw_name: Arc::from(name.as_bytes()),
        display_name: name.into(),
        kind: EntryKind::File,
        size: Some(1),
        readable: true,
        modified: None,
        access: Default::default(),
    };
    // Raw byte order would place "Zeta" before "alpha".
    mount.insert(1, vec![file(2, "Zeta"), file(3, "alpha")]);
    let (mut controller, mut events) = shared_controller(2);
    controller.locale = Some(Arc::new(Locale));
    let mut tasks = JoinSet::new();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(1),
            shared_listing(Arc::new(mount)),
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::ArchiveListed {
        result: Ok(listing),
        ..
    } = event
    else {
        panic!("archive listing failed")
    };
    let names: Vec<_> = listing
        .entries
        .iter()
        .map(|entry| entry.display_name.as_str())
        .collect();
    assert_eq!(names, ["alpha", "Zeta"]);
    while tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn unavailable_plugin_errors_keep_bounded_discovery_diagnostics() {
    let (mut controller, mut events) = shared_controller(2);
    let catalog = OnceLock::new();
    let mut diagnostics = vec!["Plugin package directory is writable by other users".to_owned()];
    diagnostics.extend((0..5).map(|index| format!("ignored package {index}")));
    catalog
        .set(Catalog {
            diagnostics,
            ..Catalog::default()
        })
        .unwrap();
    controller.catalog = Arc::new(catalog);
    let mut tasks = JoinSet::new();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(1),
            ArchiveAction::Open {
                path: std::env::temp_dir().join("unsupported.zip"),
                format: ArchiveFormat::Zip,
            },
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    let (_, result) = listed(&mut events).await;
    let message = result.unwrap_err().message;
    assert!(message.starts_with("Compatible archive filesystem plugin is unavailable"));
    assert!(message.contains("Plugin discovery: Plugin package directory is writable"));
    assert!(message.contains("2 more plugin discovery diagnostics"));
    assert!(!message.contains("ignored package 4"));
    while tasks.join_next().await.is_some() {}
}
