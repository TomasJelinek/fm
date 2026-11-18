//! Recursive metadata traversal against an in-memory adapter: pins bounded by depth, one
//! listing per folder, the name budget along the current path, entries that fail alone,
//! folders that change while they are processed, sibling case conflicts found on the
//! captured names, cancellation, and the release of a folder's snapshot and pinned entries
//! before its own edit.
use super::*;
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    sync::atomic::AtomicUsize,
    time::SystemTime,
};

/// Elapses only when traversal hangs.
const FAILURE_BOUND: Duration = Duration::from_secs(10);
/// The text both native adapters give their budget refusal, kept after the diagnostic.
const NATIVE_REFUSAL: &str =
    "The file system reported: metadata directory snapshot exceeds its resource budget";
const FULL_BUDGET: &str = "this folder's names exceed what one recursive edit can hold (at \
                           most 100,000 names or 4 MiB of names); this folder and its \
                           contents were not changed";

fn edited_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_600_000_000)
}

fn modified() -> MetadataPatch {
    MetadataPatch {
        modified: TimeEdit::Set(edited_time()),
        recursive: true,
        ..Default::default()
    }
}

fn lower(times: bool) -> MetadataPatch {
    MetadataPatch {
        case: Some(CaseChange::default()),
        modified: if times {
            TimeEdit::Set(edited_time())
        } else {
            TimeEdit::Keep
        },
        recursive: true,
        ..Default::default()
    }
}

/// Every listing a fake folder received.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Listed {
    folder: PathBuf,
    max_names: usize,
    max_bytes: usize,
    /// Free targets of the shared selection budget at that moment.
    shared_free: usize,
}

#[derive(Debug)]
struct Probe {
    /// The shared selection budget the job's root pin came from.
    shared: Arc<Semaphore>,
    listings: Mutex<Vec<Listed>>,
    /// Folders whose snapshot is alive.
    listed: Mutex<HashSet<PathBuf>>,
    /// Pinned entries alive now, and the most alive at once.
    live: Mutex<Vec<PathBuf>>,
    max_live: AtomicUsize,
    /// Every pin attempt, in order.
    pins: Mutex<Vec<PathBuf>>,
    /// Every edit, with the new name it was given.
    edited: Mutex<Vec<(PathBuf, Option<OsString>)>>,
    /// Edits that ran while their own snapshot or a pinned entry below them was alive.
    early: Mutex<Vec<PathBuf>>,
    /// Folders that another program moved: their revalidation fails.
    moved: Mutex<HashSet<PathBuf>>,
}

impl Probe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
            listings: Mutex::default(),
            listed: Mutex::default(),
            live: Mutex::default(),
            max_live: AtomicUsize::new(0),
            pins: Mutex::default(),
            edited: Mutex::default(),
            early: Mutex::default(),
            moved: Mutex::default(),
        })
    }
    fn listings(&self) -> Vec<Listed> {
        self.listings.lock().unwrap().clone()
    }
    fn listing_of(&self, folder: &Path) -> (usize, usize) {
        let listings = self.listings();
        let found: Vec<_> = listings
            .iter()
            .filter(|listed| listed.folder == folder)
            .collect();
        assert_eq!(found.len(), 1, "{folder:?} is listed once: {listings:?}");
        (found[0].max_names, found[0].max_bytes)
    }
    fn pins(&self) -> Vec<PathBuf> {
        self.pins.lock().unwrap().clone()
    }
    fn edited(&self) -> Vec<(PathBuf, Option<OsString>)> {
        self.edited.lock().unwrap().clone()
    }
    fn edited_paths(&self) -> Vec<PathBuf> {
        self.edited().into_iter().map(|(path, _)| path).collect()
    }
    fn name_given(&self, path: &Path) -> Option<OsString> {
        let edited = self.edited();
        let found: Vec<_> = edited.iter().filter(|(edited, _)| edited == path).collect();
        assert_eq!(found.len(), 1, "{path:?} is edited once: {edited:?}");
        found[0].1.clone()
    }
    /// No snapshot or pin outlives the job, and every folder's own edit ran only after its
    /// snapshot and every pinned entry below it had closed.
    fn assert_released(&self) {
        assert!(self.live.lock().unwrap().is_empty(), "pins remain");
        assert!(self.listed.lock().unwrap().is_empty(), "snapshots remain");
        let early = self.early.lock().unwrap();
        assert!(
            early.is_empty(),
            "edited before their entries closed: {early:?}"
        );
    }
}

