//! Recursive metadata traversal pins (E6-7): the executing job's own reserve, borrowing of
//! free shared targets without ever waiting, early release, frame order and diagnostics
//! that name the real cause.
use super::*;
use std::{
    collections::HashMap,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};

/// Bounds waits that elapse only when traversal waits for pins or deadlocks.
const FAILURE_BOUND: Duration = Duration::from_secs(10);
const PER_JOB_CAUSE: &str = "one metadata job pins at most 256 targets, including its \
                             selected targets and the listed entries it has not processed yet";
/// The text both native adapters give their budget refusal, kept after the diagnostic.
const NATIVE_REFUSAL: &str =
    "The file system reported: metadata directory snapshot exceeds its resource budget";

fn edited_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_600_000_000)
}

fn modified(recursive: bool) -> MetadataPatch {
    MetadataPatch {
        modified: TimeEdit::Set(edited_time()),
        recursive,
        ..Default::default()
    }
}

fn modified_at(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

/// A confirmed recursive edit of every entry of `h.request`, pinned on `budget`.
fn native_job(h: &Harness, budget: &Arc<Semaphore>, reserve: &Arc<Semaphore>) -> worker::Job {
    let cancel = Cancellation::default();
    let mut job = worker::prepare(
        h.request.clone(),
        Some(Arc::new(LocalFileSystem::new(None))),
        cancel.clone(),
        Arc::new(Mutex::new(worker::initial(&h.request))),
        budget.clone(),
    )
    .unwrap();
    let edit = job.metadata.as_mut().unwrap();
    edit.update(1, modified(true), &job.targets, &cancel);
    job.metadata_traversal = Some(reserve.clone());
    job
}

/// Runs the native worker; the bound elapses only if traversal waits.
async fn execute(job: worker::Job) -> (OperationProgress, Arc<MetadataResult>) {
    let slot = job.metadata.as_ref().unwrap().result.clone();
    let cancel = job.cancel.clone();
    let (sender, _events) = mpsc::channel(64);
    let worker = tokio::task::spawn_blocking(move || {
        job.execute(Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        })
    });
    let Ok(progress) = tokio::time::timeout(FAILURE_BOUND, worker).await else {
        cancel.cancel();
        panic!("metadata traversal must never wait for pins");
    };
    let result = slot.lock().unwrap().take().unwrap();
    (progress.unwrap(), result)
}

fn parent_with_files(names: &[&str]) -> Harness {
    let h = Harness::new(&[("PARENT", true)]);
    for name in names {
        fs::write(
            h.directory.path().join("PARENT").join(name),
            name.as_bytes(),
        )
        .unwrap();
    }
    h
}

#[tokio::test]
async fn executing_traversal_uses_its_reserve_while_other_jobs_hold_every_shared_target() {
    let h = parent_with_files(&["FIRST", "SECOND", "THIRD"]);
    let budget = Arc::new(Semaphore::new(MAX_METADATA_TARGETS));
    let reserve = Arc::new(Semaphore::new(METADATA_TRAVERSAL_RESERVE));
    let job = native_job(&h, &budget, &reserve);
    // Pending previews and queued jobs hold every shared target the root pin left.
    let others = budget
        .clone()
        .try_acquire_many_owned(budget.available_permits() as u32)
        .unwrap();
    assert_eq!(others.num_permits(), MAX_METADATA_TARGETS - 1);
    let (progress, result) = execute(job).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.changed, result.failed), (4, 4, 0));
    for name in ["PARENT/FIRST", "PARENT/SECOND", "PARENT/THIRD", "PARENT"] {
        assert_eq!(
            modified_at(&h.directory.path().join(name)),
            edited_time(),
            "{name}"
        );
    }
    assert_eq!(reserve.available_permits(), METADATA_TRAVERSAL_RESERVE);
    drop(others);
    assert_eq!(budget.available_permits(), MAX_METADATA_TARGETS);
}

