use super::*;

#[derive(Debug)]
struct Paths(Vec<PathBuf>, TransferIntent);
impl CapturedTransferSources for Paths {
    fn intent(&self) -> TransferIntent {
        self.1
    }
    fn root_count(&self) -> usize {
        self.0.len()
    }
    fn metadata_available(&self) -> bool {
        true
    }
    fn prepare(
        &self,
        service: &dyn TransferService,
        cancel: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        match self.1 {
            TransferIntent::Copy => self
                .0
                .iter()
                .map(|path| service.prepare_transfer_source(path, cancel))
                .collect::<Result<_, _>>()
                .map(PreparedCapturedTransfer::Copy),
            TransferIntent::Move => self
                .0
                .iter()
                .map(|path| service.prepare_move_source(path, cancel))
                .collect::<Result<_, _>>()
                .map(PreparedCapturedTransfer::Move),
        }
    }
}
fn captured(paths: Vec<PathBuf>, destination: PathBuf) -> CapturedTransferRequest {
    CapturedTransferRequest {
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        sources: Arc::new(Paths(paths, TransferIntent::Copy)),
        destination,
        metadata: TransferMetadataPolicy::ContentOnly,
    }
}
#[tokio::test]
async fn captured_native_ancestor_and_child_reject_before_any_destination_writes() {
    let temp = fixture();
    let parent = temp.path().join("parent");
    let child = parent.join("child");
    let destination = temp.path().join("destination");
    fs::create_dir_all(&child).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(child.join("payload"), b"original").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue_captured(captured(vec![parent, child], destination.clone()))
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    h.close().await;
}
#[tokio::test]
async fn captured_native_destination_below_source_rejects_before_writes() {
    let temp = fixture();
    let source = temp.path().join("parent");
    let destination = source.join("destination");
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("payload"), b"original").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue_captured(captured(vec![source], destination.clone()))
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    h.close().await;
}
#[tokio::test]
async fn captured_native_alias_duplicate_rejects_before_writes() {
    let temp = fixture();
    let first = temp.path().join("first");
    let alias = temp.path().join("alias");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&first, b"original").unwrap();
    fs::hard_link(&first, &alias).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue_captured(captured(vec![first, alias], destination.clone()))
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    h.close().await;
}
#[tokio::test]
async fn captured_native_regular_file_preserves_optional_modified_time() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"verified content").unwrap();
    let modified = std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let expected = fs::metadata(&source).unwrap().modified().unwrap();
    let mut request = captured(vec![source], destination.clone());
    request.metadata = TransferMetadataPolicy::RequireModifiedTime;
    let mut h = Harness::native();
    h.controller.enqueue_captured(request).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        fs::read(destination.join("source")).unwrap(),
        b"verified content"
    );
    assert_eq!(
        fs::metadata(destination.join("source"))
            .unwrap()
            .modified()
            .unwrap(),
        expected
    );
    h.close().await;
}

#[derive(Debug)]
struct DropThread(Arc<Mutex<Option<std::thread::ThreadId>>>);
impl Drop for DropThread {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = Some(std::thread::current().id());
    }
}
impl CapturedTransferSources for DropThread {
    fn intent(&self) -> TransferIntent {
        TransferIntent::Copy
    }
    fn root_count(&self) -> usize {
        1
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        _: &dyn TransferService,
        _: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        panic!("rejected/cancelled capture must not prepare")
    }
}
fn tracked(
    id: u64,
    destination: PathBuf,
) -> (
    CapturedTransferRequest,
    Arc<Mutex<Option<std::thread::ThreadId>>>,
) {
    let observed = Arc::new(Mutex::new(None));
    (
        CapturedTransferRequest {
            verification: midnight_core::TransferVerification::default(),
            id: OperationId(id),
            destination,
            sources: Arc::new(DropThread(observed.clone())),
            metadata: TransferMetadataPolicy::ContentOnly,
        },
        observed,
    )
}
async fn assert_retired_off_dispatcher(observed: Arc<Mutex<Option<std::thread::ThreadId>>>) {
    let dispatcher = std::thread::current().id();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(dropped) = *observed.lock().unwrap() {
                assert_ne!(dropped, dispatcher);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "current_thread")]