/// How one in-memory node departs from an ordinary file or folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Quirk {
    #[default]
    Plain,
    /// Its listing fails with this kind and the text "the adapter failed for another
    /// reason".
    Listing(io::ErrorKind),
    /// Its edit cancels the job.
    Cancels,
    /// Its edit fails as if another program moved its folder, which then fails its
    /// revalidation.
    MovesFolder,
    /// Times cannot be changed, like a link's.
    NoTimes,
}

/// One captured name of a fake folder.
#[derive(Clone, Debug)]
enum Entry {
    /// A plain file, created only when it is pinned, so large folders stay cheap.
    File,
    Node(Arc<Node>),
    /// Removed after the snapshot: pinning it fails with `NotFound`.
    Vanished,
}

/// An in-memory tree whose listing refuses more names or name bytes than allowed with
/// `ErrorKind::Other`, as both native adapters do.
#[derive(Debug)]
struct Node {
    item: MutationItem,
    entries: Vec<(OsString, Entry)>,
    probe: Arc<Probe>,
    quirk: Quirk,
}

impl Node {
    fn new(
        probe: &Arc<Probe>,
        path: PathBuf,
        kind: EntryKind,
        entries: Vec<(OsString, Entry)>,
        quirk: Quirk,
    ) -> Arc<Self> {
        Arc::new(Self {
            item: MutationItem {
                path,
                kind,
                metadata: Default::default(),
            },
            entries,
            probe: probe.clone(),
            quirk,
        })
    }
    fn file(probe: &Arc<Probe>, path: PathBuf) -> Arc<Self> {
        Self::new(probe, path, EntryKind::File, vec![], Quirk::Plain)
    }
    fn folder(probe: &Arc<Probe>, path: PathBuf, entries: Vec<(OsString, Entry)>) -> Arc<Self> {
        Self::new(probe, path, EntryKind::Directory, entries, Quirk::Plain)
    }
}

/// `count` plain files named 000, 001, ...
fn files(count: usize) -> Vec<(OsString, Entry)> {
    (0..count)
        .map(|index| (OsString::from(format!("{index:03}")), Entry::File))
        .collect()
}

/// A child entry of `folder` named after the last component of the child's path.
fn child(node: Arc<Node>) -> (OsString, Entry) {
    let name = node.item.path.file_name().unwrap().to_owned();
    (name, Entry::Node(node))
}

/// A name of exactly 1 KiB.
fn kibibyte_name(prefix: &str, index: usize) -> OsString {
    let stem = format!("{prefix}{index:06}");
    OsString::from(format!("{stem}{}", "x".repeat(1024 - stem.len())))
}

