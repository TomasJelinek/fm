use crate::{
    CapturedFile, Error, EventKind, HostEvent, PluginInfo,
    api::{abi, codec, messages},
    callbacks::Bridge,
    loaded::Loaded,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
};

/// Request ID of every session's Open request. Failures of the session itself rather
/// than of one request (host setup, plugin creation, drain or read-service failures and
/// the output budget) are reported with this ID too. Other requests are numbered from
/// the next ID upward.
pub const OPEN_REQUEST_ID: u64 = 1;
pub(crate) const MAX_REQUESTS: usize = 32;
pub(crate) const MAX_EVENTS: usize = 64;
pub(crate) const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;

/// Local admission failures are distinct from native request-scoped Failed
/// events. Only Capacity is temporary; callers never classify error prose.
#[derive(Clone, Debug)]
pub enum AdmissionError {
    Capacity,
    Closing,
    Invalid(Error),
}
impl AdmissionError {
    fn into_error(self) -> Error {
        match self {
            Self::Capacity => Error("Viewer request queue full".into()),
            Self::Closing => Error("Viewer is closing".into()),
            Self::Invalid(error) => error,
        }
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
pub(crate) struct Command {
    pub id: u64,
    pub kind: u32,
    pub payload: Vec<u8>,
}

/// A single application-wide background request leaves command/work capacity
/// available for foreground viewport delivery. Cancellation retains ownership
/// until the plugin actually reports its terminal or the dispatcher shuts down.
struct BackgroundPermit(Arc<AtomicBool>);
impl BackgroundPermit {
    fn acquire() -> Result<Self, AdmissionError> {
        static BUSY: OnceLock<Arc<AtomicBool>> = OnceLock::new();
        Self::reserve(
            BUSY.get_or_init(|| Arc::new(AtomicBool::new(false)))
                .clone(),
        )
    }

    fn reserve(busy: Arc<AtomicBool>) -> Result<Self, AdmissionError> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| AdmissionError::Capacity)?;
        Ok(Self(busy))
    }
}
impl Drop for BackgroundPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) struct Preparation {
    pub request: messages::PrepareSource,
    pub result_seen: bool,
}

pub(crate) struct RequestState {
    pub kind: u32,
    pub cancelled: bool,
    pub submitted: bool,
    pub cancel_delivered: bool,
    pub accepted: bool,
    pub preparation: Option<Preparation>,
    _background: Option<BackgroundPermit>,
}
impl Default for RequestState {
    fn default() -> Self {
        Self::new(abi::REQUEST_VIEWPORT)
    }
}
impl RequestState {
    pub fn new(kind: u32) -> Self {
        Self {
            kind,
            cancelled: false,
            submitted: false,
            cancel_delivered: false,
            accepted: false,
            preparation: None,
            _background: None,
        }
    }

