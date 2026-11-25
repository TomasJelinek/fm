//! A fixed backend worker set for long-lived jobs that need a stable physical thread.

use std::{
    any::Any,
    cell::Cell,
    collections::VecDeque,
    fmt,
    future::{Future, poll_fn},
    io, mem,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::{Pin, pin},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{Builder, JoinHandle},
    time::Duration,
};
use tokio::{runtime::Handle, sync::oneshot};

const EXIT_RECHECK: Duration = Duration::from_millis(5);

/// Owned work with no resource-bearing return value.
pub type Work = Box<dyn FnOnce() + Send + 'static>;

/// The external authority that seals the pool. Dropping it seals and detaches the workers.
pub struct Owner {
    shared: Arc<Shared>,
    workers: Vec<Worker>,
}

/// Submission authority. Cloning a spawner does not extend the pool owner's lifetime.
#[derive(Clone)]
pub struct Spawner {
    shared: Arc<Shared>,
}

/// Completion and pre-claim cancellation. Dropping a plain ticket detaches the work.
pub struct Ticket {
    task: Arc<TaskState>,
    done: oneshot::Receiver<Result<(), TaskError>>,
}

/// Waits for checked thread retirement. Dropping the waiter detaches the workers.
#[must_use = "await checked worker retirement or explicitly detach it"]
pub struct Shutdown {
    shared: Arc<Shared>,
    wait: Pin<Box<dyn Future<Output = Result<(), ShutdownError>> + Send>>,
}

/// The reason that the executor returned a submission to its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Full,
    Sealed,
    WorkerFailed,
}

/// An envelope ended without completing its body normally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskError {
    /// Cancellation won before worker claim and the capture was destroyed on the worker.
    Cancelled,
    /// The body or its cleanup panicked on the worker.
    Panicked,
    /// A worker failure prevented normal disposition of the envelope.
    WorkerFailed,
}

/// A refused submission carrying the original work intact.
pub struct Rejected {
    reason: Rejection,
    work: Work,
}

/// Checked worker retirement could not be confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShutdownError(ShutdownFailure);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShutdownFailure {
    CalledFromWorker,
    WorkerFailed,
    JoinUnavailable,
}

struct Worker {
    thread: JoinHandle<()>,
    exited: oneshot::Receiver<()>,
}

thread_local! {
    // The worker retains the Arc throughout its loop, so this identity is stable.
    static CURRENT_POOL: Cell<*const Shared> = const { Cell::new(std::ptr::null()) };
}

struct Shared {
    capacity: usize,
    queue: Mutex<Queue>,
    changed: Condvar,
}

struct Queue {
    state: PoolState,
    live_workers: usize,
    pending: VecDeque<Job>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PoolState {
    Open,
    Sealed,
    Failed,
}

struct TaskState {
    unclaimed: AtomicBool,
    finished: AtomicBool,
}

struct Job {
    transient: Handle,
    work: QueuedWork,
    task: Arc<TaskState>,
    done: oneshot::Sender<Result<(), TaskError>>,
}

// Only a worker in the original context may take and destroy this capture.
struct QueuedWork(Option<Work>);

impl Owner {
    /// Starts the complete worker set with an equally sized queue, without needing Tokio.
    pub fn start(name: &str, threads: usize) -> io::Result<(Self, Spawner)> {
        Self::start_with(name, threads, Builder::spawn)
    }

