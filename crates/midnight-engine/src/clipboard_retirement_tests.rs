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
    let controller = Controller::testing(Some(Arc::new(IdleService(sent))), outbox);
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
    let mut controller = Controller::testing(
        Some(Arc::new(PumpService {
            opened,
            copied,
            closed,
        })),
        outbox,
    );
    let cancel = controller.cancel.clone();
    let mut tasks = JoinSet::new();
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
}

#[derive(Debug)]
struct Capture(channel::Sender<ThreadId>);
impl midnight_core::CapturedClipboardFiles for Capture {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        panic!("unstarted clipboard must not resolve a captured source")
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}
#[test]
fn clipboard_retirement_cancelled_unstarted_worker_retains_queued_capture() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (entered, started) = channel::channel();
    let (release, gate) = channel::channel();
    runtime.spawn_blocking(move || {
        let _ = entered.send(());
        let _ = gate.recv();
    });
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    let (sent, dropped) = channel::channel();
    let (outbox, _events) = outbox();
    let mut controller = Controller::testing(None, outbox);
    let mut tasks = JoinSet::new();
    runtime.block_on(controller.copy_captured(
        DesktopRequestId(711),
        Arc::new(Capture(sent)),
        Cancellation::default(),
        &mut tasks,
    ));
    tasks.abort_all();
    controller.shutdown();
    runtime.shutdown_timeout(Duration::from_millis(10));
    let _ = release.send(());
    drop(tasks);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(2)).unwrap(),
        thread::current().id()
    );
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
        let mut controller = Controller::new(
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
        let mut tasks = JoinSet::new();
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
