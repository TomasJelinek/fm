//! Idle wakeups of the native actor. Without a native session it has nothing to pump,
//! so it sleeps until a controller change wakes it; with a session that has a waker, it
//! sleeps in the session's wait until native input or a controller change, including a
//! change made while it opened that session. A failed wait closes the session.
use super::*;
use crate::{CommittedSnapshot, SnapshotState};
use std::{
    sync::{Condvar, MutexGuard},
    time::Instant,
};

/// Elapses only when an expected step of the actor does not happen.
const FAILURE_WAIT: Duration = Duration::from_secs(10);

/// A committed Left listing at navigation 5 with one dragged file, and its selection.
fn dragged_file() -> (Snapshots, SelectedFiles) {
    let file = std::env::current_dir().unwrap().join("dragged 文档.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), file.clone(), EntryKind::File)].into();
    let snapshots = crate::test_snapshots([
        SnapshotState {
            committed: Some(CommittedSnapshot {
                navigation: NavigationId(5),
                path: file.parent().unwrap().to_owned(),
                entries: entries.clone(),
                sizes: Arc::default(),
            }),
            ..Default::default()
        },
        SnapshotState::default(),
    ]);
    let files = SelectedFiles {
        selection: Arc::default(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        fallback: Some(file),
    };
    (snapshots, files)
}

async fn next(events: &mut mpsc::Receiver<Event>, expected: impl Fn(&DragDropEvent) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(Event::DragDrop(event)) if expected(&event) => return,
                Some(_) => {}
                None => panic!("drag event stream ended"),
            }
        }
    })
    .await
    .expect("the drag event did not arrive");
}

#[tokio::test]
async fn without_a_native_session_the_actor_sleeps_until_a_controller_change() {
    let (snapshots, files) = dragged_file();
    let (sender, mut events) = mpsc::channel(16);
    let mut tasks = JoinSet::new();
    let mut resident_tasks = ResidentTasks::default();
    let (_resident_owner, mut controller) = Controller::testing(
        None,
        None,
        snapshots,
        copy_operations::Admission::default(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let host = |generation| DragHost {
        generation,
        native: NativeDragWindow::Unsupported,
        lifetime: Cancellation::default(),
    };
    let targets = SharedDropTargets::default();
    controller
        .attach(host(1), targets.clone(), &mut resident_tasks)
        .await;
    next(&mut events, |event| {
        matches!(
            event,
            DragDropEvent::Ready {
                host_generation: 1,
                supported: false
            }
        )
    })
    .await;
    let wakeups = |controller: &Controller| {
        let state = controller.state.lock().unwrap();
        (state.actor_wakeups, state.prepare_wakeups)
    };
    let asleep = wakeups(&controller);
    // An actor or preparation worker waking on a timer would wake several times in
    // this window.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        wakeups(&controller),
        asleep,
        "the idle actor or preparation worker woke"
    );

    // A queued start wakes it.
    controller
        .prepare(DragId(1), PaneId::Left, NavigationId(5), files)
        .await;
    next(&mut events, |event| {
        matches!(event, DragDropEvent::Prepared { id: DragId(1), .. })
    })
    .await;
    controller.start(DragId(1)).await;
    next(&mut events, |event| {
        matches!(
            event,
            DragDropEvent::Finished {
                id: DragId(1),
                outcome: DragFinish::Unavailable
            }
        )
    })
    .await;
    // So does a new host.
    controller
        .attach(host(2), targets, &mut resident_tasks)
        .await;
    next(&mut events, |event| {
        matches!(
            event,
            DragDropEvent::Ready {
                host_generation: 2,
                supported: false
            }
        )
    })
    .await;
    // And shutdown.
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.join_next().await.is_some() {}
        while resident_tasks.join_next().await.is_some() {}
    })
    .await
    .expect("shutdown did not stop the actors");
}

/// What the fake native session observed. The test also stands in for native input.
#[derive(Debug, Default)]
struct Observed {
    opened: usize,
    closed: usize,
    services: usize,
    started: usize,
    /// The timeout of each wait the actor began.
    waits: Vec<Duration>,
    waiting: bool,
    /// The sticky wake: set by the session's waker or native input, taken by a wait.
    woken: bool,
    /// While set, opening a session blocks, as a slow native registration can.
    hold_open: bool,
    /// Every wait fails at once, as a broken native wait would.
    wait_fails: bool,
}