    pub fn preparation(request: messages::PrepareSource) -> Self {
        Self {
            preparation: Some(Preparation {
                request,
                result_seen: false,
            }),
            ..Self::new(abi::REQUEST_PREPARE_SOURCE)
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub requests: HashMap<u64, RequestState>,
    pub opened: bool,
    pub events: VecDeque<(HostEvent, usize)>,
    pub bytes: usize,
    /// Reserved slot for the session-fatal failure (`HostEvent::fatal`). The
    /// dispatcher's `discard_output` never drops it; only `drain` takes it.
    pub fatal: Option<HostEvent>,
    /// A fatal failure was recorded. Later ones, such as failures of the teardown it
    /// started, are not reported, even after the first was drained.
    pub failed: bool,
}
pub(crate) struct Shared {
    pub state: Mutex<State>,
    pub dispatcher: crate::wake::Wake,
    /// The consumer's wake: output became ready, or (`notify_final`) the dispatcher ended.
    pub ready: crate::wake::Wake,
    pub closing: AtomicBool,
    /// Set by the dispatch thread after `dispatch::run` returned and the session quota
    /// was released, immediately before the final `ready` signal.
    pub closed: AtomicBool,
    pub generation: u64,
    pub revision: u64,
    pub length: u64,
    pub mode: u16,
}
pub struct Session {
    shared: Arc<Shared>,
    commands: SyncSender<Command>,
    ids: AtomicU64,
}
pub(crate) struct SessionPermit(pub Arc<AtomicUsize>);
impl Drop for SessionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

impl Session {
    pub(crate) fn open(
        info: &PluginInfo,
        file: CapturedFile,
        generation: u64,
        mode: u16,
        sessions: Arc<AtomicUsize>,
    ) -> Result<Self, Error> {
        if !info.modes.contains(&mode) {
            return Err(Error("Viewer mode not declared by plugin".into()));
        }
        sessions
            .try_update(Ordering::Acquire, Ordering::Relaxed, |n| {
                (n < 4).then_some(n + 1)
            })
            .map_err(|_| Error("Viewer session limit reached".into()))?;
        let permit = SessionPermit(sessions);
        let reader = file.open()?;
        let loaded = Loaded::acquire(info)?;
        let (send, receive) = mpsc::sync_channel(MAX_REQUESTS);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            dispatcher: Default::default(),
            ready: Default::default(),
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            generation,
            revision: file.revision(),
            length: file.length(),
            mode,
        });
        let open = codec::encode(&messages::Open {
            reader: 1,
            length: Some(file.length()),
            revision: file.revision(),
            mode,
            display_name: file.display_name(),
        })
        .map_err(|_| Error("Invalid open request".into()))?;
        lock(&shared.state)
            .requests
            .insert(OPEN_REQUEST_ID, RequestState::new(abi::REQUEST_OPEN));
        send.try_send(Command {
            id: OPEN_REQUEST_ID,
            kind: abi::REQUEST_OPEN,
            payload: open,
        })
        .map_err(|_| Error("Open queue full".into()))?;
        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("viewer-dispatch".into())
            .spawn(move || {
                {
                    let _permit = permit;
                    crate::dispatch::run(loaded, reader, file, worker_shared.clone(), receive);
                    // Quiescent native destruction completed before releasing background admission.
                    lock(&worker_shared.state).requests.clear();
                }
                // Release the session quota before notifying consumers that it ended.
                worker_shared.ended();
            })?;
        Ok(Self {
            shared,
            commands: send,
            ids: AtomicU64::new(OPEN_REQUEST_ID + 1),
        })
    }
    pub fn viewport(&self, viewport: messages::Viewport) -> Result<u64, Error> {
        self.try_viewport(viewport)
            .map_err(AdmissionError::into_error)
    }
    /// Typed local admission for a foreground or internal refresh request.
    pub fn try_viewport(&self, viewport: messages::Viewport) -> Result<u64, AdmissionError> {
        self.submit(abi::REQUEST_VIEWPORT, &viewport, None)
    }
    pub fn search(&self, search: messages::Search) -> Result<u64, Error> {
        self.submit(abi::REQUEST_SEARCH, &search, None)
            .map_err(AdmissionError::into_error)
    }
    /// Optional independent lookup. Unsupported plugins report a request-scoped
    /// Failed event with code UNSUPPORTED; ordinary viewing remains available.
    pub fn locate(&self, locate: messages::Locate) -> Result<u64, Error> {
        self.try_locate(locate).map_err(AdmissionError::into_error)
    }
    /// Typed admission into the shared Locate/preparation background capacity.
    pub fn try_locate(&self, locate: messages::Locate) -> Result<u64, AdmissionError> {
        self.submit(abi::REQUEST_LOCATE, &locate, None)
    }
    /// Prepare exact syntax state for a later matching foreground viewport.
    /// The typed result remains provisional until the matching Completed event.
    /// Only an opened Source session with the captured revision is eligible;
    /// older plugins can fail this optional request with UNSUPPORTED.
    pub fn prepare_source(&self, preparation: messages::PrepareSource) -> Result<u64, Error> {
        self.try_prepare_source(preparation)
            .map_err(AdmissionError::into_error)
    }
    /// Typed local admission; native rejection codes arrive unchanged in Failed.
    pub fn try_prepare_source(
        &self,
        preparation: messages::PrepareSource,
    ) -> Result<u64, AdmissionError> {
        self.submit(
            abi::REQUEST_PREPARE_SOURCE,
            &preparation,
            Some(preparation.clone()),
        )
    }

