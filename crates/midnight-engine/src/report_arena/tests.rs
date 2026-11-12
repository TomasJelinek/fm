use super::*;
use crate::retirement::RetirementDomain;
use std::{
    sync::mpsc,
    thread::{self, ThreadId},
    time::{Duration, Instant},
};

type Gate = Arc<(Mutex<bool>, Condvar)>;
struct Release(Gate);
impl Release {
    fn open(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.open();
    }
}
struct Gated {
    entered: mpsc::Sender<ThreadId>,
    gate: Gate,
}
impl Drop for Gated {
    fn drop(&mut self) {
        self.entered.send(thread::current().id()).unwrap();
        let _released = self
            .gate
            .1
            .wait_while(self.gate.0.lock().unwrap(), |open| !*open)
            .unwrap();
    }
}
fn gated() -> (Gated, mpsc::Receiver<ThreadId>, Release) {
    let (entered, receiver) = mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    (
        Gated {
            entered,
            gate: gate.clone(),
        },
        receiver,
        Release(gate),
    )
}
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "report ownership completion timed out"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
fn free(arena: &ReportArena) -> bool {
    arena.control.shared.queue.lock().unwrap().free.is_some()
}
fn stop_worker(arena: &ReportArena, panic: bool) {
    {
        let mut queue = arena.control.shared.queue.lock().unwrap();
        queue.stop_worker = true;
        queue.panic_worker = panic;
    }
    arena.control.shared.ready.notify_one();
    until(|| {
        !arena
            .control
            .shared
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accepting
    });
}

#[test]
fn startup_failure_returns_no_factory_and_releases_the_startup_lease() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let error = ReportArena::start_with(domain.try_reserve().unwrap(), |_| {
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "injected startup failure",
        ))
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    until(|| domain.try_reserve().is_ok());
}

#[test]
fn unused_reservation_recycles_its_existing_node_without_a_native_payload() {
    let arena = ReportArena::testing();
    drop(arena.reserve().unwrap());
    until(|| free(&arena));
    let again = arena.reserve().unwrap();
    assert_eq!(arena.control.shared.queue.lock().unwrap().nodes.len(), 1);
    drop(again);
}

#[test]
fn shared_prefix_token_keeps_native_owner_until_the_last_clone() {
    let arena = ReportArena::testing();
    let (payload, entered, release) = gated();
    let writer = arena.reserve().unwrap().retain(payload);
    let prefix = writer.retention();
    let second = prefix.clone();
    drop(writer);
    drop(prefix);
    assert!(entered.try_recv().is_err());
    drop(second);
    assert_ne!(
        entered.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    assert!(
        !free(&arena),
        "node is occupied until native destruction completes"
    );
    release.open();
    until(|| free(&arena));
}

#[test]
fn blocked_cleanup_never_holds_registry_lock_or_reuses_its_node_early() {
    let arena = ReportArena::testing();
    let (payload, entered, release) = gated();
    drop(arena.reserve().unwrap().retain(payload));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let another = arena.reserve().unwrap();
    assert_eq!(arena.control.shared.queue.lock().unwrap().nodes.len(), 2);
    drop(another);
    for _ in 0..32 {
        drop(arena.reserve().unwrap());
        assert_eq!(
            arena.control.shared.queue.lock().unwrap().nodes.len(),
            2,
            "cancelled creation must reuse its empty node while native cleanup is blocked"
        );
    }
    let (done, finished) = mpsc::channel();
    let closing = arena.clone();
    thread::spawn(move || {
        drop(closing);
        done.send(()).unwrap();
    });
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    release.open();
    until(|| free(&arena));
}

struct Panics(mpsc::Sender<()>);
impl Drop for Panics {
    fn drop(&mut self) {
        self.0.send(()).unwrap();
        panic!("injected native destructor panic");
    }
}
#[test]
fn destructor_panic_is_contained_and_the_worker_cleans_the_next_file() {
    let arena = ReportArena::testing();
    let (done, panicked) = mpsc::channel();
    drop(arena.reserve().unwrap().retain(Panics(done)));
    panicked.recv_timeout(Duration::from_secs(5)).unwrap();
    let (payload, entered, release) = gated();
    drop(arena.reserve().unwrap().retain(payload));
    assert_ne!(
        entered.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    release.open();
    until(|| free(&arena));
}

#[test]
fn worker_exit_rejects_new_files_and_fallback_holds_lease_through_actual_cleanup() {
    for panic in [false, true] {
        let domain = RetirementDomain::try_start(1, 0).unwrap();
        let arena = ReportArena::try_start(domain.try_reserve().unwrap()).unwrap();
        let (payload, entered, release) = gated();
        let retained = arena.reserve().unwrap().retain(payload);
        stop_worker(&arena, panic);
        assert!(matches!(arena.reserve(), Err(error) if error.kind() == io::ErrorKind::BrokenPipe));
        drop(retained);
        assert!(
            entered.try_recv().is_err(),
            "fallback remains retained by factory"
        );
        drop(arena);
        assert_ne!(
            entered.recv_timeout(Duration::from_secs(5)).unwrap(),
            thread::current().id()
        );
        assert!(
            domain.try_reserve().is_err(),
            "startup lease returned before native Drop"
        );
        release.open();
        until(|| domain.try_reserve().is_ok());
    }
}

#[test]
fn admitted_creation_can_finish_safely_after_worker_exit() {
    let arena = ReportArena::testing();
    let reservation = arena.reserve().unwrap();
    stop_worker(&arena, false);
    let (payload, entered, release) = gated();
    let retained = reservation.retain(payload);
    drop(arena);
    drop(retained);
    assert_ne!(
        entered.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    release.open();
}

#[test]
fn fallback_contains_each_destructor_panic_and_drops_remaining_files() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let arena = ReportArena::try_start(domain.try_reserve().unwrap()).unwrap();
    let (done, panicked) = mpsc::channel();
    let first = arena.reserve().unwrap().retain(Panics(done));
    let (payload, entered, release) = gated();
    let second = arena.reserve().unwrap().retain(payload);
    stop_worker(&arena, false);
    drop(first);
    drop(second);
    drop(arena);
    panicked.recv_timeout(Duration::from_secs(5)).unwrap();
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(domain.try_reserve().is_err());
    release.open();
    until(|| domain.try_reserve().is_ok());
}

#[test]
fn failed_private_file_creation_releases_its_preadmitted_empty_node() {
    let temporary = tempfile::tempdir().unwrap();
    let occupied = temporary.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let arena = ReportArena::testing();
    let reservation = arena.reserve().unwrap();
    let original = reservation.retention._token.node;
    assert!(midnight_worker_platform::spool::PrivateSpoolDirectory::open(&occupied).is_err());
    drop(reservation);
    until(|| free(&arena));
    let next = arena.reserve().unwrap();
    assert_eq!(next.retention._token.node, original);
    assert_eq!(arena.control.shared.queue.lock().unwrap().nodes.len(), 1);
}

#[test]
fn dropping_the_final_factory_never_joins_a_blocked_cleanup_worker() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let arena = ReportArena::try_start(domain.try_reserve().unwrap()).unwrap();
    let (payload, entered, release) = gated();
    drop(arena.reserve().unwrap().retain(payload));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let (returned, receive) = mpsc::channel();
    let caller = thread::spawn(move || {
        drop(arena);
        returned.send(()).unwrap();
    });
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    caller.join().unwrap();
    assert!(
        domain.try_reserve().is_err(),
        "blocked native cleanup must keep its startup lease"
    );
    release.open();
    until(|| domain.try_reserve().is_ok());
}
