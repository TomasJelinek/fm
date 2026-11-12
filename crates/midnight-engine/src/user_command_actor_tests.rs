use super::*;
use std::sync::Condvar;

fn request(id: u64, directory: &Path) -> UserCommandRequest {
    let command = UserCommand {
        id: UserCommandId::new("test").unwrap(),
        name: "Test".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec![],
        },
    };
    UserCommandRequest {
        id: UserCommandRequestId(id),
        command: command.id.clone(),
        configuration: Arc::new(vec![command]),
        context: UserCommandContext {
            active: Some(directory.to_owned()),
            other: None,
            files: None,
            focus: None,
        },
    }
}

#[tokio::test]
async fn full_queue_rejects_visibly_and_consumes_the_request_id_without_losing_original_cancel() {
    let (input, mut queued) = mpsc::channel(1);
    let controller = Controller {
        input,
        retirement: RetirementDomain::try_start(MAX_JOBS, 0).unwrap(),
        state: Arc::default(),
        stop: Cancellation::default(),
        available: true,
        task: None,
    };
    let directory = std::env::current_dir().unwrap();
    controller.start(request(1, &directory), false).unwrap();
    assert_eq!(
        controller
            .start(request(2, &directory), false)
            .unwrap_err()
            .kind,
        io::ErrorKind::WouldBlock
    );
    controller.cancel(UserCommandRequestId(1));
    assert!(queued.recv().await.unwrap().cancel.is_cancelled());
    assert!(controller.start(request(2, &directory), false).is_err());
    assert_eq!(controller.state.lock().unwrap().requests.len(), 1);
}

#[tokio::test]
async fn retaining_a_preview_row_keeps_its_quota_after_the_parent_preview_is_closed() {
    let quota = Arc::new(Semaphore::new(1));
    let retention = UserCommandRetention::new(quota.clone().acquire_owned().await.unwrap());
    let row = Arc::new(UserCommandPreviewRow {
        process: UserCommandProcess {
            working_directory: std::env::current_dir().unwrap(),
            invocation: UserCommandProcessInvocation::Program {
                executable: std::env::current_exe().unwrap(),
                arguments: vec![],
            },
        },
        retention: retention.clone(),
    });
    let preview = UserCommandPreview {
        processes: vec![row].into(),
        retention,
    };
    let kept = preview.processes[0].clone();
    drop(preview);
    assert_eq!(quota.available_permits(), 0);
    drop(kept);
    assert_eq!(quota.available_permits(), 1);
}

#[tokio::test]
async fn successful_preview_is_terminal_and_removes_active_cancellation_state() {
    let barrier = Arc::new(Barrier {
        entered: tokio::sync::Notify::new(),
        released: Mutex::new(true),
        wake: Condvar::new(),
    });
    let (children, _reaper) = mpsc::channel(1);
    let runtime = Runtime::new(
        Arc::new(Semaphore::new(1)),
        Arc::new(Semaphore::new(1)),
        children,
        crate::user_command_runtime::ProcessTracker::try_start().unwrap(),
    );
    let (sender, mut events) = mpsc::channel(16);
    let controller = Controller::new(
        Retirement::testing(),
        Some(Arc::new(Native(barrier))),
        Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
        runtime,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    let directory = tempfile::tempdir().unwrap();
    controller
        .start(request(1, directory.path()), true)
        .unwrap();
    let preview = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Event::UserCommand(event)) = events.recv().await
                && let UserCommandUpdate::Preview(preview) = event.update
            {
                break preview;
            }
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !controller.state.lock().unwrap().requests.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        events.try_recv().is_err(),
        "Successful preview emits no extra Finished event."
    );
    assert_eq!(preview.processes.len(), 1);
    drop(preview);
    controller.shutdown().await;
}

