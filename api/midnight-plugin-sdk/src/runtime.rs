use crate::{
    guard::{contain, lock},
    host::{Content, Host},
    jobs::{Group, JobError},
    outbox::{Origin, Outbox, Output, QueueError, Ticket},
};
use midnight_plugin_api::{abi, codec, codec::Model, messages};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

pub type Work = Box<dyn FnOnce(Context) + Send + 'static>;
pub type ReadContinuation = Box<dyn FnOnce(Context, messages::ReadReply, Content) + Send + 'static>;

pub struct SendError {
    pub status: abi::Status,
    pub output: Output,
    pub continuation: Work,
}

impl std::fmt::Debug for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendError")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

struct Ready {
    context: Context,
    output: Output,
    continuation: Work,
}

struct Read {
    context: Context,
    request: messages::ReadAt,
    continuation: ReadContinuation,
}

struct State {
    outbox: Outbox,
    requests: HashMap<u64, Weak<RequestState>>,
    reads: HashMap<u64, Read>,
    ready: VecDeque<Ready>,
    ready_bytes: usize,
    closing: bool,
}

struct Core {
    host: Host,
    jobs: Arc<Group>,
    state: Mutex<State>,
    services: AtomicU64,
    contexts: Arc<AtomicUsize>,
}

struct LiveContext(Arc<AtomicUsize>);
impl Drop for LiveContext {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Clone)]
pub(crate) struct Runtime(Arc<Core>);

struct RequestState {
    core: Arc<Core>,
    ticket: Ticket,
    cancelled: AtomicBool,
    done: AtomicBool,
    _live: LiveContext,
}

/// One accepted request. Keep a clone only while asynchronous work needs it.
/// Dropping the last unfinished context reports failure rather than silently hanging.
#[derive(Clone)]
pub struct Context(Arc<RequestState>);

impl Runtime {
    pub(crate) fn new(host: Host, buffer_ids: Arc<AtomicU64>) -> Self {
        Self(Arc::new(Core {
            host,
            jobs: Group::new(128),
            services: AtomicU64::new(1),
            contexts: Arc::new(AtomicUsize::new(0)),
            state: Mutex::new(State {
                outbox: Outbox::new(buffer_ids),
                requests: HashMap::new(),
                reads: HashMap::new(),
                ready: VecDeque::new(),
                ready_bytes: 0,
                closing: false,
            }),
        }))
    }

    pub(crate) fn submit(&self, origin: Origin, work: Work) -> abi::Status {
        let context = {
            let mut state = lock(&self.0.state);
            if self.0.contexts.load(Ordering::Acquire) >= 128 {
                return abi::QUEUE_FULL;
            }
            let ticket = match state.outbox.begin(origin) {
                Ok(ticket) => ticket,
                Err(error) => return status(error),
            };
            self.0.contexts.fetch_add(1, Ordering::Acquire);
            let context = Context(Arc::new(RequestState {
                core: self.0.clone(),
                ticket,
                cancelled: AtomicBool::new(false),
                done: AtomicBool::new(false),
                _live: LiveContext(self.0.contexts.clone()),
            }));
            state
                .requests
                .retain(|_, context| context.strong_count() != 0);
            state.requests.insert(origin.id, Arc::downgrade(&context.0));
            context
        };
        self.0.schedule(&context, work, true)
    }

    pub(crate) fn cancel(&self, id: u64) -> Result<(), abi::Status> {
        let context = lock(&self.0.state)
            .requests
            .get(&id)
            .and_then(Weak::upgrade);
        let context = context.ok_or(abi::UNKNOWN_HANDLE)?;
        Context(context).cancel();
        Ok(())
    }

    pub(crate) fn drain(&self) -> Result<Option<abi::Event>, abi::Status> {
        let event = lock(&self.0.state).outbox.drain_one().map_err(status)?;
        self.0.pump();
        Ok(event)
    }

    pub(crate) fn release(&self, id: u64) -> Result<(), abi::Status> {
        lock(&self.0.state).outbox.release(id).map_err(status)?;
        self.0.pump();
        Ok(())
    }

    pub(crate) fn shutdown(&self) {
        let contexts: Vec<_> = {
            let mut state = lock(&self.0.state);
            state.closing = true;
            state.outbox.begin_shutdown();
            state.requests.values().filter_map(Weak::upgrade).collect()
        };
        for context in contexts {
            Context(context).cancel();
        }
    }

    pub(crate) fn quiescent(&self) -> bool {
        let state = lock(&self.0.state);
        state.closing
            && state.outbox.is_drained()
            && state.reads.is_empty()
            && state.ready.is_empty()
            && self.0.contexts.load(Ordering::Acquire) == 0
            && self.0.jobs.active() == 0
            && self.0.host.leases() == 0
    }