#[tokio::test]
async fn real_adapter_budget_refusal_lets_traversal_borrow_beyond_its_reserve() {
    // The native adapter must refuse the two-pin enumeration with ErrorKind::Other,
    // which is what lets traversal borrow free shared targets for the rest.
    let h = parent_with_files(&["A", "B", "C", "D", "E"]);
    let budget = Arc::new(Semaphore::new(MAX_METADATA_TARGETS));
    let reserve = Arc::new(Semaphore::new(2));
    let (progress, result) = execute(native_job(&h, &budget, &reserve)).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.changed, result.failed), (6, 6, 0));
    assert_eq!(
        modified_at(&h.directory.path().join("PARENT/E")),
        edited_time()
    );
    assert_eq!(reserve.available_permits(), 2);
    assert_eq!(budget.available_permits(), MAX_METADATA_TARGETS);
}

#[tokio::test]
async fn contended_traversal_fails_promptly_and_names_the_holding_jobs() {
    let h = parent_with_files(&["FIRST", "SECOND", "THIRD"]);
    let budget = Arc::new(Semaphore::new(MAX_METADATA_TARGETS));
    let reserve = Arc::new(Semaphore::new(1));
    let job = native_job(&h, &budget, &reserve);
    let _others = budget
        .clone()
        .try_acquire_many_owned(budget.available_permits() as u32)
        .unwrap();
    let (progress, result) = execute(job).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (1, 0, 1));
    for name in ["PARENT/FIRST", "PARENT/SECOND", "PARENT/THIRD", "PARENT"] {
        assert_ne!(
            modified_at(&h.directory.path().join(name)),
            edited_time(),
            "{name}"
        );
    }
    let message = &progress.errors[0].message;
    assert!(
        message.contains("more entries than the 1 that could be pinned")
            && message.contains("other pending metadata jobs hold 255 of the 256")
            && message.contains("were not changed")
            && message.ends_with(NATIVE_REFUSAL),
        "{message}"
    );
    assert_eq!(reserve.available_permits(), 1);
}

fn select(h: &Harness, id: OperationId, chosen: impl Fn(usize) -> bool) -> PrepareOperation {
    let count = h.request.source.entries.len();
    let mut bits = vec![0u64; count.div_ceil(64)];
    for index in (0..count).filter(|index| chosen(*index)) {
        bits[index / 64] |= 1 << (index % 64);
    }
    PrepareOperation {
        id,
        selection: Arc::new(SelectionSnapshot::from_bits(h.request.view.clone(), bits).unwrap()),
        ..h.request.clone()
    }
}

async fn next_event(h: &mut Harness, deadline: Instant) -> Event {
    let Ok(event) = tokio::time::timeout_at(deadline, h.events.recv()).await else {
        panic!("operation events stopped before the failure bound");
    };
    event.expect("the operation outbox stays open")
}

async fn reviewed(h: &mut Harness, id: OperationId, revision: u64) {
    let deadline = Instant::now() + FAILURE_BOUND;
    loop {
        match next_event(h, deadline).await {
            Event::Operation(OperationEvent::MetadataPreview(preview))
                if preview.id == id && preview.revision == revision =>
            {
                return;
            }
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state.is_terminal() =>
            {
                panic!("unexpected terminal state: {progress:?}")
            }
            _ => {}
        }
    }
}

async fn queued(h: &mut Harness, id: OperationId) {
    let deadline = Instant::now() + FAILURE_BOUND;
    loop {
        match next_event(h, deadline).await {
            Event::Operation(OperationEvent::Progress(progress))
                if progress.id == id && progress.state == OperationState::Queued =>
            {
                return;
            }
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state.is_terminal() =>
            {
                panic!("unexpected terminal state: {progress:?}")
            }
            _ => {}
        }
    }
}

async fn finished(
    h: &mut Harness,
    ids: [OperationId; 2],
) -> Vec<(Arc<OperationProgress>, Arc<MetadataResult>)> {
    let deadline = Instant::now() + FAILURE_BOUND;
    let (mut results, mut terminal) = (HashMap::new(), HashMap::new());
    while terminal.len() < ids.len() {
        match next_event(h, deadline).await {
            Event::Operation(OperationEvent::MetadataResult(result)) => {
                results.insert(result.id, result);
            }
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state.is_terminal() =>
            {
                terminal.insert(progress.id, progress);
            }
            _ => {}
        }
    }
    ids.iter()
        .map(|id| (terminal.remove(id).unwrap(), results.remove(id).unwrap()))
        .collect()
}

