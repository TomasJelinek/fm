//! A second transfer starts beside a running one that copies hundreds of small files.
//! Admission never waits for the busy copy; only an overlapping item waits (with its
//! reason), and one disk slot is shared step by step instead of job by job.
use super::*;
use std::ffi::OsStr;

/// Enough small files that the busy copy outlasts every observation below: with `PACE`,
/// at least six seconds even on fast storage. Few files keep the fixture cheap to create
/// and remove; the pace, not their number, makes the copy last.
const FILES: usize = 600;
/// Each staged file takes at least this long, so the busy copy keeps publishing for
/// seconds even on fast storage. The pause is outside any namespace change.
const PACE: Duration = Duration::from_millis(10);
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
        require_exact_lookup(&tree);
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

/// Jobs run side by side only where the platform reports exact lookup keys for their
/// folders. Elsewhere (on Windows, a volume that still creates 8.3 short names, such as
/// the system drive) every Copy takes the opaque lane and the others wait for it, by
/// design: these tests then fail at once with that reason instead of timing out.
fn require_exact_lookup(folder: &Path) {
    let cancel = Cancellation::default();
    let source = midnight_platform::LocalFileSystem::new(None)
        .prepare_transfer_source(folder, &cancel)
        .unwrap();
    let probe = source
        .native_namespace_scope(&cancel)
        .unwrap()
        .map(|pin| pin.refresh(&cancel).unwrap());
    assert!(
        matches!(&probe, Some(TransferNamespaceProbe::Known(scopes)) if !scopes.is_empty()),
        "this scheduler fixture requires a supported local lookup domain: on Windows, TEMP \
         and TMP on an NTFS volume with persistent 8.3 short-name creation disabled \
         (docs/developer/TEST_EXECUTION.md); elsewhere every Copy takes the opaque lane"
    );
}

fn status(h: &Harness, id: u64) -> Option<CopyProgress> {
    let registry = h.controller.registry.lock().unwrap();
    registry
        .get(&OperationId(id))
        .map(|job| snapshot(&job.status))
}

/// Wait until job `id`'s live status satisfies `ready`, draining events meanwhile so no
/// job blocks on a full event queue. An existing folder is merged into, as a user may.
/// A job that has already ended is judged by its terminal progress at once, instead of
/// waiting out `limit` for a status that can no longer change.
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
        if let Some(done) = h.terminals.get(&OperationId(id)) {
            assert!(
                ready(done),
                "job {id} ended before reaching the expected status: {done:?}"
            );
            return (**done).clone();
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

/// "Wait for earlier transfers" still waits for the transfers unfinished at confirm time.
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
/// that copy waits as a whole, with its reason, and once the busy copy is gone copies what
/// the folder then holds. The busy copy changed the folder (its modification time) after
/// the waiting copy planned it, which a planned folder may: it is still the same folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reading_the_busy_copys_output_waits_for_it_with_its_reason() {
    let case = Busy::new();
    let h = case.harness(4);
    reading_the_busy_output_waits(&case, h).await;
}

/// The copy that waits to read the busy copy's output prepares that folder before it is
/// admitted (and keeps what it prepared while it waits, where lookups are not qualified).
/// Here the busy copy publishes into the folder while that prepared source exists, on
/// every run instead of only when the timing happens to overlap: nothing that preparation
/// holds may refuse it (Windows refuses a rename into a folder while any handle on that
/// folder denies write sharing).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_busy_copy_publishes_into_output_that_a_waiting_copy_prepared() {
    let case = Busy::new();
    let service = Arc::new(PublishedMeanwhile {
        paced: Paced(midnight_platform::LocalFileSystem::new(None)),
        output: case.destination.join("many"),
        armed: std::sync::atomic::AtomicBool::new(true),
    });
    let h = Harness::with_slots(Some(service), 4);
    reading_the_busy_output_waits(&case, h).await;
}