impl PreparedMutation for Node {
    fn item(&self) -> &MutationItem {
        &self.item
    }
    fn metadata_snapshot(&self, _: &Cancellation) -> Result<MetadataSnapshot, FsError> {
        if self.probe.moved.lock().unwrap().contains(&self.item.path) {
            return Err(FsError::from_io(
                "revalidate mutation",
                &self.item.path,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "the selected entry or its parent has been replaced",
                ),
            ));
        }
        Ok(MetadataSnapshot {
            capabilities: MetadataCapabilities {
                modified: self.quirk != Quirk::NoTimes,
                rename: true,
                ..Default::default()
            },
            ..Default::default()
        })
    }
    fn metadata_children(
        &self,
        _: &Cancellation,
        max_names: usize,
        max_bytes: usize,
    ) -> Result<Box<dyn MetadataChildren>, FsError> {
        self.probe.listings.lock().unwrap().push(Listed {
            folder: self.item.path.clone(),
            max_names,
            max_bytes,
            shared_free: self.probe.shared.available_permits(),
        });
        if let Quirk::Listing(kind) = self.quirk {
            return Err(FsError::from_io(
                "enumerate metadata targets",
                &self.item.path,
                io::Error::new(kind, "the adapter failed for another reason"),
            ));
        }
        let bytes: usize = self.entries.iter().map(|(name, _)| name.len()).sum();
        if self.entries.len() > max_names || bytes > max_bytes {
            return Err(FsError::from_io(
                "snapshot metadata targets",
                &self.item.path,
                io::Error::other("metadata directory snapshot exceeds its resource budget"),
            ));
        }
        self.probe
            .listed
            .lock()
            .unwrap()
            .insert(self.item.path.clone());
        Ok(Box::new(Snapshot {
            folder: self.item.path.clone(),
            names: self.entries.iter().map(|(name, _)| name.clone()).collect(),
            entries: self
                .entries
                .iter()
                .map(|(_, entry)| entry.clone())
                .collect(),
            probe: self.probe.clone(),
        }))
    }
    fn edit_metadata(
        &self,
        _: &MetadataPatch,
        name: Option<&OsStr>,
        cancel: &Cancellation,
    ) -> MetadataOutcome {
        let (probe, path) = (&self.probe, &self.item.path);
        let early = probe.listed.lock().unwrap().contains(path)
            || probe
                .live
                .lock()
                .unwrap()
                .iter()
                .any(|live| live != path && live.starts_with(path));
        if early {
            probe.early.lock().unwrap().push(path.clone());
        }
        probe
            .edited
            .lock()
            .unwrap()
            .push((path.clone(), name.map(OsStr::to_owned)));
        let mut outcome = MetadataOutcome::unchanged(path.clone());
        match self.quirk {
            Quirk::Cancels => cancel.cancel(),
            Quirk::MovesFolder => {
                let folder = path.parent().unwrap().to_owned();
                probe.moved.lock().unwrap().insert(folder);
                outcome.error = self.metadata_snapshot(cancel).err().or_else(|| {
                    Some(FsError::from_io(
                        "revalidate mutation",
                        path,
                        io::Error::new(
                            io::ErrorKind::NotFound,
                            "the selected entry or its parent has been replaced",
                        ),
                    ))
                });
                return outcome;
            }
            _ => {}
        }
        outcome.changes.modified = true;
        outcome
    }
    fn rename(&self, _: &OsStr, _: &Cancellation) -> Result<RenameCompletion, FsError> {
        panic!("metadata edits rename through edit_metadata")
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

/// A fake folder's captured names; entries are created only when they are pinned.
struct Snapshot {
    folder: PathBuf,
    names: Vec<OsString>,
    entries: Vec<Entry>,
    probe: Arc<Probe>,
}

impl MetadataChildren for Snapshot {
    fn names(&self) -> &[OsString] {
        &self.names
    }
    fn pin(
        &mut self,
        index: usize,
        _: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        let Some(name) = self.names.get(index).filter(|name| !name.is_empty()) else {
            return Err(FsError::from_io(
                "pin child",
                &self.folder,
                io::ErrorKind::InvalidInput.into(),
            ));
        };
        let path = self.folder.join(name);
        self.probe.pins.lock().unwrap().push(path.clone());
        let node = match &self.entries[index] {
            Entry::File => Node::file(&self.probe, path),
            Entry::Node(node) => node.clone(),
            Entry::Vanished => {
                return Err(FsError::from_io(
                    "pin child",
                    &path,
                    io::ErrorKind::NotFound.into(),
                ));
            }
        };
        self.names[index] = OsString::new();
        Ok(Arc::new(Held::new(node)))
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        self.probe.listed.lock().unwrap().remove(&self.folder);
    }
}

/// A pinned entry; the probe counts it as alive until it is dropped.
#[derive(Debug)]
struct Held(Arc<Node>);

impl Held {
    fn new(node: Arc<Node>) -> Self {
        let mut live = node.probe.live.lock().unwrap();
        live.push(node.item.path.clone());
        node.probe.max_live.fetch_max(live.len(), Ordering::SeqCst);
        drop(live);
        Self(node)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut live = self.0.probe.live.lock().unwrap();
        if let Some(index) = live.iter().position(|path| *path == self.0.item.path) {
            live.swap_remove(index);
        }
    }
}

impl PreparedMutation for Held {
    fn item(&self) -> &MutationItem {
        self.0.item()
    }
    fn metadata_snapshot(&self, cancel: &Cancellation) -> Result<MetadataSnapshot, FsError> {
        self.0.metadata_snapshot(cancel)
    }
    fn metadata_children(
        &self,
        cancel: &Cancellation,
        max_names: usize,
        max_bytes: usize,
    ) -> Result<Box<dyn MetadataChildren>, FsError> {
        self.0.metadata_children(cancel, max_names, max_bytes)
    }
    fn edit_metadata(
        &self,
        patch: &MetadataPatch,
        name: Option<&OsStr>,
        cancel: &Cancellation,
    ) -> MetadataOutcome {
        self.0.edit_metadata(patch, name, cancel)
    }
    fn rename(&self, name: &OsStr, cancel: &Cancellation) -> Result<RenameCompletion, FsError> {
        self.0.rename(name, cancel)
    }
    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<DeleteCompletion, FsError> {
        self.0.delete(mode, cancel, progress)
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

/// A confirmed edit of one in-memory root, whose pin comes from the probe's shared budget.
fn fake_job(
    probe: &Arc<Probe>,
    root: Arc<Node>,
    patch: MetadataPatch,
    policy: MetadataConflictPolicy,
) -> worker::Job {
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
    edit.update(1, patch, &job.targets, &cancel);
    edit.conflicts = policy;
    job
}

/// Runs the native worker; the bound elapses only if traversal hangs.
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
        panic!("metadata traversal hung");
    };
    let result = slot.lock().unwrap().take().unwrap();
    (progress.unwrap(), result)
}

async fn run(probe: &Arc<Probe>, root: Arc<Node>) -> (OperationProgress, Arc<MetadataResult>) {
    let job = fake_job(probe, root, modified(), MetadataConflictPolicy::Skip);
    execute(job).await
}

#[tokio::test]
async fn traversal_pins_one_target_per_depth_level() {
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let (a, b, c, d) = (
        parent.join("A"),
        parent.join("B"),
        parent.join("B/C"),
        parent.join("B/C/D"),
    );
    let chain = Node::folder(
        &probe,
        b.clone(),
        vec![child(Node::folder(
            &probe,
            c.clone(),
            vec![child(Node::folder(&probe, d.clone(), files(1)))],
        ))],
    );
    let mut entries = files(300);
    entries.insert(150, child(Node::folder(&probe, a.clone(), files(300))));
    entries.push(child(chain));
    let root = Node::folder(&probe, parent.clone(), entries);
    let (progress, result) = run(&probe, root).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!(
        (result.processed, result.changed, result.failed),
        (606, 606, 0)
    );
    let mut listed: Vec<_> = probe.listings().into_iter().map(|l| l.folder).collect();
    listed.sort();
    assert_eq!(listed, [parent, a, b, c, d], "each folder is listed once");
    assert!(
        probe
            .listings()
            .iter()
            .all(|listed| listed.shared_free == MAX_METADATA_TARGETS - 1),
        "traversal never takes a shared target: {:?}",
        probe.listings()
    );
    // B, C, D and the file in D: one pinned entry per listed level.
    assert_eq!(probe.max_live.load(Ordering::SeqCst), 4);
    assert_eq!(probe.pins().len(), 605);
    probe.assert_released();
}

#[tokio::test]
async fn depth_refusal_pins_at_most_the_depth_budget() {
    let probe = Probe::new();
    let paths: Vec<PathBuf> = (0..=MAX_METADATA_DEPTH)
        .scan(PathBuf::new(), |path, depth| {
            path.push(format!("D{depth:02}"));
            Some(path.clone())
        })
        .collect();
    let deepest = paths[MAX_METADATA_DEPTH].clone();
    let mut node = Node::folder(&probe, deepest.clone(), files(1));
    for path in paths[..MAX_METADATA_DEPTH].iter().rev() {
        node = Node::folder(&probe, path.clone(), vec![child(node)]);
    }
    let (progress, result) = run(&probe, node).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.failed), (65, 1));
    assert_eq!(progress.errors[0].path, deepest);
    assert!(
        progress.errors[0].message.contains("depth budget"),
        "{:?}",
        progress.errors[0]
    );
    assert_eq!(
        probe.listings().len(),
        MAX_METADATA_DEPTH,
        "the deepest folder is never listed"
    );
    assert_eq!(probe.max_live.load(Ordering::SeqCst), MAX_METADATA_DEPTH);
    probe.assert_released();
}

