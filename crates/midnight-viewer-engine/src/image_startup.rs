//! One source-free, one-use decoder spare shared by all windows of a service.
//!
//! The external handles never own a native process. Their final Drop only cancels and wakes the
//! backend owner, which retains starting/ready/retiring ownership until native cleanup finishes.
//! Explicit spare retirement is independent of external handle lifetime and active foreground work.
use midnight_core::ViewerAssociations;
use midnight_image_viewer_host::{PreparedImageWorker, Provider};
use std::{
    fmt, io,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const CHILD_LIMIT: usize = 4;
const PREPARE_TIMEOUT: Duration = Duration::from_secs(30);
const READY_LIFETIME: Duration = Duration::from_secs(60);
const CANCEL_POLL: Duration = Duration::from_millis(10);

/// Only committed startup-captured settings belong here. A draft Options revision is not authority.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Policy {
    pub disabled: Arc<[String]>,
    pub associations: Arc<ViewerAssociations>,
    /// Narrow installed-catalog lease revision, including invalidation of an otherwise cached Arc.
    pub catalog_revision: u64,
}

impl Policy {
    fn valid(&self) -> bool {
        midnight_core::validate_disabled_viewers(&self.disabled).is_ok()
            && self.associations.validate().is_ok()
    }

    fn allows(&self, id: &str, document: bool, extensions: &[String]) -> bool {
        !document
            && !self.disabled.iter().any(|disabled| disabled == id)
            && self
                .associations
                .overrides
                .get(id)
                .map_or(!extensions.is_empty(), |extensions| !extensions.is_empty())
    }
}

#[derive(Clone)]
pub(crate) struct Startup {
    manager: Manager<Native>,
}

/// A spare-only retirement fence that retains shared state, never the service's external Control.
pub(crate) struct SpareRetirement {
    ticket: Retirement<Native>,
}

/// Failure to confirm spare cleanup after the backend owner unwinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetirementError {
    OwnerFailed,
}

impl fmt::Display for RetirementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerFailed => {
                formatter.write_str("Image spare owner failed before confirming cleanup")
            }
        }
    }
}

impl std::error::Error for RetirementError {}

impl SpareRetirement {
    /// Memory-only and idempotent. Permanently seals spare configuration/refill and wakes its owner.
    /// Active sessions and ordinary cold foreground admissions remain usable.
    pub fn stop_spare(&self) {
        self.ticket.stop_spare();
    }

    /// Success follows late spare creation, actual cleanup and normal backend owner exit.
    /// Await outside the UI and before runtime teardown. Owner failure is terminal but does not
    /// confirm cleanup; retain this ticket through runtime shutdown and do not retry startup.
    pub async fn wait(&self) -> Result<(), RetirementError> {
        self.ticket.wait().await
    }
}

/// Keep this permit alive until the associated prepared worker/session's actual Drop has finished.
/// In an image actor, declare the permit before the session so reverse local drop order enforces it.
pub(crate) struct ChildPermit {
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for ChildPermit {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

/// Fields drop in declaration order: native cleanup finishes before releasing capacity.
pub(crate) struct Acquired {
    pub prepared: Option<PreparedImageWorker>,
    pub permit: ChildPermit,
}

impl Startup {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            manager: Manager::new(Native, move |run| {
                // This owner retains Shared, not the external Control whose Drop requests shutdown.
                // No UI-held last Arc can consequently destroy a ready process synchronously.
                handle.spawn_blocking(run);
            }),
        }
    }

    pub fn spare_retirement(&self) -> SpareRetirement {
        SpareRetirement {
            ticket: self.manager.spare_retirement(),
        }
    }

    /// Memory-only configuration. Native preparation and stale-child cleanup run on the owner.
    pub fn configure(&self, policy: Option<Policy>, catalog: Arc<[Provider]>) {
        self.manager.configure(policy, catalog);
    }

    /// Backend-only admission. A speculative child can delay capacity, never reject a fourth viewer.
    /// A stale/failed spare returns `None` under the same reservation for ordinary validated open.
    pub fn acquire(&self, provider: &Provider, cancel: &AtomicBool) -> io::Result<Acquired> {
        let acquired = self.manager.acquire(provider, cancel)?;
        Ok(Acquired {
            prepared: acquired.prepared,
            permit: acquired.permit,
        })
    }

    /// Call only after successful source Open. Failures do not create speculative retry loops.
    pub fn opened(&self, provider: &Provider) {
        self.manager.opened(provider);
    }
}

