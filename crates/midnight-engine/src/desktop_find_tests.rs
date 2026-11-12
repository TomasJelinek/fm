use super::*;
use std::sync::Mutex;
type Calls = Arc<Mutex<Vec<(&'static str, PathBuf, std::thread::ThreadId)>>>;
#[derive(Debug)]
struct Source {
    entry: FindEntry,
    allowed: bool,
    calls: Calls,
    dropped: Option<tokio::sync::oneshot::Sender<std::thread::ThreadId>>,
}
impl CapturedFindFile for Source {
    fn request(&self) -> FindValidationRequest {
        FindValidationRequest {
            session: FindSessionId(1),
            generation: FindGeneration(1),
            result: FindResultId(1),
            token: 1,
        }
    }
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn validate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.calls.lock().unwrap().push((
            "validate",
            self.entry.path.clone(),
            std::thread::current().id(),
        ));
        if self.allowed && !cancel.is_cancelled() {
            Ok(())
        } else {
            Err(find_error(
                &self.entry.path,
                io::ErrorKind::Interrupted,
                "rejected capture",
            ))
        }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        if let Some(sender) = self.dropped.take() {
            let _ = sender.send(std::thread::current().id());
        }
    }
}
struct Native(Calls);
impl OpenService for Native {
    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError> {
        self.0
            .lock()
            .unwrap()
            .push(("open", path.to_owned(), std::thread::current().id()));
        Ok(OpenEntryResult::File { process: None })
    }
}
impl EditorService for Native {
    fn default_editor(&self) -> Result<EditorCommand, FsError> {
        unreachable!()
    }
    fn launch_editor(
        &self,
        path: &Path,
        _: &EditorPreferences,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        self.0
            .lock()
            .unwrap()
            .push(("edit", path.to_owned(), std::thread::current().id()));
        Err(find_error(
            path,
            io::ErrorKind::Other,
            "simulated editor launch",
        ))
    }
}
impl IconService for Native {
    fn load_icon(&self, _: &IconKey) -> Result<Option<IconPixels>, FsError> {
        Ok(None)
    }
}
impl TerminalService for Native {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        unreachable!()
    }
}
impl PropertyService for Native {
    fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
        unreachable!()
    }
}
impl LocationService for Native {
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        Ok(Vec::new())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn find_open_and_edit_validate_in_launch_worker_and_preserve_raw_path() {
    for editing in [false, true] {
        for allowed in [false, true] {
            let calls = Calls::default();
            let native = Arc::new(Native(calls.clone()));
            let (sender, mut events) = mpsc::channel(16);
            let mut tasks = JoinSet::new();
            let desktop = Desktop::testing(
                Some(native.clone()),
                None,
                Outbox {
                    sender,
                    wakeup: Arc::new(|| {}),
                },
                &mut tasks,
            );
            let path = std::env::temp_dir().join("literal $() 文档.txt");
            let (dropped, drop_result) = tokio::sync::oneshot::channel();
            let source = Arc::new(Source {
                entry: FindEntry {
                    path: path.clone(),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                    identity: TransferIdentity {
                        device: 1,
                        object: [1; 16],
                    },
                    revision: FindRevision([1; 6]),
                },
                allowed,
                calls: calls.clone(),
                dropped: Some(dropped),
            });
            desktop
                .execute_find_file(
                    Some(native),
                    DesktopRequestId(1),
                    source,
                    if editing {
                        FindFileOperation::Edit(EditorPreferences::default())
                    } else {
                        FindFileOperation::Open
                    },
                    Cancellation::default(),
                    &mut tasks,
                )
                .await;
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if matches!(
                        events.recv().await,
                        Some(Event::DesktopActionFinished {
                            request: DesktopRequestId(1),
                            ..
                        })
                    ) {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            let calls = calls.lock().unwrap().clone();
            assert_eq!(calls.len(), if allowed { 2 } else { 1 });
            assert_eq!(calls[0].0, "validate");
            if allowed {
                assert_eq!(calls[1].0, if editing { "edit" } else { "open" });
            }
            for (_, actual, worker) in calls {
                assert_eq!(actual, path);
                assert_ne!(worker, std::thread::current().id());
            }
            assert_ne!(drop_result.await.unwrap(), std::thread::current().id());
            tasks.abort_all();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn aborted_find_action_delivery_still_retires_capture_on_worker() {
    let (sender, _events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let native = Arc::new(Native(Calls::default()));
    let desktop = Desktop::testing(
        Some(native.clone()),
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let (dropped, result) = tokio::sync::oneshot::channel();
    let source = Arc::new(Source {
        entry: FindEntry {
            path: std::env::temp_dir().join("file"),
            kind: EntryKind::File,
            metadata: EntryMetadata::default(),
            identity: TransferIdentity {
                device: 1,
                object: [1; 16],
            },
            revision: FindRevision([0; 6]),
        },
        allowed: false,
        calls: Calls::default(),
        dropped: Some(dropped),
    });
    desktop
        .execute_find_file(
            Some(native),
            DesktopRequestId(1),
            source,
            FindFileOperation::Open,
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let thread = tokio::time::timeout(Duration::from_secs(5), result)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(thread, std::thread::current().id());
}
