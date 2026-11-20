//! A second transfer starts beside a running one that copies thousands of small files.
//! Admission never waits for the busy copy; only an overlapping item waits (with its
//! reason), and one disk slot is shared step by step instead of job by job.
use super::*;
use std::ffi::OsStr;

/// Enough small files that the busy copy outlasts every observation below.
const FILES: usize = 3000;
/// Each staged file takes at least this long, so the busy copy keeps publishing for
/// seconds even on fast storage. The pause is outside any namespace change.
const PACE: Duration = Duration::from_millis(2);
/// Files the busy copy publishes before the second job is submitted.
const PUBLISHED: u64 = 20;
/// Upper bounds for the second job; a broken scheduler waits for the busy copy instead.
const STARTS_WITHIN: Duration = Duration::from_secs(10);
const COMPLETES_WITHIN: Duration = Duration::from_secs(30);
/// How long a waiting job must keep waiting before it is believed to wait.
const STILL_WAITING: Duration = Duration::from_millis(800);

struct Busy {
    _temp: tempfile::TempDir,
    root: PathBuf,
    /// A directory of `FILES` small files: the busy copy's only selected root.
    tree: PathBuf,
    destination: PathBuf,
    single: PathBuf,
}
impl Busy {
    fn new() -> Self {
        let temp = fixture();
        let root = temp.path().to_owned();
        let tree = root.join("many");
        let destination = root.join("target");
        fs::create_dir(&tree).unwrap();
        fs::create_dir(&destination).unwrap();
        for index in 0..FILES {
            fs::write(tree.join(format!("{index:05}.bin")), [index as u8; 64]).unwrap();
        }
        let single = root.join("single.bin");
        fs::write(&single, vec![0x5a; 70_001]).unwrap();
        Self {
            _temp: temp,
            root,
            tree,
            destination,
            single,
        }
    }
    fn harness(&self, slots: u8) -> Harness {
        Harness::with_slots(
            Some(Arc::new(Paced(midnight_platform::LocalFileSystem::new(
                None,
            )))),
            slots,
        )
    }
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
    async fn start(&self, h: &mut Harness) {
        h.controller
            .enqueue(request(
                1,
                std::slice::from_ref(&self.tree),
                &self.destination,
            ))
            .await;
        until(h, 1, Duration::from_secs(60), |p| {
            p.state == CopyOperationState::Running && p.files_completed >= PUBLISHED
        })
        .await;
    }
}

fn status(h: &Harness, id: u64) -> Option<CopyProgress> {
    let registry = h.controller.registry.lock().unwrap();
    registry
        .get(&OperationId(id))
        .map(|job| snapshot(&job.status))
}

