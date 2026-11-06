use midnight_core::*;
use midnight_engine::Engine;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Default)]
struct Clipboard {
    payload: Mutex<Option<FileClipboard>>,
    worker: Mutex<Option<std::thread::ThreadId>>,
    pumps: AtomicUsize,
}
struct Session(Arc<Clipboard>);
struct Service(Arc<Clipboard>);
impl ClipboardService for Service {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        *self.0.worker.lock().unwrap() = Some(std::thread::current().id());
        Ok(Box::new(Session(self.0.clone())))
    }
}
impl ClipboardSession for Session {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        assert_eq!(
            *self.0.worker.lock().unwrap(),
            Some(std::thread::current().id())
        );
        *self.0.payload.lock().unwrap() = Some(files.clone());
        Ok(())
    }
    fn read_files(&mut self) -> Result<FileClipboard, FsError> {
        Ok(self.0.payload.lock().unwrap().clone().unwrap())
    }
    fn service_pending(&mut self) -> Result<(), FsError> {
        self.0.pumps.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
fn selected(paths: &[PathBuf]) -> SelectedFiles {
    let entries: Vec<_> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| Entry::new(EntryId(i as u64), path.clone(), EntryKind::File))
        .collect();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let mut bits = vec![u64::MAX; paths.len().div_ceil(64)];
    if !paths.len().is_multiple_of(64) {
        *bits.last_mut().unwrap() = (1_u64 << (paths.len() % 64)) - 1;
    }
    SelectedFiles {
        selection: Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap()),
        view,
        fallback: None,
    }
}
async fn receive(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if !matches!(event, Event::IconLoaded { .. }) {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_clipboard_preserves_captured_order_pumps_owner_and_reads_captured_destination() {
    let clipboard = Arc::new(Clipboard::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(8);
    let engine = tokio::spawn(
        Engine::new(
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            sender,
        )
        .with_clipboard(Arc::new(Service(clipboard.clone())))
        .run(receiver),
    );
    let parent = std::env::temp_dir();
    let paths = vec![parent.join("z 文档 $(literal).txt"), parent.join("a.txt")];
    commands
        .send(Command::CopyFiles {
            request: DesktopRequestId(1),
            files: selected(&paths),
        })
        .await
        .unwrap();
    assert!(matches!(
        receive(&mut events).await,
        Event::DesktopActionFinished {
            request: DesktopRequestId(1),
            result: Ok(DesktopActionResult::ClipboardCopied)
        }
    ));
    let payload = clipboard.payload.lock().unwrap().clone().unwrap();
    assert_eq!(payload.paths, paths);
    assert_eq!(payload.intent, FileClipboardIntent::Copy);
    assert_ne!(
        *clipboard.worker.lock().unwrap(),
        Some(std::thread::current().id())
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while clipboard.pumps.load(Ordering::Relaxed) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("clipboard owner must pump while idle");
    let destination = parent.join("captured destination");
    commands
        .send(Command::ReadFileClipboard {
            request: DesktopRequestId(2),
            destination: destination.clone(),
        })
        .await
        .unwrap();
    match receive(&mut events).await {
        Event::ClipboardFilesRead {
            request: DesktopRequestId(2),
            destination: actual,
            result: Ok(files),
        } => {
            assert_eq!(actual, destination);
            assert_eq!(files.paths, paths);
        }
        event => panic!("unexpected clipboard event: {event:?}"),
    }
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), engine)
        .await
        .unwrap()
        .unwrap();
}

#[derive(Default)]
struct Menu {
    calls: Mutex<Vec<ShellMenuRequest>>,
    worker: Mutex<Option<std::thread::ThreadId>>,
}
impl ShellMenuService for Menu {
    fn show_shell_menu(&self, request: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        *self.worker.lock().unwrap() = Some(std::thread::current().id());
        self.calls.lock().unwrap().push(request.clone());
        std::thread::sleep(Duration::from_millis(150));
        Ok(ShellMenuOutcome::Managed(ShellMenuVerb::Copy))
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_menu_is_bounded_and_does_not_block_other_engine_requests() {
    let menu = Arc::new(Menu::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(8);
    let engine = tokio::spawn(
        Engine::new(
            Arc::new(midnight_platform::LocalFileSystem::new(None)),
            sender,
        )
        .with_shell_menu(menu.clone())
        .run(receiver),
    );
    let path = std::env::temp_dir().join("文档.txt");
    for id in [1, 2] {
        commands
            .send(Command::ShowShellMenu {
                request: DesktopRequestId(id),
                files: selected(std::slice::from_ref(&path)),
                owner: 0,
                position: ShellMenuPosition { x: -123, y: 45 },
                modifiers: Default::default(),
                target: ShellMenuTarget::Items,
            })
            .await
            .unwrap();
    }
    commands.send(Command::LoadLocale).await.unwrap();
    let mut locale = false;
    let mut busy = false;
    let mut completed = false;
    for _ in 0..3 {
        match receive(&mut events).await {
            Event::LocaleLoaded(Ok(_)) => locale = true,
            Event::ShellMenuFinished {
                request: DesktopRequestId(1),
                result: Ok(ShellMenuOutcome::Managed(ShellMenuVerb::Copy)),
            } => completed = true,
            Event::ShellMenuFinished {
                request: DesktopRequestId(2),
                result: Err(error),
            } => {
                assert_eq!(error.kind, std::io::ErrorKind::WouldBlock);
                busy = true;
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }
    assert!(locale && busy && completed);
    assert_ne!(
        *menu.worker.lock().unwrap(),
        Some(std::thread::current().id())
    );
    assert_eq!(menu.calls.lock().unwrap().len(), 1);
    assert_eq!(menu.calls.lock().unwrap()[0].paths, [path]);
    assert_eq!(menu.calls.lock().unwrap()[0].position.x, -123);
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
}
