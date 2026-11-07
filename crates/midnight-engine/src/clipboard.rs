//! One bounded backend worker retains native ownership and pumps requests while idle.
use crate::{Outbox, file_targets};
use midnight_core::{
    Cancellation, ClipboardService, ClipboardSession, DesktopActionResult, DesktopRequestId, Event,
    FileClipboard, FileClipboardIntent, FsError, SelectedFiles,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinSet};

enum Action {
    Text(String),
    Files(SelectedFiles, FileClipboardIntent),
    Read(PathBuf),
}
struct Request {
    id: DesktopRequestId,
    action: Action,
}
impl Request {
    fn failure(self, error: FsError) -> Event {
        match self.action {
            Action::Read(destination) => Event::ClipboardFilesRead {
                request: self.id,
                destination,
                result: Err(error),
            },
            _ => Event::DesktopActionFinished {
                request: self.id,
                result: Err(error),
            },
        }
    }
}
pub(crate) struct Controller {
    sender: Option<mpsc::Sender<Request>>,
    service: Option<Arc<dyn ClipboardService>>,
    cancel: Cancellation,
    outbox: Outbox,
}
impl Controller {
    pub(crate) fn new(service: Option<Arc<dyn ClipboardService>>, outbox: Outbox) -> Self {
        Self {
            sender: None,
            service,
            cancel: Cancellation::default(),
            outbox,
        }
    }
    fn start_worker(&mut self, tasks: &mut JoinSet<()>) -> mpsc::Sender<Request> {
        let (sender, mut receiver) = mpsc::channel::<Request>(16);
        let service = self.service.take();
        let cancel = self.cancel.clone();
        let outbox = self.outbox.clone();
        tasks.spawn_blocking(move || {
            let mut session: Option<Box<dyn ClipboardSession>> = None;
            while !cancel.is_cancelled() {
                // X11 owners and Windows clipboard windows must serve native messages
                // without waiting for another application command.
                if session
                    .as_mut()
                    .is_some_and(|s| s.service_pending().is_err())
                {
                    session = None;
                }
                let request = match receiver.try_recv() {
                    Ok(request) => request,
                    Err(mpsc::error::TryRecvError::Empty) => {
                        std::thread::park_timeout(Duration::from_millis(10));
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                };
                let event = match prepare(&request.action, &cancel) {
                    Err(error) => request.failure(error),
                    Ok(action) => {
                        let opened = if session.is_none() {
                            service
                                .as_ref()
                                .ok_or_else(|| {
                                    error(
                                        io::ErrorKind::Unsupported,
                                        "Clipboard integration is unavailable.",
                                    )
                                })
                                .and_then(|service| service.open())
                                .map(|opened| session = Some(opened))
                        } else {
                            Ok(())
                        };
                        match opened {
                            Err(error) => request.failure(error),
                            Ok(()) => {
                                execute(request.id, action, session.as_mut().unwrap().as_mut())
                            }
                        }
                    }
                };
                if !send_event(&outbox, event, &cancel, &mut session) {
                    break;
                }
            }
            // Native owners are created, serviced and destroyed on this same thread.
        });
        sender
    }
    async fn submit(&mut self, request: Request, tasks: &mut JoinSet<()>) {
        if self.sender.is_none() {
            self.sender = Some(self.start_worker(tasks));
        }
        if let Err(failed) = self.sender.as_ref().unwrap().try_send(request) {
            let event = failed.into_inner().failure(error(
                io::ErrorKind::WouldBlock,
                "Clipboard queue is busy; try again.",
            ));
            // Backpressure applies to rejected requests too; never spawn unbounded reply tasks.
            self.outbox.send(event).await;
        }
    }
    pub(crate) async fn copy(
        &mut self,
        id: DesktopRequestId,
        text: String,
        tasks: &mut JoinSet<()>,
    ) {
        self.submit(
            Request {
                id,
                action: Action::Text(text),
            },
            tasks,
        )
        .await;
    }
    pub(crate) async fn copy_files(
        &mut self,
        id: DesktopRequestId,
        files: SelectedFiles,
        tasks: &mut JoinSet<()>,
    ) {
        self.submit(
            Request {
                id,
                action: Action::Files(files, FileClipboardIntent::Copy),
            },
            tasks,
        )
        .await;
    }
    pub(crate) async fn read_files(
        &mut self,
        id: DesktopRequestId,
        destination: PathBuf,
        tasks: &mut JoinSet<()>,
    ) {
        self.submit(
            Request {
                id,
                action: Action::Read(destination),
            },
            tasks,
        )
        .await;
    }
    pub(crate) async fn cut_files(
        &mut self,
        id: DesktopRequestId,
        files: SelectedFiles,
        tasks: &mut JoinSet<()>,
    ) {
        self.submit(
            Request {
                id,
                action: Action::Files(files, FileClipboardIntent::Move),
            },
            tasks,
        )
        .await;
    }
    pub(crate) fn shutdown(self) {
        self.cancel.cancel();
    }
}
fn send_event(
    outbox: &Outbox,
    mut event: Event,
    cancel: &Cancellation,
    session: &mut Option<Box<dyn ClipboardSession>>,
) -> bool {
    while !cancel.is_cancelled() {
        match outbox.sender.try_send(event) {
            Ok(()) => {
                (outbox.wakeup)();
                return true;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(pending)) => event = pending,
        }
        // A busy UI result queue must not prevent other applications reading our clipboard.
        if session
            .as_mut()
            .is_some_and(|s| s.service_pending().is_err())
        {
            *session = None;
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
    false
}
enum Prepared {
    Text(String),
    Files(FileClipboard),
    Read(PathBuf),
}
fn prepare(action: &Action, cancel: &Cancellation) -> Result<Prepared, FsError> {
    match action {
        Action::Text(text) => {
            if text.len() > 4 * 1024 * 1024 || text.contains('\0') {
                return Err(error(
                    io::ErrorKind::InvalidInput,
                    "Clipboard text exceeds its 4 MiB resource budget or contains NUL.",
                ));
            }
            Ok(Prepared::Text(text.clone()))
        }
        Action::Files(files, intent) => {
            let paths = file_targets::resolve(
                &files.selection,
                &files.view,
                files.fallback.as_deref(),
                cancel,
            )?;
            let files = FileClipboard {
                paths,
                intent: *intent,
            };
            files.validate()?;
            Ok(Prepared::Files(files))
        }
        Action::Read(destination) => {
            if !destination.is_absolute() || destination.as_os_str().as_encoded_bytes().contains(&0)
            {
                return Err(error(
                    io::ErrorKind::InvalidInput,
                    "Paste requires an absolute destination.",
                ));
            }
            Ok(Prepared::Read(destination.clone()))
        }
    }
}
fn execute(id: DesktopRequestId, action: Prepared, session: &mut dyn ClipboardSession) -> Event {
    let result = match action {
        Prepared::Text(text) => session.copy_text(&text),
        Prepared::Files(files) => session.copy_files(&files),
        Prepared::Read(destination) => {
            return Event::ClipboardFilesRead {
                request: id,
                destination,
                result: session.read_files().and_then(|files| {
                    files.validate()?;
                    Ok(files)
                }),
            };
        }
    };
    Event::DesktopActionFinished {
        request: id,
        result: result.map(|()| DesktopActionResult::ClipboardCopied),
    }
}
fn error(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("clipboard", Path::new(""), io::Error::new(kind, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Pump(Arc<AtomicUsize>);

    #[test]
    fn native_cut_and_copy_keep_captured_raw_paths_and_distinct_intents() {
        let path = std::env::temp_dir().join("clipboard 文档 $(literal) ' 🦀.txt");
        let view = Arc::new(midnight_core::DirectoryIndex::from_entries(&[
            midnight_core::Entry::new(
                midnight_core::EntryId(1),
                path.clone(),
                midnight_core::EntryKind::File,
            ),
        ]));
        let files = SelectedFiles {
            selection: Arc::default(),
            view,
            fallback: Some(path.clone()),
        };
        for intent in [FileClipboardIntent::Copy, FileClipboardIntent::Move] {
            let Prepared::Files(actual) = prepare(
                &Action::Files(files.clone(), intent),
                &Cancellation::default(),
            )
            .unwrap() else {
                panic!("native file clipboard");
            };
            assert_eq!(actual.paths, vec![path.clone()]);
            assert_eq!(actual.intent, intent);
        }
    }

    #[tokio::test]
    async fn saturated_command_and_result_queues_do_not_spawn_rejection_tasks() {
        let (native, _native_receiver) = mpsc::channel(1);
        let (sender, mut events) = mpsc::channel(1);
        sender
            .try_send(Event::LocaleLoaded(Ok(Default::default())))
            .unwrap();
        let mut controller = Controller::new(
            None,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
        );
        controller.sender = Some(native);
        let mut tasks = JoinSet::new();
        controller
            .copy(DesktopRequestId(1), "first".into(), &mut tasks)
            .await;
        for id in 2..6 {
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(20),
                    controller.copy(DesktopRequestId(id), "rejected".into(), &mut tasks)
                )
                .await
                .is_err()
            );
            assert!(
                tasks.is_empty(),
                "a rejected request cannot create a pending output task"
            );
        }
        events.recv().await.unwrap();
        controller
            .copy(DesktopRequestId(6), "rejected".into(), &mut tasks)
            .await;
        assert!(
            matches!(events.recv().await, Some(Event::DesktopActionFinished {
            request: DesktopRequestId(6), result: Err(error),
        }) if error.kind == io::ErrorKind::WouldBlock)
        );
        controller.shutdown();
    }
    impl ClipboardSession for Pump {
        fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
            Ok(())
        }
        fn service_pending(&mut self) -> Result<(), FsError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    #[test]
    fn result_backpressure_keeps_serving_native_clipboard_requests() {
        let (sender, mut receiver) = mpsc::channel(1);
        sender
            .try_send(Event::LocaleLoaded(Ok(Default::default())))
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let consumer = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while observed.load(Ordering::Relaxed) < 2 && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(5));
            }
            let pumped = observed.load(Ordering::Relaxed);
            receiver.blocking_recv().unwrap();
            assert!(matches!(
                receiver.blocking_recv(),
                Some(Event::DesktopActionFinished { .. })
            ));
            assert!(
                pumped >= 2,
                "output backpressure must not starve the native owner"
            );
        });
        let mut session: Option<Box<dyn ClipboardSession>> = Some(Box::new(Pump(calls)));
        assert!(send_event(
            &Outbox {
                sender,
                wakeup: Arc::new(|| {})
            },
            Event::DesktopActionFinished {
                request: DesktopRequestId(1),
                result: Ok(DesktopActionResult::ClipboardCopied),
            },
            &Cancellation::default(),
            &mut session
        ));
        consumer.join().unwrap();
    }
}
