use super::*;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::Wake,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(5);

struct DropProbe(mpsc::Sender<ThreadId>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}

#[derive(Default)]
struct Signal {
    ready: Mutex<bool>,
    changed: Condvar,
}
impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        *self.ready.lock().unwrap() = true;
        self.changed.notify_one();
    }
}
fn wait(mut retirement: Retirement) -> Result<(), RetirementPanic> {
    let signal = Arc::new(Signal::default());
    let waker = Waker::from(signal.clone());
    let mut context = Context::from_waker(&waker);
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Poll::Ready(outcome) = Pin::new(&mut retirement).poll(&mut context) {
            return outcome;
        }
        let mut ready = signal.ready.lock().unwrap();
        while !*ready {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "retirement acknowledgement did not arrive"
            );
            ready = signal.changed.wait_timeout(ready, remaining).unwrap().0;
        }
        *ready = false;
    }
}

struct GatedDrop {
    started: mpsc::Sender<ThreadId>,
    release: mpsc::Receiver<()>,
    finished: mpsc::Sender<()>,
}
impl Drop for GatedDrop {
    fn drop(&mut self) {
        let _ = self.started.send(thread::current().id());
        let _ = self.release.recv_timeout(DEADLINE);
        let _ = self.finished.send(());
    }
}
fn gated() -> (
    GatedDrop,
    mpsc::Receiver<ThreadId>,
    mpsc::Sender<()>,
    mpsc::Receiver<()>,
) {
    let (started, observe_start) = mpsc::channel();
    let (release, wait_release) = mpsc::channel();
    let (finished, observe_finish) = mpsc::channel();
    (
        GatedDrop {
            started,
            release: wait_release,
            finished,
        },
        observe_start,
        release,
        observe_finish,
    )
}

#[test]
fn unpolled_owner_retires_without_a_runtime_and_off_its_caller() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let (sent, observed) = mpsc::channel();
    let owner = domain.try_reserve().unwrap().own(DropProbe(sent));
    drop(owner);
    let retired_on = observed.recv_timeout(DEADLINE).unwrap();
    assert_ne!(
        retired_on,
        thread::current().id(),
        "provider dropped on caller"
    );
}