struct Barrier {
    entered: tokio::sync::Notify,
    released: Mutex<bool>,
    wake: Condvar,
}
impl Barrier {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
struct ReleaseOnDrop(Arc<Barrier>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}
struct Native(Arc<Barrier>);
impl TerminalService for Native {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("unexpected terminal")
    }
}
impl UserCommandService for Native {
    fn launch_user_program(
        &self,
        _: &Path,
        _: &[std::ffi::OsString],
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        self.0.entered.notify_one();
        let mut released = self.0.released.lock().unwrap();
        while !*released {
            released = self.0.wake.wait(released).unwrap();
        }
        Err(cancelled())
    }
}

#[tokio::test]
async fn cancellation_and_shutdown_keep_native_launch_and_process_permits_until_the_call_returns() {
    let barrier = Arc::new(Barrier {
        entered: tokio::sync::Notify::new(),
        released: Mutex::new(false),
        wake: Condvar::new(),
    });
    let _release = ReleaseOnDrop(barrier.clone());
    let launches = Arc::new(Semaphore::new(1));
    let processes = Arc::new(Semaphore::new(1));
    let (children, _reaper) = mpsc::channel(1);
    let runtime = Runtime::new(
        launches.clone(),
        processes.clone(),
        children,
        crate::user_command_runtime::ProcessTracker::try_start().unwrap(),
    );
    let (sender, _events) = mpsc::channel(16);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let controller = Controller::new(
        Retirement::testing(),
        Some(Arc::new(Native(barrier.clone()))),
        Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
        runtime,
        outbox,
    );
    let directory = tempfile::tempdir().unwrap();
    controller
        .start(request(1, directory.path()), false)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), barrier.entered.notified())
        .await
        .unwrap();
    controller.cancel(UserCommandRequestId(1));
    assert_eq!(launches.available_permits(), 0);
    assert_eq!(processes.available_permits(), 0);
    let shutdown = tokio::spawn(controller.shutdown());
    tokio::task::yield_now().await;
    assert!(!shutdown.is_finished());
    assert_eq!(processes.available_permits(), 0);
    barrier.release();
    tokio::time::timeout(Duration::from_secs(5), shutdown)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(launches.available_permits(), 1);
    assert_eq!(processes.available_permits(), 1);
}
#[test]
fn separator_preview_and_execution_never_enter_the_worker_queue() {
    let (input, mut queued) = mpsc::channel(1);
    let controller = Controller {
        input,
        retirement: RetirementDomain::try_start(MAX_JOBS, 0).unwrap(),
        state: Arc::default(),
        stop: Cancellation::default(),
        available: true,
        task: None,
    };
    for (id, preview) in [(1, true), (2, false)] {
        let mut request = request(id, &std::env::current_dir().unwrap());
        let command = &mut Arc::make_mut(&mut request.configuration)[0];
        command.invocation = UserCommandInvocation::Separator;
        command.working_directory.clear();
        let error = controller.start(request, preview).unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("Separators"));
        assert!(queued.try_recv().is_err());
        assert!(controller.state.lock().unwrap().requests.is_empty());
    }
}

#[test]
fn separator_direct_preparation_refuses_before_native_capture_for_both_intents() {
    struct NeverCapture;
    impl TransferService for NeverCapture {
        fn prepare_transfer_source(
            &self,
            _: &Path,
            _: &Cancellation,
        ) -> Result<Box<dyn TransferSource>, FsError> {
            panic!("separator captured a source")
        }
        fn prepare_transfer_destination(
            &self,
            _: &Path,
            _: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            panic!("separator captured a directory")
        }
    }
    let mut request = request(1, &std::env::current_dir().unwrap());
    let command = &mut Arc::make_mut(&mut request.configuration)[0];
    command.invocation = UserCommandInvocation::Separator;
    command.launch = UserCommandLaunch::PerItem;
    // A leftover valid template would otherwise trigger native directory capture.
    for prepare in [
        user_command_prepare::prepare,
        user_command_prepare::prepare_preview,
    ] {
        let error = match prepare(&request, &NeverCapture, &Cancellation::default()) {
            Ok(_) => panic!("separator produced a native process plan"),
            Err(error) => error,
        };
        assert_eq!(error.kind, io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("Separators"));
    }
}
