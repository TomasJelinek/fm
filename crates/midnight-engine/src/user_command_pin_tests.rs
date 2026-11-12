//! Native pin ownership while a prepared command waits for process admission.
use super::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    sync::mpsc as channel,
    task::{Context as TaskContext, Poll, Wake, Waker},
    thread::ThreadId,
};
struct Gate {
    open: Mutex<bool>,
    wake: std::sync::Condvar,
}
impl Gate {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.wake.notify_all();
    }
    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.wake.wait(open).unwrap();
        }
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
struct Files {
    gate: Arc<Gate>,
    dropped: channel::Sender<ThreadId>,
}
impl TransferService for Files {
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("this command does not read files")
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            gate: self.gate.clone(),
            dropped: self.dropped.clone(),
        }))
    }
}
struct Directory {
    path: PathBuf,
    gate: Arc<Gate>,
    dropped: channel::Sender<ThreadId>,
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        TransferIdentity {
            device: 1,
            object: [2; 16],
        }
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        self.gate.wait();
        Ok(())
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        panic!("unexpected inspection")
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        panic!("unexpected transfer")
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("unexpected mutation")
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("unexpected stage")
    }
}
struct Native;
impl TerminalService for Native {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("no process permit is available")
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
        panic!("no process permit is available")
    }
}
struct WakeUp(channel::Sender<()>);
impl Wake for WakeUp {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
    fn wake_by_ref(self: &Arc<Self>) {
        let _ = self.0.send(());
    }
}
#[test]
fn dropping_prepared_command_awaiting_process_admission_retires_native_pins_off_caller() {
    let caller = std::thread::current().id();
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (dropped, observed) = channel::channel();
    let (children, _reaper) = mpsc::channel(1);
    let (sender, _events) = mpsc::channel(8);
    let gate = Arc::new(Gate {
        open: Mutex::new(false),
        wake: std::sync::Condvar::new(),
    });
    let _release = Release(gate.clone());
    let retirement = Retirement::testing();
    let native: Arc<dyn UserCommandService> = Arc::new(Native);
    let files: Arc<dyn TransferService> = Arc::new(Files {
        dropped,
        gate: gate.clone(),
    });
    let services = Arc::new(retirement.service.own(Services {
        native: Some(native.clone()),
        files: Some(files.clone()),
    }));
    let context = Arc::new(Context {
        native,
        files,
        services,
        runtime: Runtime::new(
            Arc::new(Semaphore::new(1)),
            Arc::new(Semaphore::new(0)),
            children,
            crate::user_command_runtime::ProcessTracker::try_start().unwrap(),
        ),
        outbox: Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        stop: Cancellation::default(),
    });
    let directory = std::env::current_dir().unwrap();
    let configuration = UserCommand {
        id: UserCommandId::new("lifetime").unwrap(),
        name: "Lifetime".into(),
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
    let work = Work {
        request: UserCommandRequest {
            id: UserCommandRequestId(1),
            command: configuration.id.clone(),
            configuration: Arc::new(vec![configuration]),
            context: UserCommandContext {
                active: Some(directory),
                other: None,
                files: None,
                focus: None,
            },
        },
        preview: false,
        cancel: Cancellation::default(),
        retention: UserCommandRetention::new(()),
    };
    let (woken, ready) = channel::channel();
    let waker = Waker::from(Arc::new(WakeUp(woken)));
    let mut poll = TaskContext::from_waker(&waker);
    let enter = tokio.enter();
    let owned = retirement.domain.try_reserve().unwrap().own(work);
    let mut execution = Box::pin(execute(owned, &context));
    assert!(matches!(execution.as_mut().poll(&mut poll), Poll::Pending));
    // The gate proves that the first poll registered the preparation JoinHandle.
    gate.release();
    ready
        .recv_timeout(Duration::from_secs(5))
        .expect("preparation never returned");
    assert!(matches!(execution.as_mut().poll(&mut poll), Poll::Pending));
    assert!(
        observed.try_recv().is_err(),
        "pins must survive process admission"
    );
    drop(execution);
    drop(enter);
    tokio.shutdown_timeout(Duration::from_millis(100));
    let retired = observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(retired, caller, "Prepared native pin Drop ran on caller");
}

struct RetiringNative(channel::Sender<ThreadId>);
impl Drop for RetiringNative {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}
impl TerminalService for RetiringNative {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("unpolled actor cannot launch")
    }
}
impl UserCommandService for RetiringNative {
    fn launch_user_program(
        &self,
        _: &Path,
        _: &[std::ffi::OsString],
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("unpolled actor cannot launch")
    }
}
#[test]
fn unpolled_actor_retires_its_last_native_service_after_runtime_shutdown() {
    let caller = std::thread::current().id();
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (dropped, observed) = channel::channel();
    let (children, _reaper) = mpsc::channel(1);
    let (sender, _events) = mpsc::channel(1);
    let (pin_drop, _pins) = channel::channel();
    let files = Files {
        dropped: pin_drop,
        gate: Arc::new(Gate {
            open: Mutex::new(true),
            wake: std::sync::Condvar::new(),
        }),
    };
    let enter = tokio.enter();
    let controller = Controller::new(
        Retirement::testing(),
        Some(Arc::new(RetiringNative(dropped))),
        Some(Arc::new(files)),
        Runtime::new(
            Arc::new(Semaphore::new(1)),
            Arc::new(Semaphore::new(1)),
            children,
            crate::user_command_runtime::ProcessTracker::try_start().unwrap(),
        ),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    drop(controller);
    drop(enter);
    tokio.shutdown_timeout(Duration::from_millis(100));
    assert_ne!(
        observed.recv_timeout(Duration::from_secs(5)).unwrap(),
        caller
    );
}