#[derive(Default)]
struct Shared {
    observed: Mutex<Observed>,
    changed: Condvar,
}
impl Shared {
    fn lock(&self) -> MutexGuard<'_, Observed> {
        self.observed.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn update(&self, change: impl FnOnce(&mut Observed)) {
        change(&mut self.lock());
        self.changed.notify_all();
    }
    /// Block until `done` holds, failing after `FAILURE_WAIT`.
    fn until(&self, what: &str, done: impl Fn(&Observed) -> bool) -> MutexGuard<'_, Observed> {
        let deadline = Instant::now() + FAILURE_WAIT;
        let mut observed = self.lock();
        while !done(&observed) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out until {what}: {:?}", *observed);
            observed = self
                .changed
                .wait_timeout(observed, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        observed
    }
}

struct WakingService(Arc<Shared>);
impl FileDragDropService for WakingService {
    fn open_session(
        &self,
        _: DragHost,
        _: SharedDropTargets,
        _: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        self.0.update(|observed| observed.opened += 1);
        drop(self.0.until("the test lets the session open", |observed| {
            !observed.hold_open
        }));
        Ok(Box::new(WakingSession(self.0.clone())))
    }
}
/// A native session whose waits end only through its waker or the test's native input.
struct WakingSession(Arc<Shared>);
impl FileDragDropSession for WakingSession {
    fn service_pending(&mut self) -> Result<(), FsError> {
        self.0.update(|observed| observed.services += 1);
        Ok(())
    }
    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        let shared = &self.0;
        let mut observed = shared.lock();
        observed.waits.push(timeout);
        if observed.wait_fails {
            return Err(error(io::ErrorKind::Other, WAIT_FAILURE));
        }
        observed.waiting = true;
        shared.changed.notify_all();
        // Bounded, so that a broken actor fails the test instead of hanging it.
        let deadline = Instant::now() + 2 * FAILURE_WAIT;
        while !observed.woken && Instant::now() < deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            observed = shared
                .changed
                .wait_timeout(observed, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        observed.woken = false;
        observed.waiting = false;
        shared.changed.notify_all();
        Ok(())
    }
    fn waker(&self) -> Option<FileDragDropWaker> {
        let shared = self.0.clone();
        Some(Arc::new(move || {
            shared.update(|observed| observed.woken = true)
        }))
    }
    fn start_drag(&mut self, _: &FileClipboard, _: &Cancellation) -> Result<DragFinish, FsError> {
        self.0.update(|observed| observed.started += 1);
        Ok(DragFinish::Copied)
    }
}
impl Drop for WakingSession {
    fn drop(&mut self) {
        self.0.update(|observed| observed.closed += 1);
    }
}

/// The message of the error that every wait of a session with `wait_fails` returns.
const WAIT_FAILURE: &str = "The native wait failed.";

/// A controller on a current-thread runtime whose native sessions are `WakingSession`s.
/// The runtime is dropped last.
struct Harness {
    controller: Controller,
    events: mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    resident_tasks: ResidentTasks,
    _resident_owner: midnight_work_budget::resident::Owner,
    runtime: tokio::runtime::Runtime,
}
impl Harness {
    fn new(shared: &Arc<Shared>, snapshots: Snapshots) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (sender, events) = mpsc::channel(16);
        let mut tasks = JoinSet::new();
        let (resident_owner, controller) = {
            let _entered = runtime.enter();
            Controller::testing(
                Some(Arc::new(WakingService(shared.clone()))),
                None,
                snapshots,
                copy_operations::Admission::default(),
                Outbox {
                    sender,
                    wakeup: Arc::new(|| {}),
                },
                &mut tasks,
            )
        };
        Self {
            controller,
            events,
            tasks,
            resident_tasks: ResidentTasks::default(),
            _resident_owner: resident_owner,
            runtime,
        }
    }
    fn attach(&mut self, generation: u64) {
        let host = DragHost {
            generation,
            native: NativeDragWindow::Windows { hwnd: 1 },
            lifetime: Cancellation::default(),
        };
        self.runtime.block_on(self.controller.attach(
            host,
            SharedDropTargets::default(),
            &mut self.resident_tasks,
        ));
    }
    fn next(&mut self, expected: impl Fn(&DragDropEvent) -> bool) {
        self.runtime.block_on(next(&mut self.events, expected));
    }
    fn shutdown(mut self) {
        self.controller.shutdown();
        let tasks = &mut self.tasks;
        let resident_tasks = &mut self.resident_tasks;
        self.runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while tasks.join_next().await.is_some() {}
                    while resident_tasks.join_next().await.is_some() {}
                })
                .await
            })
            .expect("shutdown did not stop the actors");
    }
}