/// Wait until job `id`'s live status satisfies `ready`, draining events meanwhile so no
/// job blocks on a full event queue. An existing folder is merged into, as a user may.
async fn until(
    h: &mut Harness,
    id: u64,
    limit: Duration,
    ready: impl Fn(&CopyProgress) -> bool,
) -> CopyProgress {
    let deadline = std::time::Instant::now() + limit;
    loop {
        drain(h).await;
        if let Some(progress) = status(h, id).filter(|p| ready(p)) {
            return progress;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "job {id} did not reach the expected status: {:?}",
            status(h, id)
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn drain(h: &mut Harness) {
    while let Ok(event) = h.events.try_recv() {
        match event {
            Event::Copy(CopyEvent::Progress(progress)) if progress.state.is_terminal() => {
                h.terminals.insert(progress.id, progress);
            }
            Event::Copy(CopyEvent::Conflict(conflict)) => {
                h.controller
                    .decide(CopyDecision {
                        id: conflict.id,
                        version: conflict.version,
                        choice: if conflict.can_merge {
                            CopyChoice::Merge { all: false }
                        } else {
                            CopyChoice::Skip
                        },
                    })
                    .await;
            }
            Event::Copy(CopyEvent::Failure(failure)) => {
                panic!("an overlapping item must wait, not fail: {failure:?}");
            }
            _ => {}
        }
    }
}

fn still_busy(h: &Harness) -> CopyProgress {
    let busy = status(h, 1);
    assert!(
        busy.as_ref().is_some_and(|p| !p.state.is_terminal()),
        "the busy copy finished first, so nothing ran beside it: {busy:?}"
    );
    busy.unwrap()
}

/// Cancel what still runs and collect the terminals.
async fn finish(mut h: Harness, ids: &[u64]) {
    for id in ids {
        h.controller.cancel(OperationId(*id));
    }
    for id in ids {
        let done = h.terminal(*id).await;
        assert!(done.state.is_terminal());
    }
    h.close().await;
}

async fn second_copy_progresses_beside_the_busy_copy(same_folder: bool, slots: u8) {
    let case = Busy::new();
    let mut h = case.harness(slots);
    case.start(&mut h).await;
    let destination = if same_folder {
        case.destination.clone()
    } else {
        case.folder("other target")
    };
    let submitted = std::time::Instant::now();
    h.controller
        .enqueue(request(2, std::slice::from_ref(&case.single), &destination))
        .await;
    until(&mut h, 2, STARTS_WITHIN, |p| {
        p.state == CopyOperationState::Running || p.state.is_terminal()
    })
    .await;
    let started = submitted.elapsed();
    let done = tokio::time::timeout(COMPLETES_WITHIN, h.terminal(2))
        .await
        .expect("the second copy completes beside the busy copy");
    let busy = still_busy(&h);
    eprintln!(
        "second copy (slots={slots}, same folder={same_folder}) started after {started:?}, \
         finished after {:?}; busy copy published {} files",
        submitted.elapsed(),
        busy.files_completed
    );
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.copied_bytes, 70_001);
    assert_eq!(done.verified_bytes, 70_001);
    assert_eq!(
        fs::read(destination.join("single.bin")).unwrap(),
        vec![0x5a; 70_001]
    );
    // The busy copy kept publishing meanwhile.
    let later = until(&mut h, 1, COMPLETES_WITHIN, |p| {
        p.files_completed > busy.files_completed
    })
    .await;
    assert!(!later.state.is_terminal());
    finish(h, &[1]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn immediate_copy_into_another_folder_progresses_beside_a_busy_copy() {
    second_copy_progresses_beside_the_busy_copy(false, 4).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn immediate_copy_into_the_same_folder_progresses_beside_a_busy_copy() {
    second_copy_progresses_beside_the_busy_copy(true, 4).await;
}

/// With a single disk slot both jobs still progress: the busy copy hands the slot over
/// at its step boundaries instead of keeping it for its whole job.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_disk_slot_interleaves_a_second_copy_with_a_busy_copy() {
    second_copy_progresses_beside_the_busy_copy(false, 1).await;
}

/// "Wait for other operations" still waits for the transfers unfinished at confirm time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wait_for_earlier_copy_still_waits_for_the_busy_copy() {
    let case = Busy::new();
    let mut h = case.harness(4);
    case.start(&mut h).await;
    let destination = case.folder("other target");
    let mut waiting = request(2, std::slice::from_ref(&case.single), &destination);
    waiting.start_policy = TransferStartPolicy::WaitForEarlier;
    h.controller.enqueue(waiting).await;
    until(&mut h, 2, STARTS_WITHIN, |p| {
        p.wait_reason == Some(TransferWaitReason::EarlierTransfers)
    })
    .await;
    tokio::time::sleep(STILL_WAITING).await;
    let queued = until(&mut h, 2, STARTS_WITHIN, |_| true).await;
    assert_eq!(
        queued.wait_reason,
        Some(TransferWaitReason::EarlierTransfers)
    );
    assert_eq!(queued.files_completed, 0);
    assert!(!destination.join("single.bin").exists());
    still_busy(&h);
    h.controller.cancel(OperationId(1));
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    let done = h.terminal(2).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(destination.join("single.bin").is_file());
    h.close().await;
}

/// A copy that also selects a folder of the busy copy's name into the same destination
/// starts at once: its other item completes, and only the shared item waits, with its
/// reason, while the busy copy (here paused, so the merged folder stays unchanged and
/// the user's Merge answer holds) still holds that folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_named_item_waits_alone_while_the_rest_of_the_copy_proceeds() {
    let case = Busy::new();
    let mut h = case.harness(4);
    case.start(&mut h).await;
    h.controller.set_paused(OperationId(1), true).await;
    until(&mut h, 1, STARTS_WITHIN, |p| p.paused).await;
    let again = case.folder("again/many");
    fs::write(
        again.join("extra.bin"),
        b"another file of the same folder name",
    )
    .unwrap();
    h.controller
        .enqueue(request(
            2,
            &[case.single.clone(), again.clone()],
            &case.destination,
        ))
        .await;
    let waiting = until(&mut h, 2, STARTS_WITHIN, |p| {
        p.wait_reason == Some(TransferWaitReason::SameFolders)
    })
    .await;
    assert_eq!(waiting.state, CopyOperationState::Running, "{waiting:?}");
    assert_eq!(
        waiting.completed_roots, 1,
        "the other item was copied first"
    );
    assert_eq!(
        fs::read(case.destination.join("single.bin")).unwrap(),
        vec![0x5a; 70_001]
    );
    tokio::time::sleep(STILL_WAITING).await;
    let still = until(&mut h, 2, STARTS_WITHIN, |_| true).await;
    assert_eq!(still.wait_reason, Some(TransferWaitReason::SameFolders));
    assert!(!case.destination.join("many/extra.bin").exists());
    still_busy(&h);
    finish(h, &[2, 1]).await;
}

/// Reading a folder that the busy copy is still writing is not interleaved item by item:
/// that copy waits as a whole, with its reason, and runs once the busy copy is gone (its
/// planned source folder changed meanwhile, which it reports).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reading_the_busy_copys_output_waits_for_it_with_its_reason() {
    let case = Busy::new();
    let mut h = case.harness(4);
    case.start(&mut h).await;
    let destination = case.folder("copy of output");
    h.controller
        .enqueue(request(2, &[case.destination.join("many")], &destination))
        .await;
    let waiting = until(&mut h, 2, STARTS_WITHIN, |p| {
        p.wait_reason == Some(TransferWaitReason::SameFolders)
    })
    .await;
    assert_ne!(waiting.state, CopyOperationState::Running, "{waiting:?}");
    assert_eq!(waiting.files_completed, 0);
    still_busy(&h);
    h.controller.cancel(OperationId(1));
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    let done = h.terminal(2).await;
    assert!(
        matches!(
            done.state,
            CopyOperationState::Completed | CopyOperationState::CompletedWithErrors
        ),
        "{done:?}"
    );
    h.close().await;
}

/// A Copy as the Linux dialog submits it by default: following links.
fn followed(id: u64, sources: &[PathBuf], destination: &Path) -> CopyOptionsRequest {
    CopyOptionsRequest {
        input: CopyOptionsInput::Native(MetadataTransferRequest {
            input: MetadataTransferInput::Paths(request(id, sources, destination)),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::ContentOnly,
        }),
        links: CopyLinkPolicy::FollowLinks,
    }
}

/// Start the busy copy of `many` into the destination, following links, and pause it.
async fn start_paused_followed(case: &Busy, h: &mut Harness) {
    h.controller
        .enqueue_options(followed(
            1,
            std::slice::from_ref(&case.tree),
            &case.destination,
        ))
        .await;
    until(h, 1, Duration::from_secs(60), |p| {
        p.state == CopyOperationState::Running && p.files_completed >= PUBLISHED
    })
    .await;
    h.controller.set_paused(OperationId(1), true).await;
    until(h, 1, STARTS_WITHIN, |p| p.paused).await;
}

/// A paused copy of one folder (following links, the default) holds nothing that stops
/// a copy of a sibling folder into another folder: that job starts and completes at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paused_followed_copy_does_not_hold_back_a_sibling_folder_copy() {
    let case = Busy::new();
    let mut h = case.harness(4);
    start_paused_followed(&case, &mut h).await;
    let small = case.folder("small");
    fs::write(small.join("a.bin"), [1; 4096]).unwrap();
    fs::write(small.join("b.bin"), [2; 4096]).unwrap();
    let t2 = case.folder("t2");
    let submitted = std::time::Instant::now();
    h.controller
        .enqueue_options(followed(2, std::slice::from_ref(&small), &t2))
        .await;
    let done = tokio::time::timeout(COMPLETES_WITHIN, h.terminal(2))
        .await
        .expect("the sibling copy completes while the other copy is paused");
    eprintln!(
        "sibling copy finished {:?} after it was submitted",
        submitted.elapsed()
    );
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 2);
    assert_eq!(fs::read(t2.join("small/a.bin")).unwrap(), [1; 4096]);
    assert_eq!(fs::read(t2.join("small/b.bin")).unwrap(), [2; 4096]);
    let paused = still_busy(&h);
    assert!(paused.paused, "{paused:?}");
    finish(h, &[1]).await;
}

