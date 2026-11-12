use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use tokio::sync::Semaphore;

struct Child {
    exited: Arc<AtomicBool>,
    mode: usize,
    polled: mpsc::Sender<usize>,
    dropped: mpsc::Sender<thread::ThreadId>,
    release: Option<mpsc::Receiver<()>>,
}
impl TerminalProcess for Child {
    fn id(&self) -> u32 {
        9
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let mode = std::mem::take(&mut self.mode);
        let _ = self.polled.send(mode);
        if mode == 1 {
            return Err(io::Error::other("uncertain exit"));
        }
        if mode == 2 {
            panic!("injected wait panic");
        }
        Ok(self
            .exited
            .load(Ordering::Acquire)
            .then(ExitStatus::default))
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.dropped.send(thread::current().id());
        if let Some(release) = &self.release {
            let _ = release.recv_timeout(Duration::from_secs(5));
        }
    }
}
struct Exit(Arc<AtomicBool>);
impl Drop for Exit {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
fn permit() -> (Arc<Semaphore>, Arc<OwnedSemaphorePermit>) {
    let quota = Arc::new(Semaphore::new(1));
    let permit = Arc::new(quota.clone().try_acquire_owned().unwrap());
    (quota, permit)
}
fn returned(quota: &Semaphore) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while quota.available_permits() == 0 {
        assert!(std::time::Instant::now() < deadline, "quota never returned");
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn startup_failure_is_reported_before_accepting_any_payload() {
    let called = AtomicUsize::new(0);
    let result = ProcessTracker::start(|_worker| {
        called.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("injected thread failure"))
    });
    assert!(result.is_err());
    assert_eq!(called.load(Ordering::SeqCst), 1);
}
#[test]
fn idle_tracker_stops_after_its_last_handle_without_a_runtime() {
    let (finished, done) = mpsc::channel();
    let tracker = ProcessTracker::start(|worker| {
        thread::Builder::new().spawn(move || {
            worker();
            let _ = finished.send(());
        })
    })
    .unwrap();
    drop(tracker);
    done.recv_timeout(Duration::from_secs(5)).unwrap();
}
#[test]
fn tickets_bound_admission_and_cancelled_unlaunched_reservation_returns_capacity() {
    let tracker = ProcessTracker::try_start().unwrap();
    let mut tickets: Vec<_> = (0..MAX_PROCESSES)
        .map(|_| tracker.try_reserve().unwrap())
        .collect();
    assert_eq!(
        tracker.try_reserve().err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(tickets.pop());
    let replacement = tracker.try_reserve().unwrap();
    assert!(tracker.try_reserve().is_err());
    drop(replacement);
}
#[test]
fn uncertain_waits_keep_quota_after_all_producers_close() {
    for mode in [1, 2] {
        let tracker = ProcessTracker::try_start().unwrap();
        let ticket = tracker.try_reserve().unwrap();
        let (quota, permit) = permit();
        let exited = Arc::new(AtomicBool::new(false));
        let _exit = Exit(exited.clone());
        let (polled, polls) = mpsc::channel();
        let (dropped, drops) = mpsc::channel();
        let child = ticket.own(
            Box::new(Child {
                exited: exited.clone(),
                mode,
                polled,
                dropped,
                release: None,
            }),
            permit,
            Arc::new(()),
        );
        drop(tracker);
        drop(child);
        assert_eq!(polls.recv_timeout(Duration::from_secs(5)).unwrap(), mode);
        assert_eq!(quota.available_permits(), 0);
        assert!(drops.try_recv().is_err());
        exited.store(true, Ordering::Release);
        assert_ne!(
            drops.recv_timeout(Duration::from_secs(5)).unwrap(),
            thread::current().id()
        );
        returned(&quota);
    }
}
#[test]
fn confirmed_exit_keeps_ticket_and_quota_until_actual_native_drop_finishes() {
    let tracker = ProcessTracker::try_start().unwrap();
    let mut tickets: Vec<_> = (0..MAX_PROCESSES)
        .map(|_| tracker.try_reserve().unwrap())
        .collect();
    let (quota, permit) = permit();
    let (polled, _) = mpsc::channel();
    let (dropped, drops) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let child = tickets.pop().unwrap().own(
        Box::new(Child {
            exited: Arc::new(AtomicBool::new(true)),
            mode: 0,
            polled,
            dropped,
            release: Some(wait),
        }),
        permit,
        Arc::new(()),
    );
    drop(child);
    assert_ne!(
        drops.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    assert_eq!(
        quota.available_permits(),
        0,
        "blocked native Drop retains quota"
    );
    assert!(
        tracker.try_reserve().is_err(),
        "blocked native Drop retains ticket"
    );
    release.send(()).unwrap();
    returned(&quota);
    let _replacement = tracker.try_reserve().unwrap();
}
#[test]
fn normal_reaper_confirmed_exit_is_not_polled_again_during_retirement() {
    let tracker = ProcessTracker::try_start().unwrap();
    let (quota, permit) = permit();
    let (polled, polls) = mpsc::channel();
    let (dropped, drops) = mpsc::channel();
    let mut child = tracker.try_reserve().unwrap().own(
        Box::new(Child {
            exited: Arc::new(AtomicBool::new(true)),
            mode: 0,
            polled,
            dropped,
            release: None,
        }),
        permit,
        Arc::new(()),
    );
    assert!(child.try_wait().unwrap().is_some());
    assert_eq!(polls.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    drop(child);
    assert_ne!(
        drops.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    returned(&quota);
    assert!(polls.try_recv().is_err());
}

struct PanicId(Child);
impl TerminalProcess for PanicId {
    fn id(&self) -> u32 {
        panic!("injected id panic")
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }
}
#[test]
fn id_panic_preserves_the_child_and_quota_through_actual_native_destruction() {
    let tracker = ProcessTracker::try_start().unwrap();
    let ticket = tracker.try_reserve().unwrap();
    let (quota, permit) = permit();
    let (polled, _) = mpsc::channel();
    let (dropped, drops) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let result = catch_unwind(AssertUnwindSafe(|| {
        ticket.own(
            Box::new(PanicId(Child {
                exited: Arc::new(AtomicBool::new(true)),
                mode: 0,
                polled,
                dropped,
                release: Some(wait),
            })),
            permit,
            Arc::new(()),
        )
    }));
    assert!(result.is_err());
    assert_ne!(
        drops.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    assert_eq!(quota.available_permits(), 0);
    release.send(()).unwrap();
    returned(&quota);
}

#[test]
fn opener_tracker_has_sixteen_independent_admissions() {
    let terminals = ProcessTracker::try_start().unwrap();
    let openers = ProcessTracker::try_start_openers().unwrap();
    let terminal_tickets: Vec<_> = (0..MAX_PROCESSES)
        .map(|_| terminals.try_reserve().unwrap())
        .collect();
    let opener_tickets: Vec<_> = (0..MAX_OPEN_PROCESSES)
        .map(|_| openers.try_reserve().unwrap())
        .collect();
    assert_eq!(terminal_tickets.len(), 8);
    assert_eq!(opener_tickets.len(), 16);
    assert!(terminals.try_reserve().is_err());
    assert!(openers.try_reserve().is_err());
    drop(terminal_tickets);
    assert!(
        openers.try_reserve().is_err(),
        "terminal retirement cannot free opener slots"
    );
    assert!(terminals.try_reserve().is_ok());
}
