//! Per-file report ownership with cleanup independent of executors and runtimes.
//! Nodes are admitted before native creation and reused only after actual cleanup.
use crate::retirement::{RetirementLease, WorkerOwned};
use std::{
    fmt, io,
    ops::Deref,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex},
    thread,
};

pub(crate) const RETIREMENT_CAPACITY: usize = 1;
type Payload = Box<dyn Send + 'static>;
type Worker = Box<dyn FnOnce() + Send + 'static>;
type SharedOwner = Arc<WorkerOwned<Shared>>;

/// Explicit report factory prepared at engine startup, before native report creation.
/// Clones retain cleanup availability without retaining original filesystem services.
#[derive(Clone)]
pub struct ReportArena {
    control: Arc<Control>,
}
impl fmt::Debug for ReportArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReportArena").finish_non_exhaustive()
    }
}
struct Control {
    shared: SharedOwner,
}
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}
struct Queue {
    nodes: Vec<Node>,
    free: Option<usize>,
    head: Option<usize>,
    tail: Option<usize>,
    accepting: bool,
    closing: bool,
    #[cfg(test)]
    stop_worker: bool,
    #[cfg(test)]
    panic_worker: bool,
}
struct Node {
    payload: Option<Payload>,
    next: Option<usize>,
}
struct Token {
    control: Arc<Control>,
    node: usize,
}

/// Shared by every raw writer/prefix reference to the same native file.
#[derive(Clone)]
pub(crate) struct Retention {
    _token: Arc<Token>,
}
pub(crate) struct Reservation {
    retention: Retention,
}
/// Field order releases the raw reference before publishing its last token.
pub(crate) struct Retained<T: Send + Sync + 'static> {
    value: Arc<T>,
    retention: Retention,
}
impl<T: Send + Sync + 'static> Deref for Retained<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T: Send + Sync + 'static> Retained<T> {
    pub(crate) fn retention(&self) -> Retention {
        self.retention.clone()
    }
}

impl ReportArena {
    pub(crate) fn try_start(lease: RetirementLease) -> io::Result<Self> {
        Self::start_with(lease, |worker| {
            thread::Builder::new()
                .name("fm-report-cleanup".into())
                .spawn(worker)
                .map(drop)
        })
    }
    fn start_with(
        lease: RetirementLease,
        spawn: impl FnOnce(Worker) -> io::Result<()>,
    ) -> io::Result<Self> {
        let shared = Arc::new(lease.own(Shared {
            queue: Mutex::new(Queue {
                nodes: Vec::new(),
                free: None,
                head: None,
                tail: None,
                accepting: true,
                closing: false,
                #[cfg(test)]
                stop_worker: false,
                #[cfg(test)]
                panic_worker: false,
            }),
            ready: Condvar::new(),
        }));
        let arena = Self {
            control: Arc::new(Control {
                shared: shared.clone(),
            }),
        };
        // No native file or public factory can escape if worker startup fails.
        spawn(Box::new(move || run(shared)))?;
        Ok(arena)
    }
    pub(crate) fn reserve(&self) -> io::Result<Reservation> {
        let shared = &self.control.shared;
        let mut queue = shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !queue.accepting {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Report cleanup worker is unavailable",
            ));
        }
        let node = if let Some(node) = queue.free {
            queue.free = queue.nodes[node].next.take();
            node
        } else {
            queue.nodes.try_reserve(1).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "Report ownership allocation failed",
                )
            })?;
            let node = queue.nodes.len();
            queue.nodes.push(Node {
                payload: None,
                next: None,
            });
            node
        };
        drop(queue);
        Ok(Reservation {
            retention: Retention {
                _token: Arc::new(Token {
                    control: self.control.clone(),
                    node,
                }),
            },
        })
    }
    #[cfg(test)]
    pub(crate) fn testing() -> Self {
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        Self::try_start(domain.try_reserve().unwrap()).unwrap()
    }
}
impl Reservation {
    /// Call on the creating native worker, before any raw value escapes its closure.
    /// An admitted node also protects adoption if the cleanup worker exits meanwhile.
    pub(crate) fn retain<T: Send + Sync + 'static>(self, value: T) -> Retained<T> {
        let value = Arc::new(value);
        let token = &self.retention._token;
        let mut queue = token
            .control
            .shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.nodes[token.node].payload = Some(Box::new(value.clone()));
        drop(queue);
        Retained {
            value,
            retention: self.retention,
        }
    }
}
impl Drop for Token {
    fn drop(&mut self) {
        let shared = &self.control.shared;
        let mut queue = shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if queue.nodes[self.node].payload.is_none() {
            // Cancelled/failed creation accepted no native owner. Return metadata
            // immediately rather than accumulating empty work behind a blocked close.
            queue.nodes[self.node].next = queue.free;
            queue.free = Some(self.node);
            return;
        }
        if let Some(tail) = queue.tail {
            queue.nodes[tail].next = Some(self.node);
        } else {
            queue.head = Some(self.node);
        }
        queue.tail = Some(self.node);
        drop(queue);
        shared.ready.notify_one();
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closing = true;
        self.shared.ready.notify_one();
    }
}
struct WorkerExit(SharedOwner);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .accepting = false;
        self.0.ready.notify_all();
    }
}
fn run(shared: SharedOwner) {
    let _exit = WorkerExit(shared.clone());
    // Unexpected worker unwinding leaves the guarded registry intact. No later
    // admission succeeds; existing tokens retain fallback cleanup independently.
    let _ = catch_unwind(AssertUnwindSafe(|| drain(&shared)));
}
fn drain(shared: &Shared) {
    loop {
        let (node, payload) = {
            let mut queue = shared
                .queue
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            loop {
                #[cfg(test)]
                if queue.stop_worker {
                    assert!(!queue.panic_worker, "injected report worker panic");
                    return;
                }
                if let Some(node) = queue.head {
                    queue.head = queue.nodes[node].next.take();
                    if queue.head.is_none() {
                        queue.tail = None;
                    }
                    break (node, queue.nodes[node].payload.take());
                }
                if queue.closing {
                    return;
                }
                queue = shared
                    .ready
                    .wait(queue)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        // Never hold the metadata mutex while a native close/destroy can block.
        let _ = catch_unwind(AssertUnwindSafe(|| drop(payload)));
        let mut queue = shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.nodes[node].next = queue.free;
        queue.free = Some(node);
    }
}
impl Drop for Shared {
    fn drop(&mut self) {
        // This fallback runs through the startup WorkerOwned lease. Tokens retain
        // it until their raw references have dropped, even after worker failure.
        let queue = self
            .queue
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        for node in &mut queue.nodes {
            let payload = node.payload.take();
            let _ = catch_unwind(AssertUnwindSafe(|| drop(payload)));
        }
    }
}

#[cfg(test)]
#[path = "report_arena/tests.rs"]
mod tests;
