use super::*;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    task::Wake,
    time::Duration,
};

#[derive(Default)]
struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn exhaust(domain: &RetirementDomain, count: usize) {
    let leases: Vec<_> = (0..count).map(|_| domain.try_reserve().unwrap()).collect();
    assert!(domain.try_reserve().is_err());
    let unique: std::collections::HashSet<_> =
        leases.iter().map(|lease| lease.slot.unwrap()).collect();
    assert_eq!(unique.len(), count);
    drop(leases);
}

#[test]
fn nowait_reserve_and_unused_rollback_ignore_a_held_payload_queue_lock() {
    let domain = RetirementDomain::try_start(2, 0).unwrap();
    let queue = domain.control.shared.queue.lock().unwrap();
    let worker_domain = domain.clone();
    let (sent, received) = mpsc::channel();
    let worker = thread::spawn(move || {
        let first = worker_domain.try_reserve().unwrap();
        let slot = first.slot.unwrap();
        drop(first);
        let again = worker_domain.try_reserve().unwrap();
        let reused = again.slot == Some(slot);
        drop(again);
        sent.send(reused).unwrap();
    });
    let observation = received.recv_timeout(Duration::from_secs(2));
    drop(queue);
    worker.join().unwrap();
    assert!(observation.expect("unused rollback must not wait for the queue lock"));
    exhaust(&domain, 2);
}

#[test]
fn immediate_and_waiter_reservations_share_the_same_exact_slot_capacity() {
    let domain = RetirementDomain::try_start(3, 1).unwrap();
    let first = domain.try_reserve().unwrap();
    let second = domain.try_reserve().unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    let wake = Waker::from(Arc::new(WakeCount::default()));
    let mut context = Context::from_waker(&wake);
    let mut pending = Box::pin(waiter.reserve());
    let Poll::Ready(third) = pending.as_mut().poll(&mut context) else {
        panic!("third actual slot missing");
    };
    assert_ne!(first.slot, second.slot);
    assert_ne!(first.slot, third.slot);
    assert_ne!(second.slot, third.slot);
    assert!(domain.try_reserve().is_err());
    drop((first, second, third, pending));
    exhaust(&domain, 3);
}

#[test]
fn unused_return_wakes_a_registered_waiter_while_payload_queue_is_locked() {
    let domain = RetirementDomain::try_start(1, 1).unwrap();
    let lease = domain.try_reserve().unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    let count = Arc::new(WakeCount::default());
    let waker = Waker::from(count.clone());
    let mut context = Context::from_waker(&waker);
    let mut pending = Box::pin(waiter.reserve());
    assert!(pending.as_mut().poll(&mut context).is_pending());
    let queue = domain.control.shared.queue.lock().unwrap();
    let (sent, received) = mpsc::channel();
    let worker = thread::spawn(move || {
        drop(lease);
        sent.send(()).unwrap();
    });
    let returned = received.recv_timeout(Duration::from_secs(2));
    let wake_count = count.0.load(Ordering::SeqCst);
    drop(queue);
    worker.join().unwrap();
    assert!(
        returned.is_ok(),
        "unused release parked on the payload queue"
    );
    assert!(
        wake_count > 0,
        "metadata release must wake independently of the native worker"
    );
    let Poll::Ready(lease) = pending.as_mut().poll(&mut context) else {
        panic!("returned actual slot unavailable");
    };
    drop((lease, pending));
    exhaust(&domain, 1);
}

#[test]
fn unused_return_does_not_wait_for_a_waiter_cell_and_preserves_notification() {
    let domain = RetirementDomain::try_start(1, 1).unwrap();
    let lease = domain.try_reserve().unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    let index = waiter.slot;
    let count = Arc::new(WakeCount::default());
    let waker = Waker::from(count);
    let mut context = Context::from_waker(&waker);
    let mut pending = Box::pin(waiter.reserve());
    assert!(pending.as_mut().poll(&mut context).is_pending());
    let cell = domain.control.shared.waiters[index].waker.lock().unwrap();
    let (sent, received) = mpsc::channel();
    let worker = thread::spawn(move || {
        drop(lease);
        sent.send(()).unwrap();
    });
    let returned = received.recv_timeout(Duration::from_secs(2));
    let notified = domain.control.shared.waiters[index]
        .notified
        .load(Ordering::Acquire);
    drop(cell);
    worker.join().unwrap();
    assert!(returned.is_ok(), "unused release parked on a waiter cell");
    assert!(
        notified,
        "contended notification must survive until the polling owner observes it"
    );
    let Poll::Ready(lease) = pending.as_mut().poll(&mut context) else {
        panic!("returned slot lost under waiter contention");
    };
    drop((lease, pending));
    exhaust(&domain, 1);
}

#[test]
fn concurrent_unused_returns_never_leave_a_polled_reservation_asleep() {
    let domain = RetirementDomain::try_start(1, 1).unwrap();
    let mut waiter = domain.new_waiter().unwrap();
    for _ in 0..64 {
        // Exclude the prior completed round's harmless stale notification.
        domain.control.shared.waiters[waiter.slot]
            .notified
            .store(false, Ordering::SeqCst);
        let lease = domain.try_reserve().unwrap();
        let start = Arc::new(std::sync::Barrier::new(2));
        let worker_start = start.clone();
        let worker = thread::spawn(move || {
            worker_start.wait();
            drop(lease);
        });
        let count = Arc::new(WakeCount::default());
        let waker = Waker::from(count.clone());
        let mut context = Context::from_waker(&waker);
        let mut pending = Box::pin(waiter.reserve());
        start.wait();
        let first = pending.as_mut().poll(&mut context);
        worker.join().unwrap();
        let lease = match first {
            Poll::Ready(lease) => lease,
            Poll::Pending => {
                assert!(
                    count.0.load(Ordering::SeqCst) > 0,
                    "Pending returned without a wake after the racing slot return"
                );
                let Poll::Ready(lease) = pending.as_mut().poll(&mut context) else {
                    panic!("returned slot lost in registration race");
                };
                lease
            }
        };
        drop((lease, pending));
    }
    exhaust(&domain, 1);
}

#[test]
fn rollback_reuses_another_slot_while_actual_native_destruction_is_held() {
    struct Held {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self.entered.send(());
            let _ = self.release.recv_timeout(Duration::from_secs(5));
        }
    }
    let domain = RetirementDomain::try_start(2, 0).unwrap();
    let (entered, entering) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let retired = domain.try_reserve().unwrap().retire(Held {
        entered,
        release: released,
    });
    let started = entering.recv_timeout(Duration::from_secs(2));
    let worker_domain = domain.clone();
    let (sent, received) = mpsc::channel();
    let worker = thread::spawn(move || {
        for _ in 0..64 {
            let unused = worker_domain.try_reserve().unwrap();
            assert!(
                worker_domain.try_reserve().is_err(),
                "held native slot was reused early"
            );
            drop(unused);
        }
        sent.send(()).unwrap();
    });
    let rolled_back = received.recv_timeout(Duration::from_secs(2));
    let _ = release.send(());
    worker.join().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let acknowledged =
        runtime.block_on(async { tokio::time::timeout(Duration::from_secs(2), retired).await });
    assert!(started.is_ok());
    assert!(
        rolled_back.is_ok(),
        "metadata rollback queued behind held native teardown"
    );
    assert!(matches!(acknowledged, Ok(Ok(()))));
    exhaust(&domain, 2);
}