async fn reading_the_busy_output_waits(case: &Busy, mut h: Harness) {
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
    // Whether lookups on this volume are qualified (so the folder was planned and is
    // checked again when the copy starts) or not.
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    let names = |folder: &Path| {
        let mut names: Vec<_> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        names(&destination.join("many")),
        names(&case.destination.join("many")),
        "{done:?}"
    );
    h.close().await;
}

/// A Copy as the dialog submits it with the link handling `links`.
fn submitted(
    id: u64,
    sources: &[PathBuf],
    destination: &Path,
    links: CopyLinkPolicy,
) -> CopyOptionsRequest {
    CopyOptionsRequest {
        input: CopyOptionsInput::Native(MetadataTransferRequest {
            input: MetadataTransferInput::Paths(request(id, sources, destination)),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::ContentOnly,
        }),
        links,
    }
}

/// A Copy following directory links and junctions, as Windows' dialog did before its
/// Links choice (Linux: following every link).
const FOLLOWED: CopyLinkPolicy = if cfg!(windows) {
    CopyLinkPolicy::FollowDirectoryContents
} else {
    CopyLinkPolicy::FollowLinks
};

/// Start the busy copy of `many` into the destination with `links`, and pause it.
async fn start_paused(case: &Busy, h: &mut Harness, links: CopyLinkPolicy) {
    h.controller
        .enqueue_options(submitted(
            1,
            std::slice::from_ref(&case.tree),
            &case.destination,
            links,
        ))
        .await;
    until(h, 1, Duration::from_secs(60), |p| {
        p.state == CopyOperationState::Running && p.files_completed >= PUBLISHED
    })
    .await;
    h.controller.set_paused(OperationId(1), true).await;
    until(h, 1, STARTS_WITHIN, |p| p.paused).await;
}

/// A paused copy of one folder holds nothing that stops a copy of a sibling folder into
/// another folder, whatever the Links choice of both: that job starts and completes at
/// once.
async fn paused_copy_does_not_hold_back_a_sibling_folder_copy(links: CopyLinkPolicy) {
    let case = Busy::new();
    let mut h = case.harness(4);
    start_paused(&case, &mut h, links).await;
    let small = case.folder("small");
    fs::write(small.join("a.bin"), [1; 4096]).unwrap();
    fs::write(small.join("b.bin"), [2; 4096]).unwrap();
    let t2 = case.folder("t2");
    let submitted_at = std::time::Instant::now();
    h.controller
        .enqueue_options(submitted(2, std::slice::from_ref(&small), &t2, links))
        .await;
    let done = tokio::time::timeout(COMPLETES_WITHIN, h.terminal(2))
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{links:?}: the sibling copy completes while the other copy is paused: {:?}",
                status(&h, 2)
            )
        });
    eprintln!(
        "{links:?}: sibling copy finished {:?} after it was submitted",
        submitted_at.elapsed()
    );
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 2);
    assert_eq!(fs::read(t2.join("small/a.bin")).unwrap(), [1; 4096]);
    assert_eq!(fs::read(t2.join("small/b.bin")).unwrap(), [2; 4096]);
    let paused = still_busy(&h);
    assert!(paused.paused, "{paused:?}");
    finish(h, &[1]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paused_followed_copy_does_not_hold_back_a_sibling_folder_copy() {
    paused_copy_does_not_hold_back_a_sibling_folder_copy(FOLLOWED).await;
}

/// "Keep links as links", the dialog's default on a fixed local drive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paused_copy_keeping_links_does_not_hold_back_a_sibling_folder_copy() {
    paused_copy_does_not_hold_back_a_sibling_folder_copy(CopyLinkPolicy::Preserve).await;
}

/// "Copy the files links point to".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paused_copy_of_the_files_links_point_to_does_not_hold_back_a_sibling_folder_copy() {
    paused_copy_does_not_hold_back_a_sibling_folder_copy(CopyLinkPolicy::FollowLinks).await;
}