/// Following links does not give up the selected folder's own claim: a second job that
/// writes into that folder starts, but its write there waits, with its reason, while the
/// first job (here paused) still holds the folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn followed_source_folder_still_holds_back_a_write_into_it() {
    let case = Busy::new();
    let mut h = case.harness(4);
    start_paused_followed(&case, &mut h).await;
    h.controller
        .enqueue(request(2, std::slice::from_ref(&case.single), &case.tree))
        .await;
    let waiting = until(&mut h, 2, STARTS_WITHIN, |p| {
        p.wait_reason == Some(TransferWaitReason::SameFolders)
    })
    .await;
    assert_eq!(waiting.state, CopyOperationState::Running, "{waiting:?}");
    tokio::time::sleep(STILL_WAITING).await;
    let still = until(&mut h, 2, STARTS_WITHIN, |_| true).await;
    assert_eq!(still.wait_reason, Some(TransferWaitReason::SameFolders));
    assert!(!case.tree.join("single.bin").exists());
    still_busy(&h);
    finish(h, &[2, 1]).await;
}

/// The native provider, with `PACE` before each staged file. Everything else, including
/// every namespace pin and change, is the provider's own.
struct Paced(midnight_platform::LocalFileSystem);
// Every native owner below is the local provider's own, destroyed synchronously.
impl midnight_core::AuditedSynchronousTransferService for Paced {}
impl TransferService for Paced {
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_checksum_directory(path, cancel)
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0
            .prepare_transfer_source_with_links(path, links, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0
            .prepare_transfer_destination(path, cancel)
            .map(PacedDirectory::wrap)
    }
}

