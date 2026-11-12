use super::*;
use midnight_core::TerminalProcess;
use std::sync::atomic::{AtomicBool, Ordering};

struct UncertainChild {
    polls: mpsc::UnboundedSender<usize>,
    count: usize,
    exited: Arc<AtomicBool>,
}
impl TerminalProcess for UncertainChild {
    fn id(&self) -> u32 {
        1
    }
    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.count += 1;
        let _ = self.polls.send(self.count);
        if self.count == 1 {
            Err(io::Error::other("Owned child exit is temporarily unknown"))
        } else if self.exited.load(Ordering::Acquire) {
            Ok(Some(std::process::ExitStatus::default()))
        } else {
            Ok(None)
        }
    }
}

#[tokio::test]
async fn fallback_retains_process_quota_after_wait_error_until_confirmed_exit() {
    let (children, reaper) = mpsc::channel(1);
    drop(reaper);
    let processes = Arc::new(Semaphore::new(1));
    let runtime = Runtime::new(
        Arc::new(Semaphore::new(1)),
        processes.clone(),
        children,
        ProcessTracker::try_start().unwrap(),
    );
    let (polls, mut observed) = mpsc::unbounded_channel();
    let exited = Arc::new(AtomicBool::new(false));
    let child = Box::new(UncertainChild {
        polls,
        count: 0,
        exited: exited.clone(),
    });
    let permit = processes.clone().acquire_owned().await.unwrap();
    let child = runtime
        .tracker
        .try_reserve()
        .unwrap()
        .own(child, Arc::new(permit), Arc::new(()));
    assert!(runtime.track(child).await.is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), observed.recv())
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(processes.available_permits(), 0, "Wait error is not exit.");
    assert!(processes.clone().try_acquire_owned().is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), observed.recv())
            .await
            .unwrap(),
        Some(2)
    );
    assert_eq!(processes.available_permits(), 0);
    exited.store(true, Ordering::Release);
    let returned = tokio::time::timeout(Duration::from_secs(5), processes.acquire())
        .await
        .expect("Confirmed exit must return the process slot.")
        .unwrap();
    drop(returned);
    assert_eq!(processes.available_permits(), 1);
}