    fn start_with(
        name: &str,
        threads: usize,
        mut spawn: impl FnMut(Builder, Work) -> io::Result<JoinHandle<()>>,
    ) -> io::Result<(Self, Spawner)> {
        if threads == 0 || name.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "resident workers require a positive count and name without NUL",
            ));
        }

        let mut pending = VecDeque::new();
        pending
            .try_reserve_exact(threads)
            .map_err(io::Error::other)?;
        let mut workers = Vec::new();
        workers
            .try_reserve_exact(threads)
            .map_err(io::Error::other)?;
        let shared = Arc::new(Shared {
            capacity: threads,
            queue: Mutex::new(Queue {
                state: PoolState::Open,
                live_workers: threads,
                pending,
            }),
            changed: Condvar::new(),
        });
        let mut owner = Self {
            shared: Arc::clone(&shared),
            workers,
        };

        let (ready, acknowledged) = mpsc::sync_channel(threads);
        for index in 0..threads {
            let builder = Builder::new().name(format!("{name}-{index}"));
            let (on_exit, exited) = oneshot::channel();
            let ready = ready.clone();
            let worker_shared = Arc::clone(&shared);
            let run = Box::new(move || worker_main(worker_shared, ready, on_exit));
            match spawn(builder, run) {
                Ok(thread) => owner.workers.push(Worker { thread, exited }),
                Err(error) => {
                    owner.stop_empty_workers();
                    return Err(error);
                }
            }
        }
        drop(ready);
        for _ in 0..threads {
            if acknowledged.recv().is_err() {
                owner.stop_empty_workers();
                return Err(io::Error::other("resident worker exited before readiness"));
            }
        }
        Ok((owner, Spawner { shared }))
    }

    fn stop_empty_workers(&mut self) {
        // The spawner has never been published. These workers can only be idle.
        self.seal();
        join_threads(mem::take(&mut self.workers));
    }

    /// Rejects new work and requests that workers skip all unclaimed jobs.
    /// Running bodies still depend on their own stop controls.
    pub fn seal(&self) {
        let mut queue = self.shared.lock();
        if queue.state == PoolState::Open {
            queue.state = PoolState::Sealed;
            self.shared.changed.notify_all();
        }
    }

    /// Seals immediately and hands back an asynchronous retirement check.
    /// Poll the check outside this pool with a functioning waker. Keep the supplied original
    /// transient runtime alive with its time driver enabled and actively driven while waiting.
    /// A current-thread runtime needs its owning `Runtime` driven, for example by
    /// `Runtime::block_on`. The check may be polled on another executor.
    ///
    /// The supplied runtime schedules any rechecks and the final joins. An undriven timer may
    /// leave the check pending. A detected unavailable timer or join returns a checked error.
    pub fn shutdown(mut self, transient: &Handle) -> Shutdown {
        self.seal();
        let workers = mem::take(&mut self.workers);
        let transient = transient.clone();
        let shared = Arc::clone(&self.shared);
        Shutdown {
            shared: Arc::clone(&shared),
            wait: Box::pin(wait_for_workers(shared, workers, transient)),
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.seal();
    }
}

impl Spawner {
    /// Queues work with the supplied runtime for both execution and skipped capture cleanup.
    /// The caller retains responsibility for disposing of a returned envelope in its own context.
    pub fn spawn(&self, transient: &Handle, work: Work) -> Result<Ticket, Rejected> {
        let mut queue = self.shared.lock();
        let refusal = match queue.state {
            PoolState::Failed => Some(Rejection::WorkerFailed),
            PoolState::Sealed => Some(Rejection::Sealed),
            PoolState::Open if queue.pending.len() == self.shared.capacity => Some(Rejection::Full),
            PoolState::Open => None,
        };
        if let Some(reason) = refusal {
            return Err(Rejected { reason, work });
        }

        let task = Arc::new(TaskState {
            unclaimed: AtomicBool::new(true),
            finished: AtomicBool::new(false),
        });
        let (send, done) = oneshot::channel();
        queue.pending.push_back(Job {
            transient: transient.clone(),
            work: QueuedWork(Some(work)),
            task: Arc::clone(&task),
            done: send,
        });
        self.shared.changed.notify_one();
        Ok(Ticket { task, done })
    }
}

impl Ticket {
    /// Cancels an unclaimed body. A claimed body proceeds with its own stop controls.
    pub fn abort(&self) {
        self.task.unclaimed.store(false, Ordering::Relaxed);
    }