#[tokio::test]
async fn a_folder_over_the_name_count_is_refused_before_any_entry_is_pinned() {
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let names = (0..=MAX_METADATA_NAMES)
        .map(|index| (OsString::from(format!("{index:06}")), Entry::File))
        .collect();
    let (progress, result) = run(&probe, Node::folder(&probe, parent.clone(), names)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (1, 0, 1));
    assert_eq!(progress.errors[0].kind, io::ErrorKind::Other);
    let message = &progress.errors[0].message;
    assert!(
        message.starts_with(FULL_BUDGET) && message.ends_with(NATIVE_REFUSAL),
        "{message}"
    );
    assert_eq!(
        probe.listing_of(&parent),
        (MAX_METADATA_NAMES, MAX_METADATA_NAMES_BYTES)
    );
    assert!(probe.pins().is_empty());
    assert!(probe.edited().is_empty());
    probe.assert_released();
}

#[tokio::test]
async fn name_budget_spans_the_current_path_and_returns_when_a_folder_is_done() {
    // The root holds 3 MiB of names (3,072 names of 1 KiB). Each pin releases its name's
    // bytes; the count stays charged until the root is done.
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let folder = |name: &OsString, count: usize, prefix: &str| {
        let path = parent.join(name);
        let entries = (0..count)
            .map(|index| (kibibyte_name(prefix, index), Entry::File))
            .collect();
        Node::folder(&probe, path, entries)
    };
    let (a, b, c) = (
        kibibyte_name("A", 0),
        kibibyte_name("B", 0),
        kibibyte_name("C", 0),
    );
    let mut entries = vec![
        (a.clone(), Entry::Node(folder(&a, 2048, "a"))),
        (b.clone(), Entry::Node(folder(&b, 1024, "b"))),
        (c.clone(), Entry::Node(folder(&c, 1, "c"))),
    ];
    entries.extend((3..3072).map(|index| (kibibyte_name("F", index), Entry::File)));
    let (progress, result) = run(&probe, Node::folder(&probe, parent.clone(), entries)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    // The root, its 3,072 entries, B's 1,024 files and C's file; A alone failed.
    assert_eq!((result.processed, result.failed), (4098, 1));
    let free_names = MAX_METADATA_NAMES - 3072;
    let mebibyte = 1024 * 1024;
    // A (2 MiB) finds 1 MiB and its own released 1 KiB name free.
    assert_eq!(
        probe.listing_of(&parent.join(&a)),
        (free_names, mebibyte + 1024)
    );
    let refusal = &progress.errors[0];
    assert_eq!(refusal.path, parent.join(&a));
    assert!(
        refusal.message.starts_with(
            "this folder's names exceed the 96,928 names or 1.0 MiB of names still free; one \
             recursive edit holds at most 100,000 names or 4 MiB of names at once, and the \
             folders above it that are still being processed hold the rest; this folder and \
             its contents were not changed"
        ) && refusal.message.ends_with(NATIVE_REFUSAL),
        "{}",
        refusal.message
    );
    // B (1 MiB) fits; once B is done its whole share returns, so C sees exactly one more
    // released name than B did.
    assert_eq!(
        probe.listing_of(&parent.join(&b)),
        (free_names, mebibyte + 2 * 1024)
    );
    assert_eq!(
        probe.listing_of(&parent.join(&c)),
        (free_names, mebibyte + 3 * 1024)
    );
    let edited = probe.edited_paths();
    assert!(!edited.iter().any(|path| path.starts_with(parent.join(&a))));
    assert_eq!(edited.len(), 4098 - 1);
    assert!(edited.contains(&parent) && edited.contains(&parent.join(&c)));
    probe.assert_released();
}

#[tokio::test]
async fn a_subfolder_after_its_finished_siblings_sees_their_name_bytes_returned() {
    // 3,000 names of 1 KiB, then a subfolder holding 2,000 more: charged whole, the
    // siblings would leave it about 1 MiB; released as they are pinned, they leave 4 MiB.
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let last = kibibyte_name("S", 0);
    let subfolder = parent.join(&last);
    let mut entries: Vec<_> = (0..3000)
        .map(|index| (kibibyte_name("F", index), Entry::File))
        .collect();
    let nested = (0..2000)
        .map(|index| (kibibyte_name("N", index), Entry::File))
        .collect();
    entries.push((
        last,
        Entry::Node(Node::folder(&probe, subfolder.clone(), nested)),
    ));
    let (progress, result) = run(&probe, Node::folder(&probe, parent, entries)).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.failed), (5002, 0));
    assert_eq!(
        probe.listing_of(&subfolder),
        (MAX_METADATA_NAMES - 3001, MAX_METADATA_NAMES_BYTES)
    );
    probe.assert_released();
}

