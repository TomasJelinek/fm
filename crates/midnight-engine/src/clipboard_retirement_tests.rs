//! Native clipboard ownership must survive cancellation and result backpressure.
use super::*;
use std::{
    sync::mpsc as channel,
    thread::{self, ThreadId},
};

struct IdleService(channel::Sender<ThreadId>);
impl ClipboardService for IdleService {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        panic!("idle clipboard must not open a native session")
    }
}
impl Drop for IdleService {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}
fn outbox() -> (Outbox, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(1);
    (
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        events,
    )
}
#[test]
fn clipboard_retirement_idle_service_never_drops_on_caller() {
    let (sent, dropped) = channel::channel();
    let (outbox, _events) = outbox();
    let (_owner, controller) = Controller::testing(Some(Arc::new(IdleService(sent))), outbox);
    drop(controller);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(2)).unwrap(),
        thread::current().id()
    );
}

struct PumpService {
    opened: channel::Sender<ThreadId>,
    copied: channel::Sender<ThreadId>,
    closed: channel::Sender<ThreadId>,
}
struct Session {
    copied: channel::Sender<ThreadId>,
    closed: channel::Sender<ThreadId>,
    owner: ThreadId,
}
impl ClipboardService for PumpService {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        let owner = thread::current().id();
        self.opened.send(owner).unwrap();
        Ok(Box::new(Session {
            copied: self.copied.clone(),
            closed: self.closed.clone(),
            owner,
        }))
    }
}
impl ClipboardSession for Session {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        assert_eq!(self.owner, thread::current().id());
        self.copied.send(thread::current().id()).unwrap();
        Ok(())
    }
    fn service_pending(&mut self) -> Result<(), FsError> {
        assert_eq!(self.owner, thread::current().id());
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.closed.send(thread::current().id());
    }
}
#[test]
fn clipboard_retirement_controller_drop_stops_full_outbox_on_native_thread() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (opened, opens) = channel::channel();
    let (copied, copies) = channel::channel();
    let (closed, closes) = channel::channel();
    let (outbox, events) = outbox();
    outbox
        .sender
        .try_send(Event::LocaleLoaded(Ok(Default::default())))
        .unwrap();
    let (resident_owner, mut controller) = Controller::testing(
        Some(Arc::new(PumpService {
            opened,
            copied,
            closed,
        })),
        outbox,
    );
    let cancel = controller.cancel.clone();
    let mut tasks = ResidentTasks::default();
    runtime.block_on(controller.copy(DesktopRequestId(710), "owned fixture".into(), &mut tasks));
    let owner = opens.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(copies.recv_timeout(Duration::from_secs(2)).unwrap(), owner);
    drop(controller);
    let closed = closes.recv_timeout(Duration::from_secs(2));
    // Always release the pre-fix worker too; failed regressions must not hang the runtime.
    cancel.cancel();
    drop(events);
    runtime.block_on(async { while tasks.join_next().await.is_some() {} });
    assert_eq!(
        closed.expect("dropping Controller must stop a backpressured native pump"),
        owner
    );
    assert_ne!(owner, thread::current().id());
    runtime
        .block_on(resident_owner.shutdown(runtime.handle()))
        .unwrap();
}

#[derive(Debug)]
struct Capture {
    retired: channel::Sender<ThreadId>,
    resolved: Arc<std::sync::atomic::AtomicBool>,
}
impl midnight_core::CapturedClipboardFiles for Capture {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        self.resolved
            .store(true, std::sync::atomic::Ordering::SeqCst);
        panic!("unstarted clipboard must not resolve a captured source")
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.retired.send(thread::current().id());
    }
}
#[test]
fn clipboard_retirement_cancelled_unstarted_worker_retains_queued_capture() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_time()
        .build()
        .unwrap();
    let (sent, dropped) = channel::channel();
    let (outbox, _events) = outbox();
    let (owner, mut controller) = Controller::testing(None, outbox);
    let holders =
        crate::engine_resident_tests::hold_workers(&controller.resident, runtime.handle(), 1);
    let resolved = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut tasks = ResidentTasks::default();
    runtime.block_on(controller.copy_captured(
        DesktopRequestId(711),
        Arc::new(Capture {
            retired: sent,
            resolved: resolved.clone(),
        }),
        Cancellation::default(),
        &mut tasks,
    ));
    assert!(
        dropped.try_recv().is_err(),
        "queued work still owns the capture"
    );
    // The current-thread runtime is idle. Release the worker immediately
    // after abort, before any async waiter could be polled or destroyed.
    tasks.abort_all();
    drop(holders);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(2)).unwrap(),
        thread::current().id()
    );
    assert!(!resolved.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        runtime.block_on(tasks.join_next()),
        Some(Err(midnight_work_budget::resident::TaskError::Cancelled))
    );
    controller.shutdown();
    runtime.block_on(owner.shutdown(runtime.handle())).unwrap();
}