    /// Reports that the worker has disposed of the job or reported a worker failure.
    pub fn is_finished(&self) -> bool {
        self.task.finished.load(Ordering::Acquire)
    }
}

impl Future for Ticket {
    type Output = Result<(), TaskError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.done)
            .poll(cx)
            .map(|result| result.unwrap_or(Err(TaskError::WorkerFailed)))
    }
}

impl Future for Shutdown {
    type Output = Result<(), ShutdownError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if CURRENT_POOL.with(|pool| pool.get() == Arc::as_ptr(&self.shared)) {
            return Poll::Ready(Err(ShutdownError(ShutdownFailure::CalledFromWorker)));
        }
        self.wait.as_mut().poll(cx)
    }
}

impl Rejected {
    /// Returns the reason without inspecting the rejected closure.
    pub fn reason(&self) -> Rejection {
        self.reason
    }

    /// Transfers the work back to the caller.
    pub fn into_work(self) -> Work {
        self.work
    }
}

impl fmt::Debug for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rejected")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Rejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.reason {
            Rejection::Full => "the resident worker queue is full",
            Rejection::Sealed => "the resident worker pool is sealed",
            Rejection::WorkerFailed => "a resident worker failed",
        })
    }
}

impl std::error::Error for Rejected {}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "resident work was cancelled before it started",
            Self::Panicked => "resident work panicked",
            Self::WorkerFailed => "resident work disposition is unconfirmed",
        })
    }
}

impl std::error::Error for TaskError {}

impl fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            ShutdownFailure::CalledFromWorker => "a resident worker cannot await its own pool",
            ShutdownFailure::WorkerFailed => "resident worker retirement is unconfirmed",
            ShutdownFailure::JoinUnavailable => "resident worker join did not complete",
        })
    }
}

impl std::error::Error for ShutdownError {}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next(&self) -> Option<(Job, bool)> {
        let mut queue = self.lock();
        loop {
            if let Some(job) = queue.pending.pop_front() {
                // The swap orders claim against ticket abort. The lock orders it against seal.
                let unclaimed = job.task.unclaimed.swap(false, Ordering::Relaxed);
                let execute = queue.state == PoolState::Open && unclaimed;
                return Some((job, execute));
            }
            if queue.state != PoolState::Open {
                return None;
            }
            queue = self
                .changed
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn worker_exited(&self, healthy: bool) -> VecDeque<Job> {
        let mut queue = self.lock();
        if !healthy {
            queue.state = PoolState::Failed;
            self.changed.notify_all();
        }
        queue.live_workers -= 1;
        if queue.live_workers == 0 {
            // No surviving worker can establish context for any remaining work.
            return mem::take(&mut queue.pending);
        }
        VecDeque::new()
    }

    fn mark_failed(&self) {
        self.lock().state = PoolState::Failed;
        self.changed.notify_all();
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        // Normal teardown leaves this empty. Even a failed exit protocol may not move
        // capture destruction or completion wakeups onto the last owner/spawner caller.
        if !self.pending.is_empty() {
            mem::forget(mem::take(&mut self.pending));
        }
    }
}

impl Drop for QueuedWork {
    fn drop(&mut self) {
        if let Some(work) = self.0.take() {
            mem::forget(work);
        }
    }
}

impl Job {
    fn run(self, shared: &Shared, execute: bool) -> bool {
        let Self {
            transient,
            mut work,
            task,
            done,
        } = self;
        let (result, healthy) = match catch_unwind(AssertUnwindSafe(|| {
            let _context = transient.enter();
            let work = work
                .0
                .take()
                .expect("a queued job carries one work envelope");
            match catch_unwind(AssertUnwindSafe(move || {
                if execute {
                    work();
                } else {
                    drop(work);
                }
            })) {
                Ok(()) if execute => (Ok(()), true),
                Ok(()) => (Err(TaskError::Cancelled), true),
                Err(payload) => (Err(TaskError::Panicked), dispose_panic(payload)),
            }
        })) {
            Ok(result) => result,
            Err(payload) => {
                dispose_panic(payload);
                (Err(TaskError::WorkerFailed), false)
            }
        };

        if !healthy {
            shared.mark_failed();
        }
        drop(work);
        complete(shared, &transient, task, done, result) && healthy
    }