#[tokio::test]
async fn a_child_that_cannot_be_pinned_fails_alone() {
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let mut entries = files(5);
    entries[1].1 = Entry::Vanished;
    let (progress, result) = run(&probe, Node::folder(&probe, parent.clone(), entries)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (6, 5, 1));
    assert_eq!(progress.errors.len(), 1);
    assert_eq!(progress.errors[0].path, parent.join("001"));
    assert_eq!(progress.errors[0].kind, io::ErrorKind::NotFound);
    let mut expected: Vec<_> = ["000", "002", "003", "004"]
        .iter()
        .map(|name| parent.join(name))
        .collect();
    expected.push(parent);
    assert_eq!(
        probe.edited_paths(),
        expected,
        "siblings and then the folder"
    );
    probe.assert_released();
}

#[tokio::test]
async fn a_folder_that_changed_stops_once_with_the_count_of_unprocessed_entries() {
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let mut entries = files(10);
    entries[1].1 = Entry::Node(Node::new(
        &probe,
        parent.join("001"),
        EntryKind::File,
        vec![],
        Quirk::MovesFolder,
    ));
    let (progress, result) = run(&probe, Node::folder(&probe, parent.clone(), entries)).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (3, 1, 2));
    assert_eq!(probe.pins(), [parent.join("000"), parent.join("001")]);
    assert_eq!(
        probe.edited_paths(),
        [parent.join("000"), parent.join("001")],
        "the folder itself is not edited"
    );
    let stopped = &progress.errors[1];
    assert_eq!(stopped.path, parent);
    assert_eq!(stopped.kind, io::ErrorKind::NotFound);
    assert!(
        stopped.message.starts_with(
            "this folder changed while its entries were processed, so 8 of its 10 entries \
             and the folder itself were not changed"
        ) && stopped.message.ends_with("has been replaced"),
        "{}",
        stopped.message
    );
    probe.assert_released();
}

