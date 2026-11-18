//! Idle wakeups of the native clipboard worker (review finding E6-4). The measurement with
//! the real Windows session is in `clipboard_wake_windows_tests.rs`.
use super::*;
use std::{
    sync::{Condvar, MutexGuard},
    time::Instant,
};

/// Elapses only when an expected step of the worker does not happen.
const FAILURE_WAIT: Duration = Duration::from_secs(10);

/// What the fake native owner observed, and what the test lets it do.
#[derive(Debug, Default)]
struct State {
    /// The session offers a waker (as on Windows and Linux) or not (a session without a
    /// wake event).
    wakes: bool,
    owns: bool,
    fail_waits: bool,
    /// Wake every newly opened session before its first wait. This forces what a submit's
    /// wake does when it races the worker: the worker can take the request, open a session
    /// and install its waker before the controller calls the waker, which then ends the
    /// first idle wait of that session early. The waker contract allows such a wake.
    wake_on_open: bool,
    /// The sticky wake: set by the waker, consumed by the next wait.
    woken: bool,
    /// Waits the test lets end as elapsed timeouts; only then does virtual time pass.
    expire: usize,
    /// Hold the next wait after the worker found its inbox empty, before it waits.
    hold: bool,
    held: bool,
    waiting: bool,
    timeouts: Vec<Duration>,
    ended_by_wake: usize,
    ended_by_expiry: usize,
    /// A wait outlived its failure-only deadline.
    stuck: bool,
    services: usize,
    opened: usize,
    closed: usize,
    copied: Vec<String>,
}

/// Counters of the fake owner; tests compare them with a baseline taken while idle.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Counts {
    waits: usize,
    services: usize,
    ended_by_wake: usize,
    ended_by_expiry: usize,
}
impl State {
    fn counts(&self) -> Counts {
        Counts {
            waits: self.timeouts.len(),
            services: self.services,
            ended_by_wake: self.ended_by_wake,
            ended_by_expiry: self.ended_by_expiry,
        }
    }
    /// What happened since `base`.
    fn since(&self, base: Counts) -> Counts {
        let now = self.counts();
        Counts {
            waits: now.waits - base.waits,
            services: now.services - base.services,
            ended_by_wake: now.ended_by_wake - base.ended_by_wake,
            ended_by_expiry: now.ended_by_expiry - base.ended_by_expiry,
        }
    }
    fn waits_only_with(&self, timeout: Duration) -> bool {
        self.timeouts.iter().all(|waited| *waited == timeout)
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
impl Shared {
    fn new(setup: impl FnOnce(&mut State)) -> Arc<Self> {
        let shared = Arc::new(Self::default());
        shared.update(setup);
        shared
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let result = change(&mut self.lock());
        self.changed.notify_all();
        result
    }
    /// One bounded wait for a change; marks the owner stuck once `deadline` has passed.
    fn wait<'a>(
        &self,
        mut state: MutexGuard<'a, State>,
        deadline: Instant,
    ) -> MutexGuard<'a, State> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            state.stuck = true;
            return state;
        }
        let (state, _) = self
            .changed
            .wait_timeout(state, left)
            .unwrap_or_else(PoisonError::into_inner);
        state
    }
    /// Block until `done` holds, failing after `FAILURE_WAIT`; returns that state.
    fn until(&self, what: &str, done: impl Fn(&State) -> bool) -> MutexGuard<'_, State> {
        let deadline = Instant::now() + FAILURE_WAIT;
        let mut state = self.lock();
        while !done(&state) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let report = format!("timed out waiting until {what}: {state:?}");
                drop(state);
                panic!("{report}");
            }
            let (next, _) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner);
            state = next;
        }
        state
    }
    /// Block until the worker idles in a wait with no wake pending; returns that state.
    /// Call it after a submit returned: the controller called the waker before returning,
    /// so a wake that raced the worker onto a new session has ended a wait already or is
    /// still pending, and nothing but the test ends the wait found here.
    fn idle(&self, what: &str) -> MutexGuard<'_, State> {
        self.until(what, |state| state.waiting && !state.woken)
    }
}