#[tokio::test]
async fn queued_metadata_job_holding_pins_cannot_stall_a_running_recursive_edit() {
    let files: Vec<_> = (1..MAX_METADATA_TARGETS)
        .map(|index| format!("F{index:03}"))
        .collect();
    let mut entries = vec![("PARENT", true)];
    entries.extend(files.iter().map(|name| (name.as_str(), false)));
    let mut h = Harness::new(&entries);
    for name in ["FIRST", "SECOND", "THIRD"] {
        fs::write(h.directory.path().join("PARENT").join(name), name).unwrap();
    }
    let (recursive, waiting) = (OperationId(1), OperationId(2));
    // A running Copy's mutation-gate share keeps the confirmed edit waiting to start.
    let copy = h.gate.clone().try_acquire_owned().unwrap();
    h.controller
        .prepare(select(&h, recursive, |i| i == 0))
        .await;
    reviewed(&mut h, recursive, 0).await;
    h.controller
        .preview_metadata(recursive, 1, modified(true))
        .await;
    reviewed(&mut h, recursive, 1).await;
    h.controller
        .confirm_metadata(recursive, 1, MetadataConflictPolicy::Skip)
        .await;
    queued(&mut h, recursive).await;
    // The next job pins the 255 siblings and keeps them while it waits for the edit.
    h.controller.prepare(select(&h, waiting, |i| i > 0)).await;
    reviewed(&mut h, waiting, 0).await;
    h.controller
        .preview_metadata(waiting, 1, modified(false))
        .await;
    reviewed(&mut h, waiting, 1).await;
    h.controller
        .confirm_metadata(waiting, 1, MetadataConflictPolicy::Skip)
        .await;
    queued(&mut h, waiting).await;
    drop(copy);
    let done = finished(&mut h, [recursive, waiting]).await;
    let (progress, result) = &done[0];
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.changed, result.failed), (4, 4, 0));
    for name in ["PARENT/FIRST", "PARENT/SECOND", "PARENT/THIRD", "PARENT"] {
        assert_eq!(
            modified_at(&h.directory.path().join(name)),
            edited_time(),
            "{name}"
        );
    }
    let (progress, result) = &done[1];
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.changed), (255, 255));
}

/// Free (reserve, shared) pins a fake native call observed.
type Observed = (usize, usize);

#[derive(Debug)]
struct Probe {
    reserve: Arc<Semaphore>,
    shared: Arc<Semaphore>,
    /// Every enumeration: directory, requested `max_targets` and the free pins seen.
    listings: Mutex<Vec<(PathBuf, usize, Observed)>>,
    /// Free pins seen while each target's metadata snapshot was read.
    snapshots: Mutex<Vec<(PathBuf, Observed)>>,
    /// Free pins seen at every `item()` read of a `Quirk::Watched` node.
    item_reads: Mutex<Vec<Observed>>,
    /// Shared targets a `Quirk::Seizes` node took for other jobs.
    others: Mutex<Option<OwnedSemaphorePermit>>,
    edited: Mutex<Vec<PathBuf>>,
}

impl Probe {
    fn new(reserve: usize) -> Arc<Self> {
        Arc::new(Self {
            reserve: Arc::new(Semaphore::new(reserve)),
            shared: Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
            listings: Mutex::default(),
            snapshots: Mutex::default(),
            item_reads: Mutex::default(),
            others: Mutex::default(),
            edited: Mutex::default(),
        })
    }
    fn observed(&self) -> Observed {
        (
            self.reserve.available_permits(),
            self.shared.available_permits(),
        )
    }
    fn listings_of(&self, path: &Path) -> Vec<(usize, Observed)> {
        let listings = self.listings.lock().unwrap();
        listings
            .iter()
            .filter(|(listed, _, _)| listed == path)
            .map(|(_, max_targets, observed)| (*max_targets, *observed))
            .collect()
    }
    fn snapshots_below(&self, parent: &Path) -> Vec<Observed> {
        let snapshots = self.snapshots.lock().unwrap();
        snapshots
            .iter()
            .filter(|(path, _)| path.parent() == Some(parent))
            .map(|(_, observed)| *observed)
            .collect()
    }
}