#[test]
fn startup_failure_precedes_any_possible_payload_admission() {
    let attempts = AtomicUsize::new(0);
    let result = RetirementDomain::start_with(2, 0, |_| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("injected worker startup refusal"))
    });
    assert!(matches!(result, Err(error) if error.to_string() == "injected worker startup refusal"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn zero_capacity_is_rejected_without_starting_a_worker() {
    let result = RetirementDomain::start_with(0, 0, |_| panic!("must validate before spawn"));
    assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
}

#[test]
fn unused_reservations_return_capacity_without_consuming_a_payload() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let lease = domain.try_reserve().unwrap();
    let (sent, observed) = mpsc::channel();
    let payload = DropProbe(sent);
    assert!(matches!(domain.try_reserve(), Err(RetirementBusy)));
    assert!(observed.try_recv().is_err());
    drop(lease);
    wait(domain.try_reserve().unwrap().retire(payload)).unwrap();
    assert_ne!(
        observed.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
}

#[test]
fn blocked_destruction_holds_capacity_and_acknowledgement_until_it_finishes() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let (payload, started, release, finished) = gated();
    let mut retirement = domain.try_reserve().unwrap().retire(payload);
    assert_ne!(
        started.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
    assert!(matches!(domain.try_reserve(), Err(RetirementBusy)));
    assert!(finished.try_recv().is_err());
    assert!(
        Pin::new(&mut retirement)
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    release.send(()).unwrap();
    wait(retirement).unwrap();
    finished.recv_timeout(DEADLINE).unwrap();
    assert!(domain.try_reserve().is_ok(), "ack follows capacity release");
}

#[test]
fn saturated_queue_never_starts_replacement_workers_or_drops_inline() {
    let domain = RetirementDomain::try_start(2, 0).unwrap();
    let (payload, started, release, _) = gated();
    let first = domain.try_reserve().unwrap().retire(payload);
    let worker = started.recv_timeout(DEADLINE).unwrap();
    let (sent, observed) = mpsc::channel();
    let second = domain.try_reserve().unwrap().retire(DropProbe(sent));
    assert!(matches!(domain.try_reserve(), Err(RetirementBusy)));
    assert!(
        observed.try_recv().is_err(),
        "second payload must remain queued"
    );
    release.send(()).unwrap();
    wait(first).unwrap();
    wait(second).unwrap();
    assert_eq!(observed.recv_timeout(DEADLINE).unwrap(), worker);
}

#[test]
fn leases_keep_worker_alive_after_last_domain_handle_is_dropped() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let lease = domain.try_reserve().unwrap();
    let (sent, observed) = mpsc::channel();
    drop(domain);
    wait(lease.retire(DropProbe(sent))).unwrap();
    assert_ne!(
        observed.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
}

#[test]
fn domain_drop_does_not_wait_for_a_blocked_destructor_and_drains_pending_jobs() {
    let domain = RetirementDomain::try_start(2, 0).unwrap();
    let (payload, started, release, _) = gated();
    let first = domain.try_reserve().unwrap().retire(payload);
    let worker = started.recv_timeout(DEADLINE).unwrap();
    let (sent, observed) = mpsc::channel();
    let second = domain.try_reserve().unwrap().retire(DropProbe(sent));
    let (dropped, observe_drop) = mpsc::channel();
    let caller = thread::spawn(move || {
        drop(domain);
        dropped.send(()).unwrap();
    });
    observe_drop.recv_timeout(DEADLINE).unwrap();
    release.send(()).unwrap();
    caller.join().unwrap();
    wait(first).unwrap();
    wait(second).unwrap();
    assert_eq!(observed.recv_timeout(DEADLINE).unwrap(), worker);
}

#[test]
fn dropping_acknowledgement_does_not_cancel_retirement() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let (payload, started, release, finished) = gated();
    drop(domain.try_reserve().unwrap().retire(payload));
    started.recv_timeout(DEADLINE).unwrap();
    assert!(matches!(domain.try_reserve(), Err(RetirementBusy)));
    drop(domain);
    release.send(()).unwrap();
    finished.recv_timeout(DEADLINE).unwrap();
}

struct Nested {
    child: Option<WorkerOwned<DropProbe>>,
    observed: mpsc::Sender<(ThreadId, bool)>,
}
impl Drop for Nested {
    fn drop(&mut self) {
        let mut acknowledgement = self.child.take().unwrap().retire();
        let ready = Pin::new(&mut acknowledgement)
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready();
        let _ = self.observed.send((thread::current().id(), ready));
    }
}
#[test]
fn nested_same_domain_retirement_finishes_on_the_current_cleanup_worker() {
    let domain = RetirementDomain::try_start(2, 0).unwrap();
    let (child_sent, child_observed) = mpsc::channel();
    let (outer_sent, outer_observed) = mpsc::channel();
    let child = domain.try_reserve().unwrap().own(DropProbe(child_sent));
    let outer = domain.try_reserve().unwrap().own(Nested {
        child: Some(child),
        observed: outer_sent,
    });
    wait(outer.retire()).unwrap();
    let (outer_thread, nested_ready) = outer_observed.recv_timeout(DEADLINE).unwrap();
    assert!(
        nested_ready,
        "same-domain cleanup must not queue behind itself"
    );
    assert_eq!(child_observed.recv_timeout(DEADLINE).unwrap(), outer_thread);
    assert_ne!(outer_thread, thread::current().id());
    let _first = domain.try_reserve().unwrap();
    let _second = domain.try_reserve().unwrap();
}

struct PanickingDrop;
impl Drop for PanickingDrop {
    fn drop(&mut self) {
        panic!("injected destructor failure");
    }
}
#[test]
fn panicking_destructor_reports_failure_and_worker_continues() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    assert_eq!(
        wait(domain.try_reserve().unwrap().retire(PanickingDrop)),
        Err(RetirementPanic)
    );
    let (sent, observed) = mpsc::channel();
    wait(domain.try_reserve().unwrap().retire(DropProbe(sent))).unwrap();
    assert_ne!(
        observed.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
}

#[test]
fn worker_handoff_keeps_the_same_reservation_through_a_type_change() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let owner = domain.try_reserve().unwrap().own(41usize);
    let second_domain = domain.clone();
    let transformed = thread::spawn(move || {
        let (value, lease) = owner.into_parts();
        assert!(matches!(second_domain.try_reserve(), Err(RetirementBusy)));
        lease.own(value.to_string())
    })
    .join()
    .unwrap();
    assert_eq!(&*transformed, "41");
    assert!(matches!(domain.try_reserve(), Err(RetirementBusy)));
    wait(transformed.retire()).unwrap();
    assert!(domain.try_reserve().is_ok());
}

#[test]
fn pinned_future_can_be_polled_without_moving_its_inner_allocation() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let (sent, observed) = mpsc::channel();
    let probe = DropProbe(sent);
    let future = Box::pin(async move {
        let _probe = probe;
        std::future::pending::<()>().await;
    });
    let mut owner = domain.try_reserve().unwrap().own(future);
    assert!(
        owner
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    wait(owner.retire()).unwrap();
    assert_ne!(
        observed.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
}

struct WakeCounter(AtomicUsize);
impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn counter() -> (Arc<WakeCounter>, Waker) {
    let count = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(count.clone());
    (count, waker)
}

#[test]
fn waiter_registration_budget_is_reserved_before_payload_admission() {
    let domain = RetirementDomain::try_start(1, 2).unwrap();
    let first = domain.new_waiter().unwrap();
    let _second = domain.new_waiter().unwrap();
    assert!(matches!(domain.new_waiter(), Err(RetirementBusy)));
    drop(first);
    assert!(domain.new_waiter().is_ok());
    assert!(
        RetirementDomain::try_start(1, 0)
            .unwrap()
            .new_waiter()
            .is_err()
    );
}

#[test]
fn registered_waiter_wakes_on_capacity_return_without_a_runtime() {
    let domain = RetirementDomain::try_start(1, 1).unwrap();
    let held = domain.try_reserve().unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    let mut reserve = Box::pin(waiter.reserve());
    let (count, waker) = counter();
    let mut context = Context::from_waker(&waker);
    assert!(reserve.as_mut().poll(&mut context).is_pending());
    drop(held);
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
    let Poll::Ready(lease) = reserve.as_mut().poll(&mut context) else {
        panic!("slot returned")
    };
    assert!(matches!(domain.try_reserve(), Err(RetirementBusy)));
    drop(lease);
}

#[test]
fn cancelled_acquisition_unregisters_waker_and_token_can_be_reused() {
    let domain = RetirementDomain::try_start(1, 1).unwrap();
    let held = domain.try_reserve().unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    let (count, waker) = counter();
    let mut context = Context::from_waker(&waker);
    let mut reserve = Box::pin(waiter.reserve());
    assert!(reserve.as_mut().poll(&mut context).is_pending());
    drop(reserve);
    drop(held);
    assert_eq!(
        count.0.load(Ordering::SeqCst),
        0,
        "cancelled future must not retain a wake target"
    );
    let mut reserve = Box::pin(waiter.reserve());
    assert!(reserve.as_mut().poll(&mut context).is_ready());
}

#[test]
fn waiter_replaces_old_waker_and_wakes_only_latest_observer() {
    let domain = RetirementDomain::try_start(1, 1).unwrap();
    let held = domain.try_reserve().unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    let mut reserve = Box::pin(waiter.reserve());
    let (first_count, first) = counter();
    let (second_count, second) = counter();
    assert!(
        reserve
            .as_mut()
            .poll(&mut Context::from_waker(&first))
            .is_pending()
    );
    assert!(
        reserve
            .as_mut()
            .poll(&mut Context::from_waker(&second))
            .is_pending()
    );
    drop(held);
    assert_eq!(first_count.0.load(Ordering::SeqCst), 0);
    assert_eq!(second_count.0.load(Ordering::SeqCst), 1);
}

#[test]
fn every_registered_waiter_is_notified_and_repolls_against_real_capacity() {
    let domain = RetirementDomain::try_start(2, 2).unwrap();
    let held_first = domain.try_reserve().unwrap();
    let held_second = domain.try_reserve().unwrap();
    let mut first = domain.new_waiter().unwrap();
    let mut second = domain.new_waiter().unwrap();
    let mut first_reserve = Box::pin(first.reserve());
    let mut second_reserve = Box::pin(second.reserve());
    let (first_count, first_waker) = counter();
    let (second_count, second_waker) = counter();
    let mut first_context = Context::from_waker(&first_waker);
    let mut second_context = Context::from_waker(&second_waker);
    assert!(first_reserve.as_mut().poll(&mut first_context).is_pending());
    assert!(
        second_reserve
            .as_mut()
            .poll(&mut second_context)
            .is_pending()
    );
    drop(held_first);
    assert_eq!(first_count.0.load(Ordering::SeqCst), 1);
    assert_eq!(second_count.0.load(Ordering::SeqCst), 1);
    let Poll::Ready(first_lease) = first_reserve.as_mut().poll(&mut first_context) else {
        panic!("first free slot");
    };
    assert!(
        second_reserve
            .as_mut()
            .poll(&mut second_context)
            .is_pending()
    );
    drop(held_second);
    assert_eq!(second_count.0.load(Ordering::SeqCst), 2);
    let Poll::Ready(second_lease) = second_reserve.as_mut().poll(&mut second_context) else {
        panic!("second free slot");
    };
    assert!(matches!(domain.try_reserve(), Err(RetirementBusy)));
    drop((first_lease, second_lease));
}

#[test]
fn waiter_token_keeps_domain_alive_until_its_later_retirement_finishes() {
    let domain = RetirementDomain::try_start(1, 1).unwrap();
    let held = domain.try_reserve().unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    let mut reserve = Box::pin(waiter.reserve());
    let (_, waker) = counter();
    let mut context = Context::from_waker(&waker);
    assert!(reserve.as_mut().poll(&mut context).is_pending());
    drop(domain);
    drop(held);
    let Poll::Ready(lease) = reserve.as_mut().poll(&mut context) else {
        panic!("slot returned")
    };
    drop(reserve);
    drop(waiter);
    let (sent, observed) = mpsc::channel();
    wait(lease.retire(DropProbe(sent))).unwrap();
    assert_ne!(
        observed.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
}

#[test]
fn worker_exits_after_last_owner_and_queued_payload_are_retired() {
    let (exited, observed_exit) = mpsc::channel();
    let (join_sent, join_observed) = mpsc::channel();
    let domain = RetirementDomain::start_with(1, 0, |worker| {
        let join = thread::spawn(move || {
            worker();
            let _ = exited.send(());
        });
        join_sent.send(join).unwrap();
        Ok(())
    })
    .unwrap();
    let (payload, started, release, finished) = gated();
    let acknowledgement = domain.try_reserve().unwrap().retire(payload);
    started.recv_timeout(DEADLINE).unwrap();
    drop(domain);
    assert!(observed_exit.try_recv().is_err());
    release.send(()).unwrap();
    wait(acknowledgement).unwrap();
    finished.recv_timeout(DEADLINE).unwrap();
    observed_exit.recv_timeout(DEADLINE).unwrap();
    join_observed
        .recv_timeout(DEADLINE)
        .unwrap()
        .join()
        .unwrap();
}

struct DroppedWaker {
    completion: std::sync::Weak<Completion>,
    observed: mpsc::Sender<bool>,
}
#[allow(
    clippy::manual_noop_waker,
    reason = "The observer destructor is under test"
)]
impl Wake for DroppedWaker {
    fn wake(self: Arc<Self>) {}
}
impl Drop for DroppedWaker {
    fn drop(&mut self) {
        let unlocked = self
            .completion
            .upgrade()
            .is_some_and(|value| value.state.try_lock().is_ok());
        let _ = self.observed.send(unlocked);
    }
}
#[test]
fn replacing_acknowledgement_waker_destroys_old_observer_outside_state_lock() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let (payload, started, release, _) = gated();
    let mut acknowledgement = domain.try_reserve().unwrap().retire(payload);
    started.recv_timeout(DEADLINE).unwrap();
    let (observed, receiver) = mpsc::channel();
    let old = Waker::from(Arc::new(DroppedWaker {
        completion: Arc::downgrade(&acknowledgement.completion),
        observed,
    }));
    assert!(
        Pin::new(&mut acknowledgement)
            .poll(&mut Context::from_waker(&old))
            .is_pending()
    );
    drop(old);
    assert!(
        Pin::new(&mut acknowledgement)
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let unlocked = receiver.recv_timeout(DEADLINE).unwrap();
    release.send(()).unwrap();
    wait(acknowledgement).unwrap();
    assert!(unlocked, "waker destructor ran inside completion lock");
}
