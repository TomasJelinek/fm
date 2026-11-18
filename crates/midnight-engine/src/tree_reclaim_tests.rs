//! The inline tree is its mount's only lister. After applying completions it
//! releases the provider references its model no longer holds, so churn under a
//! watched expanded tree keeps both reference tables bounded, while live rows,
//! checkpoints and captured targets keep working.
use super::*;
use crate::providers::NativeProvider;
use midnight_core::{
    DirectoryIndex, DirectoryService, Entry, FileClipboardIntent, FsError, SpaceInfo,
    StorageService, TreeClipboardRequest, TreeClipboardScope, TreeEvent, TreeRefreshRequest,
    TreeSelectionAction, Volume, VolumeService,
};
use std::{
    io,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64},
};

/// Released references the tree tolerates before reclaiming (`RECLAIM_MIN`).
const MIN_RELEASED: usize = 4_096;

/// A watched native folder whose temporary entries are replaced between sweeps:
/// the root holds `keep/` and fresh `*.tmp` files; `keep/` holds `inner/` and
/// fresh temporary folders, and can fail after enumerating them.
struct Churn {
    root: PathBuf,
    generation: AtomicU64,
    files: u64,
    folders: u64,
    fail_keep: AtomicBool,
}

impl Churn {
    fn new(files: u64, folders: u64) -> Arc<Self> {
        Arc::new(Self {
            root: std::env::current_dir()
                .unwrap()
                .join("_build")
                .join("tree-reclaim-test"),
            generation: AtomicU64::new(0),
            files,
            folders,
            fail_keep: AtomicBool::new(false),
        })
    }

    fn keep(&self) -> PathBuf {
        self.root.join("keep")
    }

    fn listing(&self, path: &Path) -> Vec<Entry> {
        let generation = self.generation.load(Ordering::SeqCst);
        let (stable, fresh) = if path == self.root {
            let files = (0..self.files).map(|index| {
                let name = format!("tmp-{generation}-{index}.tmp");
                (self.root.join(name), EntryKind::File)
            });
            (self.keep(), files.collect::<Vec<_>>())
        } else if path == self.keep() {
            let folders = (0..self.folders).map(|index| {
                let name = format!("tmpdir-{generation}-{index}");
                (self.keep().join(name), EntryKind::Directory)
            });
            (self.keep().join("inner"), folders.collect())
        } else {
            return Vec::new();
        };
        std::iter::once((stable, EntryKind::Directory))
            .chain(fresh)
            .enumerate()
            .map(|(index, (path, kind))| Entry::new(EntryId(index as u64 + 1), path, kind))
            .collect()
    }
}

impl DirectoryService for Churn {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        for entry in self.listing(path) {
            if !emit(entry) {
                return Ok(());
            }
        }
        if path == self.keep() && self.fail_keep.load(Ordering::SeqCst) {
            return Err(FsError::from_io(
                "read directory",
                path,
                io::Error::other("fixture failure after enumeration"),
            ));
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Churn {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Churn {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(FsError::from_io(
            "test",
            path,
            io::ErrorKind::Unsupported.into(),
        ))
    }
}

/// A session over the production native adapter, seeded like a completed root.
fn native_session(churn: &Arc<Churn>) -> Session {
    let entries: Arc<[Entry]> = churn.listing(&churn.root).into();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(7),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries: entries.clone(),
    };
    let cancel = Cancellation::default();
    let opened = NativeProvider::new(churn.clone())
        .capture_root(&churn.root, &entries, &cancel)
        .unwrap();
    Session::new(
        root,
        opened,
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &cancel,
        crate::ReportArena::testing(),
    )
    .unwrap()
}

fn row(session: &Session, name: &str) -> TreeRowId {
    session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.display_name == name)
        .unwrap()
        .id
}

