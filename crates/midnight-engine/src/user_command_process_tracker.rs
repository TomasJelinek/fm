//! Fixed process ownership after the async desktop reaper or its runtime disappears.
use midnight_core::TerminalProcess;
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    process::ExitStatus,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};
use tokio::sync::OwnedSemaphorePermit;

pub(crate) const MAX_PROCESSES: usize = 8;
pub(crate) const MAX_OPEN_PROCESSES: usize = 16;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
type Worker = Box<dyn FnOnce() + Send>;

#[derive(Clone)]
pub(crate) struct ProcessTracker {
    control: Arc<Control>,
}
struct Control {
    shared: Arc<Shared>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
struct State {
    capacity: usize,
    reserved: [bool; MAX_OPEN_PROCESSES],
    queued: [Option<Queued>; MAX_OPEN_PROCESSES],
    closing: bool,
}
struct Payload {
    child: Box<dyn TerminalProcess>,
    services: Arc<dyn Send + Sync>,
    quota: Arc<OwnedSemaphorePermit>,
}
struct Queued {
    payload: Payload,
    exited: bool,
}
pub(crate) struct ProcessTicket {
    control: Arc<Control>,
    slot: Option<usize>,
}
pub(crate) struct TrackedProcess {
    payload: Option<Payload>,
    ticket: Option<ProcessTicket>,
    id: u32,
    exited: bool,
}

impl ProcessTracker {
    /// Startup only: no provider or child exists before the fixed worker starts.
    pub(crate) fn try_start() -> io::Result<Self> {
        Self::start(|worker| {
            thread::Builder::new()
                .name("fm-command-processes".into())
                .spawn(worker)
        })
    }
    pub(crate) fn try_start_openers() -> io::Result<Self> {
        Self::start_with_capacity(MAX_OPEN_PROCESSES, |worker| {
            thread::Builder::new()
                .name("fm-opener-processes".into())
                .spawn(worker)
        })
    }
    fn start(spawn: impl FnOnce(Worker) -> io::Result<thread::JoinHandle<()>>) -> io::Result<Self> {
        Self::start_with_capacity(MAX_PROCESSES, spawn)
    }
    fn start_with_capacity(
        capacity: usize,
        spawn: impl FnOnce(Worker) -> io::Result<thread::JoinHandle<()>>,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                capacity,
                reserved: [false; MAX_OPEN_PROCESSES],
                queued: std::array::from_fn(|_| None),
                closing: false,
            }),
            changed: Condvar::new(),
        });
        let worker_state = shared.clone();
        let thread = spawn(Box::new(move || run(worker_state)))?;
        // No joining in Drop. Tickets and queued children keep bounded ownership
        // until native exit; a long-running user process must not block the UI.
        drop(thread);
        Ok(Self {
            control: Arc::new(Control { shared }),
        })
    }
    pub(crate) fn try_reserve(&self) -> io::Result<ProcessTicket> {
        let mut state = self
            .control
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let slot = state
            .reserved
            .iter()
            .take(state.capacity)
            .position(|reserved| !reserved)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!("Process cleanup capacity is full ({}).", state.capacity),
                )
            })?;
        state.reserved[slot] = true;
        Ok(ProcessTicket {
            control: self.control.clone(),
            slot: Some(slot),
        })
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closing = true;
        self.shared.changed.notify_one();
    }
}
impl ProcessTicket {
    /// Called immediately after native launch, before returning its owned result.
    pub(crate) fn own(
        self,
        child: Box<dyn TerminalProcess>,
        quota: Arc<OwnedSemaphorePermit>,
        services: Arc<dyn Send + Sync>,
    ) -> TrackedProcess {
        let mut owned = TrackedProcess {
            payload: Some(Payload {
                child,
                services,
                quota,
            }),
            ticket: Some(self),
            id: 0,
            exited: false,
        };
        // Even a provider's id() panic must retain the admitted native owner.
        owned.id = owned
            .payload
            .as_ref()
            .expect("owned child is present")
            .child
            .id();
        owned
    }
    fn submit(mut self, payload: Payload, exited: bool) {
        let slot = self.slot.take().expect("process ticket is consumed once");
        let shared = &self.control.shared;
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        debug_assert!(state.reserved[slot] && state.queued[slot].is_none());
        state.queued[slot] = Some(Queued { payload, exited });
        drop(state);
        shared.changed.notify_one();
    }
}
impl Drop for ProcessTicket {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            release(&self.control.shared, slot);
        }
    }
}
impl TrackedProcess {
    pub(crate) fn quota(&self) -> Arc<OwnedSemaphorePermit> {
        self.payload
            .as_ref()
            .expect("live process owns its quota")
            .quota
            .clone()
    }
}
impl TerminalProcess for TrackedProcess {
    fn id(&self) -> u32 {
        self.id
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let result = self
            .payload
            .as_mut()
            .expect("live process owns its child")
            .child
            .try_wait();
        if matches!(result, Ok(Some(_))) {
            self.exited = true;
        }
        result
    }
}
impl Drop for TrackedProcess {
    fn drop(&mut self) {
        if let Some(payload) = self.payload.take() {
            self.ticket
                .take()
                .expect("live child retains its ticket")
                .submit(payload, self.exited);
        }
    }
}
fn release(shared: &Shared, slot: usize) {
    shared
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reserved[slot] = false;
    shared.changed.notify_one();
}
fn run(shared: Arc<Shared>) {
    let mut active: [Option<Queued>; MAX_OPEN_PROCESSES] = std::array::from_fn(|_| None);
    loop {
        {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            for (slot, child) in active.iter_mut().enumerate() {
                if child.is_none() {
                    *child = state.queued[slot].take();
                }
            }
        }
        for (slot, queued) in active.iter_mut().enumerate() {
            let Some(child) = queued.as_mut() else {
                continue;
            };
            // A wait error or one unwinding provider call never proves exit.
            let exited = child.exited
                || matches!(
                    catch_unwind(AssertUnwindSafe(|| child.payload.child.try_wait())),
                    Ok(Ok(Some(_)))
                );
            if exited {
                let Queued { payload, .. } = queued.take().expect("observed child is owned");
                let Payload {
                    child,
                    services,
                    quota,
                } = payload;
                let _ = catch_unwind(AssertUnwindSafe(|| drop(child)));
                let _ = catch_unwind(AssertUnwindSafe(|| drop(services)));
                // Native Drop has finished before either reservation is reusable.
                // Make the tracker slot available before publishing process capacity.
                release(&shared, slot);
                drop(quota);
            }
        }
        let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closing && state.reserved.iter().all(|reserved| !reserved) {
            return;
        }
        if state.queued.iter().any(Option::is_some) {
            continue;
        }
        if active.iter().any(Option::is_some) {
            let _ = shared
                .changed
                .wait_timeout(state, POLL_INTERVAL)
                .unwrap_or_else(|e| e.into_inner());
        } else {
            drop(
                shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner()),
            );
        }
    }
}

#[cfg(test)]
#[path = "user_command_process_tracker_tests.rs"]
mod tests;