    pub(crate) fn reply(
        &self,
        origin: Origin,
        reply: messages::ReadReply,
        content: Content,
    ) -> abi::Status {
        if reply.validate().is_err() || reply.bytes as usize != content.as_ref().len() {
            return abi::INVALID;
        }
        let read = {
            let mut state = lock(&self.0.state);
            let Some(read) = state.reads.get(&origin.id) else {
                return abi::UNKNOWN_HANDLE;
            };
            let expected = read.context.0.ticket.origin();
            if expected.session != origin.session
                || expected.generation != origin.generation
                || reply.revision != read.request.revision
                || reply.offset != read.request.offset
                || reply.bytes > read.request.length
            {
                return abi::INVALID;
            }
            state.reads.remove(&origin.id).expect("validated read")
        };
        if read.context.finished() {
            return abi::OK;
        }
        let continuation = read.continuation;
        self.0.schedule(
            &read.context,
            Box::new(move |context| {
                continuation(context, reply, content);
            }),
            false,
        )
    }
}

impl Core {
    fn schedule(&self, context: &Context, work: Work, admission: bool) -> abi::Status {
        let worker_context = context.clone();
        let task = Box::new(move || {
            if worker_context.finished() {
                return;
            }
            if worker_context.is_cancelled() {
                worker_context.cancel();
                return;
            }
            if contain(|| work(worker_context.clone())).is_err() {
                let _ = worker_context.fail(abi::PANICKED, "Plugin callback panicked");
            }
        });
        let registered = match self.jobs.register(task) {
            Ok(job) => job,
            Err((error, task)) => {
                let code = match error {
                    JobError::Full => abi::QUEUE_FULL,
                    JobError::Exhausted => abi::INVALID,
                };
                self.reject(context, admission, code);
                drop(task);
                return code;
            }
        };
        let result = self
            .host
            .schedule(&registered, context.0.ticket.origin().id, abi::WORK_CPU);
        if result == abi::OK {
            registered.accept();
        } else {
            self.reject(context, admission, result);
        }
        result
    }

    fn reject(&self, context: &Context, admission: bool, code: abi::Status) {
        if admission {
            let mut state = lock(&self.state);
            if state.outbox.withdraw(context.0.ticket).is_ok() {
                context.0.done.store(true, Ordering::Release);
            }
        } else {
            let _ = context.fail(code, "Host rejected continuation work");
        }
    }

    fn finish(
        &self,
        context: &RequestState,
        kind: u32,
        payload: Box<[u8]>,
    ) -> Result<(), abi::Status> {
        let discarded = {
            let mut state = lock(&self.state);
            if context.done.load(Ordering::Acquire) {
                return Err(abi::INVALID);
            }
            if kind == abi::EVENT_COMPLETED
                && state
                    .ready
                    .iter()
                    .any(|r| r.context.0.ticket == context.ticket)
            {
                return Err(abi::INVALID);
            }
            state
                .outbox
                .finish(context.ticket, kind, payload)
                .map_err(status)?;
            context.done.store(true, Ordering::Release);
            let mut discarded = Vec::new();
            for _ in 0..state.ready.len() {
                let ready = state.ready.pop_front().expect("pending output");
                if ready.context.0.ticket == context.ticket {
                    state.ready_bytes -= ready.output.byte_len();
                    discarded.push(ready);
                } else {
                    state.ready.push_back(ready);
                }
            }
            discarded
        };
        drop(discarded);
        self.host.notify();
        Ok(())
    }

    fn pump(&self) {
        let mut resumed = Vec::new();
        let mut failed = Vec::new();
        {
            let mut state = lock(&self.state);
            for _ in 0..state.ready.len() {
                let ready = state.ready.pop_front().expect("pending output");
                let bytes = ready.output.byte_len();
                match state.outbox.emit(ready.context.0.ticket, ready.output) {
                    Ok(()) => {
                        state.ready_bytes -= bytes;
                        resumed.push((ready.context, ready.continuation));
                    }
                    Err((QueueError::Full, output)) => {
                        state.ready.push_back(Ready { output, ..ready })
                    }
                    Err((error, output)) => {
                        state.ready_bytes -= bytes;
                        failed.push((status(error), Ready { output, ..ready }));
                    }
                }
            }
        }
        if !resumed.is_empty() {
            self.host.notify();
        }
        for (context, work) in resumed {
            self.schedule(&context, work, false);
        }
        for (code, ready) in failed {
            let _ = ready.context.fail(code, "Output delivery failed");
        }
    }
}