fn expand(session: &mut Session, name: &str) {
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let request = session.last_request.checked_next().unwrap();
    let row = row(session, name);
    session
        .branch(&view, request, row, BranchAction::Expand)
        .unwrap();
    let job = session.take_work().unwrap().unwrap();
    assert!(session.apply(job.run(), &Cancellation::default()).unwrap());
}

/// The root mount's references: (issued, native records, live occurrences).
fn references(session: &Session) -> (usize, usize, usize) {
    let (issued, native) = session.tree.source_reference_counts().unwrap();
    (
        issued,
        native.unwrap_or(issued),
        session.tree.source_occurrence_count(),
    )
}

/// After each completion, both tables hold fewer released references than the
/// reclamation threshold. Returns the larger table.
fn assert_bounded(session: &Session) -> usize {
    let (issued, native, live) = references(session);
    let held = issued.max(native);
    assert!(
        held < live + live.max(MIN_RELEASED),
        "{issued} issued and {native} native references for {live} occurrences"
    );
    held
}

/// One watch-requested maintenance sweep of the loaded branches. Returns its
/// event and the number of completions that published a projection.
fn sweep(session: &mut Session, bounded: bool) -> (TreeEvent, usize) {
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let request = TreeRefreshRequest {
        view,
        request: session.last_request.checked_next().unwrap(),
    };
    session
        .refresh_loaded(&request, &Cancellation::default())
        .unwrap();
    let mut published = 0;
    for _ in 0..16 {
        if let Some(job) = session.take_work().unwrap() {
            published += usize::from(session.apply(job.run(), &Cancellation::default()).unwrap());
            if bounded {
                assert_bounded(session);
            }
        }
        if let Some(event) = session.finish_refresh() {
            return (event, published);
        }
    }
    panic!("bounded maintenance did not finish")
}

#[test]
fn watched_temporary_churn_keeps_reference_tables_bounded_and_live_rows_intact() {
    let churn = Churn::new(1_000, 100);
    let mut session = native_session(&churn);
    expand(&mut session, "keep");
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let (keep, inner) = (row(&session, "keep"), row(&session, "inner"));
    let keep_branch = session.projection_state.branch(keep).unwrap();
    session
        .select(&TreeSelectionRequest {
            view: view.clone(),
            revision: SelectionRevision(session.selection.revision.0 + 1),
            seed: None,
            seed_revision: session.selection.revision,
            action: TreeSelectionAction::Toggle(keep),
        })
        .unwrap();
    // A queued copy source whose row and reference will be gone.
    let temporary = row(&session, "tmp-0-0.tmp");
    let copy_source = session
        .capture_target(&view, temporary, &Cancellation::default())
        .unwrap();

    let mut previous = assert_bounded(&session);
    let mut reclaimed = 0;
    // 6,600 fresh names: without reclamation the tables pass the bound in sweep 4.
    for generation in 1..=6 {
        churn.generation.store(generation, Ordering::SeqCst);
        let (event, published) = sweep(&mut session, true);
        assert!(matches!(event, TreeEvent::Refreshed { result: Ok(()), .. }));
        assert_eq!(published, 2, "both churning branches changed");
        let held = assert_bounded(&session);
        reclaimed += usize::from(held < previous);
        previous = held;
        // Every sweep's watch scope resolves its loaded branches and children.
        let view = session.projection().clone();
        assert!(session.commit(&view));
        assert_eq!(session.watch_scope().unwrap().unwrap().targets.len(), 2);
    }
    assert!(reclaimed >= 1, "fresh names were never reclaimed");

    // Surviving rows keep their occurrence, expansion and mark.
    assert_eq!(row(&session, "keep"), keep);
    assert_eq!(row(&session, "inner"), inner);
    assert_eq!(session.projection_state.branch(keep), Some(keep_branch));
    assert!(
        session
            .projection()
            .rows
            .iter()
            .any(|row| row.id == keep && row.expanded)
    );
    assert!(session.selection.snapshot.contains_row(keep));
    assert!(matches!(
        copy_source.target.value(),
        Ok(ProviderTarget::Native(path)) if path == &churn.root.join("tmp-0-0.tmp")
    ));

    // Release everything no occurrence holds; listing the same names again keeps
    // every live token, so the unchanged sweep publishes nothing.
    session.tree.reclaim_eagerly();
    let published_view = session.projection().clone();
    let (event, published) = sweep(&mut session, true);
    assert!(matches!(event, TreeEvent::Refreshed { result: Ok(()), .. }));
    assert_eq!(published, 0);
    let (issued, native, live) = references(&session);
    assert_eq!((issued, native), (live, live));
    let (event, published) = sweep(&mut session, true);
    assert!(matches!(event, TreeEvent::Refreshed { result: Ok(()), .. }));
    assert_eq!(published, 0, "reclamation changed a live token");
    assert!(Arc::ptr_eq(session.projection(), &published_view));
    assert_eq!(references(&session), (live, live, live));
}

