use midnight_core::*;
use midnight_engine::Engine;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Default)]
struct Service {
    calls: Mutex<Vec<(PathBuf, EditorPreferences, std::thread::ThreadId)>>,
}
impl DirectoryService for Service {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!()
    }
}
impl StorageService for Service {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!()
    }
}
impl VolumeService for Service {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl EditorService for Service {
    fn default_editor(&self) -> Result<EditorCommand, FsError> {
        Ok(EditorCommand {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["{file}".into()],
        })
    }
    fn launch_editor(
        &self,
        file: &Path,
        editor: &EditorPreferences,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        self.calls
            .lock()
            .unwrap()
            .push((file.into(), editor.clone(), std::thread::current().id()));
        Err(FsError::from_io(
            "editor launch",
            file,
            io::Error::new(io::ErrorKind::NotFound, "configured program missing"),
        ))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn editor_dispatch_preserves_request_and_reports_worker_launch_error() {
    let service = Arc::new(Service::default());
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(64);
    let task = tokio::spawn(
        Engine::new(service.clone(), sender)
            .with_editor(service.clone())
            .run(receiver),
    );
    let path = std::env::current_dir()
        .unwrap()
        .join("文档 & $HOME; ' .txt");
    let editor = EditorPreferences {
        executable: Some(std::env::current_exe().unwrap()),
        arguments: vec!["{file}".into(), "literal $HOME; \"".into()],
    };
    commands
        .send(Command::EditFile {
            request: DesktopRequestId(42),
            path: path.clone(),
            editor: editor.clone(),
        })
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let Event::DesktopActionFinished { request, result } = event {
            assert_eq!(request, DesktopRequestId(42));
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("configured program missing")
            );
            break;
        }
    }
    let calls = service.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, path);
    assert_eq!(calls[0].1, editor);
    assert_ne!(
        calls[0].2,
        std::thread::current().id(),
        "native service ran on the async/UI owner thread"
    );
    commands.send(Command::ResolveDefaultEditor).await.unwrap();
    loop {
        if let Event::EditorDefaultResolved(result) =
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            assert_eq!(result.unwrap().executable, std::env::current_exe().unwrap());
            break;
        }
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test]
async fn editor_rejections_keep_backpressure_when_event_consumer_stops() {
    let service = Arc::new(Service::default());
    let (commands, receiver) = mpsc::channel(2);
    let (sender, mut events) = mpsc::channel(1);
    let task = tokio::spawn(Engine::new(service, sender).run(receiver));
    commands
        .send(Command::EditFile {
            request: DesktopRequestId(1),
            path: "file".into(),
            editor: Default::default(),
        })
        .await
        .unwrap();
    loop {
        if let Event::DesktopActionFinished { .. } =
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            break;
        }
    }
    let mut producer = tokio::spawn(async move {
        for request in 2..34 {
            commands
                .send(Command::EditFile {
                    request: DesktopRequestId(request),
                    path: "file".into(),
                    editor: Default::default(),
                })
                .await
                .unwrap();
        }
    });
    let result = tokio::time::timeout(Duration::from_millis(200), &mut producer).await;
    producer.abort();
    task.abort();
    drop(events);
    assert!(
        result.is_err(),
        "rejected editor launches must stay bounded while the UI is stalled"
    );
}

#[derive(Debug)]
struct Captured {
    path: PathBuf,
    cancel_during_resolve: bool,
    resolved: Arc<Mutex<Vec<std::thread::ThreadId>>>,
    dropped: Arc<Mutex<Vec<std::thread::ThreadId>>>,
}
impl CapturedEditorFile for Captured {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(3),
            session: TreeSessionId(4),
            revision: TreeRevision(5),
        }
    }
    fn row(&self) -> TreeRowId {
        TreeRowId(6)
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        self.resolved
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        if self.cancel_during_resolve {
            cancel.cancel();
            return Ok(self.path.clone());
        }
        if cancel.is_cancelled() {
            Err(FsError::from_io(
                "captured editor",
                &self.path,
                io::ErrorKind::Interrupted.into(),
            ))
        } else {
            Ok(self.path.clone())
        }
    }
}
impl Drop for Captured {
    fn drop(&mut self) {
        self.dropped
            .lock()
            .unwrap()
            .push(std::thread::current().id());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_tree_editor_resolves_and_retires_only_on_workers_including_rejection() {
    for (available, cancelled, during) in [
        (true, false, false),
        (true, true, false),
        (false, false, false),
        (true, false, true),
    ] {
        let service = Arc::new(Service::default());
        let (commands, receiver) = mpsc::channel(8);
        let (sender, mut events) = mpsc::channel(64);
        let mut engine = Engine::new(service.clone(), sender);
        if available {
            engine = engine.with_editor(service.clone());
        }
        let task = tokio::spawn(engine.run(receiver));
        let resolved = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let path = std::env::current_dir()
            .unwrap()
            .join("exact 文档 & $HOME.txt");
        let cancel = Cancellation::default();
        if cancelled {
            cancel.cancel();
        }
        commands
            .send(Command::EditCapturedFile {
                request: DesktopRequestId(67),
                source: Arc::new(Captured {
                    path: path.clone(),
                    cancel_during_resolve: during,
                    resolved: resolved.clone(),
                    dropped: dropped.clone(),
                }),
                editor: EditorPreferences::default(),
                cancel,
            })
            .await
            .unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::DesktopActionFinished { request, result } = event {
                assert_eq!(request, DesktopRequestId(67));
                let error = result.unwrap_err();
                if available && !cancelled && !during {
                    assert!(error.to_string().contains("configured program missing"));
                }
                break;
            }
        }
        let owner = std::thread::current().id();
        {
            let resolution = resolved.lock().unwrap();
            assert_eq!(resolution.len(), usize::from(available && !cancelled));
            assert!(resolution.iter().all(|thread| *thread != owner));
        }
        {
            let drops = dropped.lock().unwrap();
            assert_eq!(drops.len(), 1, "source must retire before terminal result");
            assert_ne!(drops[0], owner);
        }
        {
            let calls = service.calls.lock().unwrap();
            assert_eq!(calls.len(), usize::from(available && !cancelled && !during));
            if let Some((actual, _, thread)) = calls.first() {
                assert_eq!(actual, &path);
                assert_ne!(*thread, owner);
            }
        }
        commands.send(Command::Shutdown).await.unwrap();
        drop(events);
        task.await.unwrap();
    }
}
