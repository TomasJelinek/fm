use super::*;
use crate::{task_lifetime::TaskLifetime, user_command_runtime::ProcessTracker};
use midnight_core::{TerminalProcess, TerminalRequest, TerminalService, UserCommandService};
use std::{
    ffi::OsString,
    sync::{Mutex, atomic::AtomicBool},
    time::Duration,
};

struct Native {
    entered: std::sync::mpsc::Sender<()>,
    proceed: Mutex<std::sync::mpsc::Receiver<()>>,
    exited: Arc<AtomicBool>,
    child_dropped: std::sync::mpsc::Sender<std::thread::ThreadId>,
    id_panics: bool,
    captures_dropped: Arc<AtomicUsize>,
    seen_arguments: Mutex<Vec<OsString>>,
}
impl TerminalService for Native {
    fn shell_flavor(&self) -> midnight_core::ShellFlavor {
        midnight_core::ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("comparator must never use terminal launch")
    }
}
impl UserCommandService for Native {
    fn launch_user_program(
        &self,
        _: &Path,
        arguments: &[OsString],
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        assert_eq!(
            self.captures_dropped.load(Ordering::SeqCst),
            2,
            "both captures must confirm release before native launch"
        );
        *self.seen_arguments.lock().unwrap() = arguments.to_vec();
        self.entered.send(()).unwrap();
        self.proceed
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        Ok(Box::new(Child {
            exited: self.exited.clone(),
            dropped: self.child_dropped.clone(),
            id_panics: self.id_panics,
        }))
    }
}
struct Child {
    exited: Arc<AtomicBool>,
    dropped: std::sync::mpsc::Sender<std::thread::ThreadId>,
    id_panics: bool,
}
impl TerminalProcess for Child {
    fn id(&self) -> u32 {
        assert!(!self.id_panics, "child id panic");
        4711
    }
    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        Ok(self
            .exited
            .load(Ordering::Acquire)
            .then(std::process::ExitStatus::default))
    }
}
impl Drop for Child {
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

fn armed(
    fixture: &Fixture,
    cancel: Cancellation,
) -> (
    ArmedExternalComparison,
    std::sync::mpsc::Receiver<ExternalCompareOutcome>,
) {
    let paths = paths();
    let (reply, receiver) = std::sync::mpsc::sync_channel(1);
    let request = fixture
        .front
        .prepare(
            process(&paths),
            paths,
            &fixture.capture([Cleanup::Success; 2]),
            Arc::new(Session(fixture.sessions.clone())),
            cancel,
            reply,
        )
        .unwrap();
    (
        fixture
            .front
            .classify(request)
            .unwrap_or_else(|_| panic!("origin")),
        receiver,
    )
}

#[test]
fn native_child_return_across_cancel_receiver_loss_and_id_panic_keeps_actual_process_credit() {
    for id_panics in [false, true] {
        for receiver_lost in [false, true] {
            let fixture = Fixture::new();
            let cancel = Cancellation::default();
            let (armed, reply) = armed(&fixture, cancel.clone());
            let (entered, entered_rx) = std::sync::mpsc::channel();
            let (proceed, proceed_rx) = std::sync::mpsc::channel();
            let (dropped, dropped_rx) = std::sync::mpsc::channel();
            let exited = Arc::new(AtomicBool::new(false));
            let _exit = ExitOnDrop(exited.clone());
            let native = Arc::new(Native {
                entered,
                proceed: Mutex::new(proceed_rx),
                exited: exited.clone(),
                child_dropped: dropped,
                id_panics,
                captures_dropped: fixture.drops.clone(),
                seen_arguments: Mutex::default(),
            });
            let quota = Arc::new(Semaphore::new(1));
            let process = Arc::new(quota.clone().try_acquire_owned().unwrap());
            let tracker = ProcessTracker::try_start().unwrap();
            let ticket = tracker.try_reserve().unwrap();
            let (children, reaper) = tokio::sync::mpsc::channel(1);
            drop(reaper);
            let launch = NativeLaunch {
                service: native.clone(),
                ticket,
                quota: process,
                scope: TaskLifetime::testing(native.clone()),
                children,
            };
            let worker = std::thread::spawn(move || launch_on_worker(armed, launch));
            entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            cancel.cancel();
            let reply = (!receiver_lost).then_some(reply);
            proceed.send(()).unwrap();
            worker.join().unwrap();
            if let Some(reply) = reply {
                let result = reply.recv_timeout(Duration::from_secs(3)).unwrap();
                if id_panics {
                    assert!(matches!(result, ExternalCompareOutcome::Unknown { .. }));
                } else {
                    assert!(matches!(
                        result,
                        ExternalCompareOutcome::Started {
                            pid: 4711,
                            tracking_warning: Some(_),
                        }
                    ));
                }
            }
            assert_eq!(
                *native.seen_arguments.lock().unwrap(),
                paths().map(PathBuf::into_os_string)
            );
            assert_eq!(quota.available_permits(), 0);
            assert!(dropped_rx.try_recv().is_err());
            assert_eq!(fixture.front.open_budget().available_permits(), 4);
            assert_eq!(fixture.sessions.load(Ordering::SeqCst), 1);
            drop(tracker);
            exited.store(true, Ordering::Release);
            assert_ne!(
                dropped_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
                std::thread::current().id()
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while quota.available_permits() == 0 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    }
}

#[tokio::test]
async fn busy_process_admission_discards_pair_without_native_spawn() {
    let fixture = Fixture::new();
    let (request, reply) = armed(&fixture, Cancellation::default());
    let (entered, entered_rx) = std::sync::mpsc::channel();
    let (_proceed, proceed_rx) = std::sync::mpsc::channel();
    let (dropped, _dropped_rx) = std::sync::mpsc::channel();
    let native = Arc::new(Native {
        entered,
        proceed: Mutex::new(proceed_rx),
        exited: Arc::new(AtomicBool::new(true)),
        child_dropped: dropped,
        id_panics: false,
        captures_dropped: fixture.drops.clone(),
        seen_arguments: Mutex::default(),
    });
    let (children, _reaper) = tokio::sync::mpsc::channel(1);
    let context = LaunchContext {
        scope: TaskLifetime::testing(native.clone()),
        processes: Arc::new(Semaphore::new(0)),
        tracker: ProcessTracker::try_start().unwrap(),
        children,
    };
    let mut tasks = tokio::task::JoinSet::new();
    launch(context, Some(native), request, &mut tasks);
    while tasks.join_next().await.is_some() {}
    assert!(matches!(
        reply.try_recv().unwrap(),
        ExternalCompareOutcome::NotStarted(_)
    ));
    assert!(entered_rx.try_recv().is_err());
    cleanup(&fixture, 2).await;
}

#[test]
fn dropping_uninvoked_native_closure_retires_pair_without_launch_or_runtime() {
    let fixture = Fixture::new();
    let (request, reply) = armed(&fixture, Cancellation::default());
    let scope = TaskLifetime::testing(());
    let closure = scope.blocking(move || drop(request));
    drop(closure);
    assert!(matches!(
        reply.recv_timeout(Duration::from_secs(3)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while fixture.front.open_budget().available_permits() != 4 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(fixture.releases.load(Ordering::SeqCst), 2);
}

#[test]
fn runtime_shutdown_during_native_launch_keeps_child_until_confirmed_exit() {
    let fixture = Fixture::new();
    let (request, reply) = armed(&fixture, Cancellation::default());
    let (entered, entered_rx) = std::sync::mpsc::channel();
    let (proceed, proceed_rx) = std::sync::mpsc::channel();
    let (dropped, dropped_rx) = std::sync::mpsc::channel();
    let exited = Arc::new(AtomicBool::new(false));
    let _exit = ExitOnDrop(exited.clone());
    let native = Arc::new(Native {
        entered,
        proceed: Mutex::new(proceed_rx),
        exited: exited.clone(),
        child_dropped: dropped,
        id_panics: false,
        captures_dropped: fixture.drops.clone(),
        seen_arguments: Mutex::default(),
    });
    let processes = Arc::new(Semaphore::new(8));
    let (children, reaper) = tokio::sync::mpsc::channel(8);
    drop(reaper);
    let context = LaunchContext {
        scope: TaskLifetime::testing(native.clone()),
        processes: processes.clone(),
        tracker: ProcessTracker::try_start().unwrap(),
        children,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut tasks = tokio::task::JoinSet::new();
        launch(context, Some(native), request, &mut tasks);
        // The dropped async observer does not cancel or own native process work.
    });
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    runtime.shutdown_timeout(Duration::from_millis(10));
    assert_eq!(processes.available_permits(), 7);
    proceed.send(()).unwrap();
    assert!(matches!(
        reply.recv_timeout(Duration::from_secs(3)).unwrap(),
        ExternalCompareOutcome::Started { pid: 4711, .. }
    ));
    assert!(dropped_rx.try_recv().is_err());
    assert_eq!(processes.available_permits(), 7);
    exited.store(true, Ordering::Release);
    assert_ne!(
        dropped_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        std::thread::current().id()
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while processes.available_permits() != 8 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn unconfirmed_final_release_never_reaches_native_launcher_and_reports_without_waiting() {
    for side in 0..2 {
        for mode in [Cleanup::Fail, Cleanup::Panic, Cleanup::Unknown] {
            let fixture = Fixture::new();
            let paths = paths();
            let mut modes = [Cleanup::Success; 2];
            modes[side] = mode;
            let (reply, outcome) = std::sync::mpsc::sync_channel(1);
            let request = fixture
                .front
                .prepare(
                    process(&paths),
                    paths,
                    &fixture.capture(modes),
                    Arc::new(Session(fixture.sessions.clone())),
                    Cancellation::default(),
                    reply,
                )
                .unwrap();
            let request = fixture
                .front
                .classify(request)
                .unwrap_or_else(|_| panic!("origin"));
            let (entered, entered_rx) = std::sync::mpsc::channel();
            let (_proceed, proceed_rx) = std::sync::mpsc::channel();
            let (dropped, _dropped_rx) = std::sync::mpsc::channel();
            let native = Arc::new(Native {
                entered,
                proceed: Mutex::new(proceed_rx),
                exited: Arc::new(AtomicBool::new(true)),
                child_dropped: dropped,
                id_panics: false,
                captures_dropped: fixture.drops.clone(),
                seen_arguments: Mutex::default(),
            });
            let quota = Arc::new(Semaphore::new(1));
            let (children, _reaper) = tokio::sync::mpsc::channel(1);
            launch_on_worker(
                request,
                NativeLaunch {
                    scope: TaskLifetime::testing(native.clone()),
                    service: native,
                    ticket: ProcessTracker::try_start().unwrap().try_reserve().unwrap(),
                    quota: Arc::new(quota.clone().try_acquire_owned().unwrap()),
                    children,
                },
            );
            assert!(matches!(
                outcome.try_recv().unwrap(),
                ExternalCompareOutcome::NotStarted(_)
            ));
            assert!(entered_rx.try_recv().is_err());
            assert_eq!(quota.available_permits(), 1);
            assert_eq!(fixture.front.open_budget().available_permits(), 3);
            assert_eq!(fixture.sessions.load(Ordering::SeqCst), 0);
            assert_eq!(fixture.reads.load(Ordering::SeqCst), 0);
        }
    }
}