/// Following links does not give up the selected folder's own claim: a second job that
/// writes into that folder starts, but its write there waits, with its reason, while the
/// first job (here paused) still holds the folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn followed_source_folder_still_holds_back_a_write_into_it() {
    let case = Busy::new();
    let mut h = case.harness(4);
    start_paused(&case, &mut h, FOLLOWED).await;
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

/// Why the paused copy holds back a second copy as a whole.
#[derive(Clone, Copy, Debug)]
enum Held {
    /// The second copy reads what the paused copy wrote: the same folders.
    ReadsItsOutput,
    /// The paused copy writes into a folder that cannot confirm its names (as a Windows
    /// volume that still creates 8.3 short names), so it runs alone.
    PausedUnconfirmed,
    /// The paused copy's names were latched unconfirmed, as by a failed check.
    PausedLatched,
    /// The second copy writes into a folder that cannot confirm its names.
    OwnUnconfirmed,
}

/// A copy held back by the paused copy waits as a whole and says why: the same folders
/// only when it uses what that copy uses; otherwise one of the two cannot confirm its
/// folders, so that one runs alone, whichever folders they use. It still starts once the
/// paused copy is gone.
async fn held_back_by_the_paused_copy(held: Held) {
    let case = Busy::new();
    let t2 = case.folder("t2");
    let unconfirmed = match held {
        Held::PausedUnconfirmed => Some(case.destination.clone()),
        Held::OwnUnconfirmed => Some(t2.clone()),
        Held::ReadsItsOutput | Held::PausedLatched => None,
    };
    let mut h = match unconfirmed {
        Some(folder) => Harness::with_slots(
            Some(Arc::new(Unconfirmed {
                paced: Paced(midnight_platform::LocalFileSystem::new(None)),
                folder,
            })),
            4,
        ),
        None => case.harness(4),
    };
    start_paused(&case, &mut h, CopyLinkPolicy::Preserve).await;
    if let Held::PausedLatched = held {
        let granted = h.controller.namespace().granted_for_test();
        assert_eq!(granted.len(), 1, "{granted:?}");
        h.controller.namespace().latch_opaque_for_test(granted[0]);
    }
    let (source, expected) = if let Held::ReadsItsOutput = held {
        let output = case.destination.join("many");
        (output, TransferWaitReason::SameFolders)
    } else {
        let small = case.folder("small");
        fs::write(small.join("a.bin"), [1; 4096]).unwrap();
        (small, TransferWaitReason::UnconfirmedFolders)
    };
    h.controller
        .enqueue(request(2, std::slice::from_ref(&source), &t2))
        .await;
    let waiting = until(&mut h, 2, STARTS_WITHIN, |p| {
        matches!(
            p.wait_reason,
            Some(TransferWaitReason::SameFolders | TransferWaitReason::UnconfirmedFolders)
        )
    })
    .await;
    assert_eq!(waiting.wait_reason, Some(expected), "{held:?}: {waiting:?}");
    assert_ne!(
        waiting.state,
        CopyOperationState::Running,
        "{held:?}: {waiting:?}"
    );
    tokio::time::sleep(STILL_WAITING).await;
    let still = until(&mut h, 2, STARTS_WITHIN, |_| true).await;
    assert_eq!(still.wait_reason, Some(expected), "{held:?}: {still:?}");
    assert_eq!(still.files_completed, 0, "{held:?}: {still:?}");
    assert!(still_busy(&h).paused);
    h.controller.cancel(OperationId(1));
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    let done = h.terminal(2).await;
    assert_eq!(
        done.state,
        CopyOperationState::Completed,
        "{held:?}: {done:?}"
    );
    assert!(done.files_completed > 0, "{held:?}: {done:?}");
    h.close().await;
}