async fn captured_rejected_queue_full_invalid_and_duplicate_retire_on_blocking_workers() {
    let destination = std::env::current_dir().unwrap();
    let h = Harness::new(None);
    let reservations: Vec<_> = (0..32).map(|_| h.controller.reserve().unwrap()).collect();
    let (request, observed) = tracked(1, destination.clone());
    h.controller.enqueue_captured(request).await;
    assert_retired_off_dispatcher(observed).await;
    drop(reservations);
    let (request, observed) = tracked(2, PathBuf::from("relative"));
    h.controller.enqueue_captured(request).await;
    assert_retired_off_dispatcher(observed).await;
    let (request, observed) = tracked(2, destination);
    h.controller.enqueue_captured(request).await;
    assert_retired_off_dispatcher(observed).await;
    h.close().await;
}
#[tokio::test(flavor = "current_thread")]
async fn captured_waiting_for_mutation_and_missing_service_retire_on_blocking_workers() {
    let destination = std::env::current_dir().unwrap();
    let mut h = Harness::new(None);
    let gate = h.mutation.clone().acquire_owned().await.unwrap();
    let (request, observed) = tracked(1, destination.clone());
    h.controller.enqueue_captured(request).await;
    h.controller.cancel(OperationId(1));
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    assert_retired_off_dispatcher(observed).await;
    drop(gate);
    let (request, observed) = tracked(2, destination);
    h.controller.enqueue_captured(request).await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Failed);
    assert_retired_off_dispatcher(observed).await;
    h.close().await;
}
#[tokio::test(flavor = "current_thread")]
async fn captured_queued_shutdown_retires_pending_sources_on_blocking_worker() {
    let destination = std::env::current_dir().unwrap();
    let mut h = Harness::new(None);
    let _gate = h.mutation.clone().acquire_owned().await.unwrap();
    h.controller
        .enqueue(request(1, &[destination.join("ordinary")], &destination))
        .await;
    tokio::task::yield_now().await;
    let (request, observed) = tracked(2, destination);
    h.controller.enqueue_captured(request).await;
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    assert_retired_off_dispatcher(observed).await;
}

#[tokio::test(flavor = "current_thread")]
async fn captured_active_abort_retires_source_owner_on_blocking_worker() {
    let destination = std::env::current_dir().unwrap();
    let mut h = Harness::new(None);
    let _gate = h.mutation.clone().acquire_owned().await.unwrap();
    let (request, observed) = tracked(1, destination);
    h.controller.enqueue_captured(request).await;
    tokio::task::yield_now().await;
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    assert_retired_off_dispatcher(observed).await;
}

#[derive(Debug)]
struct GatedDrop {
    started: Arc<Notify>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Drop for GatedDrop {
    fn drop(&mut self) {
        self.started.notify_one();
        self.release
            .get_mut()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
    }
}
impl CapturedTransferSources for GatedDrop {
    fn intent(&self) -> TransferIntent {
        TransferIntent::Copy
    }
    fn root_count(&self) -> usize {
        1
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        _: &dyn TransferService,
        _: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        panic!("send rejection must not prepare")
    }
}
#[tokio::test(flavor = "current_thread")]
async fn captured_send_failure_finishes_source_retirement_before_capacity_and_terminal() {
    let mut h = Harness::new(None);
    let reservation = h.controller.reserve().unwrap();
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    let started = Arc::new(Notify::new());
    let (release, released) = std::sync::mpsc::channel();
    let request = CapturedTransferRequest {
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        destination: std::env::current_dir().unwrap(),
        metadata: TransferMetadataPolicy::ContentOnly,
        sources: Arc::new(GatedDrop {
            started: started.clone(),
            release: Mutex::new(released),
        }),
    };
    let capacity = h.controller.capacity();
    let events = &mut h.events;
    let enqueue = h
        .controller
        .enqueue_input_reserved(request.into(), reservation);
    let verify = async {
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let available = capacity.available_permits();
        let mut terminal_early = false;
        while let Ok(event) = events.try_recv() {
            if let Event::Copy(CopyEvent::Progress(progress)) = event {
                terminal_early |= progress.state.is_terminal();
            }
        }
        release.send(()).unwrap();
        assert_eq!(
            available,
            MAX_JOBS - 1,
            "source owner retains admission until retirement finishes"
        );
        assert!(
            !terminal_early,
            "terminal delivery waits for the original provider destructor"
        );
    };
    tokio::join!(enqueue, verify);
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Failed);
    h.close().await;
}