/// Native boundaries are injected only to make launch and delayed-exit races deterministic in tests.
trait Backend: Send + Sync + 'static {
    type Provider: Clone + Send + Sync + 'static;
    type Prepared: Send + 'static;

    fn eligible(provider: &Self::Provider, policy: &Policy) -> bool;
    fn same_provider(left: &Self::Provider, right: &Self::Provider) -> bool;
    /// Exact in-memory catalog/protocol/resource matching; must never perform native I/O.
    fn prepared_matches(prepared: &Self::Prepared, provider: &Self::Provider) -> bool;
    fn prepare(&self, provider: &Self::Provider, cancel: &AtomicBool)
    -> io::Result<Self::Prepared>;
    /// Installed identity only, never idle Process.wait, liveness polling, or a source operation.
    fn validate(&self, prepared: &Self::Prepared, provider: &Self::Provider) -> io::Result<()>;
}

struct Native;

impl Backend for Native {
    type Provider = Provider;
    type Prepared = PreparedImageWorker;

    fn eligible(provider: &Provider, policy: &Policy) -> bool {
        policy.allows(&provider.id, provider.is_document(), &provider.extensions)
    }

    fn same_provider(left: &Provider, right: &Provider) -> bool {
        // This chooses an eligible catalog entry only. Exact executable/protocol/resource matching
        // is performed by PreparedImageWorker before any adoption, not inferred from these fields.
        left.id == right.id
            && left.name == right.name
            && left.version == right.version
            && left.extensions == right.extensions
            && left.package_dir() == right.package_dir()
            && left.is_document() == right.is_document()
    }

    fn prepared_matches(prepared: &PreparedImageWorker, provider: &Provider) -> bool {
        prepared.catalog_matches(provider)
    }

    fn prepare(&self, provider: &Provider, cancel: &AtomicBool) -> io::Result<PreparedImageWorker> {
        PreparedImageWorker::prepare(provider, cancel, PREPARE_TIMEOUT)
    }

    fn validate(&self, prepared: &PreparedImageWorker, provider: &Provider) -> io::Result<()> {
        prepared.validate_for(provider)
    }
}

struct Manager<B: Backend> {
    control: Arc<Control<B>>,
}

impl<B: Backend> Clone for Manager<B> {
    fn clone(&self) -> Self {
        Self {
            control: self.control.clone(),
        }
    }
}

struct Control<B: Backend> {
    shared: Arc<Shared<B>>,
}

impl<B: Backend> Drop for Control<B> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.shutdown = true;
        state.stop_spare();
        self.shared.changed.notify_all();
    }
}

struct Retirement<B: Backend> {
    shared: Arc<Shared<B>>,
}

impl<B: Backend> Retirement<B> {
    fn stop_spare(&self) {
        let mut state = self.shared.lock();
        state.stop_spare();
        self.shared.changed.notify_all();
    }

    async fn wait(&self) -> Result<(), RetirementError> {
        self.shared.completion.wait().await
    }
}

#[derive(Default)]
struct Completion {
    result: Mutex<Option<Result<(), RetirementError>>>,
    changed: tokio::sync::Notify,
}

impl Completion {
    fn finish(&self, result: Result<(), RetirementError>) {
        *self
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(result);
        self.changed.notify_waiters();
    }

    async fn wait(&self) -> Result<(), RetirementError> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            // Register before checking the committed state, including for concurrent waiters.
            changed.as_mut().enable();
            let result = *self
                .result
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(result) = result {
                return result;
            }
            changed.await;
        }
    }
}

struct Shared<B: Backend> {
    backend: B,
    state: Mutex<State<B>>,
    changed: Condvar,
    completion: Arc<Completion>,
}

struct State<B: Backend> {
    configuration: Option<Configuration<B>>,
    generation: u64,
    shutdown: bool,
    spare_stopped: bool,
    children: usize,
    demands: usize,
    spare: Spare<B>,
    refill: Option<B::Provider>,
}

impl<B: Backend> State<B> {
    fn stop_spare(&mut self) {
        self.spare_stopped = true;
        self.refill = None;
        self.invalidate();
    }

    fn invalidate(&mut self) {
        match &mut self.spare {
            Spare::Starting { cancel, .. } => cancel.store(true, Ordering::Release),
            Spare::Ready { retire, .. } => *retire = true,
            Spare::Empty | Spare::Retiring => {}
        }
    }

    fn eligible(&self, provider: &B::Provider) -> Option<&B::Provider> {
        self.configuration.as_ref()?.eligible(provider)
    }
}