/// A real overlap still says that the jobs use the same folders.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reading_a_paused_copys_output_says_it_waits_for_the_same_folders() {
    held_back_by_the_paused_copy(Held::ReadsItsOutput).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_copy_into_unconfirmed_folders_holds_back_a_sibling_and_says_why() {
    held_back_by_the_paused_copy(Held::PausedUnconfirmed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_copy_latched_unconfirmed_holds_back_a_sibling_and_says_why() {
    held_back_by_the_paused_copy(Held::PausedLatched).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_into_unconfirmed_folders_waits_for_a_paused_sibling_and_says_why() {
    held_back_by_the_paused_copy(Held::OwnUnconfirmed).await;
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

/// `Paced`, except that `folder` cannot confirm the names written into it: it reports no
/// lookup keys for them, as a Windows volume that still creates 8.3 short names does.
/// A job that writes there has an unknown footprint, so it runs alone.
struct Unconfirmed {
    paced: Paced,
    folder: PathBuf,
}
// Every native owner below is the local provider's own, destroyed synchronously.
impl midnight_core::AuditedSynchronousTransferService for Unconfirmed {}
impl TransferService for Unconfirmed {
    /// The destination a job plans its claims with (its read-only probe).
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        let directory = self.paced.prepare_checksum_directory(path, cancel)?;
        Ok(PacedDirectory::wrap_as(directory, path == self.folder))
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.paced.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.paced
            .prepare_transfer_source_with_links(path, links, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        let directory = self.paced.0.prepare_transfer_destination(path, cancel)?;
        Ok(PacedDirectory::wrap_as(directory, path == self.folder))
    }
}

/// `Paced`, except that the first preparation of `output` as a source holds that prepared
/// source until the busy copy has published two more files into `output` (or for five
/// seconds: a refused publication fails the busy copy, which `drain` reports).
struct PublishedMeanwhile {
    paced: Paced,
    output: PathBuf,
    armed: std::sync::atomic::AtomicBool,
}
// Every native owner below is the local provider's own, destroyed synchronously.
impl midnight_core::AuditedSynchronousTransferService for PublishedMeanwhile {}
impl TransferService for PublishedMeanwhile {
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.paced.prepare_checksum_directory(path, cancel)
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.prepare_transfer_source_with_links(path, CopyLinkPolicy::Reject, cancel)
    }
    fn prepare_transfer_source_with_links(
        &self,
        path: &Path,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let source = self
            .paced
            .prepare_transfer_source_with_links(path, links, cancel)?;
        if path == self.output && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let published = || {
                fs::read_dir(path).map_or(0, |entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|entry| !entry.file_name().to_string_lossy().starts_with(".me-"))
                        .count()
                })
            };
            let wanted = published() + 2;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while published() < wanted && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(source)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.paced.prepare_transfer_destination(path, cancel)
    }
}

/// A paced folder; an unconfirmed one (`.1`), and every folder created in it, reports no
/// lookup keys for the names written into it.
struct PacedDirectory(Arc<dyn TransferDirectory>, bool);
impl PacedDirectory {
    fn wrap(inner: Arc<dyn TransferDirectory>) -> Arc<dyn TransferDirectory> {
        Self::wrap_as(inner, false)
    }
    fn wrap_as(inner: Arc<dyn TransferDirectory>, unconfirmed: bool) -> Arc<dyn TransferDirectory> {
        Arc::new(Self(inner, unconfirmed))
    }
}
impl TransferDirectory for PacedDirectory {
    fn native_child_namespace_scope(
        &self,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        if self.1 {
            return Ok(None);
        }
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
        let directory = self.0.create_directory(name, cancel)?;
        Ok(Self::wrap_as(directory, self.1))
    }
    fn open_merge_directory(
        &self,
        name: &OsStr,
        expected: &TransferEntry,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        let directory = self.0.open_merge_directory(name, expected, cancel)?;
        Ok(Self::wrap_as(directory, self.1))
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
