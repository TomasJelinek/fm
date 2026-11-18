//! Disclosure probes ask for presence only: they stop at the first folder, issue
//! no child reference and are not bounded by a listing's row limit.
use super::*;
use midnight_core::NodeRef;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Lists the root as one folder (1), folder 1 as `rows`, and counts every entry
/// emitted after the root listing and every observed folder listing.
struct Emitting {
    root: Vec<ProviderEntry>,
    rows: Vec<ProviderEntry>,
    emitted: AtomicUsize,
    observed: AtomicUsize,
}
struct EmittingProvider(Arc<Emitting>);

const OBSERVATION: [u8; 32] = [7; 32];

impl MountedFilesystem for Emitting {
    fn list_folders_observed(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<Option<[u8; 32]>, ProviderError> {
        self.observed.fetch_add(1, Ordering::SeqCst);
        self.list(node, cancel, emit).map(|()| Some(OBSERVATION))
    }

    fn list(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        let rows = if *node == token(0) {
            &self.root
        } else if *node == token(1) {
            &self.rows
        } else {
            return Ok(());
        };
        for row in rows {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            self.emitted.fetch_add(1, Ordering::SeqCst);
            if !emit(row.clone()) {
                break;
            }
        }
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
}
impl FilesystemProvider for EmittingProvider {
    fn resolve(
        &self,
        _: &StoredLocation,
        _: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: self.0.clone(),
            root: token(0),
        })
    }
}

fn emitting_tree(rows: Vec<ProviderEntry>) -> (ProviderRegistry, Arc<Emitting>, DirectoryTree) {
    let emitting = Arc::new(Emitting {
        root: vec![directory(1)],
        rows,
        emitted: AtomicUsize::new(0),
        observed: AtomicUsize::new(0),
    });
    let address = StoredLocation::new(
        ProviderId::new("test.tree.probe").unwrap(),
        NonZeroU32::MIN,
        Arc::from([]),
    );
    let registry = ProviderRegistry::default();
    registry
        .register(
            address.provider().clone(),
            Arc::new(EmittingProvider(emitting.clone())),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let mut tree = DirectoryTree::new(
        location,
        TreeLimits::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    run(&mut tree, None, TreeRequest::Expand);
    emitting.emitted.store(0, Ordering::SeqCst);
    (registry, emitting, tree)
}

fn issued(tree: &DirectoryTree, value: u64) -> bool {
    let (location, root) = tree.navigation_target(tree.root()).unwrap();
    location
        .validate_reference(&NodeRef {
            mount: root.mount,
            token: token(value),
        })
        .is_ok()
}

#[test]
fn a_probe_stops_at_the_first_folder_and_issues_no_child_reference() {
    let mut rows = (100..1_100)
        .map(|value| entry(value, EntryKind::File))
        .collect::<Vec<_>>();
    rows.extend([directory(4), directory(5)]);
    rows.extend((1_100..2_100).map(|value| entry(value, EntryKind::File)));
    let (_registry, provider, mut tree) = emitting_tree(rows);
    let branch = row(&tree, 1).id;
    run(&mut tree, Some(branch), TreeRequest::Probe);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Collapsed);
    assert_eq!(
        provider.emitted.load(Ordering::SeqCst),
        1_001,
        "enumeration stops at the first folder"
    );
    assert!(
        !issued(&tree, 4) && !issued(&tree, 5),
        "a probe grants no child authority"
    );
    // Expansion still lists every folder and issues their references.
    run(&mut tree, Some(branch), TreeRequest::Expand);
    assert_eq!(tree.rows().len(), 3);
    assert!(issued(&tree, 4) && issued(&tree, 5));
}

#[test]
fn a_child_that_repeats_an_ancestor_fails_the_expansion_not_the_probe() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    let parent = row(&tree, 1).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let branch = row(&tree, 4).id;
    // The hidden root's reference reappears below its descendant.
    graph.set(4, vec![directory(0)]);
    run(&mut tree, Some(branch), TreeRequest::Probe);
    assert_eq!(row(&tree, 4).disclosure, Disclosure::Collapsed);
    assert_eq!(row(&tree, 4).error, None);
    run(&mut tree, Some(branch), TreeRequest::Expand);
    assert_eq!(row(&tree, 4).disclosure, Disclosure::Failed);
    assert_eq!(
        row(&tree, 4).error,
        Some(ProviderError::InvalidResponse(
            "ancestor cycle in directory branch"
        ))
    );
}

#[test]
fn a_probe_validates_only_the_entries_before_the_first_folder() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    let parent = row(&tree, 1).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let branch = row(&tree, 4).id;
    let duplicate = Some(ProviderError::InvalidResponse(
        "duplicate or self-referencing listing node",
    ));
    // A token repeated before the first folder fails the probe.
    graph.set(
        4,
        vec![
            entry(8, EntryKind::File),
            entry(8, EntryKind::File),
            directory(9),
        ],
    );
    run(&mut tree, Some(branch), TreeRequest::Probe);
    assert_eq!(row(&tree, 4).disclosure, Disclosure::Failed);
    assert_eq!(row(&tree, 4).error, duplicate);
    // The probe never inspects one repeated after it; the expansion rejects it.
    graph.set(4, vec![directory(9), directory(9)]);
    run(&mut tree, Some(branch), TreeRequest::Probe);
    assert_eq!(row(&tree, 4).disclosure, Disclosure::Collapsed);
    assert_eq!(row(&tree, 4).error, None);
    run(&mut tree, Some(branch), TreeRequest::Expand);
    assert_eq!(row(&tree, 4).disclosure, Disclosure::Failed);
    assert_eq!(row(&tree, 4).error, duplicate);
}