/// How one in-memory node departs from an ordinary file or folder.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Quirk {
    Plain,
    /// Records the free pins at every `item()` read.
    Watched,
    /// When edited, takes every free shared target, as other jobs' previews would.
    Seizes,
    /// Fails every listing with ErrorKind::Other, as Windows reports a folder replaced
    /// between revalidation and enumeration, although that is no budget refusal.
    Replaced,
}

/// An in-memory tree whose enumeration refuses more entries than the pins taken for it,
/// with ErrorKind::Other before any child is opened, as both native adapters do.
#[derive(Debug)]
struct Node {
    item: MutationItem,
    children: Vec<Arc<Node>>,
    probe: Arc<Probe>,
    quirk: Quirk,
}

impl Node {
    fn file(probe: &Arc<Probe>, path: PathBuf) -> Arc<Self> {
        Self::new(probe, path, EntryKind::File, vec![], Quirk::Plain)
    }
    fn directory(probe: &Arc<Probe>, path: PathBuf, children: Vec<Arc<Node>>) -> Arc<Self> {
        Self::new(probe, path, EntryKind::Directory, children, Quirk::Plain)
    }
    fn new(
        probe: &Arc<Probe>,
        path: PathBuf,
        kind: EntryKind,
        children: Vec<Arc<Node>>,
        quirk: Quirk,
    ) -> Arc<Self> {
        Arc::new(Self {
            item: MutationItem {
                path,
                kind,
                metadata: Default::default(),
            },
            children,
            probe: probe.clone(),
            quirk,
        })
    }
}

impl PreparedMutation for Node {
    fn item(&self) -> &MutationItem {
        if self.quirk == Quirk::Watched {
            let observed = self.probe.observed();
            self.probe.item_reads.lock().unwrap().push(observed);
        }
        &self.item
    }
    fn metadata_snapshot(&self, _: &Cancellation) -> Result<MetadataSnapshot, FsError> {
        let observed = self.probe.observed();
        let mut snapshots = self.probe.snapshots.lock().unwrap();
        snapshots.push((self.item.path.clone(), observed));
        Ok(MetadataSnapshot {
            capabilities: MetadataCapabilities {
                modified: true,
                ..Default::default()
            },
            ..Default::default()
        })
    }
    fn metadata_children(
        &self,
        _: &Cancellation,
        max_targets: usize,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        let observed = self.probe.observed();
        let mut listings = self.probe.listings.lock().unwrap();
        listings.push((self.item.path.clone(), max_targets, observed));
        if self.quirk == Quirk::Replaced {
            return Err(FsError::from_io(
                "enumerate metadata targets",
                &self.item.path,
                io::Error::other("the adapter failed for another reason"),
            ));
        }
        if self.children.len() > max_targets {
            return Err(FsError::from_io(
                "snapshot metadata targets",
                &self.item.path,
                io::Error::other("metadata directory snapshot exceeds its resource budget"),
            ));
        }
        Ok(self
            .children
            .iter()
            .map(|child| child.clone() as Arc<dyn PreparedMutation>)
            .collect())
    }
    fn edit_metadata(
        &self,
        _: &MetadataPatch,
        _: Option<&OsStr>,
        _: &Cancellation,
    ) -> MetadataOutcome {
        self.probe
            .edited
            .lock()
            .unwrap()
            .push(self.item.path.clone());
        if self.quirk == Quirk::Seizes {
            let shared = &self.probe.shared;
            let free = shared.available_permits() as u32;
            let held = shared.clone().try_acquire_many_owned(free).unwrap();
            *self.probe.others.lock().unwrap() = Some(held);
        }
        let mut outcome = MetadataOutcome::unchanged(self.item.path.clone());
        outcome.changes.modified = true;
        outcome
    }
    fn rename(&self, _: &OsStr, _: &Cancellation) -> Result<RenameCompletion, FsError> {
        panic!("time edits never rename")
    }
    fn delete(
        &self,
        _: DeleteMode,
        _: &Cancellation,
        _: &mut dyn FnMut(&Path),
    ) -> Result<DeleteCompletion, FsError> {
        panic!("metadata edits never delete")
    }
}