#[tokio::test]
async fn captured_move_nested_native_roots_remove_verified_sources() {
    let (_temp, protected) = moves::directories::tree_fixture();
    let source = protected.join("source");
    let destination = protected.join("target");
    fs::create_dir(&destination).unwrap();
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    fs::write(source.join("nested/payload"), b"verified nested bytes").unwrap();
    let mut h = Harness::native();
    let mut request = captured(vec![source.clone()], destination.clone());
    request.sources = Arc::new(Paths(vec![source.clone()], TransferIntent::Move));
    h.controller.enqueue_captured(request).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        fs::read(destination.join("source/nested/payload")).unwrap(),
        b"verified nested bytes"
    );
    assert!(destination.join("source/nested/empty").is_dir());
    assert!(
        !source.exists(),
        "captured Move must remove the verified source"
    );
    assert_eq!(done.intent, TransferIntent::Move);
    h.close().await;
}

#[derive(Debug)]
struct InvalidPrepared {
    bound: TransferIntent,
    returned: TransferIntent,
    count: usize,
}
impl CapturedTransferSources for InvalidPrepared {
    fn intent(&self) -> TransferIntent {
        self.bound
    }
    fn root_count(&self) -> usize {
        self.count
    }
    fn metadata_available(&self) -> bool {
        false
    }
    fn prepare(
        &self,
        _: &dyn TransferService,
        _: &Cancellation,
    ) -> Result<PreparedCapturedTransfer, FsError> {
        Ok(match self.returned {
            TransferIntent::Copy => PreparedCapturedTransfer::Copy(Default::default()),
            TransferIntent::Move => PreparedCapturedTransfer::Move(Default::default()),
        })
    }
}
#[tokio::test]
async fn captured_prepared_authority_and_count_mismatch_reject_before_destination() {
    let temp = fixture();
    // Missing destination proves that mismatch is reported before destination preparation.
    let destination = temp.path().join("absent");
    let mut h = Harness::native();
    for (id, bound, returned) in [
        (1, TransferIntent::Copy, TransferIntent::Move),
        (2, TransferIntent::Move, TransferIntent::Copy),
        (3, TransferIntent::Move, TransferIntent::Move),
    ] {
        h.controller
            .enqueue_captured(CapturedTransferRequest {
                verification: midnight_core::TransferVerification::default(),
                id: OperationId(id),
                destination: destination.clone(),
                metadata: TransferMetadataPolicy::ContentOnly,
                sources: Arc::new(InvalidPrepared {
                    bound,
                    returned,
                    count: 1,
                }),
            })
            .await;
        let done = h.terminal(id).await;
        assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
        assert!(
            done.errors
                .iter()
                .any(|error| error.kind == std::io::ErrorKind::InvalidData),
            "{done:?}"
        );
        assert!(!destination.exists());
    }
    h.close().await;
}

#[tokio::test]
async fn captured_move_optional_modified_time_is_exact_and_source_removed() {
    let (_temp, destination) = moves::protected_fixture();
    let source = destination.join("original");
    let output = destination.join("out");
    fs::create_dir(&output).unwrap();
    fs::write(&source, b"metadata move").unwrap();
    let modified = std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    fs::File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let expected = fs::metadata(&source).unwrap().modified().unwrap();
    let mut request = captured(vec![source.clone()], output.clone());
    request.sources = Arc::new(Paths(vec![source.clone()], TransferIntent::Move));
    request.metadata = TransferMetadataPolicy::RequireModifiedTime;
    let mut h = Harness::native();
    h.controller.enqueue_captured(request).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(fs::read(output.join("original")).unwrap(), b"metadata move");
    assert_eq!(
        fs::metadata(output.join("original"))
            .unwrap()
            .modified()
            .unwrap(),
        expected
    );
    assert!(!source.exists());
    h.close().await;
}