struct SlowNative {
    entered: channel::Sender<ThreadId>,
    closed: channel::Sender<ThreadId>,
    gate: std::sync::Mutex<Option<channel::Receiver<()>>>,
}

struct SlowSession {
    thread: ThreadId,
    entered: channel::Sender<ThreadId>,
    closed: channel::Sender<ThreadId>,
    gate: channel::Receiver<()>,
}

impl ClipboardService for SlowNative {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        Ok(Box::new(SlowSession {
            thread: thread::current().id(),
            entered: self.entered.clone(),
            closed: self.closed.clone(),
            gate: self.gate.lock().unwrap().take().unwrap(),
        }))
    }
}

impl ClipboardSession for SlowSession {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        assert_eq!(thread::current().id(), self.thread);
        self.entered.send(self.thread).unwrap();
        let _ = self.gate.recv_timeout(Duration::from_secs(5));
        Ok(())
    }
}

impl Drop for SlowSession {
    fn drop(&mut self) {
        let _ = self.closed.send(thread::current().id());
    }
}

#[tokio::test]
async fn clipboard_shutdown_stays_unconfirmed_until_native_returns_and_releases_its_lease() {
    let (entered, started) = channel::channel();
    let (closed, closes) = channel::channel();
    let (release, gate) = channel::channel();
    let service = Arc::new(SlowNative {
        entered,
        closed,
        gate: std::sync::Mutex::new(Some(gate)),
    });
    let domain = crate::retirement::RetirementDomain::try_start(2, 1).unwrap();
    let (owner, resident) =
        midnight_work_budget::resident::Owner::start("clipboard-test", 1).unwrap();
    let (outbox, _events) = outbox();
    let mut controller = Controller::new(
        resident,
        Retirement {
            worker: domain.try_reserve().unwrap(),
            rejected: domain.new_waiter().unwrap(),
        },
        TaskLifetime::testing(service.clone()),
        Some(service),
        outbox,
    );
    let mut tasks = ResidentTasks::default();
    controller
        .copy(DesktopRequestId(713), "owned".into(), &mut tasks)
        .await;
    let native_thread = started.recv_timeout(Duration::from_secs(2)).unwrap();
    let neutral = controller.resident.clone();
    // Keep the rejected-request reservation occupied independently of the inbox.
    let rejected = domain.try_reserve().unwrap();
    controller.shutdown();
    tasks.abort_all();
    assert!(
        tokio::time::timeout(Duration::ZERO, tasks.join_next())
            .await
            .is_err(),
        "aborting a claimed actor must leave its completion pending"
    );
    let transient = tokio::runtime::Handle::current();
    let mut shutdown = Box::pin(owner.shutdown(&transient));
    assert!(
        tokio::time::timeout(Duration::ZERO, &mut shutdown)
            .await
            .is_err(),
        "cleanup cannot be confirmed while the native session is still live"
    );
    assert!(domain.try_reserve().is_err());
    assert!(closes.try_recv().is_err());
    release.send(()).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), tasks.join_next())
            .await
            .unwrap(),
        Some(Ok(()))
    );
    tokio::time::timeout(Duration::from_secs(5), shutdown)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closes.try_recv().unwrap(), native_thread);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(lease) = domain.try_reserve() {
            drop(lease);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    drop(rejected);
    crate::engine_resident_tests::assert_sealed(&neutral, &transient);
}

