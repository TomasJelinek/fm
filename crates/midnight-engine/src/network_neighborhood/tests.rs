use super::*;
use std::sync::{Condvar, Mutex};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Default)]
struct Fake {
    names: Vec<&'static str>,
    fail_root: bool,
    fail_list: bool,
    fail_after_row: bool,
    native: bool,
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Option<Arc<(Mutex<bool>, Condvar)>>,
}
fn address() -> NetworkAddress {
    NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([1]),
        display: "Network".into(),
    }
}
impl NetworkNeighborhood for Fake {
    fn root(&self, _: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        if self.fail_root {
            Err(failure("unavailable"))
        } else {
            Ok(address())
        }
    }
    fn parse(&self, _: &str, cancel: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        self.root(cancel)
    }
    fn parent(
        &self,
        _: &NetworkAddress,
        _: &Cancellation,
    ) -> Result<Option<NetworkAddress>, NetworkFailure> {
        Ok(None)
    }
    fn list(
        &self,
        _: &NetworkAddress,
        _: &Cancellation,
        emit: &mut dyn FnMut(NetworkNode) -> bool,
    ) -> Result<(), NetworkFailure> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        if let Some(gate) = &self.release {
            let (lock, ready) = gate.as_ref();
            let _guard = ready
                .wait_while(lock.lock().unwrap(), |released| !*released)
                .unwrap();
        }
        if self.fail_list {
            return Err(NetworkFailure::new(NetworkFailureKind::Offline, "offline"));
        }
        let names: &[&str] = if self.names.is_empty() {
            &["server"]
        } else {
            &self.names
        };
        for (index, name) in names.iter().enumerate() {
            let mut target = address();
            target.opaque = Arc::from((index as u64).to_le_bytes());
            if !emit(NetworkNode {
                address: target,
                display: (*name).into(),
            }) {
                return Ok(());
            }
        }
        if self.fail_after_row {
            return Err(NetworkFailure::new(NetworkFailureKind::Offline, "offline"));
        }
        Ok(())
    }
    fn activate(
        &self,
        address: &NetworkAddress,
        _: &Cancellation,
    ) -> Result<NetworkDestination, NetworkFailure> {
        if self.native {
            Ok(NetworkDestination::Native(std::path::PathBuf::from(env!(
                "CARGO_MANIFEST_DIR"
            ))))
        } else {
            Ok(NetworkDestination::Browse(address.clone()))
        }
    }
    fn refresh(&self, _: &NetworkAddress, _: &Cancellation) -> Result<(), NetworkFailure> {
        Ok(())
    }
}
fn controller(provider: Fake) -> (Controller, mpsc::Receiver<Event>, Snapshots) {
    let (sender, events) = mpsc::channel(16);
    let snapshots = Arc::new(Mutex::new(Default::default()));
    let loaded: OnceLock<Result<Arc<dyn NetworkNeighborhood>, NetworkFailure>> = OnceLock::new();
    loaded.set(Ok(Arc::new(provider))).unwrap();
    (
        Controller {
            locale: None,
            provider: Arc::new(loaded),
            ready: Arc::new(Notify::new()),
            budget: Arc::new(Semaphore::new(2)),
            cancellations: Default::default(),
            high_water: [None; 2],
            snapshots: snapshots.clone(),
            outbox: Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
        },
        events,
        snapshots,
    )
}
async fn drain(tasks: &mut JoinSet<()>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn early_failure_restores_the_previous_native_or_network_authority() {
    for previous in [None, Some(NavigationId(3))] {
        for fail_list in [false, true] {
            let (mut controller, mut events, snapshots) = controller(Fake {
                fail_root: !fail_list,
                fail_list,
                ..Default::default()
            });
            snapshots.lock().unwrap()[0].network_navigation = previous;
            let old_entries: Arc<[Entry]> = Arc::from([Entry::from_network(
                EntryId(9),
                NetworkNode {
                    address: address(),
                    display: "old server".into(),
                },
            )]);
            snapshots.lock().unwrap()[0].network =
                previous.map(|navigation| (navigation, old_entries.clone()));
            let mut tasks = JoinSet::new();
            controller.navigate(
                PaneId::Left,
                NavigationId(4),
                NetworkAction::OpenRoot,
                SortSpec::default(),
                &mut tasks,
            );
            drain(&mut tasks).await;
            assert!(
                matches!(events.try_recv().unwrap(), Event::NetworkUpdated { update, .. } if matches!(*update, NetworkUpdate::Failed(_)))
            );
            assert_eq!(snapshots.lock().unwrap()[0].network_navigation, previous);
            if previous.is_some() {
                assert!(Arc::ptr_eq(
                    &snapshots.lock().unwrap()[0].network.as_ref().unwrap().1,
                    &old_entries
                ));
            }
        }
    }
}

#[tokio::test]
async fn published_listing_keeps_native_operations_disabled() {
    let (mut controller, mut events, snapshots) = controller(Fake::default());
    let mut tasks = JoinSet::new();
    controller.navigate(
        PaneId::Left,
        NavigationId(4),
        NetworkAction::OpenRoot,
        SortSpec::default(),
        &mut tasks,
    );
    drain(&mut tasks).await;
    let mut last_entries = None;
    while let Ok(Event::NetworkUpdated { update, .. }) = events.try_recv() {
        if let NetworkUpdate::Listing { listing, .. } = *update {
            last_entries = Some(listing.entries);
        }
    }
    assert!(Arc::ptr_eq(
        &snapshots.lock().unwrap()[0].network.as_ref().unwrap().1,
        &last_entries.unwrap()
    ));
    assert_eq!(
        snapshots.lock().unwrap()[0].network_navigation,
        Some(NavigationId(4))
    );
    assert!(!snapshots.lock().unwrap()[0].has_native_view());
}

#[tokio::test]
async fn a_replaced_generation_cannot_publish_a_late_listing() {
    for (fail_list, cancel_request) in [(false, false), (true, false), (false, true), (true, true)]
    {
        let (entered, ready) = oneshot::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (mut controller, mut events, snapshots) = controller(Fake {
            fail_list,
            entered: Mutex::new(Some(entered)),
            release: Some(gate.clone()),
            ..Default::default()
        });
        let mut tasks = JoinSet::new();
        controller.navigate(
            PaneId::Left,
            NavigationId(4),
            NetworkAction::OpenRoot,
            SortSpec::default(),
            &mut tasks,
        );
        tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        if cancel_request {
            controller.cancel(PaneId::Left);
        } else {
            snapshots.lock().unwrap()[0].network_navigation = Some(NavigationId(5));
        }
        assert_eq!(
            controller.budget.available_permits(),
            1,
            "cancellation released a running call"
        );
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        drain(&mut tasks).await;
        assert!(events.try_recv().is_err(), "stale worker published");
        assert_eq!(
            snapshots.lock().unwrap()[0].network_navigation,
            Some(NavigationId(if cancel_request { 4 } else { 5 }))
        );
        assert!(snapshots.lock().unwrap()[0].network.is_none());
        assert_eq!(controller.budget.available_permits(), 2);
    }
}

#[tokio::test]
async fn partial_listing_error_keeps_the_new_view_and_typed_failure() {
    let (mut controller, mut events, snapshots) = controller(Fake {
        fail_after_row: true,
        ..Default::default()
    });
    let mut tasks = JoinSet::new();
    controller.navigate(
        PaneId::Left,
        NavigationId(4),
        NetworkAction::OpenRoot,
        SortSpec::default(),
        &mut tasks,
    );
    drain(&mut tasks).await;
    let mut partial_error = None;
    while let Ok(Event::NetworkUpdated { update, .. }) = events.try_recv() {
        if let NetworkUpdate::Listing {
            done: true, error, ..
        } = *update
        {
            partial_error = error;
        }
    }
    assert_eq!(partial_error.unwrap().kind, NetworkFailureKind::Offline);
    let states = snapshots.lock().unwrap();
    assert_eq!(states[0].network_navigation, Some(NavigationId(4)));
    assert_eq!(states[0].network.as_ref().unwrap().1.len(), 1);
}

#[tokio::test]
async fn native_handoff_keeps_network_authority_until_native_commit() {
    let (mut controller, mut events, snapshots) = controller(Fake {
        native: true,
        ..Default::default()
    });
    let mut tasks = JoinSet::new();
    controller.navigate(
        PaneId::Left,
        NavigationId(4),
        NetworkAction::Activate(address()),
        SortSpec::default(),
        &mut tasks,
    );
    drain(&mut tasks).await;
    assert!(
        matches!(events.try_recv().unwrap(), Event::NetworkUpdated { update, .. } if matches!(*update, NetworkUpdate::Native(_)))
    );
    assert_eq!(
        snapshots.lock().unwrap()[0].network_navigation,
        Some(NavigationId(4))
    );
    assert!(!snapshots.lock().unwrap()[0].has_native_view());
}

#[tokio::test]
async fn a_full_outbox_does_not_hold_the_snapshot_lock_or_publish_a_canceled_failure() {
    let (mut controller, mut events, snapshots) = controller(Fake {
        fail_root: true,
        ..Default::default()
    });
    for _ in 0..16 {
        controller
            .outbox
            .sender
            .try_send(Event::NetworkAvailability(true))
            .unwrap();
    }
    let mut tasks = JoinSet::new();
    controller.navigate(
        PaneId::Left,
        NavigationId(4),
        NetworkAction::OpenRoot,
        SortSpec::default(),
        &mut tasks,
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(snapshots.try_lock().is_ok());
    controller.cancel(PaneId::Left);
    drain(&mut tasks).await;
    for _ in 0..16 {
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::NetworkAvailability(true)
        ));
    }
    assert!(events.try_recv().is_err());
    assert_eq!(
        snapshots.lock().unwrap()[0].network_navigation,
        Some(NavigationId(4))
    );
}

