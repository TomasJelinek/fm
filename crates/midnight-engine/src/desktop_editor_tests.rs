use super::*;
use std::sync::Mutex;

#[derive(Debug)]
struct Source(Mutex<Option<tokio::sync::oneshot::Sender<std::thread::ThreadId>>>);
impl CapturedEditorFile for Source {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        }
    }
    fn row(&self) -> TreeRowId {
        TreeRowId(1)
    }
    fn resolve(&self, _: &Cancellation) -> Result<PathBuf, FsError> {
        Err(FsError::from_io(
            "cancelled editor fixture",
            Path::new(""),
            io::ErrorKind::Interrupted.into(),
        ))
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        if let Some(sender) = self.0.lock().unwrap().take() {
            let _ = sender.send(std::thread::current().id());
        }
    }
}
struct Editor;
impl EditorService for Editor {
    fn default_editor(&self) -> Result<EditorCommand, FsError> {
        unreachable!()
    }
    fn launch_editor(
        &self,
        _: &Path,
        _: &EditorPreferences,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("failed source resolution must not spawn an editor")
    }
}

#[tokio::test(flavor = "current_thread")]
async fn editor_capture_abort_before_delivery_task_first_poll_retires_on_worker() {
    let (sender, _events) = mpsc::channel(8);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let mut tasks = JoinSet::new();
    let desktop = Desktop::testing(None, None, outbox, &mut tasks);
    let (dropped, observe_drop) = tokio::sync::oneshot::channel();
    desktop
        .edit_captured_file(
            Some(Arc::new(Editor)),
            DesktopRequestId(1),
            Arc::new(Source(Mutex::new(Some(dropped)))),
            EditorPreferences::default(),
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    // This current-thread executor cannot poll a newly spawned async task before
    // the synchronous abort. The source must already belong to a blocking worker.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let retired_on = tokio::time::timeout(Duration::from_secs(5), observe_drop)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        retired_on,
        std::thread::current().id(),
        "provider was dropped by the async/UI executor"
    );
}

#[derive(Debug)]
struct GatedSource {
    source: Source,
    entered: Option<std::sync::mpsc::Sender<std::thread::ThreadId>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    panic: bool,
}
impl CapturedEditorFile for GatedSource {
    fn stamp(&self) -> TreeViewStamp {
        self.source.stamp()
    }
    fn row(&self) -> TreeRowId {
        self.source.row()
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        self.source.resolve(cancel)
    }
}
impl Drop for GatedSource {
    fn drop(&mut self) {
        if let Some(entered) = &self.entered {
            let _ = entered.send(std::thread::current().id());
        }
        if let Some(release) = self.release.lock().unwrap().as_ref() {
            let _ = release.recv_timeout(Duration::from_secs(5));
        }
        assert!(!self.panic, "injected captured editor destruction failure");
    }
}
#[tokio::test(flavor = "current_thread")]
async fn editor_capture_cleanup_holds_lease_and_launch_permits_through_actual_drop() {
    let (sender, _events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let mut desktop = Desktop::testing(
        None,
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let domain =
        crate::retirement::RetirementDomain::try_start(EDITOR_RETIREMENT_CAPACITY, 1).unwrap();
    desktop.editor_intake = tokio::sync::Mutex::new(domain.new_waiter().unwrap());
    let (entered, observe_entry) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (dropped, observe_drop) = tokio::sync::oneshot::channel();
    desktop
        .edit_captured_file(
            Some(Arc::new(Editor)),
            DesktopRequestId(1),
            Arc::new(GatedSource {
                source: Source(Mutex::new(Some(dropped))),
                entered: Some(entered),
                release: Mutex::new(Some(wait)),
                panic: false,
            }),
            EditorPreferences::default(),
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    let on = observe_entry.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(on, std::thread::current().id());
    let leases: Vec<_> = (0..EDITOR_RETIREMENT_CAPACITY - 1)
        .map(|_| domain.try_reserve().unwrap())
        .collect();
    assert!(
        domain.try_reserve().is_err(),
        "capture cleanup released its lease early"
    );
    assert_eq!(
        desktop.open_budget.available_permits(),
        MAX_OPEN_LAUNCHES - 1
    );
    assert_eq!(
        desktop.terminal_slots.available_permits(),
        MAX_TERMINALS - 1
    );
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    assert!(
        domain.try_reserve().is_err(),
        "aborted delivery released the native owner"
    );
    release.send(()).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), observe_drop)
            .await
            .unwrap()
            .unwrap(),
        on
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if desktop.open_budget.available_permits() == MAX_OPEN_LAUNCHES
                && desktop.terminal_slots.available_permits() == MAX_TERMINALS
                && let Ok(lease) = domain.try_reserve()
            {
                drop(lease);
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(desktop.open_budget.available_permits(), MAX_OPEN_LAUNCHES);
    assert_eq!(desktop.terminal_slots.available_permits(), MAX_TERMINALS);
    drop(leases);
}
#[tokio::test(flavor = "current_thread")]
async fn rejected_editor_reports_capture_destruction_failure() {
    let (sender, mut events) = mpsc::channel(128);
    let mut tasks = JoinSet::new();
    let desktop = Desktop::testing(
        None,
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    desktop
        .edit_captured_file(
            None,
            DesktopRequestId(29),
            Arc::new(GatedSource {
                source: Source(Mutex::new(None)),
                entered: None,
                release: Mutex::new(None),
                panic: true,
            }),
            EditorPreferences::default(),
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::DesktopActionFinished { request, result } = events.recv().await.unwrap()
                && request == DesktopRequestId(29)
            {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("Editor cleanup failed")
                );
                break;
            }
        }
    })
    .await
    .unwrap();
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

#[test]
fn cancelled_queued_editor_work_keeps_capture_guard_without_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (started, started_rx) = std::sync::mpsc::channel();
    let (release_worker, wait_worker) = std::sync::mpsc::channel();
    let enter = runtime.enter();
    let blocker = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        let _ = wait_worker.recv_timeout(Duration::from_secs(5));
    });
    drop(enter);
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (dropped, mut observe_drop) = tokio::sync::oneshot::channel();
    let (desktop, tasks) = runtime.block_on(async {
        let (sender, _events) = mpsc::channel(8);
        let mut tasks = JoinSet::new();
        let desktop = Desktop::testing(
            None,
            None,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        desktop
            .edit_captured_file(
                Some(Arc::new(Editor)),
                DesktopRequestId(1),
                Arc::new(Source(Mutex::new(Some(dropped)))),
                EditorPreferences::default(),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
        (desktop, tasks)
    });
    let launches = desktop.open_budget.clone();
    assert_eq!(launches.available_permits(), MAX_OPEN_LAUNCHES - 1);
    drop(tasks);
    runtime.shutdown_timeout(Duration::ZERO);
    drop(desktop);
    // Runtime is gone; the capture must still retire without a replacement Tokio task.
    release_worker.send(()).unwrap();
    drop(blocker);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let on = loop {
        if let Ok(on) = observe_drop.try_recv() {
            break on;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "queued capture was not retired"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_ne!(on, std::thread::current().id());
    while launches.available_permits() != MAX_OPEN_LAUNCHES {
        assert!(
            std::time::Instant::now() < deadline,
            "launch permit was not returned"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
