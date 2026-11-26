//! A closed comparator window is released, with its session and any output left in its
//! mailbox, on the next turn of the event loop: never inside its own callback, and
//! without waiting for another comparator to open. Until the engine has released a
//! closed comparator's session, the window list counts it as the engine does.
use super::identity_tests::{TestRuntime, platform, runtime, text_service};
use super::*;
use midnight_compare_engine::{
    FileComparatorPinError, FileComparatorPolicySource, OpenError, OwnedFileComparatorPolicy,
};
use midnight_core::{
    ExternalCompareSessionCredit, FileComparatorPolicyStore, FileComparatorPreferences,
    FileComparatorSnapshot,
};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
};

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 12.0,
        foreground: [240, 240, 240, 255],
        background: [24, 28, 36, 255],
        selection: [60, 80, 120, 255],
        selection_text: [0; 4],
        accent: [100, 150, 220, 255],
    }
}

/// A comparison without files: its window asks for them.
fn launch(service: &Service) -> FileCompareLaunch {
    let snapshot = service.comparator_snapshot();
    FileCompareLaunch {
        sources: Vec::new(),
        disabled_plugins: Arc::from([]),
        comparators: snapshot.preferences.clone(),
        comparator_policy: snapshot.id,
    }
}

/// Open a comparator without files (it asks for them) and return the window it added.
fn open(manager: &ComparatorWindows, service: &Service) -> Result<Rc<OpenWindow>, String> {
    manager.open(
        launch(service),
        appearance(),
        slint::LogicalSize::new(640.0, 400.0),
        Rc::new(|_, _| None),
    )?;
    Ok(manager.windows.borrow().last().unwrap().clone())
}

#[test]
fn a_closed_comparator_is_released_on_the_next_loop_turn() {
    let _native = platform();
    let runtime = runtime();
    let (service, _) = text_service(&runtime);
    let manager = ComparatorWindows::new_polled(service.clone());
    for close in ["Close button", "title bar"] {
        let open = open(&manager, &service).unwrap();
        let (window, released) = (open.window.as_weak(), Rc::downgrade(&open));
        drop(open);
        // The worker's chooser waits unread in the mailbox: no timer has run.
        std::thread::sleep(Duration::from_millis(100));
        let shown = window.upgrade().unwrap();
        match close {
            "Close button" => shown.invoke_close_comparator(),
            _ => shown
                .window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested),
        }
        drop(shown);
        // Still alive right after its own callback returned.
        assert!(
            released.upgrade().is_some(),
            "{close}: kept through its callback"
        );
        slint::platform::update_timers_and_animations();
        assert!(
            released.upgrade().is_none(),
            "{close}: the window is released"
        );
        assert!(
            window.upgrade().is_none(),
            "{close}: the component is released"
        );
        assert!(manager.windows.borrow().is_empty(), "{close}");
    }
    // The two-window limit counts open windows only.
    let first = open(&manager, &service).unwrap();
    let _second = open(&manager, &service).unwrap();
    assert!(open(&manager, &service).is_err(), "a third is refused");
    first.window.invoke_close_comparator();
    drop(first);
    slint::platform::update_timers_and_animations();
    assert_eq!(manager.windows.borrow().len(), 1);
    // The engine counts the closed session until its worker has torn down, a moment
    // after the window is released, and the window list counts it with the engine: once
    // the engine reports the session released, the next opening is admitted.
    released_by_engine(&manager, 1);
    let _third = open(&manager, &service).expect("admitted once the engine released it");
    manager.close_all();
}

