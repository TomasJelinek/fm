//! Recursive metadata traversal through the real native adapters: folders far larger than
//! the 256-target selection budget, other jobs holding every shared target, previews
//! prepared while a recursive edit runs, the native name snapshot and its pins, entries
//! changed between a folder's snapshot and their pin, and sibling case conflicts.
use super::*;
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio::time::Instant;

/// Elapses only when a job stalls; large fixtures stay far below it.
const FAILURE_BOUND: Duration = Duration::from_secs(60);

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

fn files(folder: &Path, prefix: &str, count: usize) {
    for index in 0..count {
        fs::write(folder.join(format!("{prefix}{index:04}.TXT")), b"payload").unwrap();
    }
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

/// Waits for `id`'s preview of `revision`; any terminal state first fails the test.
async fn reviewed(h: &mut Harness, id: OperationId, revision: u64) -> Arc<MetadataPreview> {
    let deadline = Instant::now() + FAILURE_BOUND;
    loop {
        match next_event(h, deadline).await {
            Event::Operation(OperationEvent::MetadataPreview(preview))
                if preview.id == id && preview.revision == revision =>
            {
                return preview;
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

async fn conflict(h: &mut Harness, id: OperationId) -> Arc<MetadataConflict> {
    let deadline = Instant::now() + FAILURE_BOUND;
    loop {
        match next_event(h, deadline).await {
            Event::Operation(OperationEvent::MetadataConflict(conflict)) if conflict.id == id => {
                return conflict;
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

/// The terminal progress and result of every job in `ids`, in that order.
async fn finished(
    h: &mut Harness,
    ids: &[OperationId],
) -> Vec<(Arc<OperationProgress>, Arc<MetadataResult>)> {
    let deadline = Instant::now() + FAILURE_BOUND;
    let (mut results, mut terminal) = (HashMap::new(), HashMap::new());
    while ids
        .iter()
        .any(|id| !terminal.contains_key(id) || !results.contains_key(id))
    {
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

/// Prepares, previews and confirms `request` with `patch` and `policy`.
async fn confirmed(
    h: &mut Harness,
    request: PrepareOperation,
    patch: MetadataPatch,
    policy: MetadataConflictPolicy,
) {
    let id = request.id;
    h.controller.prepare(request).await;
    reviewed(h, id, 0).await;
    h.controller.preview_metadata(id, 1, patch).await;
    reviewed(h, id, 1).await;
    h.controller.confirm_metadata(id, 1, policy).await;
}

#[tokio::test]
async fn recursive_edit_changes_every_entry_of_a_folder_with_1000_entries() {
    // Only the files change case. Windows refuses to rename a folder while any file below
    // it is open, and a virus scanner may still be reading the files just created.
    let mut h = Harness::new(&[("parent", true)]);
    let parent = h.directory.path().join("parent");
    files(&parent, "FILE", 1000);
    fs::create_dir(parent.join("sub")).unwrap();
    files(&parent.join("sub"), "NESTED", 300);
    fs::create_dir_all(parent.join("d1/d2/d3")).unwrap();
    fs::write(parent.join("d1/d2/d3/DEEP.TXT"), b"deep").unwrap();
    let patch = MetadataPatch {
        case: Some(CaseChange::default()),
        ..modified(true)
    };
    let request = h.request.clone();
    confirmed(&mut h, request, patch, MetadataConflictPolicy::Skip).await;
    let done = finished(&mut h, &[OperationId(1)]).await;
    let (progress, result) = &done[0];
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!(
        (
            result.processed,
            result.changed,
            result.skipped,
            result.failed
        ),
        (1306, 1306, 0, 0)
    );
    let mut expected = vec![
        parent.clone(),
        parent.join("sub"),
        parent.join("d1"),
        parent.join("d1/d2"),
        parent.join("d1/d2/d3"),
        parent.join("d1/d2/d3/deep.txt"),
    ];
    expected.extend((0..1000).map(|index| parent.join(format!("file{index:04}.txt"))));
    expected.extend((0..300).map(|index| parent.join(format!("sub/nested{index:04}.txt"))));
    for path in expected {
        let name = path.file_name().unwrap().to_owned();
        let listed = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .any(|entry| entry.unwrap().file_name() == name);
        assert!(listed, "{path:?} is renamed to lower case");
        assert_eq!(modified_at(&path), edited_time(), "{path:?}");
    }
}

#[tokio::test]
async fn recursive_edit_ignores_shared_targets_held_by_other_jobs() {
    let h = Harness::new(&[("PARENT", true)]);
    files(&h.directory.path().join("PARENT"), "FILE", 1000);
    let cancel = Cancellation::default();
    let budget = Arc::new(Semaphore::new(MAX_METADATA_TARGETS));
    let mut job = worker::prepare(
        h.request.clone(),
        Some(Arc::new(LocalFileSystem::new(None))),
        cancel.clone(),
        Arc::new(Mutex::new(worker::initial(&h.request))),
        budget.clone(),
    )
    .unwrap();
    job.metadata
        .as_mut()
        .unwrap()
        .update(1, modified(true), &job.targets, &cancel);
    // Pending previews and queued jobs hold every shared target the root pin left.
    let others = budget
        .clone()
        .try_acquire_many_owned(budget.available_permits() as u32)
        .unwrap();
    assert_eq!(others.num_permits(), MAX_METADATA_TARGETS - 1);
    let (progress, result) = execute(job).await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!(
        (result.processed, result.changed, result.failed),
        (1001, 1001, 0)
    );
    let parent = h.directory.path().join("PARENT");
    assert_eq!(modified_at(&parent), edited_time());
    for index in 0..1000 {
        let path = parent.join(format!("FILE{index:04}.TXT"));
        assert_eq!(modified_at(&path), edited_time(), "{path:?}");
    }
    drop(others);
    assert_eq!(budget.available_permits(), MAX_METADATA_TARGETS);
}

#[tokio::test]
async fn a_running_recursive_edit_leaves_the_shared_budget_to_previews() {
    let names: Vec<_> = (1..MAX_METADATA_TARGETS)
        .map(|index| format!("F{index:03}"))
        .collect();
    let mut entries = vec![("PARENT", true)];
    entries.extend(names.iter().map(|name| (name.as_str(), false)));
    let mut h = Harness::new(&entries);
    let parent = h.directory.path().join("PARENT");
    files(&parent, "ENTRY", 198);
    // Upper case maps "straße" to the existing "STRASSE" on every file system, so the
    // recursive edit stops at a conflict prompt in the middle of PARENT.
    fs::write(parent.join("straße"), b"sharp s").unwrap();
    fs::write(parent.join("STRASSE"), b"double s").unwrap();
    let (recursive, preview) = (OperationId(1), OperationId(2));
    let upper = MetadataPatch {
        case: Some(CaseChange {
            mode: CaseMode::Upper,
            scope: CaseScope::Whole,
        }),
        recursive: true,
        ..Default::default()
    };
    let request = select(&h, recursive, |index| index == 0);
    confirmed(&mut h, request, upper, MetadataConflictPolicy::Skip).await;
    let asked = conflict(&mut h, recursive).await;
    // While the edit waits for that decision, a preview pins all 255 other entries.
    h.controller
        .prepare(select(&h, preview, |index| index > 0))
        .await;
    let previewed = reviewed(&mut h, preview, 0).await;
    assert_eq!(previewed.total, 255);
    h.controller
        .resolve_metadata_conflict(recursive, asked.token, MetadataConflictPolicy::SkipAll)
        .await;
    let done = finished(&mut h, &[recursive]).await;
    let (progress, result) = &done[0];
    assert_eq!(
        progress.state,
        OperationState::CompletedWithErrors,
        "{progress:?}"
    );
    // Only "straße" is a conflict: "STRASSE" keeps its name.
    assert_eq!(
        (result.processed, result.skipped, result.failed),
        (201, 1, 0),
        "{progress:?}"
    );
    assert_eq!(fs::read(parent.join("straße")).unwrap(), b"sharp s");
    assert_eq!(fs::read(parent.join("STRASSE")).unwrap(), b"double s");
    h.controller.cancel(preview);
}

#[tokio::test]
async fn queued_metadata_jobs_holding_every_shared_target_cannot_limit_running_recursive_edits() {
    let names: Vec<_> = (0..MAX_METADATA_TARGETS - 3)
        .map(|index| format!("F{index:03}"))
        .collect();
    let mut entries = vec![("PARENT", true), ("PARENT2", true)];
    entries.extend(names.iter().map(|name| (name.as_str(), false)));
    let mut h = Harness::new(&entries);
    files(&h.directory.path().join("PARENT"), "FILE", 1000);
    files(&h.directory.path().join("PARENT2"), "FILE", 1000);
    let (first, second, flat) = (OperationId(1), OperationId(2), OperationId(3));
    // A running Copy's mutation-gate share keeps every confirmed job queued.
    let copy = h.gate.clone().try_acquire_owned().unwrap();
    let request = select(&h, first, |index| index == 0);
    confirmed(
        &mut h,
        request,
        modified(true),
        MetadataConflictPolicy::Skip,
    )
    .await;
    queued(&mut h, first).await;
    let request = select(&h, second, |index| index == 1);
    confirmed(
        &mut h,
        request,
        modified(true),
        MetadataConflictPolicy::Skip,
    )
    .await;
    queued(&mut h, second).await;
    // The last job pins all 253 files and keeps them while it waits for both edits.
    let request = select(&h, flat, |index| index > 1);
    confirmed(
        &mut h,
        request,
        modified(false),
        MetadataConflictPolicy::Skip,
    )
    .await;
    queued(&mut h, flat).await;
    drop(copy);
    let done = finished(&mut h, &[first, second, flat]).await;
    for ((progress, result), processed) in done.iter().zip([1001, 1001, 253]) {
        assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
        assert_eq!(
            (result.processed, result.changed, result.failed),
            (processed, processed, 0),
            "{progress:?}"
        );
    }
    for folder in ["PARENT", "PARENT2"] {
        let folder = h.directory.path().join(folder);
        assert_eq!(modified_at(&folder), edited_time());
        for index in 0..1000 {
            let path = folder.join(format!("FILE{index:04}.TXT"));
            assert_eq!(modified_at(&path), edited_time(), "{path:?}");
        }
    }
}

#[tokio::test]
async fn sibling_case_conflicts_skip_only_the_renaming_entry() {
    let mut h = Harness::new(&[("PARENT", true)]);
    let parent = h.directory.path().join("PARENT");
    // Distinct names even on case-insensitive NTFS; Upper maps "straße" to "STRASSE".
    for (name, contents) in [
        ("straße", "sharp s"),
        ("STRASSE", "double s"),
        ("Other", "o"),
    ] {
        fs::write(parent.join(name), contents).unwrap();
    }
    let upper = MetadataPatch {
        case: Some(CaseChange {
            mode: CaseMode::Upper,
            scope: CaseScope::Whole,
        }),
        recursive: true,
        ..Default::default()
    };
    let request = h.request.clone();
    confirmed(&mut h, request, upper, MetadataConflictPolicy::SkipAll).await;
    let done = finished(&mut h, &[OperationId(1)]).await;
    let (progress, result) = &done[0];
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!(
        (
            result.processed,
            result.changed,
            result.skipped,
            result.failed
        ),
        (4, 1, 1, 0),
        "{progress:?}"
    );
    assert_eq!(progress.errors[0].path, parent.join("straße"));
    assert_eq!(progress.errors[0].kind, io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(parent.join("straße")).unwrap(), b"sharp s");
    assert_eq!(fs::read(parent.join("STRASSE")).unwrap(), b"double s");
    let names: Vec<_> = fs::read_dir(&parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(names.contains(&OsString::from("OTHER")), "{names:?}");
}

#[test]
fn native_listing_refuses_more_names_or_name_bytes_than_allowed() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("PARENT");
    fs::create_dir(&parent).unwrap();
    for name in ["ab", "文", "c"] {
        fs::write(parent.join(name), name.as_bytes()).unwrap();
    }
    let cancel = Cancellation::default();
    let prepared = LocalFileSystem::new(None)
        .prepare(&parent, &cancel)
        .unwrap();
    // The names are 2 + 3 + 1 bytes, each counted as its `OsStr::len`.
    for (max_names, max_bytes) in [(2, MAX_METADATA_NAMES_BYTES), (3, 5)] {
        let Err(refusal) = prepared.metadata_children(&cancel, max_names, max_bytes) else {
            panic!("{max_names} names and {max_bytes} bytes must be refused");
        };
        assert_eq!(refusal.kind, io::ErrorKind::Other, "{refusal:?}");
    }
    let mut children = prepared.metadata_children(&cancel, 3, 6).unwrap();
    let mut names = children.names().to_vec();
    names.sort();
    assert_eq!(names, ["ab", "c", "文"]);
    let position = |names: &[OsString], name: &str| {
        names.iter().position(|captured| captured == name).unwrap()
    };
    // An entry removed after the snapshot fails alone and keeps its captured name.
    fs::remove_file(parent.join("c")).unwrap();
    let removed = position(children.names(), "c");
    let missing = children.pin(removed, &cancel).err().unwrap();
    assert_eq!(missing.kind, io::ErrorKind::NotFound, "{missing:?}");
    assert_eq!(children.names()[removed], "c");
    for name in ["ab", "文"] {
        let index = position(children.names(), name);
        let pinned = children.pin(index, &cancel).unwrap();
        assert_eq!(pinned.item().path, parent.join(name));
        assert_eq!(pinned.item().kind, EntryKind::File);
        assert!(
            children.names()[index].is_empty(),
            "pinning releases the name"
        );
        let again = children.pin(index, &cancel).err().unwrap();
        assert_eq!(again.kind, io::ErrorKind::InvalidInput, "{again:?}");
    }
    let outside = children.pin(3, &cancel).err().unwrap();
    assert_eq!(outside.kind, io::ErrorKind::InvalidInput, "{outside:?}");
}

/// A selected folder whose snapshot runs `hook` with each captured name just before that
/// entry is pinned, as another program changing the folder in between.
struct Hooked {
    inner: Arc<dyn PreparedMutation>,
    hook: Arc<dyn Fn(&OsStr) + Send + Sync>,
}

impl PreparedMutation for Hooked {
    fn item(&self) -> &MutationItem {
        self.inner.item()
    }
    fn metadata_snapshot(&self, cancel: &Cancellation) -> Result<MetadataSnapshot, FsError> {
        self.inner.metadata_snapshot(cancel)
    }
    fn metadata_name_available(&self, name: &OsStr, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.metadata_name_available(name, cancel)
    }
    fn metadata_children(
        &self,
        cancel: &Cancellation,
        max_names: usize,
        max_bytes: usize,
    ) -> Result<Box<dyn MetadataChildren>, FsError> {
        Ok(Box::new(HookedChildren {
            inner: self.inner.metadata_children(cancel, max_names, max_bytes)?,
            hook: self.hook.clone(),
        }))
    }
    fn edit_metadata(
        &self,
        patch: &MetadataPatch,
        name: Option<&OsStr>,
        cancel: &Cancellation,
    ) -> MetadataOutcome {
        self.inner.edit_metadata(patch, name, cancel)
    }
    fn rename(&self, name: &OsStr, cancel: &Cancellation) -> Result<RenameCompletion, FsError> {
        self.inner.rename(name, cancel)
    }
    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<DeleteCompletion, FsError> {
        self.inner.delete(mode, cancel, progress)
    }
}

struct HookedChildren {
    inner: Box<dyn MetadataChildren>,
    hook: Arc<dyn Fn(&OsStr) + Send + Sync>,
}

impl MetadataChildren for HookedChildren {
    fn names(&self) -> &[OsString] {
        self.inner.names()
    }
    fn pin(
        &mut self,
        index: usize,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        if let Some(name) = self.inner.names().get(index) {
            (self.hook)(name);
        }
        self.inner.pin(index, cancel)
    }
}

/// One captured root, edited without a pane selection.
struct Captured(Arc<dyn PreparedMutation>);

impl std::fmt::Debug for Captured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Captured")
            .field(&self.0.item().path)
            .finish()
    }
}

impl CapturedMetadataSources for Captured {
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
        1
    }
    fn covered_roots(&self) -> &[bool] {
        &[false]
    }
    fn prepare(
        &self,
        _: &dyn MutationService,
        _: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        Ok(vec![self.0.clone()])
    }
}

/// Runs a confirmed job on the native worker; the bound elapses only on a stall.
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
        panic!("the recursive edit stalled");
    };
    let result = slot.lock().unwrap().take().unwrap();
    (progress.unwrap(), result)
}

/// Edits the real folder at `path` recursively, running `hook` before each pin.
async fn edit_hooked(
    path: &Path,
    hook: impl Fn(&OsStr) + Send + Sync + 'static,
) -> (OperationProgress, Arc<MetadataResult>) {
    let cancel = Cancellation::default();
    let service: Arc<dyn MutationService> = Arc::new(LocalFileSystem::new(None));
    let root = Hooked {
        inner: service.prepare(path, &cancel).unwrap(),
        hook: Arc::new(hook),
    };
    let sources = Captured(Arc::new(root));
    let scope = input::MetadataScope::capture(&sources);
    let directory = PathBuf::from("captured");
    let status = worker::initial_for(
        OperationId(1),
        MutationKind::EditMetadata,
        directory.clone(),
    );
    let mut job = worker::prepare_captured_metadata(
        PrepareCapturedMetadata {
            id: OperationId(1),
            sources: Arc::new(sources),
            source_directory: directory,
        },
        scope,
        Some(service),
        cancel.clone(),
        Arc::new(Mutex::new(status)),
        Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
    )
    .unwrap();
    job.metadata
        .as_mut()
        .unwrap()
        .update(1, modified(true), &job.targets, &cancel);
    execute(job).await
}

fn folder_of_three(root: &Path, name: &str) -> PathBuf {
    let folder = root.join(name);
    fs::create_dir(&folder).unwrap();
    for name in ["A", "B", "C"] {
        fs::write(folder.join(name), b"captured").unwrap();
    }
    folder
}

#[tokio::test]
async fn children_replaced_or_removed_after_the_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let aside = directory.path().join("ASIDE");
    fs::create_dir(&aside).unwrap();

    // Replaced: the entry found at the captured name is edited under its new identity;
    // the captured one, moved aside, is not.
    let parent = folder_of_three(directory.path(), "REPLACED");
    let (folder, held) = (parent.clone(), aside.join("B_old"));
    let (progress, result) = edit_hooked(&parent, move |name| {
        if name == "B" {
            fs::rename(folder.join("B"), &held).unwrap();
            fs::write(folder.join("B"), b"replacement").unwrap();
        }
    })
    .await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.changed, result.failed), (4, 4, 0));
    assert_eq!(fs::read(parent.join("B")).unwrap(), b"replacement");
    for path in [
        &parent,
        &parent.join("A"),
        &parent.join("B"),
        &parent.join("C"),
    ] {
        assert_eq!(modified_at(path), edited_time(), "{path:?}");
    }
    assert_ne!(modified_at(&aside.join("B_old")), edited_time());

    // Removed: that entry fails alone, and its siblings and the folder are still edited.
    let parent = folder_of_three(directory.path(), "REMOVED");
    let folder = parent.clone();
    let (progress, result) = edit_hooked(&parent, move |name| {
        if name == "B" {
            fs::remove_file(folder.join("B")).unwrap();
        }
    })
    .await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (4, 3, 1));
    assert_eq!(progress.errors.len(), 1);
    assert_eq!(progress.errors[0].path, parent.join("B"));
    assert_eq!(progress.errors[0].kind, io::ErrorKind::NotFound);
    for path in [&parent, &parent.join("A"), &parent.join("C")] {
        assert_eq!(modified_at(path), edited_time(), "{path:?}");
    }

    // Moved away before its first entry is pinned: that entry fails its revalidation, and
    // the folder then stops once instead of failing each remaining entry the same way.
    let parent = folder_of_three(directory.path(), "MOVED");
    let (folder, away) = (parent.clone(), directory.path().join("AWAY"));
    let (target, moved) = (away.clone(), std::sync::atomic::AtomicBool::new(false));
    let (progress, result) = edit_hooked(&parent, move |_| {
        if !moved.swap(true, Ordering::SeqCst) {
            fs::rename(&folder, &target).unwrap();
        }
    })
    .await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!((result.processed, result.changed, result.failed), (2, 0, 2));
    // The entry's own failure names the moved folder on Linux and the entry on Windows.
    let entry = &result.sample[0];
    assert_eq!(entry.original_path.parent(), Some(parent.as_path()));
    assert_eq!(entry.error.as_ref().unwrap().kind, io::ErrorKind::NotFound);
    let stopped = &progress.errors[1];
    assert_eq!(stopped.path, parent);
    assert!(
        stopped.message.starts_with(
            "this folder changed while its entries were processed, so 2 of its 3 entries \
             and the folder itself were not changed"
        ),
        "{stopped:?}"
    );
    for path in [&away, &away.join("A"), &away.join("B"), &away.join("C")] {
        assert_ne!(modified_at(path), edited_time(), "{path:?}");
    }
}

#[cfg(unix)]
#[path = "operation_metadata_unix_tests.rs"]
mod unix;
#[cfg(windows)]
#[path = "operation_metadata_windows_tests.rs"]
mod windows;