#[test]
fn with_a_waking_session_the_actor_waits_in_it_until_native_input_or_a_controller_change() {
    let shared = Arc::new(Shared::default());
    let (snapshots, files) = dragged_file();
    let mut h = Harness::new(&shared, snapshots);
    h.attach(1);
    // The idle actor waits in the open session, for its idle bound, not a pump interval.
    let idle = shared.until("the actor waits in its session", |observed| {
        observed.waiting
    });
    assert_eq!(idle.opened, 1);
    assert_eq!(idle.waits, [IDLE_WAIT]);
    let services = idle.services;
    drop(idle);

    // Native input ends the wait: the actor serves it once and waits again.
    shared.update(|observed| observed.woken = true);
    let idle = shared.until("the actor waits again", |observed| {
        observed.waiting && observed.waits.len() == 2
    });
    assert_eq!(idle.services, services + 1, "one pass per native wakeup");
    assert_eq!(idle.waits[1], IDLE_WAIT);
    drop(idle);

    // A queued start ends the wait through the session's waker.
    h.runtime.block_on(
        h.controller
            .prepare(DragId(1), PaneId::Left, NavigationId(5), files),
    );
    h.next(|event| matches!(event, DragDropEvent::Prepared { id: DragId(1), .. }));
    h.runtime.block_on(h.controller.start(DragId(1)));
    h.next(|event| {
        matches!(
            event,
            DragDropEvent::Finished {
                id: DragId(1),
                outcome: DragFinish::Copied
            }
        )
    });
    assert_eq!(shared.lock().started, 1);

    // So does a detach, after which the actor closes the session.
    drop(shared.until("the actor waits after the drag", |observed| {
        observed.waiting
    }));
    h.controller.detach(1);
    drop(shared.until("the actor closes its session", |observed| {
        observed.closed == 1
    }));
    h.shutdown();
}

#[test]
fn a_change_made_while_the_actor_opens_a_session_needs_no_waker_of_that_session() {
    let shared = Arc::new(Shared::default());
    let mut h = Harness::new(&shared, dragged_file().0);
    h.attach(1);
    drop(
        shared.until("the actor waits in its first session", |observed| {
            observed.waiting
        }),
    );
    // A new host replaces that session; opening its own blocks until the test allows it.
    shared.update(|observed| observed.hold_open = true);
    h.attach(2);
    let opening = shared.until("the actor opens the second session", |observed| {
        observed.opened == 2
    });
    // The actor dropped the first session, and with it the waker the controller calls,
    // before it began to open this one: only the actor's channel can hold this detach.
    assert_eq!(opening.closed, 1);
    drop(opening);
    h.controller.detach(2);
    shared.update(|observed| observed.hold_open = false);
    // The actor takes that signal before it would wait in the new session, whose wait
    // no waker call ends and which outlasts this test; it closes the session instead.
    let closed = shared.until("the actor closes the detached host's session", |observed| {
        observed.closed == 2
    });
    assert_eq!(
        closed.waits.len(),
        1,
        "the actor never waited in the detached host's session"
    );
    drop(closed);
    h.shutdown();
}

#[test]
fn a_failed_session_wait_closes_the_session_and_revokes_readiness() {
    let shared = Arc::new(Shared::default());
    shared.update(|observed| observed.wait_fails = true);
    let mut h = Harness::new(&shared, dragged_file().0);
    h.attach(1);
    h.next(|event| {
        matches!(
            event,
            DragDropEvent::Ready {
                host_generation: 1,
                supported: true
            }
        )
    });
    // The failure is reported before readiness is revoked, as for a failed pump.
    h.next(|event| {
        matches!(
            event,
            DragDropEvent::Failed { id: None, error } if error.message == WAIT_FAILURE
        )
    });
    h.next(|event| {
        matches!(
            event,
            DragDropEvent::Ready {
                host_generation: 1,
                supported: false
            }
        )
    });
    let observed = shared.lock();
    assert_eq!(
        (observed.opened, observed.closed, observed.waits.len()),
        (1, 1, 1),
        "the actor dropped the session whose wait failed and did not reopen it"
    );
    drop(observed);
    h.shutdown();
}
