//! Process-wide fair I/O admission. Never hold a permit during decoding, hashing or channel waits.
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::Duration,
};

pub const DEFAULT_IO_LIMIT: u8 = 4;
pub const MAX_IO_LIMIT: u8 = 32;
const POLL: Duration = Duration::from_millis(10);

pub fn shared_io() -> Arc<Gate> {
    static IO: OnceLock<Arc<Gate>> = OnceLock::new();
    IO.get_or_init(|| Arc::new(Gate::new(DEFAULT_IO_LIMIT)))
        .clone()
}

/// Request semantics survive changes to the configured limit while queued or held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Shared,
    Exclusive,
}

/// Only positive identification of local SSD storage admits concurrent source I/O.
pub fn storage_admission(confirmed_local_ssd: bool) -> Admission {
    if confirmed_local_ssd {
        Admission::Shared
    } else {
        Admission::Exclusive
    }
}

pub struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}
struct State {
    limit: u8,
    active_shared: u8,
    exclusive: bool,
    next: u64,
    queue: VecDeque<(u64, Admission)>,
}
/// Observational counters for diagnostics and coordination tests; never used to admit work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Remaining shared capacity; zero while exclusive or draining after a decrease.
    pub available: u8,
    pub queued: usize,
    /// May temporarily exceed the current limit while preexisting owners drain.
    pub active_shared: u8,
    pub exclusive: bool,
}
impl Gate {
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Snapshot {
            available: if state.exclusive {
                0
            } else {
                state.limit.saturating_sub(state.active_shared)
            },
            queued: state.queue.len(),
            active_shared: state.active_shared,
            exclusive: state.exclusive,
        }
    }
    /// Construct an isolated gate from a trusted internal limit. Validate settings with set_limit.
    pub fn new(limit: u8) -> Self {
        assert!((1..=MAX_IO_LIMIT).contains(&limit));
        Self {
            state: Mutex::new(State {
                limit,
                active_shared: 0,
                exclusive: false,
                next: 0,
                queue: VecDeque::new(),
            }),
            changed: Condvar::new(),
        }
    }
    /// Change future admission without revoking held permits or changing queued request semantics.
    pub fn set_limit(&self, limit: u8) -> Result<(), &'static str> {
        if !(1..=MAX_IO_LIMIT).contains(&limit) {
            return Err("Disk I/O concurrency must be between 1 and 32.");
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.limit = limit;
        self.changed.notify_all();
        Ok(())
    }
    pub fn effective_limit(&self) -> u8 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .limit
    }
    /// FIFO prevents an exclusive HDD request from being starved by a stream of SSD requests.
    /// Cancellation removes a queued ticket within one short polling interval.
    pub fn acquire(
        self: &Arc<Self>,
        admission: Admission,
        cancelled: impl Fn() -> bool,
    ) -> Option<Permit> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let ticket = state.next;
        state.next = state.next.wrapping_add(1);
        state.queue.push_back((ticket, admission));
        loop {
            if cancelled() {
                if let Some(index) = state.queue.iter().position(|(id, _)| *id == ticket) {
                    state.queue.remove(index);
                }
                self.changed.notify_all();
                return None;
            }
            let eligible = !state.exclusive
                && match admission {
                    Admission::Shared => state.active_shared < state.limit,
                    Admission::Exclusive => state.active_shared == 0,
                };
            if state.queue.front().is_some_and(|(id, _)| *id == ticket) && eligible {
                state.queue.pop_front();
                match admission {
                    Admission::Shared => state.active_shared += 1,
                    Admission::Exclusive => state.exclusive = true,
                }
                self.changed.notify_all();
                return Some(Permit {
                    gate: self.clone(),
                    admission,
                });
            }
            state = self
                .changed
                .wait_timeout(state, POLL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}
/// Held until the admitted call actually returns, including after cancellation or a limit decrease.
pub struct Permit {
    gate: Arc<Gate>,
    admission: Admission,
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self
            .gate
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match self.admission {
            Admission::Shared => state.active_shared -= 1,
            Admission::Exclusive => state.exclusive = false,
        }
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

    // Authored before the semantic Gate implementation; execution is not authorized.
    struct Waiting {
        cancel: Arc<AtomicBool>,
        release: Option<mpsc::Sender<()>>,
        entered: mpsc::Receiver<()>,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Waiting {
        fn new(gate: Arc<Gate>, admission: Admission) -> Self {
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = cancel.clone();
            let (release, released) = mpsc::channel();
            let (send, entered) = mpsc::channel();
            let worker = thread::spawn(move || {
                if let Some(_permit) =
                    gate.acquire(admission, || worker_cancel.load(Ordering::Acquire))
                {
                    send.send(()).unwrap();
                    // Test cleanup closes this channel even when an assertion unwinds.
                    let _ = released.recv();
                }
            });
            Self {
                cancel,
                release: Some(release),
                entered,
                worker: Some(worker),
            }
        }
        fn assert_entered(&self) {
            self.entered.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        fn assert_waiting(&self) {
            assert!(matches!(
                self.entered.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
        }
    }
    impl Drop for Waiting {
        fn drop(&mut self) {
            self.cancel.store(true, Ordering::Release);
            self.release.take();
            if let Some(worker) = self.worker.take() {
                let result = worker.join();
                if !thread::panicking() {
                    result.unwrap();
                }
            }
        }
    }
    fn queued(gate: &Gate, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while gate.snapshot().queued != count {
            assert!(Instant::now() < deadline, "request did not enqueue");
            thread::yield_now();
        }
    }

    #[test]
    fn exclusive_waiter_precedes_later_shared_work_even_after_growth() {
        let gate = Arc::new(Gate::new(4));
        let held = gate.acquire(Admission::Shared, || false).unwrap();
        let first = Waiting::new(gate.clone(), Admission::Exclusive);
        queued(&gate, 1);
        let second = Waiting::new(gate.clone(), Admission::Shared);
        queued(&gate, 2);
        gate.set_limit(8).unwrap();
        first.assert_waiting();
        second.assert_waiting();
        drop(held);
        first.assert_entered();
        second.assert_waiting();
        assert!(gate.snapshot().exclusive);
        drop(first);
        second.assert_entered();
    }

    #[test]
    fn cancelled_head_does_not_block_following_eligible_work() {
        let gate = Arc::new(Gate::new(4));
        let _held = gate.acquire(Admission::Shared, || false).unwrap();
        let first = Waiting::new(gate.clone(), Admission::Exclusive);
        queued(&gate, 1);
        let second = Waiting::new(gate.clone(), Admission::Shared);
        queued(&gate, 2);
        drop(first);
        second.assert_entered();
        assert_eq!(gate.snapshot().active_shared, 2);
    }

    #[test]
    fn global_budget_is_shared_and_unknown_is_exclusive() {
        assert!(Arc::ptr_eq(&shared_io(), &shared_io()));
        assert_eq!(storage_admission(false), Admission::Exclusive);
        assert_eq!(storage_admission(true), Admission::Shared);
        assert_eq!(Gate::new(DEFAULT_IO_LIMIT).effective_limit(), 4);
    }

    #[test]
    fn shrinking_keeps_existing_shared_owners_charged_until_they_return() {
        let gate = Arc::new(Gate::new(4));
        let mut held: Vec<_> = (0..4)
            .map(|_| gate.acquire(Admission::Shared, || false).unwrap())
            .collect();
        gate.set_limit(1).unwrap();
        assert_eq!(gate.effective_limit(), 1);
        assert_eq!(gate.snapshot().active_shared, 4);
        assert_eq!(gate.snapshot().available, 0);
        let next = Waiting::new(gate.clone(), Admission::Shared);
        queued(&gate, 1);
        for remaining in (1..4).rev() {
            held.pop();
            assert_eq!(gate.snapshot().active_shared, remaining);
            next.assert_waiting();
        }
        held.pop();
        next.assert_entered();
        assert_eq!(gate.snapshot().active_shared, 1);
    }

    #[test]
    fn growing_wakes_a_shared_waiter_without_waiting_for_the_old_owner() {
        let gate = Arc::new(Gate::new(1));
        let _held = gate.acquire(Admission::Shared, || false).unwrap();
        let next = Waiting::new(gate.clone(), Admission::Shared);
        queued(&gate, 1);
        gate.set_limit(2).unwrap();
        next.assert_entered();
        assert_eq!(gate.snapshot().active_shared, 2);
    }

    #[test]
    fn an_exclusive_owner_stays_exclusive_across_growth_and_shrink() {
        let gate = Arc::new(Gate::new(4));
        let held = gate.acquire(Admission::Exclusive, || false).unwrap();
        let next = Waiting::new(gate.clone(), Admission::Shared);
        queued(&gate, 1);
        gate.set_limit(32).unwrap();
        next.assert_waiting();
        assert!(gate.snapshot().exclusive);
        assert_eq!(gate.snapshot().available, 0);
        gate.set_limit(1).unwrap();
        next.assert_waiting();
        drop(held);
        next.assert_entered();
        assert!(!gate.snapshot().exclusive);
    }

    #[test]
    fn queued_exclusive_keeps_its_meaning_when_the_limit_shrinks() {
        let gate = Arc::new(Gate::new(4));
        let one = gate.acquire(Admission::Shared, || false).unwrap();
        let two = gate.acquire(Admission::Shared, || false).unwrap();
        let next = Waiting::new(gate.clone(), Admission::Exclusive);
        queued(&gate, 1);
        gate.set_limit(1).unwrap();
        drop(one);
        next.assert_waiting();
        drop(two);
        next.assert_entered();
        assert!(gate.snapshot().exclusive);
        assert_eq!(gate.snapshot().active_shared, 0);
        drop(next);
        assert_eq!(gate.snapshot().available, 1);
    }

    #[test]
    fn invalid_limits_leave_live_policy_and_ownership_unchanged() {
        let gate = Arc::new(Gate::new(4));
        let _held = gate.acquire(Admission::Shared, || false).unwrap();
        for invalid in [0, 33, u8::MAX] {
            assert!(gate.set_limit(invalid).is_err());
            assert_eq!(gate.effective_limit(), 4);
            assert_eq!(gate.snapshot().active_shared, 1);
            assert_eq!(gate.snapshot().available, 3);
        }
    }

    #[test]
    fn cancelling_an_active_operation_does_not_return_its_permit() {
        let gate = Arc::new(Gate::new(2));
        let active = Waiting::new(gate.clone(), Admission::Shared);
        active.assert_entered();
        gate.set_limit(1).unwrap();
        active.cancel.store(true, Ordering::Release);
        let next = Waiting::new(gate.clone(), Admission::Shared);
        queued(&gate, 1);
        next.assert_waiting();
        assert_eq!(gate.snapshot().active_shared, 1);
        drop(active);
        next.assert_entered();
    }

    #[test]
    fn already_cancelled_request_never_takes_admission() {
        let gate = Arc::new(Gate::new(1));
        assert!(gate.acquire(Admission::Exclusive, || true).is_none());
        assert_eq!(gate.snapshot().queued, 0);
        assert_eq!(gate.snapshot().available, 1);
    }
}