    fn submit<T: codec::Model>(
        &self,
        kind: u32,
        value: &T,
        preparation: Option<messages::PrepareSource>,
    ) -> Result<u64, AdmissionError> {
        let payload = codec::encode(value).map_err(|_| {
            AdmissionError::Invalid(Error("Invalid or oversized viewer request".into()))
        })?;
        let id = self
            .ids
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| AdmissionError::Invalid(Error("Request IDs exhausted".into())))?;
        let mut state = lock(&self.shared.state);
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(AdmissionError::Closing);
        }
        if state.requests.len() >= MAX_REQUESTS {
            return Err(AdmissionError::Capacity);
        }
        if let Some(preparation) = &preparation
            && (!state.opened
                || self.shared.mode != crate::api::viewer::MODE_SOURCE
                || preparation.revision != self.shared.revision
                || preparation.viewport.source_anchor > self.shared.length)
        {
            return Err(AdmissionError::Invalid(Error(
                "Preparation does not match the opened source".into(),
            )));
        }
        let mut request =
            preparation.map_or_else(|| RequestState::new(kind), RequestState::preparation);
        if matches!(kind, abi::REQUEST_LOCATE | abi::REQUEST_PREPARE_SOURCE) {
            request._background = Some(BackgroundPermit::acquire()?);
        }
        state.requests.insert(id, request);
        if let Err(error) = self.commands.try_send(Command { id, kind, payload }) {
            state.requests.remove(&id);
            return Err(match error {
                mpsc::TrySendError::Full(_) => AdmissionError::Capacity,
                mpsc::TrySendError::Disconnected(_) => AdmissionError::Closing,
            });
        }
        drop(state);
        self.shared.dispatcher.notify();
        Ok(id)
    }
    /// Cancellation has reserved capacity and is never queued behind data requests.
    pub fn cancel(&self, id: u64) {
        let mut state = lock(&self.shared.state);
        if let Some(request) = state.requests.get_mut(&id) {
            request.cancelled = true;
        }
        for (event, _) in &mut state.events {
            if event.request_id == id && matches!(event.kind, EventKind::Completed) {
                event.kind = EventKind::Cancelled;
            }
        }
        let mut removed = 0;
        state.events.retain(|(event, bytes)| {
            let keep = event.request_id != id
                || matches!(
                    event.kind,
                    EventKind::Completed | EventKind::Cancelled | EventKind::Failed(_)
                );
            if !keep {
                removed += bytes;
            }
            keep
        });
        state.bytes -= removed;
        drop(state);
        self.shared.dispatcher.notify();
    }
    /// Register a bounded consumer wake. It must only enqueue work, returning
    /// whether the signal was accepted; plugin callbacks never reenter consumers.
    /// It can run on any thread: on backend threads, the dispatcher included, and
    /// synchronously on the caller of this method (when a signal is already pending)
    /// or of `drain` (when output remains). So it must never block, nor take a lock
    /// that the consumer holds while it calls those methods. Ready output signals it
    /// at most once until the next `drain`. When the dispatcher has finished
    /// (`is_closed` is true), it is signalled once more even if earlier output was
    /// never drained, so a consumer can wait for a closing session without polling.
    pub fn notify_on_ready(&self, notify: Arc<dyn Fn() -> bool + Send + Sync>) {
        self.shared.ready.bind(notify);
    }
    /// Take at most `maximum` events (64 at most per call). A session-fatal failure
    /// (`HostEvent::fatal`) comes first, then older events in their order.
    pub fn drain(&self, maximum: usize) -> Vec<HostEvent> {
        if maximum == 0 {
            return Vec::new();
        }
        let mut state = lock(&self.shared.state);
        self.shared.ready.reset();
        let mut events = Vec::with_capacity(maximum.min(MAX_EVENTS));
        if maximum != 0
            && let Some(fatal) = state.fatal.take()
        {
            events.push(fatal);
        }
        for _ in events.len()..maximum.min(MAX_EVENTS) {
            let Some((event, bytes)) = state.events.pop_front() else {
                break;
            };
            state.bytes -= bytes;
            events.push(event);
        }
        let remaining = state.fatal.is_some() || !state.events.is_empty();
        drop(state);
        if remaining {
            self.shared.ready.notify();
        }
        if !events.is_empty() {
            self.shared.dispatcher.notify();
        }
        events
    }
    /// Returns immediately; cancellation, buffer release and native destruction run on backend.
    pub fn close(&self) {
        self.shared.closing.store(true, Ordering::Release);
        self.shared.dispatcher.notify();
    }
    /// The dispatcher finished: the plugin instance was destroyed, its workers and output
    /// were released and the session quota was returned. Only an undrained fatal failure
    /// can still be drained. The `notify_on_ready` callback is signalled after this.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

