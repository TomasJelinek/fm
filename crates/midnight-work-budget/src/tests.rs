use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    task::Wake,
    thread,
    time::Instant,
};

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
            if let Some(_permit) = gate.acquire(admission, || worker_cancel.load(Ordering::Acquire))
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
/// The gate lock makes this synchronous: a wrongly admitted waiter is visible at once,
/// even before its thread is scheduled to report entry.
fn assert_counters(gate: &Gate, queued: usize, active_shared: u8, exclusive: bool) {
    let snapshot = gate.snapshot();
    assert_eq!(
        (snapshot.queued, snapshot.active_shared, snapshot.exclusive),
        (queued, active_shared, exclusive)
    );
}

#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Wakes {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}
fn poll<F: Future + Unpin>(future: &mut F, wakes: &Arc<Wakes>) -> Poll<F::Output> {
    let waker = Waker::from(wakes.clone());
    Pin::new(future).poll(&mut Context::from_waker(&waker))
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
    assert_counters(&gate, 2, 1, false);
    first.assert_waiting();
    second.assert_waiting();
    drop(held);
    first.assert_entered();
    assert_counters(&gate, 1, 0, true);
    second.assert_waiting();
    drop(first);
    second.assert_entered();
    assert_counters(&gate, 0, 1, false);
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
    assert_counters(&gate, 0, 2, false);
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
        assert_counters(&gate, 1, remaining, false);
        next.assert_waiting();
    }
    held.pop();
    next.assert_entered();
    assert_counters(&gate, 0, 1, false);
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
    assert_counters(&gate, 1, 0, true);
    next.assert_waiting();
    assert_eq!(gate.snapshot().available, 0);
    gate.set_limit(1).unwrap();
    assert_counters(&gate, 1, 0, true);
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
    assert_counters(&gate, 1, 1, false);
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
    assert_counters(&gate, 1, 1, false);
    next.assert_waiting();
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

#[test]
fn try_admission_respects_capacity_and_identifies_the_actual_guard() {
    let gate = Arc::new(Gate::new(2));
    let other = Arc::new(Gate::new(2));
    let first = gate.try_acquire(Admission::Shared).unwrap();
    let second = gate.try_acquire(Admission::Shared).unwrap();
    assert!(first.belongs_to(&gate));
    assert!(!first.belongs_to(&other));
    assert!(first.is_shared());
    assert!(first.is_held());
    assert!(gate.try_acquire(Admission::Shared).is_none());
    assert!(gate.try_acquire(Admission::Exclusive).is_none());
    drop(first);
    assert!(gate.try_acquire(Admission::Shared).is_some());
    drop(second);
    let exclusive = gate.try_acquire(Admission::Exclusive).unwrap();
    assert!(exclusive.belongs_to(&gate));
    assert!(!exclusive.is_shared());
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(exclusive);
    assert_eq!(gate.snapshot().available, 2);
}

#[test]
fn try_admission_cannot_bypass_a_queued_exclusive_request() {
    let gate = Arc::new(Gate::new(2));
    let held = gate.try_acquire(Admission::Shared).unwrap();
    let exclusive = Waiting::new(gate.clone(), Admission::Exclusive);
    queued(&gate, 1);
    assert_eq!(gate.snapshot().available, 1);
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(held);
    exclusive.assert_entered();
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(exclusive);
    assert!(gate.try_acquire(Admission::Shared).is_some());
}

#[test]
fn try_admission_waits_for_a_decreased_limit_to_drain() {
    let gate = Arc::new(Gate::new(3));
    let first = gate.try_acquire(Admission::Shared).unwrap();
    let second = gate.try_acquire(Admission::Shared).unwrap();
    let third = gate.try_acquire(Admission::Shared).unwrap();
    gate.set_limit(1).unwrap();
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(first);
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(second);
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(third);
    let resumed = gate.try_acquire(Admission::Shared).unwrap();
    assert_eq!(gate.snapshot().active_shared, 1);
    drop(resumed);
    assert_eq!(gate.snapshot().available, 1);
}

