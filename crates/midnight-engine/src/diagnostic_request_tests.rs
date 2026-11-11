use super::*;
use std::{
    sync::{
        Condvar,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Debug, Default)]
struct Probe {
    entered: AtomicUsize,
    release: Mutex<bool>,
    wake: Condvar,
    reads: Mutex<Vec<std::thread::ThreadId>>,
    drops: Mutex<Vec<std::thread::ThreadId>>,
}
impl Probe {
    fn release(&self) {
        *self.release.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
struct ReleaseOnDrop(Arc<Probe>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Debug)]
struct Source(Arc<Probe>);
impl GeneratedTextSource for Source {
    fn length(&self) -> u64 {
        6
    }
    fn read_at(&self, _: u64, output: &mut [u8]) -> io::Result<usize> {
        self.0
            .reads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        self.0.entered.fetch_add(1, Ordering::SeqCst);
        let mut release = self.0.release.lock().unwrap();
        while !*release {
            release = self.0.wake.wait(release).unwrap();
        }
        output[..6].copy_from_slice(b"report");
        Ok(6)
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.0
            .drops
            .lock()
            .unwrap()
            .push(std::thread::current().id());
    }
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
async fn entered(probe: &Probe, expected: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while probe.entered.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn stop(controller: &mut Controller, tasks: &mut JoinSet<()>) {
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn preview_reads_and_retires_captured_sources_off_the_caller_and_correlates_reply() {
    let caller = std::thread::current().id();
    let probe = Arc::new(Probe::default());
    probe.release();
    let (mut controller, mut events, mut tasks) = setup();
    controller
        .preview(
            DesktopRequestId(57),
            Arc::new(Source(probe.clone())),
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    let Event::DiagnosticTextReady { request, result } = events.recv().await.unwrap() else {
        panic!()
    };
    assert_eq!(request, DesktopRequestId(57));
    assert_eq!(result.unwrap().text, "report");
    stop(&mut controller, &mut tasks).await;
    assert_eq!(probe.reads.lock().unwrap().len(), 1);
    assert!(
        probe
            .reads
            .lock()
            .unwrap()
            .iter()
            .all(|thread| *thread != caller)
    );
    assert_eq!(probe.drops.lock().unwrap().len(), 1);
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
async fn cancelled_late_read_is_not_published() {
    let probe = Arc::new(Probe::default());
    let _release = ReleaseOnDrop(probe.clone());
    let (mut controller, mut events, mut tasks) = setup();
    let cancel = Cancellation::default();
    controller
        .preview(
            DesktopRequestId(58),
            Arc::new(Source(probe.clone())),
            cancel.clone(),
            &mut tasks,
        )
        .await;
    entered(&probe, 1).await;
    cancel.cancel();
    probe.release();
    tokio::time::timeout(Duration::from_secs(5), async {
        while probe.drops.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        events.try_recv().is_err(),
        "request cancellation alone suppresses late delivery"
    );
    stop(&mut controller, &mut tasks).await;
}

#[tokio::test]
async fn preview_queue_uses_existing_two_workers_and_eight_pending_budget() {
    let caller = std::thread::current().id();
    let probe = Arc::new(Probe::default());
    let _release = ReleaseOnDrop(probe.clone());
    let (mut controller, mut events, mut tasks) = setup();
    for id in 0..WORKERS {
        controller
            .preview(
                DesktopRequestId(id as u64),
                Arc::new(Source(probe.clone())),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
    }
    entered(&probe, WORKERS).await;
    for id in WORKERS..WORKERS + PENDING + 1 {
        controller
            .preview(
                DesktopRequestId(id as u64),
                Arc::new(Source(probe.clone())),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
    }
    let Event::DiagnosticTextReady { request, result } = events.recv().await.unwrap() else {
        panic!()
    };
    assert_eq!(request, DesktopRequestId((WORKERS + PENDING) as u64));
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::WouldBlock);
    assert_eq!(probe.entered.load(Ordering::SeqCst), WORKERS);
    assert_eq!(probe.drops.lock().unwrap().len(), 1);
    assert_ne!(probe.drops.lock().unwrap()[0], caller);
    probe.release();
    for _ in 0..WORKERS + PENDING {
        assert!(matches!(
            events.recv().await.unwrap(),
            Event::DiagnosticTextReady { result: Ok(_), .. }
        ));
    }
    stop(&mut controller, &mut tasks).await;
}

#[derive(Debug)]
struct Panicking;
impl GeneratedTextSource for Panicking {
    fn length(&self) -> u64 {
        1
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("injected source failure")
    }
}

#[tokio::test]
async fn a_source_panic_becomes_a_correlated_error_without_losing_the_pool() {
    let (mut controller, mut events, mut tasks) = setup();
    controller
        .preview(
            DesktopRequestId(91),
            Arc::new(Panicking),
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    let Event::DiagnosticTextReady { request, result } = events.recv().await.unwrap() else {
        panic!()
    };
    assert_eq!(request, DesktopRequestId(91));
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::Other);
    let probe = Arc::new(Probe::default());
    probe.release();
    controller
        .preview(
            DesktopRequestId(92),
            Arc::new(Source(probe)),
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::DiagnosticTextReady {
            request: DesktopRequestId(92),
            result: Ok(_),
        }
    ));
    stop(&mut controller, &mut tasks).await;
}