#[derive(Debug)]
struct Roots {
    roots: Vec<Arc<Node>>,
    covered: Vec<bool>,
}

impl CapturedMetadataSources for Roots {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(7),
            session: TreeSessionId(2),
            revision: TreeRevision(3),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(4)
    }
    fn root_count(&self) -> usize {
        self.roots.len()
    }
    fn covered_roots(&self) -> &[bool] {
        &self.covered
    }
    fn prepare(
        &self,
        _: &dyn MutationService,
        _: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        Ok(self
            .roots
            .iter()
            .map(|root| root.clone() as Arc<dyn PreparedMutation>)
            .collect())
    }
}

/// A confirmed recursive edit of one in-memory root, holding its root pin on the probe's
/// shared budget and owning the probe's reserve as the executing job.
fn fake_job(probe: &Arc<Probe>, root: Arc<Node>) -> worker::Job {
    let cancel = Cancellation::default();
    let sources = Roots {
        roots: vec![root],
        covered: vec![false],
    };
    let scope = input::MetadataScope::capture(&sources);
    let source_directory = PathBuf::from("captured");
    let status = worker::initial_for(
        OperationId(1),
        MutationKind::EditMetadata,
        source_directory.clone(),
    );
    let mut job = worker::prepare_captured_metadata(
        PrepareCapturedMetadata {
            id: OperationId(1),
            sources: Arc::new(sources),
            source_directory,
        },
        scope,
        Some(Arc::new(LocalFileSystem::new(None))),
        cancel.clone(),
        Arc::new(Mutex::new(status)),
        probe.shared.clone(),
    )
    .unwrap();
    let edit = job.metadata.as_mut().unwrap();
    edit.update(1, modified(true), &job.targets, &cancel);
    job.metadata_traversal = Some(probe.reserve.clone());
    job
}

fn files(probe: &Arc<Probe>, parent: &Path, count: usize) -> Vec<Arc<Node>> {
    (0..count)
        .map(|index| Node::file(probe, parent.join(format!("{index:03}"))))
        .collect()
}

#[tokio::test]
async fn traversal_borrows_shared_targets_only_beyond_its_reserve_and_trims_them() {
    // Five entries exceed a two-pin reserve: the refused enumeration borrows the free
    // shared targets once, and the unused ones return before any child snapshot is read.
    let probe = Probe::new(2);
    let parent = PathBuf::from("PARENT");
    let root = Node::directory(&probe, parent.clone(), files(&probe, &parent, 5));
    let (progress, result) = execute(fake_job(&probe, root)).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.failed), (6, 0));
    assert_eq!(
        probe.listings_of(&parent),
        [(2, (0, 255)), (255, (0, 2))],
        "the reserve is enumerated first; the per-job cap bounds the borrowed retry"
    );
    assert_eq!(probe.snapshots_below(&parent), [(0, 252); 5]);
    assert_eq!(probe.observed(), (2, MAX_METADATA_TARGETS));

    // A folder that fits in the free reserve never touches the shared budget.
    let probe = Probe::new(METADATA_TRAVERSAL_RESERVE);
    let root = Node::directory(&probe, parent.clone(), files(&probe, &parent, 3));
    let (progress, _) = execute(fake_job(&probe, root)).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!(probe.listings_of(&parent), [(64, (0, 255))]);
    assert_eq!(probe.snapshots_below(&parent), [(61, 255); 3]);

    // Frames processed first hold the reserve, so it refills as they finish: the
    // subfolder after one finished sibling enumerates from the reserve alone.
    let probe = Probe::new(METADATA_TRAVERSAL_RESERVE);
    let folder = parent.join("001");
    let mut children = files(&probe, &parent, 70);
    children[1] = Node::directory(&probe, folder.clone(), files(&probe, &folder, 1));
    let root = Node::directory(&probe, parent.clone(), children);
    let (progress, result) = execute(fake_job(&probe, root)).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.failed), (72, 0));
    // The borrowed retry asks for the whole per-job cap; trimming to the 70 entries
    // leaves 249 shared targets free while the frames are processed.
    assert_eq!(probe.listings_of(&parent), [(64, (0, 255)), (255, (0, 64))]);
    assert_eq!(probe.listings_of(&folder), [(1, (0, 249))]);

    // With no free reserve pin, the folder borrows at once instead of first asking
    // the adapter to list it with no pins at all.
    let probe = Probe::new(0);
    let root = Node::directory(&probe, parent.clone(), files(&probe, &parent, 3));
    let (progress, _) = execute(fake_job(&probe, root)).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!(probe.listings_of(&parent), [(255, (0, 0))]);
    assert_eq!(probe.snapshots_below(&parent), [(0, 252); 3]);
}