struct ShutdownChild {
    polled: Arc<tokio::sync::Notify>,
    exited: Arc<AtomicBool>,
    dropped: std::sync::mpsc::Sender<std::thread::ThreadId>,
}
impl TerminalProcess for ShutdownChild {
    fn id(&self) -> u32 {
        7
    }
    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.polled.notify_one();
        Ok(self
            .exited
            .load(Ordering::Acquire)
            .then(std::process::ExitStatus::default))
    }
}
impl Drop for ShutdownChild {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}
struct ExitOnDrop(Arc<AtomicBool>);
impl Drop for ExitOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
#[test]
fn fallback_survives_runtime_shutdown_and_retires_only_after_confirmed_exit() {
    let caller = std::thread::current().id();
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (children, reaper) = mpsc::channel(1);
    drop(reaper);
    let processes = Arc::new(Semaphore::new(1));
    let runtime = Runtime::new(
        Arc::new(Semaphore::new(1)),
        processes.clone(),
        children,
        ProcessTracker::try_start().unwrap(),
    );
    let exited = Arc::new(AtomicBool::new(false));
    let _exit = ExitOnDrop(exited.clone());
    let polled = Arc::new(tokio::sync::Notify::new());
    let (dropped, observed) = std::sync::mpsc::channel();
    tokio.block_on(async {
        let slot = processes.clone().acquire_owned().await.unwrap();
        let child = Box::new(ShutdownChild {
            polled: polled.clone(),
            exited: exited.clone(),
            dropped,
        });
        let child = runtime
            .tracker
            .try_reserve()
            .unwrap()
            .own(child, Arc::new(slot), Arc::new(()));
        assert!(runtime.track(child).await.is_err());
        tokio::time::timeout(Duration::from_secs(5), polled.notified())
            .await
            .expect("the fallback must begin tracking");
    });
    drop(runtime);
    tokio.shutdown_timeout(Duration::from_millis(100));
    assert_eq!(
        processes.available_permits(),
        0,
        "runtime loss must not release a live process quota"
    );
    assert!(
        observed.try_recv().is_err(),
        "live process owner was dropped"
    );
    exited.store(true, Ordering::Release);
    let retired = observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(retired, caller, "native process Drop ran on the caller");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while processes.available_permits() == 0 {
        assert!(std::time::Instant::now() < deadline, "quota never returned");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[tokio::test]
async fn unlaunched_reservations_and_cancelled_admission_return_both_budgets() {
    let (children, _reaper) = mpsc::channel(1);
    let launches = Arc::new(Semaphore::new(1));
    let processes = Arc::new(Semaphore::new(1));
    let tracker = ProcessTracker::try_start().unwrap();
    let tickets: Vec<_> = (0..MAX_PROCESSES)
        .map(|_| tracker.try_reserve().unwrap())
        .collect();
    let runtime = Runtime::new(
        launches.clone(),
        processes.clone(),
        children,
        tracker.clone(),
    );
    let cancel = Cancellation::default();
    assert_eq!(
        runtime.reserve(&cancel).await.err().unwrap().kind,
        io::ErrorKind::WouldBlock
    );
    assert_eq!(launches.available_permits(), 1);
    assert_eq!(processes.available_permits(), 1);
    drop(tickets);
    let reserved = runtime.reserve(&cancel).await.unwrap();
    assert_eq!(launches.available_permits(), 0);
    assert_eq!(processes.available_permits(), 0);
    cancel.cancel();
    drop(reserved);
    assert_eq!(launches.available_permits(), 1);
    assert_eq!(processes.available_permits(), 1);
    assert_eq!(
        runtime.reserve(&cancel).await.err().unwrap().kind,
        io::ErrorKind::Interrupted
    );
    let _tickets: Vec<_> = (0..MAX_PROCESSES)
        .map(|_| tracker.try_reserve().unwrap())
        .collect();
}

#[tokio::test]
async fn cancelled_tracking_send_and_normal_receiver_loss_preserve_running_children() {
    let caller = std::thread::current().id();
    let (children, reaper) = mpsc::channel(1);
    let processes = Arc::new(Semaphore::new(2));
    let runtime = Runtime::new(
        Arc::new(Semaphore::new(1)),
        processes.clone(),
        children,
        ProcessTracker::try_start().unwrap(),
    );
    let exited = Arc::new(AtomicBool::new(false));
    let _exit = ExitOnDrop(exited.clone());
    let (dropped, observed) = std::sync::mpsc::channel();
    let mut owned = Vec::new();
    for _ in 0..2 {
        let reservation = runtime.reserve(&Cancellation::default()).await.unwrap();
        let child = reservation.ticket.own(
            Box::new(ShutdownChild {
                polled: Arc::new(tokio::sync::Notify::new()),
                exited: exited.clone(),
                dropped: dropped.clone(),
            }),
            reservation.process,
            Arc::new(()),
        );
        drop(reservation.launch);
        owned.push(child);
    }
    runtime.track(owned.pop().unwrap()).await.unwrap();
    let mut pending = Box::pin(runtime.track(owned.pop().unwrap()));
    std::future::poll_fn(|context| {
        assert!(
            pending.as_mut().poll(context).is_pending(),
            "tracking queue must be full"
        );
        std::task::Poll::Ready(())
    })
    .await;
    drop(pending);
    drop(reaper);
    drop(runtime);
    assert_eq!(processes.available_permits(), 0);
    assert!(
        observed.try_recv().is_err(),
        "running children must remain owned"
    );
    exited.store(true, Ordering::Release);
    for _ in 0..2 {
        assert_ne!(
            observed.recv_timeout(Duration::from_secs(5)).unwrap(),
            caller
        );
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while processes.available_permits() != 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "process slots were lost"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