struct Owner(Arc<Shared>);
impl ClipboardService for Owner {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        self.0.update(|state| {
            state.opened += 1;
            state.woken |= state.wake_on_open;
        });
        Ok(Box::new(Session(self.0.clone())))
    }
}
/// A native owner whose waits end only through its waker or a test-granted expiry.
struct Session(Arc<Shared>);
impl ClipboardSession for Session {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError> {
        self.0.update(|state| state.copied.push(text.into()));
        Ok(())
    }
    fn service_pending(&mut self) -> Result<(), FsError> {
        self.0.update(|state| state.services += 1);
        Ok(())
    }
    fn owns_clipboard(&self) -> bool {
        self.0.lock().owns
    }
    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        let shared = &self.0;
        let deadline = Instant::now() + 2 * FAILURE_WAIT;
        let mut state = shared.lock();
        state.timeouts.push(timeout);
        if state.hold {
            state.held = true;
            shared.changed.notify_all();
            while state.hold && !state.stuck {
                state = shared.wait(state, deadline);
            }
            state.held = false;
        }
        state.waiting = true;
        shared.changed.notify_all();
        while !state.stuck {
            if std::mem::take(&mut state.woken) {
                state.ended_by_wake += 1;
                break;
            }
            if state.expire > 0 {
                state.expire -= 1;
                state.ended_by_expiry += 1;
                break;
            }
            state = shared.wait(state, deadline);
        }
        state.waiting = false;
        // A stuck wait fails, so that a broken worker closes this session instead of spinning.
        let failed = state.fail_waits || state.stuck;
        drop(state);
        shared.changed.notify_all();
        if failed {
            Err(error(io::ErrorKind::Other, "The native wait failed."))
        } else {
            Ok(())
        }
    }
    fn waker(&self) -> Option<ClipboardWaker> {
        let shared = self.0.clone();
        let wakes = self.0.lock().wakes;
        wakes.then(|| Arc::new(move || shared.update(|state| state.woken = true)) as ClipboardWaker)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.0.update(|state| state.closed += 1);
    }
}

/// A controller with the fake owner, driven from a plain test thread. Fields drop in
/// order: the controller stops and wakes the worker first (also when a failed assertion
/// unwinds), so the worker thread ends by itself; the runtime does not wait for it.
struct Harness {
    controller: Option<Controller>,
    tasks: ResidentTasks,
    events: mpsc::Receiver<Event>,
    _owner: midnight_work_budget::resident::Owner,
    runtime: tokio::runtime::Runtime,
}
impl Harness {
    fn new(shared: &Arc<Shared>) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, events) = mpsc::channel(4);
        let (owner, controller) = Controller::testing(
            Some(Arc::new(Owner(shared.clone()))),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
        );
        Self {
            runtime,
            controller: Some(controller),
            tasks: ResidentTasks::default(),
            events,
            _owner: owner,
        }
    }
    /// Submit a copy; returns after the controller queued it and called the waker.
    fn copy(&mut self, id: u64, text: &str) {
        let Self {
            runtime,
            controller,
            tasks,
            ..
        } = self;
        let controller = controller.as_mut().expect("running controller");
        runtime.block_on(controller.copy(DesktopRequestId(id), text.into(), tasks));
    }
    fn copied(&mut self, id: u64) {
        let Self {
            runtime, events, ..
        } = self;
        let event =
            runtime.block_on(async { tokio::time::timeout(FAILURE_WAIT, events.recv()).await });
        assert!(
            matches!(
                &event,
                Ok(Some(Event::DesktopActionFinished {
                    request,
                    result: Ok(DesktopActionResult::ClipboardCopied),
                })) if *request == DesktopRequestId(id)
            ),
            "request {id} was not served promptly: {event:?}"
        );
    }
    fn wake_installed(&self) -> bool {
        let controller = self.controller.as_ref().expect("running controller");
        controller.wake.0.lock().unwrap().is_some()
    }
    fn shutdown(mut self) {
        self.controller
            .take()
            .expect("running controller")
            .shutdown();
        let Self { runtime, tasks, .. } = &mut self;
        let finished = runtime.block_on(async {
            tokio::time::timeout(FAILURE_WAIT, async {
                while tasks.join_next().await.is_some() {}
            })
            .await
        });
        assert!(finished.is_ok(), "the native worker did not stop");
    }
}