#[tokio::test]
async fn traversal_that_exhausts_its_own_budget_fails_without_waiting() {
    // A root and its 255 pinned subfolders fill the per-job budget, so the first
    // subfolder cannot pin its file. It fails at once; later ones reuse the reserve.
    let probe = Probe::new(METADATA_TRAVERSAL_RESERVE);
    let parent = PathBuf::from("PARENT");
    let folders: Vec<_> = (0..MAX_METADATA_TARGETS - 1)
        .map(|index| {
            let folder = parent.join(format!("{index:03}"));
            Node::directory(&probe, folder.clone(), files(&probe, &folder, 1))
        })
        .collect();
    let root = Node::directory(&probe, parent.clone(), folders);
    let (progress, result) = execute(fake_job(&probe, root)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!(
        (result.processed, result.changed, result.failed),
        (510, 509, 1)
    );
    let failed = parent.join("000");
    assert_eq!(progress.errors.len(), 1);
    assert_eq!(progress.errors[0].path, failed);
    assert!(
        progress.errors[0].message.contains(PER_JOB_CAUSE),
        "{:?}",
        progress.errors[0]
    );
    let edited = probe.edited.lock().unwrap();
    assert!(!edited.iter().any(|path| path.starts_with(&failed)));
    assert_eq!(edited.len(), 509);
    // The parent borrows once beyond its reserve: its first 64 entries hold reserve
    // pins and the other 191 borrowed ones, which leaves 64 shared targets free.
    let reserve = METADATA_TRAVERSAL_RESERVE;
    let listings = probe.listings.lock().unwrap();
    assert_eq!(
        listings[..2],
        [
            (parent.clone(), reserve, (0, MAX_METADATA_TARGETS - 1)),
            (parent.clone(), MAX_METADATA_TARGETS - 1, (0, reserve)),
        ]
    );
    // Subfolder k may pin k more targets: the root, the subfolder itself and its 254 - k
    // waiting siblings hold the rest. It is listed once, with only the reserve pins its
    // finished siblings returned, and the shared targets the finished siblings borrowed
    // stay free, so no subfolder borrows a single shared target.
    let subfolders: Vec<_> = (0..MAX_METADATA_TARGETS - 1)
        .map(|k| {
            let folder = parent.join(format!("{k:03}"));
            (folder, k.min(reserve), (0, k.max(reserve)))
        })
        .collect();
    assert_eq!(listings[2..], subfolders);
}

#[tokio::test]
async fn depth_refusal_never_reserves_or_borrows_targets() {
    // One reserve pin more than the chain needs and the free shared targets would both
    // show any pin the refused folder took in the counts its `item()` reads observe.
    let probe = Probe::new(METADATA_TRAVERSAL_RESERVE + 1);
    let paths: Vec<PathBuf> = (0..=MAX_METADATA_DEPTH)
        .scan(PathBuf::new(), |path, depth| {
            path.push(format!("D{depth:02}"));
            Some(path.clone())
        })
        .collect();
    let deepest = paths[MAX_METADATA_DEPTH].clone();
    let mut node = Node::new(
        &probe,
        deepest.clone(),
        EntryKind::Directory,
        files(&probe, &deepest, 1),
        Quirk::Watched,
    );
    for path in paths[..MAX_METADATA_DEPTH].iter().rev() {
        node = Node::directory(&probe, path.clone(), vec![node]);
    }
    let (progress, result) = execute(fake_job(&probe, node)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.failed), (65, 1));
    assert_eq!(progress.errors[0].path, deepest);
    assert!(
        progress.errors[0].message.contains("depth budget"),
        "{:?}",
        progress.errors[0]
    );
    // The 64 folders below the root hold reserve pins and the root its shared one. Every
    // read of the refused folder, from its parent's snapshot pass through the read that
    // builds its depth error to its outcome, sees the same free pins: it took none.
    let reads = probe.item_reads.lock().unwrap();
    assert!(
        !reads.is_empty()
            && reads
                .iter()
                .all(|read| *read == (1, MAX_METADATA_TARGETS - 1)),
        "{reads:?}"
    );
    let listings = probe.listings.lock().unwrap();
    assert_eq!(
        listings.len(),
        MAX_METADATA_DEPTH,
        "the deepest folder is never listed"
    );
    assert!(
        listings
            .iter()
            .all(|(_, _, (_, shared))| *shared == MAX_METADATA_TARGETS - 1),
        "a chain within the depth budget fits in the reserve"
    );
}

