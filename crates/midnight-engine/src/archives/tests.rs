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