/// A paste request whose destination is `row` (no marks, so the fallback row).
fn paste_request(session: &mut Session, row: TreeRowId) -> TreeClipboardRequest {
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    TreeClipboardRequest {
        scope: TreeClipboardScope::SelectionOrFallback,
        intent: FileClipboardIntent::Copy,
        view: publication.view,
        request: session.last_request.checked_next().unwrap(),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        fallback: Some(row),
        cancel: Cancellation::default(),
    }
}

#[test]
fn watch_paste_and_drop_targets_of_a_vanished_row_keep_resolving_after_reclamation() {
    let churn = Churn::new(10, 10);
    let mut session = native_session(&churn);
    expand(&mut session, "keep");
    expand(&mut session, "tmpdir-0-0");
    session.tree.reclaim_eagerly();
    let folder = churn.keep().join("tmpdir-0-0");
    let folder_row = row(&session, "tmpdir-0-0");
    // An old watch scope observing the temporary folder, and a paste and a drop
    // destination captured on its row.
    let request = paste_request(&mut session, folder_row);
    let scope = session.watch_scope().unwrap().unwrap();
    let watched = scope
        .targets
        .iter()
        .find(|target| target.path == folder)
        .unwrap()
        .clone();
    let paste = session
        .capture_clipboard(
            &request,
            &Arc::new(Semaphore::new(1)),
            &Cancellation::default(),
        )
        .unwrap();
    let drop_target = session
        .capture_drop_directory(
            &request.view,
            folder_row,
            session.last_request.checked_next().unwrap(),
            &Arc::new(Semaphore::new(1)),
            &Cancellation::default(),
        )
        .unwrap();

    // The folder disappears and the sweep's completions release its reference.
    churn.generation.store(1, Ordering::SeqCst);
    let (event, _) = sweep(&mut session, true);
    assert!(matches!(event, TreeEvent::Refreshed { result: Ok(()), .. }));
    assert!(!session.retains_row(folder_row));
    let (issued, native, live) = references(&session);
    assert_eq!((issued, native), (live, live));

    // `ScopeTarget::probe` checks exactly this before and after each observation,
    // so the old scope keeps probing without a spurious failure until replaced.
    assert!(matches!(
        watched._mounted.value(),
        Ok(ProviderTarget::Native(path)) if *path == watched.path
    ));
    // Paste resolves its destination before and after the native clipboard read.
    let files = paste.resolve(&Cancellation::default()).unwrap();
    assert_eq!(files.paths, std::slice::from_ref(&folder));
    assert_eq!(
        drop_target.resolve(&Cancellation::default()).unwrap(),
        folder
    );
    let view = session.projection().clone();
    assert!(session.commit(&view));
    assert_eq!(session.watch_scope().unwrap().unwrap().targets.len(), 2);
}

