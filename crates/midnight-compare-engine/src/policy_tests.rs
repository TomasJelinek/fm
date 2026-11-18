//! Policy admission and ownership through the real comparison Service.
//! No native capture, compiled matcher or backend retirement worker is simulated.
use super::*;
use crate::policy_test_support::FakePolicySource;
use midnight_core::{
    ExternalCompareOutcome, ExternalCompareProcess, ExternalCompareSessionCredit,
    FileComparatorPreferences, FileComparatorSnapshot, FileComparatorTarget, FileCompareSource,
    FsError,
};
use std::{
    io,
    path::Path,
    sync::mpsc::{self, SyncSender},
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(5);
static DISCOVERY_CALLS: AtomicUsize = AtomicUsize::new(0);

fn forbidden_discovery() -> Result<Vec<midnight_image_viewer_host::Provider>, String> {
    DISCOVERY_CALLS.fetch_add(1, Ordering::AcqRel);
    Err("Unexpected image discovery during policy admission.".into())
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(2)
        .build()
        .unwrap()
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "policy Service test did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn launch(snapshot: &FileComparatorSnapshot) -> FileCompareLaunch {
    FileCompareLaunch {
        sources: vec![
            FileCompareSource::Unspecified,
            FileCompareSource::Unspecified,
        ],
        disabled_plugins: Arc::from([]),
        comparators: snapshot.preferences.clone(),
        comparator_policy: snapshot.id,
    }
}

fn open(service: &Service, launch: FileCompareLaunch) -> Result<Session, OpenError> {
    service.open(
        launch,
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [30, 30, 30, 255],
            selection: [50, 80, 120, 255],
            accent: [90, 150, 220, 255],
        },
        ViewerGeometry {
            width: 320,
            height: 160,
            scale: 1.0,
        },
    )
}

fn refused(result: Result<Session, OpenError>) -> OpenError {
    match result {
        Err(error) => error,
        Ok(session) => {
            session.close();
            panic!("expected synchronous policy admission refusal")
        }
    }
}

struct NoIoLauncher(AtomicUsize);

impl ExternalCompareLauncher for NoIoLauncher {
    fn submit(
        &self,
        _process: ExternalCompareProcess,
        _paths: [PathBuf; 2],
        _session: Arc<dyn ExternalCompareSessionCredit>,
        _cancel: Cancellation,
        _reply: SyncSender<ExternalCompareOutcome>,
    ) -> Result<(), FsError> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Err(FsError::from_io(
            "policy test",
            Path::new(""),
            io::Error::other("unexpected launch"),
        ))
    }
}