#[tokio::test]
async fn shortfall_counts_only_the_shared_targets_other_jobs_hold() {
    // Five entries exceed a two-pin reserve, so the parent borrows shared pins for its
    // last three. While its second file is edited, other jobs take every free shared
    // target. The subfolder that follows holds a borrowed pin, its two waiting siblings
    // hold one each, and the two reserve pins its finished siblings returned cannot
    // pin its three entries.
    let probe = Probe::new(2);
    let parent = PathBuf::from("PARENT");
    let folder = parent.join("002");
    let mut children = files(&probe, &parent, 5);
    children[1] = Node::new(
        &probe,
        parent.join("001"),
        EntryKind::File,
        vec![],
        Quirk::Seizes,
    );
    children[2] = Node::directory(&probe, folder.clone(), files(&probe, &folder, 3));
    let root = Node::directory(&probe, parent.clone(), children);
    let (progress, result) = execute(fake_job(&probe, root)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (6, 5, 1));
    assert_eq!(probe.listings_of(&parent), [(2, (0, 255)), (255, (0, 2))]);
    assert_eq!(
        probe.listings_of(&folder),
        [(2, (0, 0))],
        "nothing is free to borrow, so the folder is not listed again"
    );
    // Other jobs hold every shared target but this job's root pin and the borrowed pins
    // of the subfolder and its two waiting siblings; the diagnostic counts exactly those.
    let others = probe.others.lock().unwrap().take().unwrap();
    assert_eq!(others.num_permits(), MAX_METADATA_TARGETS - 4);
    assert_eq!(progress.errors.len(), 1);
    assert_eq!(progress.errors[0].path, folder);
    let message = &progress.errors[0].message;
    assert!(
        message.contains("more entries than the 2 that could be pinned")
            && message.contains("other pending metadata jobs hold 252 of the 256 shared targets")
            && message.ends_with(NATIVE_REFUSAL),
        "{message}"
    );
    assert_eq!(probe.observed(), (2, MAX_METADATA_TARGETS - 252));
}

#[tokio::test]
async fn shortfall_keeps_the_adapter_text_of_a_misclassified_listing_error() {
    // The trait reserves ErrorKind::Other for the pin-budget refusal, but an adapter (a
    // plugin, a future platform) may still report another failure with it. Taken for a
    // budget refusal, it costs one more listing with borrowed targets, and its own text
    // stays visible after the pin diagnostic.
    let probe = Probe::new(METADATA_TRAVERSAL_RESERVE);
    let parent = PathBuf::from("PARENT");
    let root = Node::new(
        &probe,
        parent.clone(),
        EntryKind::Directory,
        files(&probe, &parent, 1),
        Quirk::Replaced,
    );
    let (progress, result) = execute(fake_job(&probe, root)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (1, 0, 1));
    assert_eq!(probe.listings_of(&parent), [(64, (0, 255)), (255, (0, 64))]);
    let message = &progress.errors[0].message;
    assert!(
        message.ends_with("The file system reported: the adapter failed for another reason"),
        "{message}"
    );
    assert_eq!(probe.observed(), (64, MAX_METADATA_TARGETS));
}