/// Wait for the engine to release closed comparators' sessions until `counted` remain.
fn released_by_engine(manager: &ComparatorWindows, counted: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while manager.counted() > counted {
        assert!(
            std::time::Instant::now() < deadline,
            "the engine never released a closed comparator's session"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Whether a closed comparator's worker is held in its teardown. While held, the policy
/// it pinned keeps the session's admission credit (`HeldPolicy`'s `Drop`).
#[derive(Default)]
struct Teardown {
    held: Mutex<bool>,
    changed: Condvar,
}

impl Teardown {
    /// Hold teardowns until the returned guard is dropped, also when a test fails.
    fn hold(self: &Arc<Self>) -> Held {
        self.set(true);
        Held(self.clone())
    }

    fn set(&self, held: bool) {
        *self.held.lock().unwrap_or_else(|error| error.into_inner()) = held;
        self.changed.notify_all();
    }
}

struct Held(Arc<Teardown>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

struct HeldSource {
    snapshot: FileComparatorSnapshot,
    teardown: Arc<Teardown>,
}

/// Dropped on the comparison worker when it ends; the credit goes after this `Drop`.
struct HeldPolicy {
    snapshot: FileComparatorSnapshot,
    teardown: Arc<Teardown>,
    _session: Arc<dyn ExternalCompareSessionCredit>,
}

impl Drop for HeldPolicy {
    fn drop(&mut self) {
        let held = self
            .teardown
            .held
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _released =
            self.teardown
                .changed
                .wait_timeout_while(held, Duration::from_secs(30), |held| *held);
    }
}

impl FileComparatorPolicySource for HeldSource {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.snapshot.clone()
    }

    fn pin(
        &self,
        _expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<Box<dyn OwnedFileComparatorPolicy>, FileComparatorPinError> {
        Ok(Box::new(HeldPolicy {
            snapshot: self.snapshot.clone(),
            teardown: self.teardown.clone(),
            _session: session,
        }))
    }
}

impl OwnedFileComparatorPolicy for HeldPolicy {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.snapshot.clone()
    }

    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.snapshot.preferences
    }

    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        None
    }
}

fn held_service(runtime: &TestRuntime) -> (Service, Arc<Teardown>) {
    let teardown = Arc::new(Teardown::default());
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        runtime.spawner(),
        Arc::new(|_, _| Err("Rejected by UI test capture fixture.".into())),
    )
    .with_comparator_policies(Arc::new(HeldSource {
        snapshot: FileComparatorSnapshot {
            id: FileComparatorPolicyStore::default().prepare_id().unwrap(),
            preferences: Arc::default(),
        },
        teardown: teardown.clone(),
    }));
    (service, teardown)
}

/// Whether the engine itself admits an opening now (the session is dropped at once).
fn engine_admits(service: &Service) -> bool {
    let geometry = ViewerGeometry {
        width: 640,
        height: 300,
        scale: 1.0,
    };
    match service.open(launch(service), appearance(), geometry) {
        Ok(_) => true,
        Err(OpenError::SessionLimit) => false,
        Err(error) => panic!("{error}"),
    }
}

/// Backlog COMPARE-03: the engine counts a closed comparator's session until its worker
/// has torn down, after its window is released. The window list counts it too: an
/// opening in that moment is refused by the window list as the engine would refuse it,
/// and admitted once the engine has released the session.
#[test]
fn a_closing_comparator_is_counted_until_the_engine_releases_it() {
    let _native = platform();
    let runtime = runtime();
    let (service, teardown) = held_service(&runtime);
    let manager = ComparatorWindows::new_polled(service.clone());
    let first = open(&manager, &service).unwrap();
    let _second = open(&manager, &service).unwrap();
    let held = teardown.hold();
    first.window.invoke_close_comparator();
    drop(first);
    slint::platform::update_timers_and_animations();
    assert_eq!(manager.windows.borrow().len(), 1, "the window is released");
    // Its teardown is held: the window list and the engine both still count it.
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        manager.counted(),
        2,
        "the window list counts the closing one"
    );
    assert!(!engine_admits(&service), "the engine counts it");
    assert_eq!(
        open(&manager, &service).err().as_deref(),
        Some("Close a comparator before opening another (maximum two).")
    );
    assert_eq!(manager.windows.borrow().len(), 1);
    drop(held);
    released_by_engine(&manager, 1);
    let _third = open(&manager, &service).expect("admitted once the engine released it");
    assert_eq!((manager.windows.borrow().len(), manager.counted()), (2, 2));
    manager.close_all();
}