struct PacedDirectory(Arc<dyn TransferDirectory>);
impl PacedDirectory {
    fn wrap(inner: Arc<dyn TransferDirectory>) -> Arc<dyn TransferDirectory> {
        Arc::new(Self(inner))
    }
}
impl TransferDirectory for PacedDirectory {
    fn native_child_namespace_scope(
        &self,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.0.native_child_namespace_scope(name, kind, cancel)
    }
    fn admit_link(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.0.admit_link(representation, cancel)
    }
    fn stage_link(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferLink>, TransferLinkStageError> {
        self.0.stage_link(representation, cancel)
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.0.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.0.capabilities()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn set_published_permissions(
        &self,
        unix_mode: u32,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.0.set_published_permissions(unix_mode, cancel)
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.0.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.0.prepare_target(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.0.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.create_directory(name, cancel).map(Self::wrap)
    }
    fn open_merge_directory(
        &self,
        name: &OsStr,
        expected: &TransferEntry,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0
            .open_merge_directory(name, expected, cancel)
            .map(Self::wrap)
    }
    fn open_read_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0.open_read_child(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        std::thread::sleep(PACE);
        self.0.stage_file(cancel)
    }
    fn stage_file_with_metadata(
        &self,
        policy: TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        std::thread::sleep(PACE);
        self.0.stage_file_with_metadata(policy, cancel)
    }
    fn stage_file_for_target(
        &self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.0.stage_file_for_target(name, target, policy, cancel)
    }
    fn stage_file_preserving_target(
        &self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.0
            .stage_file_preserving_target(name, target, policy, cancel)
    }
}