struct Configuration<B: Backend> {
    policy: Option<Policy>,
    catalog: Arc<[B::Provider]>,
}

impl<B: Backend> Configuration<B> {
    fn eligible(&self, provider: &B::Provider) -> Option<&B::Provider> {
        let policy = self.policy.as_ref()?;
        self.catalog.iter().find(|candidate| {
            B::same_provider(candidate, provider) && B::eligible(candidate, policy)
        })
    }

    fn first(&self) -> Option<B::Provider> {
        let policy = self.policy.as_ref()?;
        self.catalog
            .iter()
            .find(|provider| B::eligible(provider, policy))
            .cloned()
    }
}

enum Spare<B: Backend> {
    Empty,
    Starting {
        generation: u64,
        provider: B::Provider,
        cancel: Arc<AtomicBool>,
    },
    Ready {
        generation: u64,
        ready: ReadyResource<B::Prepared>,
        expires: Instant,
        retire: bool,
    },
    /// Its resource is exclusively held by backend cleanup outside the state lock.
    Retiring,
}

struct ReadyResource<R> {
    prepared: R,
    permit: ChildPermit,
}

struct Acquisition<R> {
    prepared: Option<R>,
    permit: ChildPermit,
}

enum Work<B: Backend> {
    Prepare {
        generation: u64,
        provider: B::Provider,
        cancel: Arc<AtomicBool>,
        permit: ChildPermit,
    },
    Retire(ReadyResource<B::Prepared>),
    Stop,
}

/// Guards queued-demand priority even if backend identity validation unwinds.
struct Demand<B: Backend> {
    shared: Arc<Shared<B>>,
    registered: bool,
}

impl<B: Backend> Demand<B> {
    fn finish(&mut self, state: &mut State<B>) {
        state.demands -= 1;
        self.registered = false;
        self.shared.changed.notify_all();
    }
}

impl<B: Backend> Drop for Demand<B> {
    fn drop(&mut self) {
        if self.registered {
            let mut state = self.shared.lock();
            state.demands -= 1;
            self.shared.changed.notify_all();
        }
    }
}