#[test]
fn a_probe_reports_a_branch_with_more_folders_than_a_listing_can_hold() {
    let folders = TreeLimits::default().nodes as u64 + 1;
    let (_registry, provider, mut tree) =
        emitting_tree((10..10 + folders).map(directory).collect());
    let branch = row(&tree, 1).id;
    run(&mut tree, Some(branch), TreeRequest::Probe);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Collapsed);
    assert_eq!(row(&tree, 1).error, None);
    assert_eq!(provider.emitted.load(Ordering::SeqCst), 1);
    // Only an expansion has to hold every child within the listing budget.
    run(&mut tree, Some(branch), TreeRequest::Expand);
    assert_eq!(row(&tree, 1).error, Some(ProviderError::ResourceLimited));
}

#[test]
fn only_an_expansion_in_a_watched_tree_keeps_the_provider_observation() {
    let (_registry, provider, mut tree) = emitting_tree(vec![directory(4)]);
    let branch = row(&tree, 1).id;
    let observation = |tree: &DirectoryTree| tree.watch_listing(branch).unwrap().2;
    // An unwatched tree never asks its provider for an observation.
    run(&mut tree, Some(branch), TreeRequest::Expand);
    assert_eq!(observation(&tree), None);
    tree.observe_listings();
    // A probe asks for presence only, and a refresh starts no branch watch.
    let child = row(&tree, 4).id;
    run(&mut tree, Some(child), TreeRequest::Probe);
    run(&mut tree, Some(branch), TreeRequest::Refresh);
    assert_eq!(observation(&tree), None);
    assert_eq!(provider.observed.load(Ordering::SeqCst), 0);
    run(&mut tree, Some(branch), TreeRequest::Expand);
    assert_eq!(provider.observed.load(Ordering::SeqCst), 1);
    assert_eq!(observation(&tree), Some(OBSERVATION));
    // A later refresh replaces the listing and drops its observation, so that
    // branch's first watch probe still requests maintenance.
    run(&mut tree, Some(branch), TreeRequest::Refresh);
    assert_eq!(observation(&tree), None);
    assert_eq!(provider.observed.load(Ordering::SeqCst), 1);
}

#[test]
fn an_observed_native_folder_listing_matches_the_watch_probe() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("file.txt"), b"contents").unwrap();
    std::fs::write(directory.path().join(".hidden"), b"").unwrap();
    std::fs::create_dir(directory.path().join("folder")).unwrap();
    let filesystem: Arc<dyn midnight_core::FileSystem> =
        Arc::new(midnight_platform::LocalFileSystem::new(None));
    let address = native_location(directory.path()).unwrap();
    let registry = ProviderRegistry::default();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(filesystem.clone())),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let cancel = Cancellation::default();
    let (rows, observation) = location
        .list_folders_observed(location.root(), &cancel, 8)
        .unwrap();
    assert_eq!(rows.len(), 1, "only the folder becomes a row");
    let probe =
        crate::watch::signature::probe(filesystem.as_ref(), directory.path(), false, &cancel)
            .unwrap();
    assert_eq!(
        observation,
        Some(probe.full),
        "files and hidden entries join the observation"
    );
}

/// Before/after evidence for presence probes on a real directory. Run with
/// `--ignored --nocapture probe_timing`; the previous probe enumerated with
/// `list_folders` at the tree's 10,000-row limit.
#[test]
#[ignore = "timing harness for presence probes on a real directory"]
fn probe_timing_on_a_native_directory() {
    use std::time::{Duration, Instant};
    const FILES: usize = 20_000;
    const FOLDERS: usize = 2_000;
    let directory = tempfile::tempdir().unwrap();
    let create = |name: &str, files: usize, folders: &[&str]| {
        let path = directory.path().join(name);
        std::fs::create_dir(&path).unwrap();
        for index in 0..files {
            std::fs::File::create(path.join(format!("file-{index:05}.txt"))).unwrap();
        }
        for folder in folders {
            std::fs::create_dir(path.join(folder)).unwrap();
        }
    };
    // NTFS enumerates in name order: "~" sorts after "file-", "0" before it.
    create("files-then-folder", FILES, &["~folder"]);
    create("folder-then-files", FILES, &["0-folder"]);
    let folders = (0..FOLDERS)
        .map(|index| format!("folder-{index:05}"))
        .collect::<Vec<_>>();
    create(
        "folders",
        0,
        &folders.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let address = native_location(directory.path()).unwrap();
    let registry = ProviderRegistry::default();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(
                midnight_platform::LocalFileSystem::new(None),
            ))),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let mut tree = DirectoryTree::new(
        location,
        TreeLimits::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    run(&mut tree, None, TreeRequest::Expand);
    let median = |mut samples: Vec<Duration>| {
        samples.sort();
        samples[samples.len() / 2]
    };
    let cancel = Cancellation::default();
    for name in ["files-then-folder", "folder-then-files", "folders"] {
        let branch = tree
            .rows()
            .into_iter()
            .find(|row| row.entry.display_name == name)
            .unwrap()
            .id;
        let (location, node) = tree.navigation_target(branch).unwrap();
        let timed = |sample: &mut dyn FnMut()| {
            median(
                (0..9)
                    .map(|_| {
                        let start = Instant::now();
                        sample();
                        start.elapsed()
                    })
                    .collect(),
            )
        };
        let listing = timed(&mut || {
            location.list_folders(&node, &cancel, 10_000).unwrap();
        });
        let presence = timed(&mut || {
            assert!(location.has_folders(&node, &cancel).unwrap());
        });
        let probe = timed(&mut || {
            run(&mut tree, Some(branch), TreeRequest::Probe);
        });
        println!(
            "{name}: folder listing (previous probe enumeration) {listing:?}, \
             has_folders {presence:?}, tree probe {probe:?}"
        );
    }
}
