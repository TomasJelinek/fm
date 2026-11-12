use super::*;
use midnight_core::{GeneratedTextSource, SizeWarningSource, SizeWarningSummary};
use std::sync::{
    Condvar,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Debug, Default)]
struct Probe {
    entered: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    released: Mutex<bool>,
    wake: Condvar,
    threads: Mutex<Vec<std::thread::ThreadId>>,
    drops: Mutex<Vec<std::thread::ThreadId>>,
}
impl Probe {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
#[derive(Debug)]
struct Source(Arc<Probe>);
impl Drop for Source {
    fn drop(&mut self) {
        self.0
            .drops
            .lock()
            .unwrap()
            .push(std::thread::current().id());
    }
}
impl SizeWarningSource for Source {
    fn snapshot(&self, cancel: &Cancellation) -> Result<Arc<dyn GeneratedTextSource>, FsError> {
        self.0
            .threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        let active = self.0.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.peak.fetch_max(active, Ordering::SeqCst);
        self.0.entered.fetch_add(1, Ordering::SeqCst);
        let mut released = self.0.released.lock().unwrap();
        while !*released && !cancel.is_cancelled() {
            released = self
                .0
                .wake
                .wait_timeout(released, Duration::from_millis(10))
                .unwrap()
                .0;
        }
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            Err(error(io::ErrorKind::Interrupted, "cancelled"))
        } else {
            Ok(Arc::new(Text))
        }
    }
}
#[derive(Debug)]
struct Text;
impl GeneratedTextSource for Text {
    fn length(&self) -> u64 {
        0
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
}
fn report(probe: &Arc<Probe>) -> SizeWarningReport {
    SizeWarningReport::new(
        Arc::new(Source(probe.clone())),
        SizeWarningSummary {
            observed: 1,
            stored: 1,
            tooltip: "denied".into(),
        },
    )
}
async fn entered(probe: &Probe, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while probe.entered.load(Ordering::SeqCst) < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn setup() -> (Controller, mpsc::Receiver<Event>, JoinSet<()>) {
    let (sender, receiver) = mpsc::channel(32);
    (
        Controller::new(Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        }),
        receiver,
        JoinSet::new(),
    )
}

#[tokio::test]
async fn snapshots_run_on_two_workers_and_full_queue_retires_ownership_off_caller() {
    let caller = std::thread::current().id();
    let probe = Arc::new(Probe::default());
    let (mut controller, mut events, mut tasks) = setup();
    for id in 0..WORKERS {
        controller
            .open(
                DesktopRequestId(id as u64),
                report(&probe),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
    }
    entered(&probe, WORKERS).await;
    for id in WORKERS..WORKERS + PENDING + 1 {
        controller
            .open(
                DesktopRequestId(id as u64),
                report(&probe),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
    }
    let Event::SizeWarningsReady { result, .. } = events.recv().await.unwrap() else {
        panic!()
    };
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::WouldBlock);
    assert_eq!(probe.entered.load(Ordering::SeqCst), WORKERS);
    assert_eq!(probe.drops.lock().unwrap().len(), 1);
    probe.release();
    for _ in 0..WORKERS + PENDING {
        let Event::SizeWarningsReady { result, .. } = events.recv().await.unwrap() else {
            panic!()
        };
        assert!(result.is_ok());
    }
    controller.shutdown();
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert_eq!(probe.peak.load(Ordering::SeqCst), WORKERS);
    assert!(
        probe
            .threads
            .lock()
            .unwrap()
            .iter()
            .all(|thread| *thread != caller)
    );
    assert_eq!(probe.drops.lock().unwrap().len(), WORKERS + PENDING + 1);
    assert!(
        probe
            .drops
            .lock()
            .unwrap()
            .iter()
            .all(|thread| *thread != caller)
    );
}

#[tokio::test]
async fn shutdown_cancels_active_snapshots_and_retires_pending_sources_on_workers() {
    let caller = std::thread::current().id();
    let probe = Arc::new(Probe::default());
    let (mut controller, _events, mut tasks) = setup();
    for id in 0..WORKERS {
        controller
            .open(
                DesktopRequestId(id as u64),
                report(&probe),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
    }
    entered(&probe, WORKERS).await;
    for id in WORKERS..WORKERS + PENDING {
        controller
            .open(
                DesktopRequestId(id as u64),
                report(&probe),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
    }
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(probe.entered.load(Ordering::SeqCst), WORKERS);
    assert_eq!(probe.drops.lock().unwrap().len(), WORKERS + PENDING);
    assert!(
        probe
            .drops
            .lock()
            .unwrap()
            .iter()
            .all(|thread| *thread != caller)
    );
}

#[tokio::test]
async fn cancelled_request_is_correlated_and_does_not_open_the_source() {
    let probe = Arc::new(Probe::default());
    let (mut controller, mut events, mut tasks) = setup();
    let cancel = Cancellation::default();
    cancel.cancel();
    controller
        .open(DesktopRequestId(41), report(&probe), cancel, &mut tasks)
        .await;
    let Event::SizeWarningsReady { request, result } = events.recv().await.unwrap() else {
        panic!()
    };
    assert_eq!(request, DesktopRequestId(41));
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::Interrupted);
    assert_eq!(probe.entered.load(Ordering::SeqCst), 0);
    controller.shutdown();
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}
