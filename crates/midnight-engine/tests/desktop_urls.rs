//! Followed web and mail links reach the system's default handler only through the
//! typed `Command::OpenUrl`, on a backend worker, and answer as `DesktopActionFinished`.
use midnight_core::*;
use midnight_engine::{CommandSender, Engine};
use std::{
    io,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::ThreadId,
    time::Duration,
};
use tokio::{
    sync::{Notify, mpsc},
    task::JoinHandle,
};

/// What the fake desktop does with a link.
#[derive(Clone, Copy, Debug)]
enum Handler {
    /// The system itself accepts the link, as the Windows Shell does.
    Accepts,
    /// The system refuses it.
    Refuses,
    /// A helper program takes it, as xdg-open does on Linux; the test decides its exit.
    Helper,
}

/// A helper process whose exit the test decides. Nothing is started.
#[derive(Default)]
struct HelperState {
    exit: Mutex<Option<ExitStatus>>,
    polls: AtomicUsize,
    dropped: AtomicBool,
    changed: Notify,
}

impl HelperState {
    fn end(&self, status: ExitStatus) {
        *self.exit.lock().unwrap_or_else(PoisonError::into_inner) = Some(status);
    }
}

struct Helper(Arc<HelperState>);
impl TerminalProcess for Helper {
    fn id(&self) -> u32 {
        4242
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let exit = *self.0.exit.lock().unwrap_or_else(PoisonError::into_inner);
        self.0.polls.fetch_add(1, Ordering::SeqCst);
        self.0.changed.notify_one();
        Ok(exit)
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
        self.0.changed.notify_one();
    }
}

/// Lets a helper that a failing test left running exit, so that the engine can stop.
struct EndHelper(Arc<HelperState>);
impl Drop for EndHelper {
    fn drop(&mut self) {
        self.0
            .exit
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(ExitStatus::default);
    }
}

struct Desktop {
    handler: Handler,
    links: Mutex<Vec<(ExternalUrl, ThreadId)>>,
    helper: Arc<HelperState>,
}

impl Desktop {
    fn new(handler: Handler) -> Arc<Self> {
        Arc::new(Self {
            handler,
            links: Mutex::default(),
            helper: Arc::default(),
        })
    }

    /// Every link handed over, with the thread that handed it over.
    fn links(&self) -> Vec<(ExternalUrl, ThreadId)> {
        self.links
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl OpenService for Desktop {
    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        panic!("a link must never be opened as a path: {path:?}")
    }

    fn open_url(&self, url: &ExternalUrl) -> Result<Option<Box<dyn TerminalProcess>>, FsError> {
        self.links
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.clone(), std::thread::current().id()));
        match self.handler {
            Handler::Accepts => Ok(None),
            Handler::Refuses => Err(FsError::from_io(
                "open link",
                Path::new(""),
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "No program opens this kind of link.",
                ),
            )),
            Handler::Helper => Ok(Some(Box::new(Helper(self.helper.clone())))),
        }
    }
}
impl IconService for Desktop {
    fn load_icon(&self, _: &IconKey) -> Result<Option<IconPixels>, FsError> {
        Ok(None)
    }
}
impl TerminalService for Desktop {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("a link must never start a terminal")
    }
}
impl PropertyService for Desktop {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        panic!("a link must never show properties")
    }
}
impl LocationService for Desktop {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        Ok(Vec::new())
    }
}
impl DirectoryService for Desktop {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        panic!("a link must never list a folder")
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("a link must never read metadata")
    }
}
impl VolumeService for Desktop {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Desktop {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        panic!("a link must never query free space")
    }
}

/// An engine with `services` as its file system and, when `install`, its desktop.
fn start(
    services: Arc<Desktop>,
    install: bool,
) -> (CommandSender, mpsc::Receiver<Event>, JoinHandle<()>) {
    let (sender, events) = mpsc::channel(64);
    let mut engine = Engine::new(Engine::lifetime().unwrap(), services.clone(), sender);
    let (commands, receiver) = engine.command_channel(16).unwrap();
    if install {
        engine.configure_desktop().unwrap().install(services);
    }
    (commands, events, tokio::spawn(engine.run(receiver)))
}

async fn stop(commands: CommandSender, events: mpsc::Receiver<Event>, task: JoinHandle<()>) {
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

fn open(request: u64, link: &str) -> Command {
    Command::OpenUrl {
        request: DesktopRequestId(request),
        url: ExternalUrl::parse(link).unwrap(),
    }
}

/// The answer to `request`, past unrelated startup events.
async fn finished(
    events: &mut mpsc::Receiver<Event>,
    request: u64,
) -> Result<DesktopActionResult, FsError> {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
            .await
            .expect("the engine answers")
            .expect("the event channel stays open");
        if let Event::DesktopActionFinished {
            request: answered,
            result,
        } = event
        {
            assert_eq!(answered, DesktopRequestId(request));
            return result;
        }
    }
}