impl Shared {
    /// Runs once on the dispatch thread after `dispatch::run` returned and the session
    /// permit was dropped. The final signal reaches a consumer that stopped draining.
    pub(crate) fn ended(&self) {
        self.closed.store(true, Ordering::Release);
        self.ready.notify_final();
    }
    pub fn discard_output(&self) {
        let discarded = {
            let mut state = lock(&self.state);
            state.bytes = 0;
            std::mem::take(&mut state.events)
        };
        drop(discarded);
    }
    pub fn fatal(&self, id: u64, message: String) {
        let mut message = message;
        if message.len() > 4096 {
            let mut end = 4096;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        let mut state = lock(&self.state);
        // Background admission stays owned until a native terminal or quiescent shutdown.
        if state
            .requests
            .get(&id)
            .is_none_or(|request| request._background.is_none())
        {
            state.requests.remove(&id);
        }
        let first = !std::mem::replace(&mut state.failed, true);
        if first {
            state.fatal = Some(HostEvent {
                request_id: id.max(OPEN_REQUEST_ID),
                generation: self.generation,
                kind: EventKind::Failed(messages::Failure {
                    code: abi::INVALID,
                    message,
                }),
                fatal: true,
            });
        }
        // Under the state lock: `push` queues nothing newer than the fatal failure.
        self.closing.store(true, Ordering::Release);
        drop(state);
        if first {
            self.ready.notify();
        }
        self.dispatcher.notify();
    }
    pub fn cancelled(&self, id: u64) -> bool {
        self.closing.load(Ordering::Acquire)
            || lock(&self.state)
                .requests
                .get(&id)
                .is_none_or(|request| request.cancelled)
    }
    /// A request-scoped failure: the session continues.
    pub fn failure(&self, id: u64, message: String) {
        self.push(
            HostEvent {
                request_id: id,
                generation: self.generation,
                kind: EventKind::Failed(messages::Failure {
                    code: abi::INVALID,
                    message,
                }),
                fatal: false,
            },
            8192,
        );
    }
    pub fn push(&self, mut event: HostEvent, bytes: usize) {
        let terminal = matches!(
            event.kind,
            EventKind::Completed | EventKind::Cancelled | EventKind::Failed(_)
        );
        let mut state = lock(&self.state);
        if state
            .requests
            .get(&event.request_id)
            .is_some_and(|request| request.cancelled)
        {
            if !terminal {
                return;
            }
            if matches!(event.kind, EventKind::Completed) {
                event.kind = EventKind::Cancelled;
            }
        }
        if terminal {
            state.requests.remove(&event.request_id);
        }
        if self.closing.load(Ordering::Acquire) {
            return;
        }
        if state.events.len() < MAX_EVENTS && bytes <= MAX_EVENT_BYTES - state.bytes {
            state.bytes += bytes;
            state.events.push_back((event, bytes));
        } else {
            if !std::mem::replace(&mut state.failed, true) {
                state.fatal = Some(HostEvent {
                    request_id: OPEN_REQUEST_ID,
                    generation: self.generation,
                    kind: EventKind::Failed(messages::Failure {
                        code: abi::QUEUE_FULL,
                        message: "Viewer output budget exceeded".into(),
                    }),
                    fatal: true,
                });
            }
            self.closing.store(true, Ordering::Release);
        }
        drop(state);
        self.ready.notify();
    }
    pub fn room(&self) -> bool {
        let state = lock(&self.state);
        self.closing.load(Ordering::Acquire)
            || (state.events.len() < MAX_EVENTS - MAX_REQUESTS
                && state.bytes <= MAX_EVENT_BYTES - 4 * 1024 * 1024)
    }
}

#[cfg(test)]
#[path = "source_preparation_session_tests.rs"]
mod source_preparation;

pub(crate) fn worker(
    bridge: Arc<Bridge>,
    loaded: Arc<Loaded>,
    receive: mpsc::Receiver<crate::callbacks::Job>,
) {
    let _library = loaded;
    run_jobs(bridge, receive);
}

/// Block until work arrives. `Bridge::close` disconnects the queue after the
/// dispatcher's final quiescence check; already accepted jobs still run or drop.
pub(crate) fn run_jobs(bridge: Arc<Bridge>, receive: mpsc::Receiver<crate::callbacks::Job>) {
    while let Ok(job) = receive.recv() {
        if !bridge.shared.cancelled(job.0.request_id) {
            // SAFETY: accepted job owns its opaque context and pins the library on this worker.
            unsafe {
                job.0.run.unwrap()(job.0.context);
            }
        }
        // SAFETY: exactly one drop per accepted job, including cancelled jobs.
        unsafe {
            job.0.drop.unwrap()(job.0.context);
        }
        bridge.jobs.fetch_sub(1, Ordering::Release);
        bridge.shared.dispatcher.notify();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State::default()),
            dispatcher: Default::default(),
            ready: Default::default(),
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            generation: 7,
            revision: 9,
            length: 64,
            mode: crate::api::viewer::MODE_TEXT,
        })
    }
    #[test]
    fn fatal_diagnostic_survives_backend_output_cleanup_in_reserved_slot() {
        let shared = shared();
        let (commands, _) = mpsc::sync_channel(1);
        let session = Session {
            shared: shared.clone(),
            commands,
            ids: AtomicU64::new(2),
        };
        shared.fatal(1, "Malformed plugin output".into());
        shared.discard_output();
        assert!(matches!(
            session.drain(64).as_slice(),
            [HostEvent {
                kind: EventKind::Failed(_),
                ..
            }]
        ));
        assert!(session.drain(64).is_empty());
        assert_eq!(lock(&shared.state).bytes, 0);
    }

    #[test]
    fn cancelling_discards_already_queued_nonterminal_output() {
        let shared = shared();
        lock(&shared.state)
            .requests
            .insert(1, RequestState::new(abi::REQUEST_VIEWPORT));
        shared.push(
            HostEvent {
                request_id: 1,
                generation: 7,
                kind: EventKind::Progress(messages::Progress {
                    completed: 1,
                    total: Some(10),
                    phase: 0,
                }),
                fatal: false,
            },
            1024,
        );
        let (commands, _) = mpsc::sync_channel(1);
        let session = Session {
            shared: shared.clone(),
            commands,
            ids: AtomicU64::new(2),
        };
        session.cancel(1);
        assert!(session.drain(64).is_empty());
        assert_eq!(lock(&shared.state).bytes, 0);
    }

    fn detached(shared: &Arc<Shared>) -> Session {
        let (commands, _) = mpsc::sync_channel(1);
        Session {
            shared: shared.clone(),
            commands,
            ids: AtomicU64::new(OPEN_REQUEST_ID + 1),
        }
    }

    /// Queue an ordinary event of a tracked request, as dispatch does for plugin output.
    fn queue(shared: &Shared, id: u64, kind: EventKind) {
        lock(&shared.state)
            .requests
            .insert(id, RequestState::default());
        shared.push(
            HostEvent {
                request_id: id,
                generation: 7,
                kind,
                fatal: false,
            },
            1024,
        );
    }

    #[test]
    fn fatal_failure_is_marked_and_drained_ahead_of_older_output_beyond_one_drain() {
        let shared = shared();
        let session = detached(&shared);
        // More older output than a consumer's 32-event drain, including a request-scoped
        // failure the session survives.
        shared.failure(41, "Plugin rejected request (3)".into());
        for id in 0..39 {
            queue(&shared, 100 + id, EventKind::Completed);
        }
        assert!(!shared.closing.load(Ordering::Acquire));
        shared.fatal(9, "Malformed plugin output".into());
        shared.fatal(10, "a later fatal report".into());
        queue(&shared, 200, EventKind::Completed);
        let first = session.drain(32);
        assert_eq!(first.len(), 32);
        match &first[0] {
            HostEvent {
                request_id: 9,
                generation: 7,
                kind: EventKind::Failed(failure),
                fatal: true,
            } => {
                assert_eq!(failure.code, abi::INVALID);
                assert_eq!(failure.message, "Malformed plugin output");
            }
            other => panic!("the fatal failure must lead the drain: {other:?}"),
        }
        assert!(matches!(
            &first[1],
            HostEvent {
                request_id: 41,
                kind: EventKind::Failed(_),
                fatal: false,
                ..
            }
        ));
        // A failure of the teardown it started is not reported as another fatal failure.
        shared.fatal(OPEN_REQUEST_ID, "Plugin drain failed (3)".into());
        // Until the dispatcher discards it, older output follows unmarked; nothing newer
        // than the fatal failure was queued.
        let second = session.drain(32);
        assert_eq!(second.len(), 40 - 31);
        assert!(
            first[1..]
                .iter()
                .chain(&second)
                .all(|event| !event.fatal && event.request_id < 200)
        );
        assert!(session.drain(32).is_empty());
    }

    #[test]
    fn output_overflow_and_session_failures_use_the_open_request_id() {
        let shared = shared();
        let session = detached(&shared);
        for id in 0..MAX_EVENTS as u64 {
            queue(&shared, 100 + id, EventKind::Completed);
        }
        queue(&shared, 999, EventKind::Completed);
        let drained = session.drain(1);
        assert!(matches!(
            drained.as_slice(),
            [HostEvent {
                request_id: OPEN_REQUEST_ID,
                kind: EventKind::Failed(messages::Failure {
                    code: abi::QUEUE_FULL,
                    ..
                }),
                fatal: true,
                ..
            }]
        ));
        let other = self::shared();
        other.fatal(0, "Host callback registration failed".into());
        assert!(matches!(
            detached(&other).drain(64).as_slice(),
            [HostEvent {
                request_id: OPEN_REQUEST_ID,
                fatal: true,
                ..
            }]
        ));
    }

    #[test]
    fn the_end_of_dispatch_signals_a_consumer_that_stopped_draining() {
        let shared = shared();
        let session = detached(&shared);
        let (signal, signals) = mpsc::sync_channel(4);
        session.notify_on_ready(Arc::new(move || signal.try_send(()).is_ok()));
        queue(&shared, 5, EventKind::Completed);
        queue(&shared, 6, EventKind::Completed);
        assert_eq!(signals.try_iter().count(), 1, "ready output signals once");
        // The consumer closed the session and waits for its end without draining it.
        session.close();
        assert!(!session.is_closed());
        shared.ended();
        assert!(session.is_closed());
        assert_eq!(
            signals.try_iter().count(),
            1,
            "the end is signalled although earlier output was never drained"
        );
        // A consumer that binds after the end still learns about it.
        let late = self::shared();
        late.ended();
        let (signal, signals) = mpsc::sync_channel(1);
        detached(&late).notify_on_ready(Arc::new(move || signal.try_send(()).is_ok()));
        assert_eq!(signals.try_iter().count(), 1);
    }

    #[test]
    fn command_admission_is_bounded_while_cancellation_and_close_stay_available() {
        let shared = shared();
        let (commands, _receiver) = mpsc::sync_channel(MAX_REQUESTS);
        let session = Session {
            shared: shared.clone(),
            commands,
            ids: AtomicU64::new(1),
        };
        for _ in 0..MAX_REQUESTS {
            session
                .search(messages::Search {
                    query: "x".into(),
                    source_anchor: 0,
                    backwards: false,
                    case_sensitive: true,
                })
                .unwrap();
        }
        assert!(
            session
                .search(messages::Search {
                    query: "x".into(),
                    source_anchor: 0,
                    backwards: false,
                    case_sensitive: true
                })
                .is_err()
        );
        session.cancel(1);
        assert!(shared.cancelled(1));
        session.close();
        assert!(shared.cancelled(32));
    }
}