impl<B: Backend> Manager<B> {
    fn new(backend: B, schedule: impl FnOnce(Box<dyn FnOnce() + Send>)) -> Self {
        let shared = Arc::new(Shared {
            backend,
            state: Mutex::new(State {
                configuration: None,
                generation: 0,
                shutdown: false,
                spare_stopped: false,
                children: 0,
                demands: 0,
                spare: Spare::Empty,
                refill: None,
            }),
            changed: Condvar::new(),
            completion: Arc::default(),
        });
        let owner = shared.clone();
        let completion = shared.completion.clone();
        schedule(Box::new(move || {
            let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                owner.clone().run();
            })) {
                Ok(()) => Ok(()),
                Err(_) => {
                    // Unwinding has finished, but native cleanup may be unconfirmed. Seal work
                    // without manufacturing Empty or releasing any remaining reservations.
                    let mut state = owner.lock();
                    state.stop_spare();
                    owner.changed.notify_all();
                    Err(RetirementError::OwnerFailed)
                }
            };
            // Only normal run return certifies cleanup. Failure wakes waiters without that claim.
            completion.finish(result);
        }));
        Self {
            control: Arc::new(Control { shared }),
        }
    }

    fn spare_retirement(&self) -> Retirement<B> {
        Retirement {
            shared: self.control.shared.clone(),
        }
    }

    fn configure(&self, policy: Option<Policy>, catalog: Arc<[B::Provider]>) {
        let policy = policy.filter(Policy::valid);
        let shared = &self.control.shared;
        let mut state = shared.lock();
        if state.shutdown
            || state.spare_stopped
            || state.configuration.as_ref().is_some_and(|previous| {
                previous.policy == policy && Arc::ptr_eq(&previous.catalog, &catalog)
            })
        {
            return;
        }
        let configuration = Configuration { policy, catalog };
        state.generation = state.generation.wrapping_add(1);
        state.invalidate();
        state.refill = configuration.first();
        state.configuration = Some(configuration);
        shared.changed.notify_all();
    }

    fn opened(&self, provider: &B::Provider) {
        let shared = &self.control.shared;
        let mut state = shared.lock();
        if state.shutdown || state.spare_stopped {
            return;
        }
        let Some(provider) = state.eligible(provider).cloned() else {
            return;
        };
        let already_requested = match &state.spare {
            Spare::Starting {
                generation,
                provider: starting,
                cancel,
            } => {
                *generation == state.generation
                    && !cancel.load(Ordering::Acquire)
                    && B::same_provider(starting, &provider)
            }
            Spare::Ready {
                generation,
                ready,
                retire,
                ..
            } => {
                *generation == state.generation
                    && !retire
                    && B::prepared_matches(&ready.prepared, &provider)
            }
            Spare::Empty | Spare::Retiring => false,
        };
        if already_requested {
            return;
        }
        state.invalidate();
        state.refill = Some(provider);
        shared.changed.notify_all();
    }

    fn acquire(
        &self,
        provider: &B::Provider,
        cancel: &AtomicBool,
    ) -> io::Result<Acquisition<B::Prepared>> {
        let shared = &self.control.shared;
        {
            let mut state = shared.lock();
            state.demands += 1;
            shared.changed.notify_all();
        }
        let mut demand = Demand {
            shared: shared.clone(),
            registered: true,
        };
        loop {
            let mut state = shared.lock();
            if state.shutdown || cancel.load(Ordering::Acquire) {
                drop(state);
                return Err(cancelled());
            }
            let matches = match &state.spare {
                Spare::Ready {
                    generation,
                    ready,
                    expires,
                    retire,
                } => {
                    *generation == state.generation
                        && !retire
                        && Instant::now() < *expires
                        && B::prepared_matches(&ready.prepared, provider)
                        && state.eligible(provider).is_some_and(|catalog_provider| {
                            B::prepared_matches(&ready.prepared, catalog_provider)
                        })
                }
                Spare::Empty | Spare::Starting { .. } | Spare::Retiring => false,
            };
            if matches {
                let Spare::Ready {
                    generation, ready, ..
                } = std::mem::replace(&mut state.spare, Spare::Empty)
                else {
                    unreachable!("ready matching only succeeds for the ready state");
                };
                demand.finish(&mut state);
                drop(state);
                return shared.claim(provider, cancel, generation, ready);
            }
            let matching_start = matches!(
                &state.spare,
                Spare::Starting {
                    generation,
                    provider: starting,
                    cancel,
                } if *generation == state.generation
                    && !cancel.load(Ordering::Acquire)
                    && B::same_provider(starting, provider)
                    && state.eligible(provider).is_some()
            );
            if matching_start {
                // Reuse a matching in-flight reservation rather than racing a second cold launch.
                // This waits only for source-free Ready; preparation failure still takes cold open.
                // A caller canceling its wait cannot cancel the shared launch needed by another
                // viewer. Policy invalidation, mismatched demand and service shutdown own that flag.
                let (state, _) = shared
                    .changed
                    .wait_timeout(state, CANCEL_POLL)
                    .unwrap_or_else(|error| error.into_inner());
                drop(state);
                continue;
            }
            // A miss is ordinary cold opening. Any speculative native call keeps its reservation
            // until it returns and cleanup ends; demand may wait cancellably for that real capacity.
            state.invalidate();
            shared.changed.notify_all();
            if state.children < CHILD_LIMIT {
                let permit = shared.reserve(&mut state);
                demand.finish(&mut state);
                drop(state);
                return Ok(Acquisition {
                    prepared: None,
                    permit,
                });
            }
            let (state, _) = shared
                .changed
                .wait_timeout(state, CANCEL_POLL)
                .unwrap_or_else(|error| error.into_inner());
            drop(state);
        }
    }
}

