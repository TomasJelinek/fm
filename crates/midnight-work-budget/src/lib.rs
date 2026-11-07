//! Process-wide fair I/O admission. Never hold a permit during decoding, hashing or channel waits.
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::Duration,
};

pub const IO_SLOTS: u8 = 4;
const POLL: Duration = Duration::from_millis(10);

pub fn shared_io() -> Arc<Gate> {
    static IO: OnceLock<Arc<Gate>> = OnceLock::new();
    IO.get_or_init(|| Arc::new(Gate::new(IO_SLOTS))).clone()
}

/// Only positive identification of local SSD storage admits concurrent source I/O.
pub fn storage_cost(confirmed_local_ssd: bool) -> u8 {
    if confirmed_local_ssd { 1 } else { IO_SLOTS }
}

pub struct Gate {
    capacity: u8,
    state: Mutex<State>,
    changed: Condvar,
}
struct State {
    available: u8,
    next: u64,
    queue: VecDeque<(u64, u8)>,
}
/// Observational counters for diagnostics and coordination tests; never used to admit work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub available: u8,
    pub queued: usize,
}
impl Gate {
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Snapshot {
            available: state.available,
            queued: state.queue.len(),
        }
    }
    pub fn new(capacity: u8) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            state: Mutex::new(State {
                available: capacity,
                next: 0,
                queue: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }
    /// FIFO prevents an exclusive HDD request from being starved by a stream of SSD requests.
    /// Cancellation removes a queued ticket within one short polling interval.
    pub fn acquire(self: &Arc<Self>, cost: u8, cancelled: impl Fn() -> bool) -> Option<Permit> {
        assert!(cost > 0 && cost <= self.capacity);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let ticket = state.next;
        state.next = state.next.wrapping_add(1);
        state.queue.push_back((ticket, cost));
        loop {
            if cancelled() {
                if let Some(index) = state.queue.iter().position(|(id, _)| *id == ticket) {
                    state.queue.remove(index);
                }
                self.changed.notify_all();
                return None;
            }
            if state.queue.front().is_some_and(|(id, _)| *id == ticket) && state.available >= cost {
                state.queue.pop_front();
                state.available -= cost;
                self.changed.notify_all();
                return Some(Permit {
                    gate: self.clone(),
                    cost,
                });
            }
            state = self
                .changed
                .wait_timeout(state, POLL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
    #[cfg(test)]
    fn queued(&self) -> usize {
        self.state.lock().unwrap().queue.len()
    }
}
pub struct Permit {
    gate: Arc<Gate>,
    cost: u8,
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.available += self.cost;
        self.gate.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Instant,
    };
    fn queued(gate: &Gate, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while gate.queued() != count {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
    }
    #[test]
    fn exclusive_waiter_precedes_later_small_requests() {
        let gate = Arc::new(Gate::new(4));
        let held = gate.acquire(1, || false).unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::scope(|scope| {
            let exclusive = gate.clone();
            let first_sender = sender.clone();
            scope.spawn(move || {
                let _permit = exclusive.acquire(4, || false).unwrap();
                first_sender.send(4).unwrap();
            });
            queued(&gate, 1);
            let small = gate.clone();
            scope.spawn(move || {
                let _permit = small.acquire(1, || false).unwrap();
                sender.send(1).unwrap();
            });
            queued(&gate, 2);
            assert!(receiver.try_recv().is_err());
            drop(held);
            assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), 4);
            assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        });
    }
    #[test]
    fn cancelled_head_does_not_block_following_eligible_work() {
        let gate = Arc::new(Gate::new(4));
        let _held = gate.acquire(1, || false).unwrap();
        let cancel = AtomicBool::new(false);
        thread::scope(|scope| {
            let exclusive = gate.clone();
            let cancel = &cancel;
            let waiting =
                scope.spawn(move || exclusive.acquire(4, || cancel.load(Ordering::Acquire)));
            queued(&gate, 1);
            cancel.store(true, Ordering::Release);
            assert!(waiting.join().unwrap().is_none());
            assert!(gate.acquire(3, || false).is_some());
        });
    }
    #[test]
    fn global_budget_is_shared_and_unknown_is_exclusive() {
        assert!(Arc::ptr_eq(&shared_io(), &shared_io()));
        assert_eq!(storage_cost(false), 4);
        assert_eq!(storage_cost(true), 1);
    }
}