/// Each controller test runs once as usual and once with a submit wake that raced the
/// worker onto the session it opened for that request (`State::wake_on_open`).
const RACING_WAKE: [bool; 2] = [false, true];

#[test]
fn idle_session_with_a_waker_wakes_only_when_its_safety_timeout_expires() {
    for racing_wake in RACING_WAKE {
        let shared = Shared::new(|state| {
            state.wakes = true;
            state.owns = true;
            state.wake_on_open = racing_wake;
        });
        let mut harness = Harness::new(&shared);
        harness.copy(1, "one");
        harness.copied(1);
        assert!(harness.wake_installed());
        let base = {
            let state = shared.idle("the idle worker waits");
            // Only the submit's wake may have ended an earlier wait; no timeout expired.
            assert!(state.waits_only_with(IDLE_WAIT), "{state:?}");
            assert_eq!(state.ended_by_expiry, 0, "{state:?}");
            if racing_wake {
                assert!(
                    state.ended_by_wake >= 1,
                    "the racing wake was not delivered"
                );
            }
            state.counts()
        };
        // One virtual minute passes; each wakeup is an expired safety timeout (a 10 ms pump
        // wakes up to 6,000 times in that minute).
        assert!(IDLE_WAIT >= Duration::from_secs(1));
        let expiries = (60 / IDLE_WAIT.as_secs()) as usize;
        shared.update(|state| state.expire = expiries);
        let minute = {
            let state = shared.until("a virtual minute passed", |state| {
                state.ended_by_expiry == expiries && state.waiting
            });
            assert_eq!(
                state.since(base),
                Counts {
                    waits: expiries,
                    services: expiries,
                    ended_by_wake: 0,
                    ended_by_expiry: expiries,
                },
                "one service pass per expired wait and no wakeup between them: {state:?}"
            );
            assert!(state.waits_only_with(IDLE_WAIT), "{state:?}");
            assert_eq!((state.opened, state.closed), (1, 0), "{state:?}");
            state.counts()
        };
        harness.shutdown();
        let state = shared.until("the session closed", |state| state.closed == 1);
        assert_eq!(
            state.since(minute).ended_by_wake,
            1,
            "Controller drop wakes the idle worker: {state:?}"
        );
        assert!(!state.stuck);
    }
}

#[test]
fn queued_requests_end_an_idle_wait_through_the_waker() {
    for racing_wake in RACING_WAKE {
        let shared = Shared::new(|state| {
            state.wakes = true;
            state.owns = true;
            state.wake_on_open = racing_wake;
        });
        let mut harness = Harness::new(&shared);
        harness.copy(1, "one");
        harness.copied(1);
        let base = shared.idle("the idle worker waits").counts();
        // Queued while the worker waits, which only the waker or an expiry can end; no
        // expiry is granted.
        harness.copy(2, "two");
        harness.copied(2);
        let base = {
            let state = shared.idle("the worker waits again");
            let since = state.since(base);
            assert_eq!(
                (since.waits, since.ended_by_wake, since.ended_by_expiry),
                (1, 1, 0),
                "the wake ended the wait, then the worker waited again: {state:?}"
            );
            state.counts()
        };
        // Queued after the worker found its inbox empty but before it waits (the
        // lost-wakeup window): the earlier wake must end the wait that follows. The expiry
        // ends the current wait, so that the worker reaches that window.
        shared.update(|state| {
            state.hold = true;
            state.expire = 1;
        });
        drop(shared.until(
            "the worker is between its inbox check and its wait",
            |state| state.held,
        ));
        harness.copy(3, "three");
        shared.update(|state| state.hold = false);
        harness.copied(3);
        let state = shared.idle("the worker waits again");
        let since = state.since(base);
        assert_eq!(
            (since.waits, since.ended_by_wake, since.ended_by_expiry),
            (2, 1, 1),
            "the expiry ended one wait, the earlier wake the held one: {state:?}"
        );
        assert!(state.waits_only_with(IDLE_WAIT), "{state:?}");
        assert_eq!(state.copied, ["one", "two", "three"]);
        assert_eq!((state.opened, state.closed), (1, 0), "{state:?}");
        assert!(!state.stuck);
        drop(state);
        harness.shutdown();
    }
}