#[tokio::test]
async fn sibling_case_collisions_come_from_the_name_snapshot() {
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let entry = |name: &str| (OsString::from(name), Entry::File);
    let folder = |name: &str| child(Node::folder(&probe, parent.join(name), files(1)));
    let entries = vec![
        entry("ab"),
        entry("AB"),
        folder("Dir"),
        folder("DIR"),
        entry("Cd"),
        entry("ef"),
    ];
    let root = Node::folder(&probe, parent.clone(), entries);
    let job = fake_job(&probe, root, lower(false), MetadataConflictPolicy::SkipAll);
    let (progress, result) = execute(job).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!(
        (result.processed, result.skipped, result.failed),
        (7, 3, 0),
        "{progress:?}"
    );
    // "AB" would take the existing "ab"; "Dir" and "DIR" would both become "dir".
    let mut skipped: Vec<_> = progress.errors.iter().map(|e| e.path.clone()).collect();
    skipped.sort();
    assert_eq!(
        skipped,
        [parent.join("AB"), parent.join("DIR"), parent.join("Dir")]
    );
    assert!(progress.errors.iter().all(|error| {
        error.kind == io::ErrorKind::AlreadyExists
            && error.message == "another entry of this folder has or would get the same name"
    }));
    // A name that stays keeps its other changes; neither conflicting folder is listed.
    assert_eq!(probe.name_given(&parent.join("ab")).unwrap(), "ab");
    assert_eq!(probe.name_given(&parent.join("Cd")).unwrap(), "cd");
    assert_eq!(probe.name_given(&parent.join("ef")).unwrap(), "ef");
    assert_eq!(probe.name_given(&parent).unwrap(), "parent");
    assert_eq!(probe.listings().len(), 1);
    probe.assert_released();
}

