use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, oneshot};

struct Process {
    exited: Arc<AtomicBool>,
    dropped: Option<oneshot::Sender<std::thread::ThreadId>>,
}
impl TerminalProcess for Process {
    fn id(&self) -> u32 {
        7
    }
    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        Ok(self
            .exited
            .load(Ordering::Acquire)
            .then(std::process::ExitStatus::default))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(sender) = self.dropped.take() {
            let _ = sender.send(std::thread::current().id());
        }
    }
}
struct Exit(Arc<AtomicBool>);
impl Drop for Exit {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct Service {
    child: Mutex<Option<Process>>,
    launched: Notify,
    dropped: Mutex<Option<oneshot::Sender<std::thread::ThreadId>>>,
}
impl Service {
    fn launch(&self) -> Box<dyn TerminalProcess> {
        let child = self.child.lock().unwrap().take().unwrap();
        self.launched.notify_one();
        Box::new(child)
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if let Some(sender) = self.dropped.lock().unwrap().take() {
            let _ = sender.send(std::thread::current().id());
        }
    }
}
impl IconService for Service {
    fn load_icon(&self, _: &IconKey) -> Result<Option<IconPixels>, FsError> {
        Ok(None)
    }
}
impl TerminalService for Service {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        Ok(self.launch())
    }
}
impl LocationService for Service {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        Ok(Vec::new())
    }
}
impl PropertyService for Service {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        Ok(true)
    }
}
impl OpenService for Service {
    fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
        Ok(OpenEntryResult::File {
            process: Some(self.launch()),
        })
    }
}
impl EditorService for Service {
    fn default_editor(&self) -> Result<EditorCommand, FsError> {
        unreachable!()
    }
    fn launch_editor(
        &self,
        _: &Path,
        _: &EditorPreferences,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        Ok(self.launch())
    }
}
fn desktop(service: Arc<Service>, tasks: &mut JoinSet<()>) -> Desktop {
    let (sender, _events) = mpsc::channel(16);
    Desktop::testing(
        Some(service),
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        tasks,
    )
}
#[tokio::test(flavor = "current_thread")]
async fn desktop_unpolled_service_tasks_retire_the_last_owner_off_executor() {
    let (dropped, observe) = oneshot::channel();
    let service = Arc::new(Service {
        child: Mutex::new(None),
        launched: Notify::new(),
        dropped: Mutex::new(Some(dropped)),
    });
    let mut tasks = JoinSet::new();
    drop(desktop(service, &mut tasks));
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let on = tokio::time::timeout(Duration::from_secs(5), observe)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        on,
        std::thread::current().id(),
        "final desktop service owner on executor"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn desktop_failed_tracking_preserves_ordinary_process_owners_until_exit() {
    for kind in 0..3 {
        tracking_case(kind, true).await;
    }
}
#[tokio::test(flavor = "current_thread")]
async fn desktop_cancelled_tracking_send_preserves_ordinary_process_owners_until_exit() {
    for kind in 0..3 {
        tracking_case(kind, false).await;
    }
}
async fn tracking_case(kind: usize, close_receiver: bool) {
    let (dropped, mut observe) = oneshot::channel();
    let exited = Arc::new(AtomicBool::new(false));
    let _exit = Exit(exited.clone());
    let service = Arc::new(Service {
        child: Mutex::new(Some(Process {
            exited: exited.clone(),
            dropped: Some(dropped),
        })),
        launched: Notify::new(),
        dropped: Mutex::new(None),
    });
    let mut tasks = JoinSet::new();
    let mut desktop = desktop(service.clone(), &mut tasks);
    let (tracking, receiver) = mpsc::channel(1);
    let receiver = if close_receiver {
        drop(receiver);
        None
    } else {
        let quota = Arc::new(Semaphore::new(1));
        tracking
            .try_send(TrackedChild {
                child: Box::new(Process {
                    exited: Arc::new(AtomicBool::new(true)),
                    dropped: None,
                }),
                _slot: Arc::new(quota.try_acquire_owned().unwrap()),
                completion: None,
            })
            .ok()
            .unwrap();
        Some(receiver)
    };
    desktop.children = tracking;
    match kind {
        0 => desktop.launch(
            DesktopRequestId(1),
            TerminalRequest {
                working_directory: PathBuf::from("fixture"),
                command: String::new(),
                shell: ShellFlavor::Posix,
                mode: TerminalMode::Standard,
            },
            &mut tasks,
        ),
        1 => desktop.open_entry(
            DesktopRequestId(1),
            PaneId::Left,
            NavigationId(1),
            PathBuf::from("fixture"),
            false,
            &mut tasks,
        ),
        _ => {
            desktop
                .edit_file(
                    Some(service.clone()),
                    DesktopRequestId(1),
                    PathBuf::from("fixture"),
                    EditorPreferences::default(),
                    &mut tasks,
                )
                .await
        }
    }
    service.launched.notified().await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    // In the full-channel case the delivered child is owned by the suspended send.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    drop(receiver);
    let quota = if kind == 1 {
        desktop.open_slots.clone()
    } else {
        desktop.terminal_slots.clone()
    };
    let capacity = if kind == 1 {
        MAX_OPEN_HELPERS
    } else {
        MAX_TERMINALS
    };
    assert!(
        observe.try_recv().is_err(),
        "kind {kind}: running child was destroyed"
    );
    assert_eq!(
        quota.available_permits(),
        capacity - 1,
        "kind {kind}: quota returned early"
    );
    exited.store(true, Ordering::Release);
    let on = tokio::time::timeout(Duration::from_secs(5), observe)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(on, std::thread::current().id());
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

#[tokio::test(flavor = "current_thread")]
async fn desktop_opener_completion_keeps_normal_result_and_independent_quota() {
    let (dropped, observe) = oneshot::channel();
    let service = Arc::new(Service {
        child: Mutex::new(Some(Process {
            exited: Arc::new(AtomicBool::new(true)),
            dropped: Some(dropped),
        })),
        launched: Notify::new(),
        dropped: Mutex::new(None),
    });
    let (sender, mut events) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    let desktop = Desktop::testing(
        Some(service),
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    // Exhausting terminal tickets must not consume any opener admission.
    let _terminal_tickets: Vec<_> = (0..MAX_TERMINALS)
        .map(|_| desktop.processes.terminals.try_reserve().unwrap())
        .collect();
    desktop.open_entry(
        DesktopRequestId(97),
        PaneId::Left,
        NavigationId(1),
        PathBuf::from("fixture"),
        false,
        &mut tasks,
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::DesktopActionFinished { request, result } = events.recv().await.unwrap()
                && request == DesktopRequestId(97)
            {
                assert!(matches!(result, Ok(DesktopActionResult::FileOpened)));
                break;
            }
        }
    })
    .await
    .unwrap();
    let on = tokio::time::timeout(Duration::from_secs(5), observe)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(on, std::thread::current().id());
    assert_eq!(desktop.terminal_slots.available_permits(), MAX_TERMINALS);
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}
#[test]
fn desktop_unconsumed_editor_output_survives_runtime_loss() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (dropped, mut observe) = oneshot::channel();
    let exited = Arc::new(AtomicBool::new(false));
    let _exit = Exit(exited.clone());
    let service = Arc::new(Service {
        child: Mutex::new(Some(Process {
            exited: exited.clone(),
            dropped: Some(dropped),
        })),
        launched: Notify::new(),
        dropped: Mutex::new(None),
    });
    let (desktop, tasks) = runtime.block_on(async {
        let mut tasks = JoinSet::new();
        let desktop = desktop(service.clone(), &mut tasks);
        desktop
            .edit_file(
                Some(service),
                DesktopRequestId(1),
                PathBuf::from("fixture"),
                EditorPreferences::default(),
                &mut tasks,
            )
            .await;
        (desktop, tasks)
    });
    // One native worker makes this barrier prove launch's result is complete,
    // while the current-thread executor has never polled its delivery future.
    let (finished, barrier) = std::sync::mpsc::channel();
    let enter = runtime.enter();
    let barrier_task = tokio::task::spawn_blocking(move || {
        finished.send(()).unwrap();
    });
    drop(enter);
    barrier.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(barrier_task);
    let quota = desktop.terminal_slots.clone();
    drop(tasks);
    runtime.shutdown_timeout(Duration::ZERO);
    drop(desktop);
    assert!(
        observe.try_recv().is_err(),
        "unconsumed running child was destroyed"
    );
    assert_eq!(quota.available_permits(), MAX_TERMINALS - 1);
    exited.store(true, Ordering::Release);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let on = loop {
        if let Ok(on) = observe.try_recv() {
            break on;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "child was never retired"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_ne!(on, std::thread::current().id());
}