#[tokio::test]
async fn engine_shutdown_observes_claimed_clipboard_after_200_ms_within_outer_grace() {
    use midnight_core::Command;

    let (entered, started) = channel::channel();
    let (closed, closes) = channel::channel();
    let (release, gate) = channel::channel();
    let (sender, _events) = mpsc::channel(8);
    let mut engine = crate::Engine::new(
        crate::Engine::lifetime().unwrap(),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        sender,
    );
    engine
        .configure_clipboard()
        .unwrap()
        .install(Arc::new(SlowNative {
            entered,
            closed,
            gate: std::sync::Mutex::new(Some(gate)),
        }));
    let (commands, input) = engine.command_channel(2).unwrap();
    let mut run = tokio::spawn(engine.run(input));
    commands
        .try_send(Command::CopyText {
            request: DesktopRequestId(714),
            text: "owned".into(),
        })
        .unwrap();
    let native_thread = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(thread) = started.try_recv() {
                break thread;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2500);
    commands.try_send(Command::Shutdown).unwrap();
    tokio::time::timeout_at(deadline, commands.closed())
        .await
        .unwrap();
    let remained_pending = tokio::time::timeout(Duration::from_millis(400), &mut run)
        .await
        .is_err();
    let held_affine_session = closes.try_recv().is_err();
    release.send(()).unwrap();
    assert!(
        remained_pending,
        "Engine abandoned claimed actor completion"
    );
    assert!(
        held_affine_session,
        "native session ended before gate release"
    );
    tokio::time::timeout_at(deadline, run)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closes.try_recv().unwrap(), native_thread);
}

#[derive(Debug)]
struct GatedSource {
    entered: channel::Sender<ThreadId>,
    release: std::sync::Mutex<channel::Receiver<()>>,
}
impl midnight_core::CapturedClipboardFiles for GatedSource {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        panic!("rejected source must not resolve")
    }
}
impl midnight_core::GeneratedTextSource for GatedSource {
    fn length(&self) -> u64 {
        panic!("rejected source must not be read")
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("rejected source must not be read")
    }
}
impl Drop for GatedSource {
    fn drop(&mut self) {
        let _ = self.entered.send(thread::current().id());
        let _ = self
            .release
            .get_mut()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
    }
}
#[test]
fn clipboard_retirement_cancelled_rejection_keeps_slot_through_source_destruction() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    for generated in [false, true] {
        let domain = crate::retirement::RetirementDomain::try_start(2, 1).unwrap();
        let (outbox, mut events) = outbox();
        let (_owner, resident) =
            midnight_work_budget::resident::Owner::start("clipboard-test", 1).unwrap();
        let mut controller = Controller::new(
            resident,
            Retirement {
                worker: domain.try_reserve().unwrap(),
                rejected: domain.new_waiter().unwrap(),
            },
            TaskLifetime::testing(()),
            None,
            outbox,
        );
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        controller.sender = Some(sender);
        let (entered, started) = channel::channel();
        let (release, gate) = channel::channel();
        let source = Arc::new(GatedSource {
            entered,
            release: std::sync::Mutex::new(gate),
        });
        let action = if generated {
            Action::Generated(source, Cancellation::default())
        } else {
            Action::Captured(source, Cancellation::default())
        };
        let mut tasks = ResidentTasks::default();
        let mut future = Box::pin(controller.submit(
            Request {
                id: DesktopRequestId(712),
                action,
            },
            &mut tasks,
        ));
        assert!(matches!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert_ne!(
            started.recv_timeout(Duration::from_secs(2)).unwrap(),
            thread::current().id()
        );
        assert!(
            domain.try_reserve().is_err(),
            "native cleanup still occupies rejection slot"
        );
        assert!(
            events.try_recv().is_err(),
            "refusal cannot precede actual cleanup"
        );
        drop(future);
        assert!(
            domain.try_reserve().is_err(),
            "cancelled acknowledgement cannot release payload"
        );
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(lease) = domain.try_reserve() {
                drop(lease);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "cleanup slot was not returned"
            );
            thread::sleep(Duration::from_millis(1));
        }
        assert!(tasks.is_empty(), "rejection must not spawn Tokio tasks");
        assert!(
            events.try_recv().is_err(),
            "cancelled caller cannot publish a late refusal"
        );
    }
}