async fn until(state: &HelperState, ready: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let changed = state.changed.notified();
            if ready() {
                break;
            }
            changed.await;
        }
    })
    .await
    .expect("the helper reaches the expected state");
}

/// An exit status that reports failure, without starting a process.
fn failed_exit() -> ExitStatus {
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(4)
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(4 << 8)
    }
}

#[tokio::test]
async fn a_followed_link_reaches_the_default_handler_exactly_on_a_backend_worker() {
    let caller = std::thread::current().id();
    for (text, mail) in [
        ("https://example.com/a b?q=\"quoted\"&lang=文档#top", false),
        (
            "mailto:someone@example.com?subject=Hi&attach=%2Fetc%2Fpasswd",
            true,
        ),
    ] {
        let url = ExternalUrl::parse(text).unwrap();
        let services = Desktop::new(Handler::Accepts);
        let (commands, mut events, task) = start(services.clone(), true);
        commands
            .send(Command::OpenUrl {
                request: DesktopRequestId(7),
                url: url.clone(),
            })
            .await
            .unwrap();
        let result = finished(&mut events, 7).await;
        assert!(
            matches!(result, Ok(DesktopActionResult::UrlOpened { mail: opened }) if opened == mail),
            "{text}: {result:?}"
        );
        let links = services.links();
        assert_eq!(links.len(), 1, "one link, handed over once");
        assert_eq!(links[0].0, url, "the exact address, never rewritten");
        assert_ne!(links[0].1, caller, "dispatch runs on a backend worker");
        stop(commands, events, task).await;
    }
}

#[tokio::test]
async fn a_refused_link_keeps_the_native_reason_and_names_no_path() {
    let services = Desktop::new(Handler::Refuses);
    let (commands, mut events, task) = start(services.clone(), true);
    commands
        .send(open(8, "https://example.com/report"))
        .await
        .unwrap();
    let error = finished(&mut events, 8).await.unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::NotFound);
    assert_eq!(error.message, "No program opens this kind of link.");
    assert_eq!(error.operation, "open link");
    assert!(error.path.as_os_str().is_empty());
    assert_eq!(services.links().len(), 1);
    stop(commands, events, task).await;
}

#[tokio::test]
async fn a_link_helper_is_reaped_before_success_and_a_failed_exit_is_reported() {
    // Success only once the running helper exits successfully.
    let services = Desktop::new(Handler::Helper);
    let helper = services.helper.clone();
    let _end = EndHelper(helper.clone());
    let (commands, mut events, task) = start(services.clone(), true);
    commands
        .send(open(9, "https://example.com/"))
        .await
        .unwrap();
    until(&helper, || helper.polls.load(Ordering::SeqCst) > 0).await;
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, Event::DesktopActionFinished { .. }),
            "a running helper must not report the link as opened"
        );
    }
    assert!(
        !helper.dropped.load(Ordering::SeqCst),
        "the running helper stays owned"
    );
    helper.end(ExitStatus::default());
    let result = finished(&mut events, 9).await;
    assert!(
        matches!(result, Ok(DesktopActionResult::UrlOpened { mail: false })),
        "{result:?}"
    );
    until(&helper, || helper.dropped.load(Ordering::SeqCst)).await;
    stop(commands, events, task).await;

    // A helper that fails says so, with the link's operation and no path.
    let services = Desktop::new(Handler::Helper);
    services.helper.end(failed_exit());
    let (commands, mut events, task) = start(services.clone(), true);
    commands
        .send(open(10, "mailto:someone@example.com"))
        .await
        .unwrap();
    let error = finished(&mut events, 10).await.unwrap_err();
    assert_eq!(error.operation, "open link");
    assert!(
        error.message.contains("Desktop opener exited"),
        "{}",
        error.message
    );
    assert!(error.path.as_os_str().is_empty());
    assert_eq!(services.links().len(), 1);
    stop(commands, events, task).await;
}

#[tokio::test]
async fn links_are_unsupported_without_desktop_services() {
    let services = Desktop::new(Handler::Accepts);
    let (commands, mut events, task) = start(services.clone(), false);
    commands
        .send(open(11, "https://example.com/"))
        .await
        .unwrap();
    let error = finished(&mut events, 11).await.unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert_eq!(error.operation, "open link");
    assert!(error.path.as_os_str().is_empty());
    assert!(services.links().is_empty());
    stop(commands, events, task).await;
}