#[test]
fn session_that_stops_owning_is_closed_and_the_worker_waits_on_its_inbox() {
    for racing_wake in RACING_WAKE {
        let shared = Shared::new(|state| {
            state.wakes = true;
            state.owns = true;
            state.wake_on_open = racing_wake;
        });
        let mut harness = Harness::new(&shared);
        harness.copy(1, "one");
        harness.copied(1);
        let base = shared.idle("the idle worker waits").counts();
        // The platform's ownership-loss event (Windows: WM_DESTROYCLIPBOARD) ends the wait.
        shared.update(|state| {
            state.owns = false;
            state.woken = true;
        });
        {
            let state = shared.until("the session closed", |state| state.closed == 1);
            assert_eq!(
                state.since(base).waits,
                0,
                "a closed session is not waited on: {state:?}"
            );
        }
        assert!(
            !harness.wake_installed(),
            "the controller keeps no waker of a closed session"
        );
        // The worker blocks on its inbox; the next request opens a new session.
        shared.update(|state| state.owns = true);
        harness.copy(2, "two");
        harness.copied(2);
        let state = shared.idle("the new session waits");
        assert_eq!((state.opened, state.closed), (2, 1), "{state:?}");
        assert!(state.waits_only_with(IDLE_WAIT), "{state:?}");
        assert_eq!(state.copied, ["one", "two"]);
        assert!(!state.stuck);
        drop(state);
        harness.shutdown();
    }
}

/// Cancels the worker and lets its current wait expire, also when the driver panics.
struct Release(Arc<Shared>, Cancellation);
impl Drop for Release {
    fn drop(&mut self) {
        self.1.cancel();
        self.0.update(|state| state.expire += 1);
    }
}

#[test]
fn session_without_a_waker_keeps_the_ten_millisecond_pump() {
    let shared = Shared::new(|state| {
        state.owns = true;
        state.expire = 3;
    });
    let (_sender, mut receiver) = mpsc::channel::<Request>(1);
    let cancel = Cancellation::default();
    let mut native = Native::new(Arc::default());
    native.open(Box::new(Session(shared.clone())));
    assert!(!native.wakes);
    let driver = {
        let (shared, cancel) = (shared.clone(), cancel.clone());
        std::thread::spawn(move || {
            let _release = Release(shared.clone(), cancel);
            let state = shared.until("three pumped waits and a fourth", |state| {
                state.ended_by_expiry == 3 && state.waiting
            });
            (state.timeouts.clone(), state.services)
        })
    };
    assert!(next_request(&mut receiver, &mut native, &cancel).is_none());
    let (timeouts, services) = driver.join().unwrap();
    assert_eq!(timeouts, vec![PUMP_INTERVAL; 4]);
    assert_eq!(services, 4, "every pump interval services owner events");
    assert!(
        native.session.is_some(),
        "an owning session without a waker stays open"
    );
}

#[test]
fn failing_wait_closes_the_session_and_the_worker_waits_on_its_inbox() {
    let shared = Shared::new(|state| {
        state.wakes = true;
        state.owns = true;
        state.fail_waits = true;
        state.expire = 1;
    });
    let (sender, mut receiver) = mpsc::channel(1);
    let mut native = Native::new(Arc::default());
    native.open(Box::new(Session(shared.clone())));
    let requester = {
        let shared = shared.clone();
        std::thread::spawn(move || {
            drop(shared.until("the failed session closed", |state| state.closed == 1));
            sender
                .blocking_send(Request {
                    id: DesktopRequestId(5),
                    action: Action::ReadText,
                })
                .is_ok()
        })
    };
    let request = next_request(&mut receiver, &mut native, &Cancellation::default());
    assert!(requester.join().unwrap());
    assert_eq!(request.map(|request| request.id), Some(DesktopRequestId(5)));
    assert!(native.session.is_none());
    assert!(!native.wakes);
    assert_eq!(shared.lock().timeouts, [IDLE_WAIT]);
}