/// A transfer slot released for a pause/decision wait lets a queued exclusive request and
/// a later shared request run; the transfer then rejoins the FIFO queue behind them.
#[test]
fn released_long_lived_permit_unblocks_queued_exclusive_and_later_shared_work() {
    let gate = Arc::new(Gate::new(4));
    let transfer = Arc::new(gate.try_acquire(Admission::Shared).unwrap());
    let exclusive = Waiting::new(gate.clone(), Admission::Exclusive);
    queued(&gate, 1);
    let later = Waiting::new(gate.clone(), Admission::Shared);
    queued(&gate, 2);
    assert_counters(&gate, 2, 1, false);
    exclusive.assert_waiting();
    later.assert_waiting();

    assert!(transfer.release());
    assert!(!transfer.release(), "release is idempotent");
    assert!(!transfer.is_held());
    assert!(transfer.belongs_to(&gate) && transfer.is_shared());
    exclusive.assert_entered();
    assert_counters(&gate, 1, 0, true);

    let wakes = Arc::new(Wakes::default());
    let mut resume = transfer.reacquire();
    assert!(poll(&mut resume, &wakes).is_pending());
    assert_counters(&gate, 2, 0, true);
    drop(exclusive);
    later.assert_entered();
    let deadline = Instant::now() + Duration::from_secs(3);
    while wakes.count() == 0 {
        assert!(
            Instant::now() < deadline,
            "the new queue head was not woken"
        );
        thread::yield_now();
    }
    assert!(poll(&mut resume, &wakes).is_ready());
    drop(resume);
    assert!(transfer.is_held());
    assert_counters(&gate, 0, 2, false);
    drop(later);
    drop(transfer);
    assert_counters(&gate, 0, 0, false);
}

#[test]
fn async_admission_is_fifo_wakes_only_the_new_head_and_withdraws_on_drop() {
    let gate = Arc::new(Gate::new(1));
    let held = gate.try_acquire(Admission::Shared).unwrap();
    let (first_wakes, second_wakes) = (Arc::new(Wakes::default()), Arc::new(Wakes::default()));
    let mut first = gate.acquire_retained(Admission::Shared);
    let mut second = gate.acquire_retained(Admission::Shared);
    assert!(poll(&mut first, &first_wakes).is_pending());
    assert!(poll(&mut second, &second_wakes).is_pending());
    assert_counters(&gate, 2, 1, false);
    // A later try_acquire cannot overtake queued async requests.
    drop(held);
    assert!(gate.try_acquire(Admission::Shared).is_none());
    assert_eq!((first_wakes.count(), second_wakes.count()), (1, 0));
    let Poll::Ready(first_permit) = poll(&mut first, &first_wakes) else {
        panic!("the woken head is admitted");
    };
    assert!(first_permit.is_held());
    assert_counters(&gate, 1, 1, false);
    assert!(poll(&mut second, &second_wakes).is_pending());
    // Dropping a pending request withdraws it; nothing remains queued.
    drop(second);
    assert_counters(&gate, 0, 1, false);
    drop(first_permit);
    assert!(gate.try_acquire(Admission::Shared).is_some());
}

/// A running transfer (no pause, no decision) yields its retained slot at a step boundary
/// only to call-scoped requests; other transfers queued for a slot wait for whole jobs.
#[test]
fn retained_owner_yields_only_to_queued_call_scoped_requests() {
    let gate = Arc::new(Gate::new(1));
    let running = gate.try_acquire_retained(Admission::Shared).unwrap();
    assert!(!running.is_contended());
    let wakes = Arc::new(Wakes::default());
    let mut next_transfer = gate.acquire_retained(Admission::Shared);
    assert!(poll(&mut next_transfer, &wakes).is_pending());
    assert!(
        !running.is_contended(),
        "queued transfers wait for whole jobs"
    );
    let checksum = Waiting::new(gate.clone(), Admission::Exclusive);
    queued(&gate, 2);
    assert!(running.is_contended());
    // Step boundary: yield, then rejoin the queue behind both earlier requests.
    assert!(running.release());
    let mut resume = running.reacquire();
    assert!(poll(&mut resume, &wakes).is_pending());
    let Poll::Ready(next) = poll(&mut next_transfer, &wakes) else {
        panic!("the earlier queued transfer is admitted first");
    };
    checksum.assert_waiting();
    assert!(next.is_contended(), "the checksum is still queued");
    assert!(next.release());
    checksum.assert_entered();
    assert_counters(&gate, 1, 0, true);
    drop(checksum);
    // `next` queued behind `running` again: FIFO order between transfers is kept.
    let mut next_resume = next.reacquire();
    assert!(poll(&mut next_resume, &wakes).is_pending());
    let deadline = Instant::now() + Duration::from_secs(3);
    while poll(&mut resume, &wakes).is_pending() {
        assert!(
            Instant::now() < deadline,
            "the yielded transfer was not readmitted"
        );
        thread::yield_now();
    }
    assert!(running.is_held() && !next.is_held());
    assert!(!running.is_contended());
    drop(resume);
    drop(running);
    assert!(poll(&mut next_resume, &wakes).is_ready());
}