#[test]
fn failing_listings_of_fresh_folders_keep_unissued_native_records_bounded() {
    let churn = Churn::new(10, 1_000);
    let mut session = native_session(&churn);
    expand(&mut session, "keep");
    churn.fail_keep.store(true, Ordering::SeqCst);
    let mut previous = assert_bounded(&session);
    let mut reclaimed = 0;
    // Each failed listing binds 1,000 folders natively that the lease never
    // issues; only the provider's record count can reveal them. Without
    // reclamation the native table passes the bound in sweep 5.
    for generation in 1..=6 {
        churn.generation.store(generation, Ordering::SeqCst);
        let (event, _) = sweep(&mut session, true);
        assert!(matches!(event, TreeEvent::Refreshed { result: Err(_), .. }));
        let (issued, _, live) = references(&session);
        assert!(issued < live + 200, "the lease issued a failed listing");
        let held = assert_bounded(&session);
        reclaimed += usize::from(held < previous);
        previous = held;
    }
    assert!(reclaimed >= 1, "unissued records were never reclaimed");
    // The failed branch keeps its last successful children.
    assert!(
        session
            .projection()
            .rows
            .iter()
            .any(|row| row.entry.display_name == "tmpdir-0-999")
    );
}

#[test]
fn reclamation_waits_for_an_unpublished_recursive_batch_and_rollback_restores_valid_occurrences() {
    let mut fixture = fixture();
    let session = &mut fixture.session;
    expand(session, "z-parent");
    session.tree.reclaim_eagerly();
    // The walk re-lists z-parent, which no longer contains `child`.
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![directory(7, "keep")]);
    let child = row(session, "child");
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let request = session.last_request.checked_next().unwrap();
    let parent = row(session, "z-parent");
    session
        .branch(&view, request, parent, BranchAction::ExpandRecursive)
        .unwrap();
    let job = session.take_work().unwrap().unwrap();
    assert!(job.recursive());
    assert!(session.set_recursive_batch_elapsed(false));
    assert!(!session.apply(job.run(), &Cancellation::default()).unwrap());
    assert!(session.recursive_batch_active());
    // `child` left the model, but the batch checkpoint can restore it, so its
    // reference stays issued while the batch is unpublished.
    let (issued, _, live) = references(session);
    assert_eq!(issued, live + 1);

    // Explicit input rolls the batch back; the restored row still resolves.
    assert!(session.rollback_unpublished());
    let view = session.projection().clone();
    assert!(session.commit(&view));
    let restored = session
        .capture_target(&view, child, &Cancellation::default())
        .unwrap();
    assert!(matches!(
        restored.target.value(),
        Ok(ProviderTarget::Native(path))
            if path == &fixture.graph.root_path.join("z-parent").join("child")
    ));
    // The next settled completion releases the rolled-back `keep` reference.
    expand(session, "a-parent");
    let (issued, _, live) = references(session);
    assert_eq!(issued, live);
}

/// Table sizes over 105 watch sweeps of 1,000 fresh files and 100 fresh folders
/// (115,500 names, past the 100,000-reference caps).
/// Run: cargo test -p midnight-engine --lib tree_reference_churn_harness -- --ignored --nocapture
#[test]
#[ignore = "measurement harness"]
fn tree_reference_churn_harness() {
    let churn = Churn::new(1_000, 100);
    let mut session = native_session(&churn);
    expand(&mut session, "keep");
    let started = std::time::Instant::now();
    let (mut peak, mut failures) = (0, 0);
    for generation in 1..=105 {
        churn.generation.store(generation, Ordering::SeqCst);
        let (event, _) = sweep(&mut session, false);
        let failed = !matches!(event, TreeEvent::Refreshed { result: Ok(()), .. });
        failures += usize::from(failed);
        let (issued, native, live) = references(&session);
        peak = peak.max(issued.max(native));
        if generation % 15 == 0 || (failed && failures == 1) {
            println!(
                "sweep {generation}: issued {issued}, native {native}, live {live}, \
                 failed sweeps {failures}, {:?}",
                started.elapsed()
            );
        }
    }
    println!(
        "peak {peak} references, {failures} failed sweeps, {:?}",
        started.elapsed()
    );
}