impl<B: Backend> Shared<B> {
    fn lock(&self) -> MutexGuard<'_, State<B>> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn reserve(self: &Arc<Self>, state: &mut State<B>) -> ChildPermit {
        state.children += 1;
        let shared = Arc::downgrade(self);
        ChildPermit {
            release: Some(Box::new(move || {
                if let Some(shared) = shared.upgrade() {
                    let mut state = shared.lock();
                    state.children -= 1;
                    shared.changed.notify_all();
                }
            })),
        }
    }

    fn claim(
        &self,
        provider: &B::Provider,
        cancel: &AtomicBool,
        generation: u64,
        ready: ReadyResource<B::Prepared>,
    ) -> io::Result<Acquisition<B::Prepared>> {
        // Revalidate off-lock before Open. A concurrent catalog invalidation also prevents adoption.
        // Keep the owning struct intact so unwinding also cleans the process before its permit.
        let valid_identity = self.backend.validate(&ready.prepared, provider).is_ok();
        let (current, shutdown) = {
            let state = self.lock();
            (
                state.generation == generation
                    && state.eligible(provider).is_some_and(|catalog_provider| {
                        B::prepared_matches(&ready.prepared, catalog_provider)
                    }),
                state.shutdown,
            )
        };
        if shutdown || cancel.load(Ordering::Acquire) {
            return Err(cancelled());
        }
        if valid_identity && current {
            Ok(Acquisition {
                prepared: Some(ready.prepared),
                permit: ready.permit,
            })
        } else {
            // Cleanup precedes cold fallback; retain this lifetime permit throughout.
            drop(ready.prepared);
            Ok(Acquisition {
                prepared: None,
                permit: ready.permit,
            })
        }
    }

    fn run(self: Arc<Self>) {
        loop {
            match self.next_work() {
                Work::Prepare {
                    generation,
                    provider,
                    cancel,
                    permit,
                } => {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.backend.prepare(&provider, &cancel)
                    }))
                    .unwrap_or_else(|_| Err(io::Error::other("Image preparation panicked")));
                    self.finish_prepare(generation, provider, cancel, permit, result);
                }
                Work::Retire(ready) => {
                    drop(ready);
                    self.finish_retirement();
                }
                Work::Stop => return,
            }
        }
    }

    fn next_work(self: &Arc<Self>) -> Work<B> {
        let mut state = self.lock();
        loop {
            let retire = match &state.spare {
                Spare::Ready {
                    generation,
                    expires,
                    retire,
                    ..
                } => {
                    state.shutdown
                        || state.spare_stopped
                        || *generation != state.generation
                        || *retire
                        || Instant::now() >= *expires
                }
                Spare::Empty | Spare::Starting { .. } | Spare::Retiring => false,
            };
            if retire {
                let Spare::Ready { ready, .. } =
                    std::mem::replace(&mut state.spare, Spare::Retiring)
                else {
                    unreachable!("only ready resources need idle retirement");
                };
                self.changed.notify_all();
                return Work::Retire(ready);
            }
            if (state.shutdown || state.spare_stopped) && matches!(state.spare, Spare::Empty) {
                return Work::Stop;
            }
            if matches!(state.spare, Spare::Empty)
                && state.demands == 0
                && state.children < CHILD_LIMIT
                && let Some(provider) = state.refill.take()
                && state.eligible(&provider).is_some()
            {
                let generation = state.generation;
                let cancel = Arc::new(AtomicBool::new(false));
                let permit = self.reserve(&mut state);
                state.spare = Spare::Starting {
                    generation,
                    provider: provider.clone(),
                    cancel: cancel.clone(),
                };
                self.changed.notify_all();
                return Work::Prepare {
                    generation,
                    provider,
                    cancel,
                    permit,
                };
            }
            state = if let Spare::Ready { expires, .. } = &state.spare {
                let timeout = expires.saturating_duration_since(Instant::now());
                self.changed
                    .wait_timeout(state, timeout)
                    .unwrap_or_else(|error| error.into_inner())
                    .0
            } else {
                self.changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner())
            };
        }
    }

    fn finish_prepare(
        &self,
        generation: u64,
        provider: B::Provider,
        cancel: Arc<AtomicBool>,
        permit: ChildPermit,
        result: io::Result<B::Prepared>,
    ) {
        match result {
            Ok(prepared) => {
                let mut state = self.lock();
                if !state.shutdown
                    && !state.spare_stopped
                    && state.generation == generation
                    && !cancel.load(Ordering::Acquire)
                    && B::prepared_matches(&prepared, &provider)
                    && state.eligible(&provider).is_some_and(|catalog_provider| {
                        B::prepared_matches(&prepared, catalog_provider)
                    })
                {
                    state.spare = Spare::Ready {
                        generation,
                        ready: ReadyResource { prepared, permit },
                        expires: Instant::now() + READY_LIFETIME,
                        retire: false,
                    };
                    self.changed.notify_all();
                    return;
                }
                state.spare = Spare::Retiring;
                self.changed.notify_all();
                drop(state);
                drop(prepared);
                drop(permit);
            }
            Err(_) => {
                let mut state = self.lock();
                state.spare = Spare::Retiring;
                self.changed.notify_all();
                drop(state);
                // A failed preparation has already cleaned its native child, including a canceled
                // CreateProcess that returned late. Only now can its reservation be released.
                drop(permit);
            }
        }
        self.finish_retirement();
    }

    fn finish_retirement(&self) {
        let mut state = self.lock();
        state.spare = Spare::Empty;
        self.changed.notify_all();
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Image opening was canceled")
}

#[cfg(test)]
#[path = "image_startup_tests.rs"]
mod tests;