fn service(
    runtime: &tokio::runtime::Runtime,
) -> (
    midnight_work_budget::resident::Owner,
    Service,
    Arc<AtomicUsize>,
    Arc<NoIoLauncher>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let capture_calls = calls.clone();
    let capture: NativeCapture = Arc::new(move |_, _| {
        capture_calls.fetch_add(1, Ordering::AcqRel);
        Err("Unexpected built-in capture during policy admission.".into())
    });
    let launcher = Arc::new(NoIoLauncher(AtomicUsize::new(0)));
    let (owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_external_launcher(launcher.clone());
    let service = service.with_image_viewer_discovery(forbidden_discovery);
    (owner, service, calls, launcher)
}

fn no_io(captures: &AtomicUsize, launcher: &NoIoLauncher) {
    assert_eq!(captures.load(Ordering::Acquire), 0);
    assert_eq!(DISCOVERY_CALLS.load(Ordering::Acquire), 0);
    assert_eq!(launcher.0.load(Ordering::Acquire), 0);
}

fn close(session: &Session, service: &Service) {
    session.close();
    wait_for(|| service.active.load(Ordering::Acquire) == 0);
}

#[test]
fn unconfigured_service_accepts_only_the_exact_default_automatic_snapshot() {
    let runtime = runtime();
    let (_owner, service, captures, launcher) = service(&runtime);
    let expected = FileComparatorSnapshot::default();
    let actual = service.comparator_snapshot();
    assert_eq!(actual.id, expected.id);
    assert!(Arc::ptr_eq(&actual.preferences, &expected.preferences));
    let session = open(&service, launch(&actual)).unwrap();
    assert_eq!(service.active.load(Ordering::Acquire), 1);
    close(&session, &service);

    let equal_value_but_not_default_identity = FileComparatorSnapshot {
        id: expected.id,
        preferences: Arc::new(FileComparatorPreferences::default()),
    };
    assert!(matches!(
        refused(open(
            &service,
            launch(&equal_value_but_not_default_identity)
        )),
        OpenError::Unavailable(_)
    ));
    assert_eq!(service.active.load(Ordering::Acquire), 0);
    no_io(&captures, &launcher);
}

#[test]
fn configured_nondefault_without_source_is_unavailable_never_a_stale_default_refresh() {
    let runtime = runtime();
    let (_owner, service, captures, launcher) = service(&runtime);
    let fixture = FakePolicySource::new();
    let (configured, _) = fixture.publish(FileComparatorTarget::BuiltInText);
    let default = FileComparatorSnapshot::default();
    let wrong_id_default_raw = FileComparatorSnapshot {
        id: configured.id,
        preferences: default.preferences,
    };
    for snapshot in [configured, wrong_id_default_raw] {
        assert!(matches!(
            refused(open(&service, launch(&snapshot))),
            OpenError::Unavailable(_)
        ));
        assert_eq!(service.active.load(Ordering::Acquire), 0);
    }
    no_io(&captures, &launcher);
}

#[test]
fn installed_source_pins_even_its_exact_default_snapshot() {
    let runtime = runtime();
    let source = FakePolicySource::new();
    let (_owner, service, captures, launcher) = service(&runtime);
    let service = service.with_comparator_policies(source.clone());
    let snapshot = service.comparator_snapshot();
    assert_eq!(source.snapshot_reads.load(Ordering::Acquire), 1);
    let session = open(&service, launch(&snapshot)).unwrap();
    assert_eq!(source.pins.load(Ordering::Acquire), 1);
    assert_eq!(source.live_pins.load(Ordering::Acquire), 1);
    close(&session, &service);
    wait_for(|| source.live_pins.load(Ordering::Acquire) == 0);
    no_io(&captures, &launcher);
}

#[test]
fn two_session_refusal_precedes_a_third_policy_pin() {
    let runtime = runtime();
    let source = FakePolicySource::new();
    let (snapshot, _) = source.publish(FileComparatorTarget::Automatic);
    let (_owner, service, captures, launcher) = service(&runtime);
    let service = service.with_comparator_policies(source.clone());
    let first = open(&service, launch(&snapshot)).unwrap();
    let second = open(&service, launch(&snapshot)).unwrap();
    assert!(matches!(
        refused(open(&service, launch(&snapshot))),
        OpenError::SessionLimit
    ));
    assert_eq!(source.pins.load(Ordering::Acquire), 2);
    assert_eq!(source.credits.lock().unwrap().len(), 2);
    assert_eq!(service.active.load(Ordering::Acquire), 2);
    first.close();
    second.close();
    wait_for(|| service.active.load(Ordering::Acquire) == 0);
    wait_for(|| source.live_pins.load(Ordering::Acquire) == 0);
    no_io(&captures, &launcher);
}

#[test]
fn retirement_busy_is_unavailable_and_returns_every_unused_credit_immediately() {
    let runtime = runtime();
    let source = FakePolicySource::new();
    source.set_pin_error(Some(FileComparatorPinError::RetirementBusy));
    let (_owner, service, captures, launcher) = service(&runtime);
    let service = service.with_comparator_policies(source.clone());
    let snapshot = service.comparator_snapshot();
    assert!(matches!(
        refused(open(&service, launch(&snapshot))),
        OpenError::Unavailable(_)
    ));
    assert_eq!(source.pins.load(Ordering::Acquire), 1);
    assert_eq!(
        source.snapshot_reads.load(Ordering::Acquire),
        1,
        "no automatic refresh"
    );
    assert_eq!(service.active.load(Ordering::Acquire), 0);
    assert!(source.credits.lock().unwrap()[0].upgrade().is_none());
    assert_eq!(source.live_pins.load(Ordering::Acquire), 0);

    source.set_pin_error(None);
    let retry = open(&service, launch(&snapshot)).unwrap();
    assert_eq!(source.pins.load(Ordering::Acquire), 2);
    close(&retry, &service);
    no_io(&captures, &launcher);
}

#[test]
fn exact_id_and_raw_arc_stale_refusals_return_the_second_credit_before_one_retry() {
    let runtime = runtime();
    let source = FakePolicySource::new();
    let (old, old_probe) = source.publish(FileComparatorTarget::Automatic);
    let (_owner, service, captures, launcher) = service(&runtime);
    let service = service.with_comparator_policies(source.clone());
    let first = open(&service, launch(&old)).unwrap();
    let (current, current_probe) = source.publish(FileComparatorTarget::BuiltInText);
    let rebuilt_raw = FileComparatorSnapshot {
        id: current.id,
        preferences: Arc::new(current.preferences.as_ref().clone()),
    };
    let wrong_id_same_raw = FileComparatorSnapshot {
        id: old.id,
        preferences: current.preferences.clone(),
    };
    let snapshots_before = source.snapshot_reads.load(Ordering::Acquire);
    for stale in [old, rebuilt_raw, wrong_id_same_raw] {
        assert!(matches!(
            refused(open(&service, launch(&stale))),
            OpenError::PolicyChanged
        ));
        // Synchronous assertion: do not wait for cleanup before observing credit.
        assert_eq!(service.active.load(Ordering::Acquire), 1);
        assert_eq!(source.live_pins.load(Ordering::Acquire), 1);
        assert!(
            source
                .credits
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .upgrade()
                .is_none()
        );
    }
    assert_eq!(
        source.snapshot_reads.load(Ordering::Acquire),
        snapshots_before
    );
    assert_eq!(source.pins.load(Ordering::Acquire), 4);
    assert!(old_probe.seen.lock().unwrap().is_empty());
    assert!(current_probe.seen.lock().unwrap().is_empty());
    no_io(&captures, &launcher);

    // The chooser is resident. A stale open must leave transient work runnable.
    let (sent, received) = mpsc::sync_channel(1);
    runtime.handle().spawn_blocking(move || {
        let _ = sent.try_send(());
    });
    received.recv_timeout(TIMEOUT).unwrap();

    // The caller's ONE explicit refresh uses BOTH fields from one new snapshot.
    let refreshed = service.comparator_snapshot();
    assert_eq!(refreshed.id, current.id);
    assert!(Arc::ptr_eq(&refreshed.preferences, &current.preferences));
    let second = open(&service, launch(&refreshed)).unwrap();
    assert_eq!(source.pins.load(Ordering::Acquire), 5);
    assert_eq!(service.active.load(Ordering::Acquire), 2);
    first.close();
    second.close();
    wait_for(|| service.active.load(Ordering::Acquire) == 0);
    wait_for(|| source.live_pins.load(Ordering::Acquire) == 0);
}

#[test]
fn retained_erased_session_credit_cannot_retain_the_historical_policy_owner() {
    let runtime = runtime();
    let source = FakePolicySource::new();
    let (snapshot, probe) = source.publish(FileComparatorTarget::Automatic);
    let weak_probe = Arc::downgrade(&probe);
    let (_owner, service, captures, launcher) = service(&runtime);
    let service = service.with_comparator_policies(source.clone());
    let session = open(&service, launch(&snapshot)).unwrap();
    let credit = source.credits.lock().unwrap()[0].upgrade().unwrap();
    source.publish(FileComparatorTarget::BuiltInText);
    drop(probe);
    session.close();
    wait_for(|| source.live_pins.load(Ordering::Acquire) == 0);
    wait_for(|| weak_probe.upgrade().is_none());
    assert_eq!(source.dropped_pins.load(Ordering::Acquire), 1);
    assert_eq!(
        service.active.load(Ordering::Acquire),
        1,
        "credit contains only admission"
    );
    drop(credit);
    wait_for(|| service.active.load(Ordering::Acquire) == 0);
    no_io(&captures, &launcher);
}

// Manual retry across saves is covered once, using this same fake source, by
// external_routes_tests::manual_retry_uses_the_original_session_policy_after_live_policy_changes.
// Real Engine reservation/compiled-drop/thread guarantees, including a blocked
// fm-retirement worker and a stale second open, are Backend's separate gates.
