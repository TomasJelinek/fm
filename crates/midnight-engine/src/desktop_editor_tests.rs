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
    let desktop = Desktop::new(None, None, outbox, &mut tasks);
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