#[tokio::test]
async fn an_unchanged_sibling_keeps_its_changes_next_to_one_that_fails_validation() {
    // A link "AB" cannot take a time; the file "ab" keeps its name, so it is no conflict.
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let link = Node::new(
        &probe,
        parent.join("AB"),
        EntryKind::Symlink,
        vec![],
        Quirk::NoTimes,
    );
    let entries = vec![(OsString::from("ab"), Entry::File), child(link)];
    let root = Node::folder(&probe, parent.clone(), entries);
    let job = fake_job(&probe, root, lower(true), MetadataConflictPolicy::Skip);
    let (progress, result) = execute(job).await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.skipped, result.failed), (3, 0, 1));
    assert_eq!(progress.errors[0].path, parent.join("AB"));
    assert_eq!(progress.errors[0].kind, io::ErrorKind::Unsupported);
    assert_eq!(probe.name_given(&parent.join("ab")).unwrap(), "ab");
    probe.assert_released();
}

#[tokio::test]
async fn listing_failures_keep_their_kind_and_text() {
    for (kind, expected) in [
        (
            io::ErrorKind::NotFound,
            "the adapter failed for another reason".to_owned(),
        ),
        (
            io::ErrorKind::Other,
            format!(
                "{FULL_BUDGET}. The file system reported: the adapter failed for another \
                 reason"
            ),
        ),
    ] {
        let probe = Probe::new();
        let parent = PathBuf::from("PARENT");
        let root = Node::new(
            &probe,
            parent.clone(),
            EntryKind::Directory,
            files(1),
            Quirk::Listing(kind),
        );
        let (progress, result) = run(&probe, root).await;
        assert_eq!(progress.state, OperationState::CompletedWithErrors);
        assert_eq!((result.processed, result.changed, result.failed), (1, 0, 1));
        assert_eq!(progress.errors[0].kind, kind);
        assert_eq!(progress.errors[0].message, expected);
        assert_eq!(probe.listings().len(), 1, "a refused folder is listed once");
        assert!(probe.pins().is_empty());
        probe.assert_released();
    }
}

#[tokio::test]
async fn cancelling_in_the_middle_of_a_folder_stops_pinning_and_releases_every_pin() {
    let probe = Probe::new();
    let parent = PathBuf::from("PARENT");
    let mut entries = files(10);
    entries[3].1 = Entry::Node(Node::new(
        &probe,
        parent.join("003"),
        EntryKind::File,
        vec![],
        Quirk::Cancels,
    ));
    let (progress, result) = run(&probe, Node::folder(&probe, parent.clone(), entries)).await;
    assert_eq!(progress.state, OperationState::Cancelled);
    assert_eq!((result.processed, result.failed), (4, 0));
    let processed: Vec<_> = ["000", "001", "002", "003"]
        .iter()
        .map(|name| parent.join(name))
        .collect();
    assert_eq!(
        probe.pins(),
        processed,
        "nothing is pinned after cancellation"
    );
    assert_eq!(probe.edited_paths(), processed, "the folder is not edited");
    probe.assert_released();
}
