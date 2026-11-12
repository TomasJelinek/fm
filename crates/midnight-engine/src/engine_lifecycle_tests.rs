//! Last-owner checks at the public engine lifetime boundary.
use super::*;
use midnight_core::*;
use std::{path::Path, thread::ThreadId, time::Duration};

type Notice = std::sync::mpsc::Sender<ThreadId>;
struct Files(Option<Notice>);
impl Drop for Files {
    fn drop(&mut self) {
        if let Some(notice) = &self.0 {
            let _ = notice.send(std::thread::current().id());
        }
    }
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!("lifetime fixture does not inspect files")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!("lifetime fixture does not inspect storage")
    }
}

#[derive(Debug)]
struct Sources {
    notice: Notice,
    visited: Arc<std::sync::atomic::AtomicUsize>,
}
impl Drop for Sources {
    fn drop(&mut self) {
        let _ = self.notice.send(std::thread::current().id());
    }
}
impl CapturedDeleteSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(1)
    }
    fn mode(&self) -> DeleteMode {
        DeleteMode::Permanent
    }
    fn root_count(&self) -> usize {
        self.visited
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        1
    }
    fn prepare(
        &self,
        _: &dyn MutationService,
        _: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        unreachable!("unpolled engine must never prepare mutations")
    }
}

fn off_caller(receiver: std::sync::mpsc::Receiver<ThreadId>) {
    let retired = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_ne!(
        retired,
        std::thread::current().id(),
        "provider owner dropped on caller"
    );
}

#[test]
fn unpolled_engine_retires_queued_capture_without_tokio() {
    let (notice, retired) = std::sync::mpsc::channel();
    let (commands, input) = mpsc::channel(2);
    let (events, _receiver) = mpsc::channel(1);
    commands
        .try_send(Command::PrepareCapturedDelete(PrepareCapturedDelete {
            id: OperationId(1),
            sources: Arc::new(Sources {
                notice,
                visited: Arc::default(),
            }),
            source_directory: PathBuf::from("display-only"),
        }))
        .unwrap();
    let run = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(Files(None)),
        events,
    )
    .run(input);
    drop(run);
    off_caller(retired);
}

#[test]
fn unstarted_engine_retires_its_provider_without_tokio() {
    let (notice, retired) = std::sync::mpsc::channel();
    let (events, _receiver) = mpsc::channel(1);
    let engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        Arc::new(Files(Some(notice))),
        events,
    );
    drop(engine);
    off_caller(retired);
}

fn capture(id: u64, notice: Notice, visited: Arc<std::sync::atomic::AtomicUsize>) -> Command {
    Command::PrepareCapturedDelete(PrepareCapturedDelete {
        id: OperationId(id),
        sources: Arc::new(Sources { notice, visited }),
        source_directory: PathBuf::from("display-only"),
    })
}