#[test]
fn try_reacquire_never_overtakes_a_queued_request() {
    let gate = Arc::new(Gate::new(2));
    let transfer = gate.try_acquire(Admission::Shared).unwrap();
    assert!(transfer.try_reacquire(), "a held permit stays held");
    transfer.release();
    let exclusive_holder = gate.try_acquire(Admission::Shared).unwrap();
    let exclusive = Waiting::new(gate.clone(), Admission::Exclusive);
    queued(&gate, 1);
    assert!(!transfer.try_reacquire());
    assert_counters(&gate, 1, 1, false);
    drop(exclusive_holder);
    exclusive.assert_entered();
    assert!(!transfer.try_reacquire());
    drop(exclusive);
    assert!(transfer.try_reacquire());
    assert_counters(&gate, 0, 1, false);
}

#[test]
fn dropping_a_pending_reacquire_withdraws_it_and_the_permit_stays_released() {
    let gate = Arc::new(Gate::new(1));
    let transfer = gate.try_acquire(Admission::Shared).unwrap();
    transfer.release();
    let other = gate.try_acquire(Admission::Shared).unwrap();
    let wakes = Arc::new(Wakes::default());
    let mut resume = transfer.reacquire();
    assert!(poll(&mut resume, &wakes).is_pending());
    assert_counters(&gate, 1, 1, false);
    drop(resume);
    assert_counters(&gate, 0, 1, false);
    assert!(!transfer.is_held());
    drop(other);
    drop(transfer);
    assert_counters(&gate, 0, 0, false);
}

#[test]
fn reclaim_charges_a_released_shared_permit_without_queuing() {
    let gate = Arc::new(Gate::new(1));
    let transfer = Arc::new(gate.try_acquire(Admission::Shared).unwrap());
    assert!(transfer.reclaim(), "a held permit stays held");
    transfer.release();
    let exclusive = gate.try_acquire(Admission::Exclusive).unwrap();
    let waiting = Waiting::new(gate.clone(), Admission::Shared);
    queued(&gate, 1);
    // Bounded teardown never waits behind the queue or an exclusive owner.
    assert!(transfer.reclaim());
    assert!(transfer.is_held());
    assert_counters(&gate, 1, 1, true);
    // Forgotten clones cannot retain capacity after an explicit release.
    std::mem::forget(transfer.clone());
    assert!(transfer.release());
    assert_counters(&gate, 1, 0, true);
    drop(exclusive);
    waiting.assert_entered();
    drop(waiting);
    let exclusive = Permit::new(gate.clone(), Admission::Exclusive, false, false);
    let blocker = gate.try_acquire(Admission::Shared).unwrap();
    assert!(
        !exclusive.reclaim(),
        "exclusive teardown cannot overlap shared owners"
    );
    drop(blocker);
    assert!(exclusive.reclaim());
    assert_counters(&gate, 0, 0, true);
}

#[test]
fn unwinding_cancellation_withdraws_its_queued_ticket() {
    let gate = Arc::new(Gate::new(1));
    let held = gate.try_acquire(Admission::Shared).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_gate = gate.clone();
    let worker_calls = calls.clone();
    let worker = thread::spawn(move || {
        worker_gate.acquire(Admission::Shared, || {
            if worker_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                false
            } else {
                panic!("cancellation probe failed");
            }
        })
    });
    assert!(worker.join().is_err());
    assert!(calls.load(Ordering::SeqCst) >= 2);
    assert_counters(&gate, 0, 1, false);
    drop(held);
    assert!(gate.try_acquire(Admission::Shared).is_some());
}

#[test]
fn cancellation_probe_runs_without_the_gate_lock() {
    let gate = Arc::new(Gate::new(1));
    let held = gate.try_acquire(Admission::Shared).unwrap();
    let observed = Arc::new(AtomicUsize::new(0));
    let (done, finished) = mpsc::channel();
    let worker_gate = gate.clone();
    let worker_observed = observed.clone();
    let worker = thread::spawn(move || {
        let probe_gate = worker_gate.clone();
        let permit = worker_gate.acquire(Admission::Shared, || {
            // Diagnostics on the same gate must not self-deadlock.
            worker_observed.store(probe_gate.snapshot().queued, Ordering::SeqCst);
            false
        });
        done.send(permit.is_some()).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while observed.load(Ordering::SeqCst) != 1 {
        assert!(
            Instant::now() < deadline,
            "the probe did not run while queued"
        );
        thread::yield_now();
    }
    drop(held);
    assert!(finished.recv_timeout(Duration::from_secs(3)).unwrap());
    worker.join().unwrap();
}
