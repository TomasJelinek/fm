//! One bounded backend worker retains native ownership and pumps requests while idle.
use crate::{
    Outbox, file_targets,
    retirement::{RetirementLease, RetirementWaiter},
    task_lifetime::TaskLifetime,
};
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
    Generated(Arc<dyn midnight_core::GeneratedTextSource>, Cancellation),
    ReadText,
    Files(SelectedFiles, FileClipboardIntent),
    Captured(Arc<dyn midnight_core::CapturedClipboardFiles>, Cancellation),
    Read(PathBuf),
    ReadCaptured(Arc<dyn midnight_core::CapturedClipboardFiles>, Cancellation),
}
struct Request {
    id: DesktopRequestId,
    action: Action,
}
impl Request {
    fn failure(&self, error: FsError) -> Event {
        match &self.action {
            Action::ReadText => Event::PathClipboardTextRead {
                request: self.id,
                result: Err(error),
            },
            Action::ReadCaptured(_, _) => Event::ClipboardFilesRead {
                request: self.id,
                destination: PathBuf::new(),
                result: Err(error),
            },
            Action::Read(destination) => Event::ClipboardFilesRead {
                request: self.id,
                destination: destination.clone(),
                result: Err(error),
            },
            _ => Event::DesktopActionFinished {
                request: self.id,
                result: Err(error),
            },
        }
    }
}
// One private inbox plus one serialized rejected request; existing queue bound stays 16.
pub(crate) const RETIREMENT_CAPACITY: usize = 2;
pub(crate) struct Retirement {
    pub(crate) worker: RetirementLease,
    pub(crate) rejected: RetirementWaiter,
}
struct NativeWorker {
    receiver: mpsc::Receiver<Request>,
    service: Option<Arc<dyn ClipboardService>>,
    _scope: TaskLifetime,
}
struct Rejected {
    request: Request,
    _scope: TaskLifetime,
}
pub(crate) struct Controller {
    sender: Option<mpsc::Sender<Request>>,
    service: Option<Arc<dyn ClipboardService>>,
    cancel: Cancellation,
    outbox: Outbox,
    worker: Option<RetirementLease>,
    rejected: RetirementWaiter,
    scope: TaskLifetime,
}
impl Controller {
    pub(crate) async fn copy_captured(
        &mut self,
        id: DesktopRequestId,
        source: Arc<dyn midnight_core::CapturedClipboardFiles>,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        self.submit(
            Request {
                id,
                action: Action::Captured(source, cancel),
            },
            tasks,
        )
        .await;
    }
    pub(crate) async fn read_captured_files(
        &mut self,
        id: DesktopRequestId,
        source: Arc<dyn midnight_core::CapturedClipboardFiles>,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        self.submit(
            Request {
                id,
                action: Action::ReadCaptured(source, cancel),
            },
            tasks,
        )
        .await;
    }
    pub(crate) async fn copy_generated(
        &mut self,
        id: DesktopRequestId,
        content: Arc<dyn midnight_core::GeneratedTextSource>,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        self.submit(
            Request {
                id,
                action: Action::Generated(content, cancel),
            },
            tasks,
        )
        .await;
    }
    pub(crate) fn new(
        retirement: Retirement,
        scope: TaskLifetime,
        service: Option<Arc<dyn ClipboardService>>,
        outbox: Outbox,
    ) -> Self {
        Self {
            sender: None,
            service,
            cancel: Cancellation::default(),
            outbox,
            worker: Some(retirement.worker),
            rejected: retirement.rejected,
            scope,
        }
    }
    #[cfg(test)]
    fn testing(service: Option<Arc<dyn ClipboardService>>, outbox: Outbox) -> Self {
        let domain =
            crate::retirement::RetirementDomain::try_start(RETIREMENT_CAPACITY, 1).unwrap();
        let retirement = Retirement {
            worker: domain.try_reserve().unwrap(),
            rejected: domain.new_waiter().unwrap(),
        };
        let scope = TaskLifetime::testing(service.clone());
        Self::new(retirement, scope, service, outbox)
    }
    fn start_worker(&mut self, tasks: &mut JoinSet<()>) -> mpsc::Sender<Request> {
        let (sender, receiver) = mpsc::channel::<Request>(16);
        let mut worker = self
            .worker
            .take()
            .expect("clipboard worker starts once")
            .own(NativeWorker {
                receiver,
                service: self.service.take(),
                _scope: self.scope.clone(),
            });
        let cancel = self.cancel.clone();
        let outbox = self.outbox.clone();
        tasks.spawn_blocking(self.scope.blocking(move || {
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
                let request = match worker.receiver.try_recv() {
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
                            worker
                                .service
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
                            // Opening the native session may block. A modeless
                            // window can close during that call, after preparation.
                            Ok(()) => match check_cancellation(&request.action, &cancel) {
                                Err(error) => request.failure(error),
                                Ok(()) => {
                                    execute(request.id, action, session.as_mut().unwrap().as_mut())
                                }
                            },
                        }
                    }
                };
                drop(request);
                if !send_event(&outbox, event, &cancel, &mut session) {
                    break;
                }
            }
            // Session never enters the transferable guard: create, pump and Drop stay here.
            // The guarded inbox also covers cancellation before this closure ever starts.
        }));
        sender
    }
    async fn submit(&mut self, request: Request, tasks: &mut JoinSet<()>) {
        if self.sender.is_none() {
            self.sender = Some(self.start_worker(tasks));
        }
        if let Err(failed) = self.sender.as_ref().unwrap().try_send(request) {
            let request = failed.into_inner();
            let id = request.id;
            let busy = error(
                io::ErrorKind::WouldBlock,
                "Clipboard queue is busy; try again.",
            );
            // EngineRun retains raw dispatch ownership while this private, serial intake waits.
            // Once adopted, cleanup survives cancellation of this acknowledgement or its runtime.
            let lease = self.rejected.reserve().await;
            let rejected = lease.own(Rejected {
                request,
                _scope: self.scope.clone(),
            });
            let mut event = rejected.request.failure(busy);
            if let Err(failure) = rejected.retire().await {
                event = Event::DesktopActionFinished {
                    request: id,
                    result: Err(error(
                        io::ErrorKind::Other,
                        &format!("Clipboard retirement failed: {failure}"),
                    )),
                };
            }
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
    pub(crate) async fn read_text(&mut self, id: DesktopRequestId, tasks: &mut JoinSet<()>) {
        self.submit(
            Request {
                id,
                action: Action::ReadText,
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
        drop(self);
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        // A started blocking pump cannot be aborted by dropping its JoinSet.
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
    ReadText,
    Files(FileClipboard),
    Captured(
        Arc<dyn midnight_core::CapturedClipboardFiles>,
        Cancellation,
        Cancellation,
    ),
    Read(PathBuf),
    ReadCaptured(
        Arc<dyn midnight_core::CapturedClipboardFiles>,
        Cancellation,
        Cancellation,
    ),
}
fn prepare(action: &Action, cancel: &Cancellation) -> Result<Prepared, FsError> {
    check_cancellation(action, cancel)?;
    match action {
        Action::Generated(source, _) => {
            let length = source.length();
            if length > 4 * 1024 * 1024 {
                return Err(error(
                    io::ErrorKind::InvalidInput,
                    "The generated text exceeds the clipboard's 4 MiB budget. Use a file or the internal viewer.",
                ));
            }
            let mut bytes = vec![0; length as usize];
            let mut offset = 0;
            while offset < bytes.len() {
                check_cancellation(action, cancel)?;
                let end = (offset + 256 * 1024).min(bytes.len());
                let count = source
                    .read_at(offset as u64, &mut bytes[offset..end])
                    .map_err(|e| error(e.kind(), &e.to_string()))?;
                if count == 0 || count > end - offset {
                    return Err(error(
                        io::ErrorKind::UnexpectedEof,
                        "Generated text ended before its recorded length.",
                    ));
                }
                offset += count;
            }
            check_cancellation(action, cancel)?;
            if source.length() != length {
                return Err(error(
                    io::ErrorKind::InvalidData,
                    "Generated text changed its recorded length.",
                ));
            }
            let text = String::from_utf8(bytes)
                .map_err(|_| error(io::ErrorKind::InvalidData, "Generated text is not UTF-8."))?;
            if text.contains('\0') {
                return Err(error(
                    io::ErrorKind::InvalidInput,
                    "Clipboard text cannot contain NUL.",
                ));
            }
            Ok(Prepared::Text(text))
        }
        Action::ReadText => Ok(Prepared::ReadText),
        Action::Text(text) => {
            if text.len() > 4 * 1024 * 1024 || text.contains('\0') {
                return Err(error(
                    io::ErrorKind::InvalidInput,
                    "Clipboard text exceeds its 4 MiB resource budget or contains NUL.",
                ));
            }
            Ok(Prepared::Text(text.clone()))
        }
        Action::Captured(source, request) => Ok(Prepared::Captured(
            source.clone(),
            request.clone(),
            cancel.clone(),
        )),
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
        Action::ReadCaptured(source, request) => Ok(Prepared::ReadCaptured(
            source.clone(),
            request.clone(),
            cancel.clone(),
        )),
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
fn check_cancellation(action: &Action, global: &Cancellation) -> Result<(), FsError> {
    if global.is_cancelled()
        || matches!(action, Action::Generated(_, request) | Action::Captured(_, request) | Action::ReadCaptured(_, request) if request.is_cancelled())
    {
        Err(error(
            io::ErrorKind::Interrupted,
            "Clipboard copy was cancelled.",
        ))
    } else {
        Ok(())
    }
}
fn execute(id: DesktopRequestId, action: Prepared, session: &mut dyn ClipboardSession) -> Event {
    let result = match action {
        Prepared::ReadText => {
            return Event::PathClipboardTextRead {
                request: id,
                result: session.read_text().and_then(|text| {
                    crate::clipboard_text::prepare_pasted_path(text)
                        .map_err(|message| error(io::ErrorKind::InvalidData, message))
                }),
            };
        }
        Prepared::Text(text) => session.copy_text(&text),
        Prepared::Files(files) => session.copy_files(&files),
        Prepared::Captured(source, request, global) => {
            publish_captured(source.as_ref(), &request, &global, session)
        }
        Prepared::ReadCaptured(source, request, global) => {
            let result = read_captured(source.as_ref(), &request, &global, session);
            let (destination, result) = match result {
                Ok((destination, clipboard)) => (destination, Ok(clipboard)),
                Err(error) => (PathBuf::new(), Err(error)),
            };
            return Event::ClipboardFilesRead {
                request: id,
                destination,
                result,
            };
        }
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

fn read_captured(
    source: &dyn midnight_core::CapturedClipboardFiles,
    request: &Cancellation,
    global: &Cancellation,
    session: &mut dyn ClipboardSession,
) -> Result<(PathBuf, FileClipboard), FsError> {
    let check = || {
        if request.is_cancelled() || global.is_cancelled() {
            Err(error(
                io::ErrorKind::Interrupted,
                "Clipboard paste was cancelled.",
            ))
        } else {
            Ok(())
        }
    };
    check()?;
    // Resolve after native clipboard opening, which can block long enough for
    // navigation or provider revocation to invalidate the destination.
    let target = source.resolve(global)?;
    target.validate()?;
    if target.intent != source.intent() || target.paths.len() != 1 {
        return Err(error(
            io::ErrorKind::InvalidData,
            "Paste requires one captured native folder.",
        ));
    }
    check()?;
    let files = session.read_files()?;
    files.validate()?;
    check()?;
    let current = source.resolve(global)?;
    current.validate()?;
    if current.intent != target.intent || current.paths != target.paths {
        return Err(error(
            io::ErrorKind::InvalidData,
            "The captured paste destination changed.",
        ));
    }
    check()?;
    let Some(destination) = target.paths.into_iter().next() else {
        return Err(error(
            io::ErrorKind::InvalidData,
            "The captured paste destination is missing.",
        ));
    };
    Ok((destination, files))
}

fn publish_captured(
    source: &dyn midnight_core::CapturedClipboardFiles,
    request: &Cancellation,
    global: &Cancellation,
    session: &mut dyn ClipboardSession,
) -> Result<(), FsError> {
    let check = || {
        if request.is_cancelled() || global.is_cancelled() {
            Err(error(
                io::ErrorKind::Interrupted,
                "Clipboard copy was cancelled.",
            ))
        } else {
            Ok(())
        }
    };
    check()?;
    let intent = source.intent();
    let files = source.resolve(global)?;
    files.validate()?;
    if files.intent != intent {
        return Err(error(
            io::ErrorKind::InvalidData,
            "Captured clipboard intent changed.",
        ));
    }
    check()?;
    session.copy_files(&files)
}
fn error(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("clipboard", Path::new(""), io::Error::new(kind, message))
}

#[cfg(test)]
#[path = "clipboard_retirement_tests.rs"]
mod retirement_tests;

#[cfg(test)]
#[path = "clipboard_capture_tests.rs"]
mod capture_tests;
#[cfg(test)]
#[path = "clipboard_generated_tests.rs"]
mod generated_tests;
#[cfg(test)]
#[path = "clipboard_paste_capture_tests.rs"]
mod paste_capture_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Pump(Arc<AtomicUsize>);
    struct TextReader(&'static str);
    impl ClipboardSession for TextReader {
        fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
            panic!("reading must not write the clipboard");
        }
        fn read_text(&mut self) -> Result<String, FsError> {
            Ok(self.0.into())
        }
    }
    #[test]
    fn explicit_text_read_applies_literal_path_policy_and_reports_invalid_input() {
        let id = DesktopRequestId(17);
        match execute(id, Prepared::ReadText, &mut TextReader("资料 ")) {
            Event::PathClipboardTextRead {
                request,
                result: Ok(text),
            } => {
                assert_eq!(request, id);
                assert_eq!(&*text, format!("资料 {}", std::path::MAIN_SEPARATOR));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            execute(id, Prepared::ReadText, &mut TextReader("bad\npath")),
            Event::PathClipboardTextRead { result: Err(_), .. }
        ));
    }

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
        let mut controller = Controller::testing(
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