fn pending_dispatch() -> (
    std::pin::Pin<Box<EngineRun>>,
    std::sync::mpsc::Receiver<ThreadId>,
    mpsc::Receiver<Event>,
) {
    use std::{
        future::Future,
        sync::atomic::Ordering,
        task::{Context, Waker},
    };
    let (notice, retired) = std::sync::mpsc::channel();
    let visited = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (commands, input) = mpsc::channel(2);
    commands
        .try_send(capture(1, notice.clone(), visited.clone()))
        .unwrap();
    commands
        .try_send(capture(2, notice, visited.clone()))
        .unwrap();
    let (events, receiver) = mpsc::channel(1);
    events
        .try_send(Event::PreferencesLoaded(Ok(Default::default())))
        .unwrap();
    let engine = Engine::new(Engine::lifetime().unwrap(), Arc::new(Files(None)), events);
    let mut run = Box::pin(engine.run(input));
    assert!(
        run.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(
        visited.load(Ordering::SeqCst),
        1,
        "one captured command reached dispatch"
    );
    assert!(retired.try_recv().is_err());
    (run, retired, receiver)
}

fn both_captures_retired(retired: std::sync::mpsc::Receiver<ThreadId>) {
    for _ in 0..2 {
        assert_ne!(
            retired.recv_timeout(Duration::from_secs(2)).unwrap(),
            std::thread::current().id()
        );
    }
    assert!(retired.try_recv().is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_dispatch_pending_on_full_outbox_retires_consumed_and_queued_captures() {
    let (run, retired, _events) = pending_dispatch();
    drop(run);
    both_captures_retired(retired);
}

#[test]
fn runtime_shutdown_during_pending_dispatch_keeps_both_captures_off_caller() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (run, retired, _events) = {
        let _entered = runtime.enter();
        pending_dispatch()
    };
    // Match the application deadline: active native lanes remain owned by the run.
    runtime.shutdown_timeout(Duration::from_millis(10));
    drop(run);
    both_captures_retired(retired);
}

#[test]
fn stopped_runtime_does_not_control_unpolled_retirement() {
    let (notice, retired) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let run = {
        let _entered = runtime.enter();
        let (commands, input) = mpsc::channel(1);
        commands
            .try_send(capture(1, notice, Arc::default()))
            .unwrap();
        let (events, _receiver) = mpsc::channel(1);
        Engine::new(Engine::lifetime().unwrap(), Arc::new(Files(None)), events).run(input)
    };
    drop(runtime);
    drop(run);
    off_caller(retired);
}

#[tokio::test(flavor = "current_thread")]
async fn normal_shutdown_acknowledges_queued_capture_retirement() {
    let (notice, retired) = std::sync::mpsc::channel();
    let (commands, input) = mpsc::channel(2);
    commands.try_send(Command::Shutdown).unwrap();
    commands
        .try_send(capture(1, notice, Arc::default()))
        .unwrap();
    let (events, _receiver) = mpsc::channel(1);
    let engine = Engine::new(Engine::lifetime().unwrap(), Arc::new(Files(None)), events);
    tokio::time::timeout(Duration::from_secs(3), engine.run(input))
        .await
        .unwrap();
    off_caller(retired);
    assert!(commands.is_closed());
}

#[test]
fn receiver_retirement_keeps_previously_reserved_sends_on_cleanup_worker() {
    use std::future::Future;
    use std::task::{Context, Waker};
    let domain = retirement::RetirementDomain::try_start(1, 0).unwrap();
    let (commands, mut input) = mpsc::channel(1);
    let permit = commands.clone().try_reserve_owned().unwrap();
    let (notice, retired) = std::sync::mpsc::channel();
    input.close();
    let mut ack = Box::pin(
        domain
            .try_reserve()
            .unwrap()
            .retire(engine_commands::CommandInbox(input)),
    );
    assert!(
        ack.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(
        domain.try_reserve().is_err(),
        "late send still owns reserved capacity"
    );
    let sender = permit.send(capture(1, notice, Arc::default()));
    off_caller(retired);
    drop(sender);
}

#[test]
fn interrupted_tree_shutdown_retires_queued_capture_without_tokio_blocking_capacity() {
    use std::{
        future::Future,
        task::{Context, Waker},
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, entered) = std::sync::mpsc::channel();
    let blocker = runtime.spawn_blocking(move || {
        started.send(()).unwrap();
        let _ = gate.recv();
    });
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    let (notice, retired) = std::sync::mpsc::channel();
    let (commands, input) = mpsc::channel(2);
    commands.try_send(Command::Shutdown).unwrap();
    commands
        .try_send(capture(1, notice, Arc::default()))
        .unwrap();
    let (events, _receiver) = mpsc::channel(1);
    let mut run = Box::pin(
        Engine::new(Engine::lifetime().unwrap(), Arc::new(Files(None)), events).run(input),
    );
    {
        let _entered = runtime.enter();
        assert!(
            run.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    // Close precedes tree shutdown. Both tree workers are queued behind the gated
    // blocking task, so their joins cannot complete and normal command retirement
    // has not been reached. Cancelling this exact await must still own the inbox.
    assert!(commands.is_closed());
    assert!(retired.try_recv().is_err());
    drop(run);
    off_caller(retired);
    release.send(()).unwrap();
    runtime.block_on(blocker).unwrap();
}