    fn quarantine(self, shared: &Shared) {
        let Self {
            transient,
            work,
            task,
            done,
        } = self;
        drop(work);
        complete(shared, &transient, task, done, Err(TaskError::WorkerFailed));
    }
}

fn complete(
    shared: &Shared,
    transient: &Handle,
    task: Arc<TaskState>,
    done: oneshot::Sender<Result<(), TaskError>>,
    result: Result<(), TaskError>,
) -> bool {
    task.finished.store(true, Ordering::Release);
    let Err(payload) = notify(done, result) else {
        return true;
    };
    shared.mark_failed();
    dispose_ticket_panic(transient, payload);
    false
}

fn notify<T>(sender: oneshot::Sender<T>, result: T) -> std::thread::Result<()> {
    // A Future may have been polled with an arbitrary Waker.
    catch_unwind(AssertUnwindSafe(move || {
        let _ = sender.send(result);
    }))
}

fn dispose_ticket_panic(transient: &Handle, payload: Box<dyn Any + Send>) {
    let mut payload = Some(payload);
    let disposed = catch_unwind(AssertUnwindSafe(|| {
        let _context = transient.enter();
        if let Some(payload) = payload.take() {
            dispose_panic(payload);
        }
    }));
    if let Err(context_panic) = disposed {
        if let Some(payload) = payload {
            // The original context could not be entered. Retain the payload safely.
            mem::forget(payload);
        }
        dispose_panic(context_panic);
    }
}

fn dispose_panic(payload: Box<dyn Any + Send>) -> bool {
    match catch_unwind(AssertUnwindSafe(move || drop(payload))) {
        Ok(()) => true,
        Err(payload) => {
            // Stop the failed worker rather than retrying arbitrarily panicking destructors.
            mem::forget(payload);
            false
        }
    }
}

fn worker_main(shared: Arc<Shared>, ready: SyncSender<()>, on_exit: oneshot::Sender<()>) {
    CURRENT_POOL.with(|pool| pool.set(Arc::as_ptr(&shared)));
    let _ = ready.send(());
    drop(ready);
    let healthy = match catch_unwind(AssertUnwindSafe(|| {
        while let Some((job, execute)) = shared.next() {
            if !job.run(&shared, execute) {
                return false;
            }
        }
        true
    })) {
        Ok(healthy) => healthy,
        Err(payload) => {
            dispose_panic(payload);
            false
        }
    };
    for job in shared.worker_exited(healthy) {
        job.quarantine(&shared);
    }
    if let Err(payload) = notify(on_exit, ()) {
        shared.mark_failed();
        dispose_panic(payload);
    }
}

fn join_threads(workers: Vec<Worker>) -> bool {
    let mut joined = true;
    for worker in workers {
        if let Err(payload) = worker.thread.join() {
            // User panics are contained above. An escaped payload may not be disposed here.
            mem::forget(payload);
            joined = false;
        }
    }
    joined
}

// The timer driver can invoke and release its registered waker outside Sleep::poll.
struct ExitWaker {
    waiter: Option<Waker>,
    state: Arc<Mutex<ExitWakeState>>,
}

#[derive(Default)]
struct ExitWakeState {
    failed: bool,
    latest: Option<Weak<ExitWaker>>,
}

impl ExitWaker {
    fn contain(&self, action: impl FnOnce()) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(action)) {
            // Hold the replacement before publishing failure to its regular wake path.
            // Otherwise that path could skip notification and release the last reference.
            let replacement = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.failed = true;
                state
                    .latest
                    .take()
                    .filter(|latest| !std::ptr::eq(latest.as_ptr(), self))
                    .and_then(|latest| latest.upgrade())
            };
            // Retain the exceptional payload without running its destructor on the driver.
            mem::forget(payload);

