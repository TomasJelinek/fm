//! Restarts a pane's watch worker in place on its own `midnight-watch` thread
//! after a panic, rate-limited, and reports the pane's watch status.
use super::{
    status::{StatusReport, ViewKey},
    *,
};
use midnight_core::TreeSessionId;
use std::{
    any::Any,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
};

/// While a status waits for queue capacity, the worker parks at most this long.
const UNSENT_RETRY: Duration = Duration::from_millis(10);

/// A native registration, by what it watches: a flat folder (also while it is
/// prepared), or a tree session's batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Attempt {
    Flat(PathBuf),
    Tree(NavigationId, TreeSessionId),
}

/// The restarted loop's view, whose registration outcome is reported only once
/// a probe or tree step completed: a fault that recurs before then must not
/// report `Native` between two `Restarting` reports.
struct Restored {
    key: ViewKey,
    status: Option<(WatchedView, WatchStatus)>,
}

/// Supervisor-owned state that survives a restart of the worker loop.
pub(super) struct Supervision {
    report: StatusReport,
    /// The native registration in progress, if a panic interrupts it.
    registering: Option<Attempt>,
    /// A registration a panic interrupted once.
    suspect: Option<Attempt>,
    /// A registration a panic interrupted twice in a row: it fails without a
    /// native call, so its view is only polled, until another registration is
    /// made.
    skipped: Option<Attempt>,
    restored: Option<Restored>,
    resumed: bool,
}

impl Supervision {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            report: StatusReport::new(enabled),
            registering: None,
            suspect: None,
            skipped: None,
            restored: None,
            resumed: false,
        }
    }

    /// Runs a native registration of `attempt` (whose first directory is
    /// `path`), unless a panic interrupted it twice in a row.
    pub(super) fn register<T>(
        &mut self,
        attempt: Attempt,
        path: &Path,
        register: impl FnOnce() -> Result<T, FsError>,
    ) -> Result<T, FsError> {
        if self.skipped.as_ref() == Some(&attempt) {
            return Err(FsError::from_io(
                "watch directory",
                path,
                io::Error::other("native watch registration failed unexpectedly"),
            ));
        }
        self.skipped = None;
        self.registering = Some(attempt);
        let result = register();
        if self.registering.take() == self.suspect {
            self.suspect = None;
        }
        result
    }

    /// Records a view's registration outcome for the UI.
    pub(super) fn record(&mut self, view: WatchedView, status: WatchStatus) {
        if let Some(restored) = self
            .restored
            .as_mut()
            .filter(|restored| restored.key == ViewKey::of(view))
        {
            restored.status = Some((view, status));
            return;
        }
        // The pane shows another view than the restored one.
        self.restored = None;
        self.report.set(view, status);
    }

    /// A probe or tree step completed without a panic.
    pub(super) fn completed(&mut self) {
        if let Some(Restored {
            status: Some((view, status)),
            ..
        }) = self.restored.take()
        {
            self.report.set(view, status);
        }
    }

    /// Sends an unsent status for the view the pane shows. Returns false, and
    /// stops the pane, when the event queue is closed.
    pub(super) fn flush(&mut self, worker: &Worker) -> bool {
        if self.report.flush(|| shown_view(worker), &worker.outbox) {
            return true;
        }
        worker
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopped = true;
        false
    }

    /// Parks until woken (`Shared::park`) or `timeout`; while a status is
    /// unsent, at most `UNSENT_RETRY`.
    pub(super) fn park(&self, shared: &Shared, timeout: Option<Duration>) {
        let timeout = if self.report.unsent() {
            Some(timeout.map_or(UNSENT_RETRY, |timeout| timeout.min(UNSENT_RETRY)))
        } else {
            timeout
        };
        shared.park(timeout);
    }

    pub(super) fn take_resumed(&mut self) -> bool {
        std::mem::take(&mut self.resumed)
    }

    /// The restarted loop restores `view`'s watch.
    pub(super) fn restoring(&mut self, view: Option<WatchedView>) {
        self.restored = view.map(|view| Restored {
            key: ViewKey::of(view),
            status: None,
        });
    }

    /// After a panic unwound the loop.
    fn unwound(&mut self) {
        match self.registering.take() {
            Some(attempt) if self.suspect.as_ref() == Some(&attempt) => {
                self.suspect = None;
                self.skipped = Some(attempt);
            }
            attempt => self.suspect = attempt,
        }
        self.restored = None;
    }
}

/// Runs a pane's worker loop on this thread. After a panic, the loop's targets,
/// prepared folder, tree runtime and subscriptions are already unwound on this
/// thread; the worker restarts at once the first time, and later no sooner
/// than `Timing::restart` after its previous restart, reporting `Restarting`
/// meanwhile. The shared control (dirty epochs, acknowledgements, fences and
/// dispatch records) survives; every lock of it tolerates poisoning. A panic
/// outside the loop (here, in the wait or in the drain) still ends the thread,
/// and the `WorkerOwned` guard then retires the shared captures.
pub(super) fn run(worker: Worker) {
    let _ = worker.shared.worker.set(thread::current());
    let mut supervision = Supervision::new(worker.native.is_some());
    let mut restarted: Option<Instant> = None;
    loop {
        match catch_unwind(AssertUnwindSafe(|| serve(&worker, &mut supervision))) {
            Ok(()) => return,
            Err(payload) => dispose(payload),
        }
        supervision.unwound();
        if worker.shared.stopped() {
            drop(take_scope(&worker.shared));
            return;
        }
        if let Some(previous) = restarted
            && !wait_to_restart(&worker, &mut supervision, previous + worker.timing.restart)
        {
            drop(take_scope(&worker.shared));
            return;
        }
        let now = Instant::now();
        restarted = Some(now);
        #[cfg(test)]
        worker
            .shared
            .restarts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(now);
        supervision.resumed = true;
    }
}

/// Drops a caught panic's payload. A payload whose destructor panics again is
/// forgotten, as the plugin SDK's `guard::contain` does: its destructor is
/// unknown, so it is not dropped again.
fn dispose(payload: Box<dyn Any + Send>) {
    if let Err(nested) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
        std::mem::forget(nested);
    }
}

/// Waits until `resume_at` without probes or subscriptions, reporting
/// `Restarting` for the view the pane shows, again whenever it changes.
/// Returns false when the pane stopped.
fn wait_to_restart(worker: &Worker, supervision: &mut Supervision, resume_at: Instant) -> bool {
    loop {
        if worker.shared.stopped() {
            return false;
        }
        if Instant::now() >= resume_at {
            return true;
        }
        if let Some(view) = shown_view(worker) {
            supervision.report.set(view, WatchStatus::Restarting);
        }
        if !supervision.flush(worker) {
            return false;
        }
        supervision.park(
            &worker.shared,
            Some(resume_at.saturating_duration_since(Instant::now())),
        );
    }
}

/// Takes the pane's shared scope after it stopped, so that it is released on
/// this worker.
pub(super) fn take_scope(shared: &Shared) -> (Option<tree::TreeControl>, Option<Visible>) {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (state.tree.take(), state.visible.take())
}