impl Context {
    /// Queue one output and resume on a host worker when it has been accepted by
    /// the bounded delivery queue. Backpressure retains at most one output per
    /// request, 64 outputs and 8 MiB per instance, without holding a worker.
    /// Rejection returns both values intact; no continuation then runs.
    pub fn send(&self, output: Output, continuation: Work) -> Result<(), SendError> {
        let core = &self.0.core;
        {
            let mut state = lock(&core.state);
            if state
                .ready
                .iter()
                .any(|r| r.context.0.ticket == self.0.ticket)
            {
                return Err(SendError {
                    status: abi::QUEUE_FULL,
                    output,
                    continuation,
                });
            }
            match state.outbox.emit(self.0.ticket, output) {
                Ok(()) => {}
                Err((QueueError::Full, output)) => {
                    if state.ready.len() >= 64
                        || output.byte_len() > 8 * 1024 * 1024 - state.ready_bytes
                    {
                        return Err(SendError {
                            status: abi::QUEUE_FULL,
                            output,
                            continuation,
                        });
                    }
                    state.ready_bytes += output.byte_len();
                    state.ready.push_back(Ready {
                        context: self.clone(),
                        output,
                        continuation,
                    });
                    return Ok(());
                }
                Err((error, output)) => {
                    return Err(SendError {
                        status: status(error),
                        output,
                        continuation,
                    });
                }
            }
        }
        core.host.notify();
        core.schedule(self, continuation, false);
        Ok(())
    }

    pub fn request_id(&self) -> u64 {
        self.0.ticket.origin().id
    }
    pub fn session(&self) -> u64 {
        self.0.ticket.origin().session
    }
    pub fn generation(&self) -> u64 {
        self.0.ticket.origin().generation
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire) || self.0.core.host.cancelled(self.request_id())
    }

    pub fn complete(&self) -> Result<(), abi::Status> {
        self.0
            .core
            .finish(&self.0, abi::EVENT_COMPLETED, Box::new([]))
    }

    pub fn fail(&self, code: u32, message: &str) -> Result<(), abi::Status> {
        if message.len() > 4096 {
            return Err(abi::INVALID);
        }
        let payload = codec::encode(&messages::Failure {
            code,
            message: message.to_owned(),
        })
        .map_err(|_| abi::INVALID)?
        .into_boxed_slice();
        self.0.core.finish(&self.0, abi::EVENT_FAILED, payload)
    }

    /// Schedule a bounded positional read and return the worker permit immediately.
    /// Exactly one read may be pending for this request. The callback runs on a new
    /// host permit, never inside submit_service or while an SDK mutex is held.
    pub fn read(
        &self,
        request: messages::ReadAt,
        continuation: ReadContinuation,
    ) -> Result<(), abi::Status> {
        let payload = codec::encode(&request).map_err(|_| abi::INVALID)?;
        let core = &self.0.core;
        let id = {
            let mut state = lock(&core.state);
            if self.finished() || state.closing {
                return Err(abi::SHUTTING_DOWN);
            }
            if state.reads.len() >= 64
                || state
                    .reads
                    .values()
                    .any(|r| r.context.0.ticket == self.0.ticket)
            {
                return Err(abi::QUEUE_FULL);
            }
            let id = core
                .services
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| abi::INVALID)?;
            state.reads.insert(
                id,
                Read {
                    context: self.clone(),
                    request,
                    continuation,
                },
            );
            id
        };
        let result = core.host.service(
            Origin {
                id,
                ..self.0.ticket.origin()
            },
            abi::SERVICE_READ_AT,
            &payload,
        );
        if result != abi::OK {
            let rejected = lock(&core.state).reads.remove(&id);
            drop(rejected);
            return Err(result);
        }
        Ok(())
    }

    fn finished(&self) -> bool {
        self.0.done.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        let _ = self
            .0
            .core
            .finish(&self.0, abi::EVENT_CANCELLED, Box::new([]));
    }
}

impl Drop for RequestState {
    fn drop(&mut self) {
        if !self.done.load(Ordering::Acquire) {
            let kind = if self.cancelled.load(Ordering::Acquire) {
                abi::EVENT_CANCELLED
            } else {
                abi::EVENT_FAILED
            };
            let payload = if kind == abi::EVENT_FAILED {
                codec::encode(&messages::Failure {
                    code: abi::INVALID,
                    message: "Plugin dropped unfinished request".into(),
                })
                .unwrap_or_default()
                .into_boxed_slice()
            } else {
                Box::new([])
            };
            let _ = self.core.finish(self, kind, payload);
        }
    }
}

fn status(error: QueueError) -> abi::Status {
    match error {
        QueueError::Full => abi::QUEUE_FULL,
        QueueError::Invalid | QueueError::Exhausted => abi::INVALID,
        QueueError::Unknown => abi::UNKNOWN_HANDLE,
        QueueError::Closed => abi::SHUTTING_DOWN,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