            // A concurrent poll may already have registered a replacement. The first
            // failure owns its terminal wake. A repeated failure cannot register or retry it.
            if let Some(replacement) = replacement {
                replacement.wake_waiter();
            }
        }
    }

    fn wake_waiter(&self) {
        if let Some(waiter) = &self.waiter {
            self.contain(|| waiter.wake_by_ref());
        }
    }
}

impl Wake for ExitWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let failed = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failed;
        if !failed {
            self.wake_waiter();
        }
    }
}

impl Drop for ExitWaker {
    fn drop(&mut self) {
        if let Some(waiter) = self.waiter.take() {
            self.contain(|| drop(waiter));
        }
    }
}

async fn wait_for_thread_return(workers: &[Worker], transient: &Handle) -> bool {
    let wake_state = Arc::new(Mutex::new(ExitWakeState::default()));
    let failed = || {
        wake_state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failed
    };
    while !failed() && !workers.iter().all(|worker| worker.thread.is_finished()) {
        // A receipt is visible before its waker returns. Keep the OS join out of the
        // transient blocking pool until the worker's thread function has returned.
        let delay = catch_unwind(AssertUnwindSafe(|| {
            let _context = transient.enter();
            tokio::time::sleep(EXIT_RECHECK)
        }));
        let delay = match delay {
            Ok(delay) => delay,
            Err(payload) => {
                mem::forget(payload);
                return false;
            }
        };
        let mut delay = pin!(delay);
        let tick = poll_fn(|cx| {
            if failed() {
                return Poll::Ready(Ok(()));
            }
            match catch_unwind(AssertUnwindSafe(|| {
                let waker = Arc::new(ExitWaker {
                    waiter: Some(cx.waker().clone()),
                    state: Arc::clone(&wake_state),
                });
                {
                    let mut state = wake_state.lock().unwrap_or_else(PoisonError::into_inner);
                    if state.failed {
                        return Poll::Ready(());
                    }
                    state.latest = Some(Arc::downgrade(&waker));
                }
                let waker = Waker::from(waker);
                delay.as_mut().poll(&mut Context::from_waker(&waker))
            })) {
                // Replacing a waker can synchronously drop its previous registration.
                Ok(_) if failed() => Poll::Ready(Ok(())),
                Ok(polled) => polled.map(Ok),
                Err(payload) => Poll::Ready(Err(payload)),
            }
        })
        .await;
        if let Err(payload) = tick {
            mem::forget(payload);
            return false;
        }
    }
    !failed()
}

async fn wait_for_workers(
    shared: Arc<Shared>,
    mut workers: Vec<Worker>,
    transient: Handle,
) -> Result<(), ShutdownError> {
    let mut exited = true;
    for worker in &mut workers {
        exited &= (&mut worker.exited).await.is_ok();
    }

    let joined = if wait_for_thread_return(&workers, &transient).await {
        match catch_unwind(AssertUnwindSafe(|| {
            transient.spawn_blocking(move || join_threads(workers))
        })) {
            Ok(join) => match join.await {
                Ok(joined) => Some(joined),
                Err(error) => {
                    if let Ok(payload) = error.try_into_panic() {
                        mem::forget(payload);
                    }
                    None
                }
            },
            Err(payload) => {
                mem::forget(payload);
                None
            }
        }
    } else {
        None
    };

    if !exited || joined == Some(false) || shared.lock().state == PoolState::Failed {
        Err(ShutdownError(ShutdownFailure::WorkerFailed))
    } else if joined.is_none() {
        Err(ShutdownError(ShutdownFailure::JoinUnavailable))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