#[tokio::test]
async fn failed_navigation_cannot_be_replayed_or_canceled_by_an_older_request() {
    let (mut controller, mut events, snapshots) = controller(Fake {
        fail_root: true,
        ..Default::default()
    });
    let mut tasks = JoinSet::new();
    controller.navigate(
        PaneId::Left,
        NavigationId(6),
        NetworkAction::OpenRoot,
        SortSpec::default(),
        &mut tasks,
    );
    drain(&mut tasks).await;
    events.try_recv().unwrap();
    assert_eq!(snapshots.lock().unwrap()[0].network_navigation, None);
    let original_cancel = controller.cancellations[0].clone();
    for navigation in [NavigationId(6), NavigationId(3)] {
        controller.navigate(
            PaneId::Left,
            navigation,
            NetworkAction::OpenRoot,
            SortSpec::default(),
            &mut tasks,
        );
    }
    drain(&mut tasks).await;
    assert!(events.try_recv().is_err(), "replayed failed navigation");
    assert!(
        !original_cancel.is_cancelled(),
        "stale request canceled the current owner"
    );
    controller.navigate(
        PaneId::Left,
        NavigationId(7),
        NetworkAction::OpenRoot,
        SortSpec::default(),
        &mut tasks,
    );
    drain(&mut tasks).await;
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::NetworkUpdated {
            navigation: NavigationId(7),
            ..
        }
    ));
}

#[test]
fn device_captions_use_backend_locale_keys_before_publication() {
    struct Locale;
    impl LocaleService for Locale {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn collation_key(&self, value: &std::ffi::OsStr) -> Result<Option<Vec<u8>>, FsError> {
            Ok(Some(vec![match value.to_str().unwrap() {
                "Álfa" => 0,
                "Beta" => 1,
                other => panic!("unexpected caption {other}"),
            }]))
        }
    }
    let mut final_names = Vec::new();
    run(
        &Fake {
            names: vec!["Beta", "Álfa"],
            ..Default::default()
        },
        NetworkAction::Browse(address()),
        SortSpec::default(),
        Some(&Locale),
        &Cancellation::default(),
        &mut |update| {
            if let NetworkUpdate::Listing {
                listing,
                done: true,
                ..
            } = update
            {
                final_names = listing
                    .entries
                    .iter()
                    .map(|entry| entry.display_name.clone())
                    .collect();
            }
            true
        },
    )
    .unwrap();
    assert_eq!(final_names, ["Álfa", "Beta"]);
}
