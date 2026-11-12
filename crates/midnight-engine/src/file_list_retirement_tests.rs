use super::*;
use std::{ffi::OsStr, path::Path, sync::mpsc, thread, time::Duration};

struct Probe(mpsc::Sender<thread::ThreadId>);
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}
struct Target {
    // Observe after the real native target pin has been destroyed.
    inner: Arc<dyn TransferTarget>,
    _probe: Probe,
}
impl TransferTarget for Target {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
}
struct Directory {
    inner: Arc<dyn TransferDirectory>,
    dropped: mpsc::Sender<thread::ThreadId>,
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.inner.prepare_target(name, cancel).map(|target| {
            target.map(|inner| {
                Arc::new(Target {
                    inner,
                    _probe: Probe(self.dropped.clone()),
                }) as Arc<dyn TransferTarget>
            })
        })
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("no directory mutation")
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("conflict must not write before confirmation")
    }
}
struct Native {
    filesystem: midnight_platform::LocalFileSystem,
    dropped: mpsc::Sender<thread::ThreadId>,
}
impl TransferService for Native {
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("generation never reads source files")
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.filesystem
            .prepare_transfer_destination(path, cancel)
            .map(|inner| {
                Arc::new(Directory {
                    inner,
                    dropped: self.dropped.clone(),
                }) as Arc<dyn TransferDirectory>
            })
    }
}
struct Fixture {
    root: tempfile::TempDir,
    controller: Controller,
    events: tokio::sync::mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    drops: mpsc::Receiver<thread::ThreadId>,
}
impl Fixture {
    fn new() -> Self {
        let parent = std::env::temp_dir().join("file-list-lifetime/_build/tmp");
        std::fs::create_dir_all(&parent).unwrap();
        let root = tempfile::tempdir_in(parent.canonicalize().unwrap()).unwrap();
        std::fs::write(root.path().join("list.txt"), b"original").unwrap();
        let (dropped, drops) = mpsc::channel();
        let service = Arc::new(Native {
            filesystem: midnight_platform::LocalFileSystem::new(None),
            dropped,
        });
        let (sender, events) = tokio::sync::mpsc::channel(16);
        let controller = Controller::testing(
            Some(service),
            Arc::new(Semaphore::new(1)),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            crate::test_snapshots(Default::default()),
        );
        Self {
            root,
            controller,
            events,
            tasks: JoinSet::new(),
            drops,
        }
    }
    async fn conflict(&mut self) {
        let mut request = super::tests::request(1, 1);
        request.destination = FileListDestination::File {
            path: self.root.path().join("list.txt"),
            append: false,
        };
        self.controller.start(request, &mut self.tasks).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), self.events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(event, Event::FileList(ref value)
            if matches!(value.result, Ok(FileListUpdate::Conflict { .. }))),
            "{event:?}"
        );
        while self.tasks.join_next().await.is_some() {}
    }
}
#[tokio::test(flavor = "current_thread")]
async fn file_list_retirement_conflict_cancel_replace_shutdown_and_drop_leave_native_cleanup_off_executor()
 {
    let caller = thread::current().id();
    let mut observed = Vec::new();
    for mode in 0..4 {
        let mut fixture = Fixture::new();
        fixture.conflict().await;
        let Fixture {
            root,
            mut controller,
            mut events,
            mut tasks,
            drops,
        } = fixture;
        match mode {
            0 => controller.cancel(DesktopRequestId(1), 1),
            1 => {
                controller
                    .start(super::tests::request(1, 2), &mut tasks)
                    .unwrap();
            }
            2 => controller.shutdown(),
            _ => {
                drop(controller);
                observed.push(drops.recv_timeout(Duration::from_secs(5)).unwrap());
                assert_eq!(
                    std::fs::read(root.path().join("list.txt")).unwrap(),
                    b"original"
                );
                continue;
            }
        }
        observed.push(drops.recv_timeout(Duration::from_secs(5)).unwrap());
        while tasks.join_next().await.is_some() {}
        while events.try_recv().is_ok() {}
        assert_eq!(
            std::fs::read(root.path().join("list.txt")).unwrap(),
            b"original"
        );
    }
    assert!(
        observed.iter().all(|on| *on != caller),
        "native pin drops: {observed:?}; caller: {caller:?}"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn file_list_retirement_confirmation_aborted_before_first_poll_keeps_native_pin_guarded() {
    let mut fixture = Fixture::new();
    fixture.conflict().await;
    fixture
        .controller
        .confirm(DesktopRequestId(1), 1, &mut fixture.tasks)
        .unwrap();
    // No suspension before abort: the confirmed task has never been polled.
    fixture.controller.cancel(DesktopRequestId(1), 1);
    fixture.tasks.abort_all();
    while fixture.tasks.join_next().await.is_some() {}
    let on = fixture.drops.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(
        on,
        thread::current().id(),
        "unpolled confirmation dropped native pin on executor"
    );
    assert_eq!(
        std::fs::read(fixture.root.path().join("list.txt")).unwrap(),
        b"original"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn retained_generated_outputs_bound_admission_and_preserve_the_prior_generation() {
    let (mut controller, mut events, mut tasks) = super::tests::controller();
    let mut outputs = Vec::new();
    for id in 1..=MAX_RETAINED as u64 {
        let mut request = super::tests::request(id, 1);
        request.destination = FileListDestination::Viewer;
        controller.start(request, &mut tasks).unwrap();
        let Event::FileList(event) = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("file-list result")
        };
        let FileListUpdate::Generated { content, .. } = event.result.unwrap() else {
            panic!("generated content")
        };
        outputs.push(content);
        if id != 1 {
            controller.cancel(DesktopRequestId(id), 1);
        }
        while tasks.join_next().await.is_some() {}
    }
    assert_eq!(
        controller.jobs.len(),
        1,
        "only the original visible generation remains"
    );
    assert_eq!(controller.budget.available_permits(), MAX_WORKERS);
    assert_eq!(controller.retained.available_permits(), 0);
    let failure = controller
        .start(super::tests::request(1, 2), &mut tasks)
        .unwrap_err();
    assert!(failure.to_string().contains("outputs are still in use"));
    assert_eq!(controller.jobs[&DesktopRequestId(1)].request.generation, 1);
    assert!(!controller.jobs[&DesktopRequestId(1)].cancel.is_cancelled());
    // Refusal neither consumes nor closes an already accepted viewer/clipboard output.
    let source = outputs[0].clone();
    let bytes = tokio::task::spawn_blocking(move || {
        let mut bytes = vec![0; source.length() as usize];
        assert_eq!(source.read_at(0, &mut bytes).unwrap(), bytes.len());
        bytes
    })
    .await
    .unwrap();
    assert_eq!(bytes, "文档.txt\n".as_bytes());
    drop(outputs.pop());
    tokio::time::timeout(Duration::from_secs(5), async {
        while controller.retained.available_permits() == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    controller
        .start(super::tests::request(1, 2), &mut tasks)
        .unwrap();
    while tasks.join_next().await.is_some() {}
    let Event::FileList(event) = events.recv().await.unwrap() else {
        panic!("preview")
    };
    assert_eq!(event.generation, 2);
    assert!(matches!(event.result, Ok(FileListUpdate::Preview { .. })));
    controller.shutdown();
    drop(outputs);
    tokio::time::timeout(Duration::from_secs(5), async {
        while controller.retained.available_permits() != MAX_RETAINED {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
