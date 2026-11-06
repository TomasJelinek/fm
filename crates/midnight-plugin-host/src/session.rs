use crate::{
    CapturedFile, Error, EventKind, HostEvent, PluginInfo,
    api::{abi, codec, messages},
    callbacks::Bridge,
    loaded::Loaded,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    time::Duration,
};

pub(crate) const MAX_REQUESTS: usize = 32;
pub(crate) const MAX_EVENTS: usize = 64;
pub(crate) const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
pub(crate) struct Command {
    pub id: u64,
    pub kind: u32,
    pub payload: Vec<u8>,
}
#[derive(Default)]
pub(crate) struct State {
    pub requests: HashMap<u64, bool>,
    pub events: VecDeque<(HostEvent, usize)>,
    pub bytes: usize,
    pub fatal: Option<HostEvent>,
}
pub(crate) struct Shared {
    pub state: Mutex<State>,
    pub closing: AtomicBool,
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
        lock(&shared.state).requests.insert(1, false);
        send.try_send(Command {
            id: 1,
            kind: abi::REQUEST_OPEN,
            payload: open,
        })
        .map_err(|_| Error("Open queue full".into()))?;
        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("viewer-dispatch".into())
            .spawn(move || {
                let _permit = permit;
                crate::dispatch::run(loaded, reader, file, worker_shared.clone(), receive);
                worker_shared.closed.store(true, Ordering::Release);
            })?;
        Ok(Self {
            shared,
            commands: send,
            ids: AtomicU64::new(2),
        })
    }
    pub fn viewport(&self, viewport: messages::Viewport) -> Result<u64, Error> {
        self.submit(abi::REQUEST_VIEWPORT, &viewport)
    }
    pub fn search(&self, search: messages::Search) -> Result<u64, Error> {
        self.submit(abi::REQUEST_SEARCH, &search)
    }
    fn submit<T: codec::Model>(&self, kind: u32, value: &T) -> Result<u64, Error> {
        let payload = codec::encode(value)
            .map_err(|_| Error("Invalid or oversized viewer request".into()))?;
        let id = self
            .ids
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Error("Request IDs exhausted".into()))?;
        let mut state = lock(&self.shared.state);
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(Error("Viewer is closing".into()));
        }
        if state.requests.len() >= MAX_REQUESTS {
            return Err(Error("Viewer request queue full".into()));
        }
        state.requests.insert(id, false);
        if self
            .commands
            .try_send(Command { id, kind, payload })
            .is_err()
        {
            state.requests.remove(&id);
            return Err(Error("Viewer request queue full".into()));
        }
        Ok(id)
    }
    /// Cancellation has reserved capacity and is never queued behind data requests.
    pub fn cancel(&self, id: u64) {
        let mut state = lock(&self.shared.state);
        if let Some(cancelled) = state.requests.get_mut(&id) {
            *cancelled = true;
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
    }
    pub fn drain(&self, maximum: usize) -> Vec<HostEvent> {
        let mut state = lock(&self.shared.state);
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
        events
    }
    /// Returns immediately; cancellation, buffer release and native destruction run on backend.
    pub fn close(&self) {
        self.shared.closing.store(true, Ordering::Release);
    }
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
        state.requests.remove(&id);
        state.fatal.get_or_insert(HostEvent {
            request_id: id.max(1),
            generation: self.generation,
            kind: EventKind::Failed(messages::Failure {
                code: abi::INVALID,
                message,
            }),
        });
        self.closing.store(true, Ordering::Release);
    }
    pub fn cancelled(&self, id: u64) -> bool {
        self.closing.load(Ordering::Acquire)
            || lock(&self.state).requests.get(&id).copied().unwrap_or(true)
    }
    pub fn failure(&self, id: u64, message: String) {
        self.push(
            HostEvent {
                request_id: id,
                generation: self.generation,
                kind: EventKind::Failed(messages::Failure {
                    code: abi::INVALID,
                    message,
                }),
            },
            8192,
        );
    }
    pub fn push(&self, event: HostEvent, bytes: usize) {
        let terminal = matches!(
            event.kind,
            EventKind::Completed | EventKind::Cancelled | EventKind::Failed(_)
        );
        let mut state = lock(&self.state);
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
            state.fatal.get_or_insert(HostEvent {
                request_id: 1,
                generation: self.generation,
                kind: EventKind::Failed(messages::Failure {
                    code: abi::QUEUE_FULL,
                    message: "Viewer output budget exceeded".into(),
                }),
            });
            self.closing.store(true, Ordering::Release);
        }
    }
    pub fn room(&self) -> bool {
        let state = lock(&self.state);
        self.closing.load(Ordering::Acquire)
            || (state.events.len() < MAX_EVENTS - MAX_REQUESTS
                && state.bytes <= MAX_EVENT_BYTES - 4 * 1024 * 1024)
    }
}

pub(crate) fn worker(
    bridge: Arc<Bridge>,
    loaded: Arc<Loaded>,
    receive: mpsc::Receiver<crate::callbacks::Job>,
) {
    let _library = loaded;
    run_jobs(bridge, receive);
}

pub(crate) fn run_jobs(bridge: Arc<Bridge>, receive: mpsc::Receiver<crate::callbacks::Job>) {
    while !bridge.stop.load(Ordering::Acquire) {
        if let Ok(job) = receive.recv_timeout(Duration::from_millis(10)) {
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
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            state: Mutex::new(State::default()),
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
        lock(&shared.state).requests.insert(1, false);
        shared.push(
            HostEvent {
                request_id: 1,
                generation: 7,
                kind: EventKind::Progress(messages::Progress {
                    completed: 1,
                    total: Some(10),
                    phase: 0,
                }),
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
