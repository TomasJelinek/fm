//! A closed comparator window is released, with its session and any output left in its
//! mailbox, on the next turn of the event loop: never inside its own callback, and
//! without waiting for another comparator to open. Any number of comparators open; a
//! closing one, still tearing down in the engine, never stands in the way of the next.
use super::identity_tests::{TestRuntime, platform, runtime, text_service};
use super::*;
use midnight_compare_engine::{
    FileComparatorPinError, FileComparatorPolicySource, OwnedFileComparatorPolicy,
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
    manager.close_all();
}

/// The owner's decision of 26 November 2026: no limit on the number of comparators.
#[test]
fn six_comparators_open_at_once() {
    let _native = platform();
    let runtime = runtime();
    let (service, _) = text_service(&runtime);
    let manager = ComparatorWindows::new_polled(service.clone());
    let opened: Vec<_> = (0..6).map(|_| open(&manager, &service).unwrap()).collect();
    assert_eq!(manager.windows.borrow().len(), 6);
    drop(opened);
    manager.close_all();
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

/// Backlog COMPARE-03, now without a limit: a closed comparator whose session the engine
/// is still tearing down never stands in the way of the next opening.
#[test]
fn a_closing_comparator_does_not_hold_back_the_next_opening() {
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
    // Its teardown is held, and further comparators open beside it at once.
    std::thread::sleep(Duration::from_millis(50));
    let _third = open(&manager, &service).expect("admitted while another tears down");
    let _fourth = open(&manager, &service).expect("admitted while another tears down");
    assert_eq!(manager.windows.borrow().len(), 3);
    drop(held);
    manager.close_all();
}
