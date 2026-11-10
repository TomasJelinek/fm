use super::{tree::*, *};
use midnight_core::ProviderId;
use std::{collections::HashMap, num::NonZeroU32, sync::Mutex};

#[path = "tree_failure_tests.rs"]
mod failure_tests;

#[derive(Default)]
struct Graph(Mutex<HashMap<u64, Result<Vec<ProviderEntry>, ProviderError>>>);
struct Provider(Arc<Graph>);

fn token(value: u64) -> NodeToken {
    NodeToken::new(&value.to_le_bytes()).unwrap()
}
fn entry(value: u64, kind: EntryKind) -> ProviderEntry {
    ProviderEntry {
        token: token(value),
        raw_name: Arc::from([0xff, value as u8]),
        display_name: format!("entry-{value}"),
        kind,
        metadata: EntryMetadata::default(),
    }
}
fn directory(value: u64) -> ProviderEntry {
    entry(value, EntryKind::Directory)
}
impl Graph {
    fn set(&self, parent: u64, rows: Vec<ProviderEntry>) {
        self.0.lock().unwrap().insert(parent, Ok(rows));
    }
}
impl FilesystemProvider for Provider {
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
impl MountedFilesystem for Graph {
    fn list(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        let parent = u64::from_le_bytes(node.as_bytes().try_into().unwrap());
        let rows = self
            .0
            .lock()
            .unwrap()
            .get(&parent)
            .map(|result| {
                result
                    .as_ref()
                    .map(|rows| {
                        rows.iter()
                            .map(|row| {
                                let mut copy = row.clone();
                                copy.display_name =
                                    String::with_capacity(row.display_name.capacity());
                                copy.display_name.push_str(&row.display_name);
                                copy
                            })
                            .collect::<Vec<_>>()
                    })
                    .map_err(Clone::clone)
            })
            .unwrap_or(Ok(Vec::new()))?;
        for row in rows {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            if !emit(row) {
                break;
            }
        }
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
}
fn fixture(limits: TreeLimits) -> (ProviderRegistry, Arc<Graph>, DirectoryTree) {
    let registry = ProviderRegistry::default();
    let graph = Arc::new(Graph::default());
    graph.set(
        0,
        vec![directory(1), directory(2), entry(3, EntryKind::File)],
    );
    graph.set(
        1,
        vec![
            directory(4),
            entry(5, EntryKind::File),
            entry(6, EntryKind::DirectoryLink),
        ],
    );
    graph.set(2, vec![directory(7)]);
    let address = StoredLocation::new(
        ProviderId::new("test.tree").unwrap(),
        NonZeroU32::new(1).unwrap(),
        Arc::from([]),
    );
    registry
        .register(
            address.provider().clone(),
            Arc::new(Provider(graph.clone())),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let mut tree = DirectoryTree::new(location, limits).unwrap();
    run(&mut tree, None, TreeRequest::Expand);
    (registry, graph, tree)
}

#[test]
fn abandoned_work_leaves_no_permanent_loading_state() {
    let (_registry, _graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    let work = tree.request(first, TreeRequest::Expand).unwrap();
    drop(work);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Unknown);
    let result = tree.request(first, TreeRequest::Expand).unwrap().run();
    drop(result);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Unknown);
}

#[test]
fn an_expanded_branch_requires_refresh_instead_of_a_probe_that_can_contradict_its_rows() {
    let (_registry, _graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Expand);
    assert!(matches!(
        tree.request(first, TreeRequest::Probe),
        Err(ProviderError::Unsupported)
    ));
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Expanded);
}

#[test]
fn diagnostics_are_bounded_without_cutting_a_unicode_scalar() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    graph
        .0
        .lock()
        .unwrap()
        .insert(1, Err(ProviderError::Failed("文".repeat(20_000))));
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Expand);
    let Some(ProviderError::Failed(message)) = row(&tree, 1).error else {
        panic!("expected failure");
    };
    assert!(message.len() <= 4096);
    assert!(message.starts_with('文'));
}

#[test]
fn ignored_regular_files_cannot_hide_duplicate_or_contradictory_provider_tokens() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    for invalid in [
        vec![entry(4, EntryKind::File), directory(4)],
        vec![directory(4), entry(4, EntryKind::File)],
        vec![entry(4, EntryKind::File), entry(4, EntryKind::File)],
        vec![entry(1, EntryKind::File)],
    ] {
        graph.set(1, invalid);
        run(&mut tree, Some(first), TreeRequest::Expand);
        assert!(matches!(
            row(&tree, 1).error,
            Some(ProviderError::InvalidResponse(_))
        ));
        assert_eq!(
            tree.rows().len(),
            3,
            "failed branch cannot publish partial rows"
        );
    }
    graph.set(1, vec![directory(4)]);
    run(&mut tree, Some(first), TreeRequest::Expand);
    graph.set(1, vec![entry(4, EntryKind::File)]);
    run(&mut tree, Some(first), TreeRequest::Refresh);
    assert!(matches!(
        row(&tree, 1).error,
        Some(ProviderError::InvalidResponse(_))
    ));
    assert_eq!(row(&tree, 4).entry.kind, EntryKind::Directory);
}

#[test]
fn retained_name_capacity_counts_against_tree_payload_budget() {
    let (_registry, graph, mut tree) = fixture(TreeLimits {
        bytes: 8192,
        ..TreeLimits::default()
    });
    let mut child = directory(4);
    child.display_name = String::with_capacity(65_536);
    child.display_name.push('a');
    graph.set(1, vec![child]);
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Expand);
    assert_eq!(row(&tree, 1).error, Some(ProviderError::ResourceLimited));
    assert_eq!(tree.rows().len(), 3);
}

#[test]
fn a_container_mount_never_reinterprets_the_file_token_in_another_mount() {
    let (registry, _graph, mut tree) = fixture(TreeLimits::default());
    let file = row(&tree, 3);
    assert!(matches!(
        tree.request(file.id, TreeRequest::Expand),
        Err(ProviderError::Unsupported)
    ));
    let address = StoredLocation::new(
        ProviderId::new("test.tree").unwrap(),
        NonZeroU32::new(1).unwrap(),
        Arc::from([]),
    );
    let other_mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let target = other_mount.root().clone();
    tree.attach_container(file.id, other_mount).unwrap();
    run(&mut tree, Some(file.id), TreeRequest::Expand);
    let rows = tree.rows();
    let position = rows.iter().position(|row| row.id == file.id).unwrap();
    let child = &rows[position + 1];
    assert_ne!(child.id, rows[0].id);
    assert_ne!(child.node.mount, rows[0].node.mount);
    assert_eq!(child.node.token, rows[0].node.token);
    assert_eq!(tree.navigation_target(file.id).unwrap().1, target);
    assert_eq!(
        rows.len(),
        5,
        "container descendants omit their regular files"
    );
}

#[test]
fn an_unavailable_container_can_be_replaced_without_replacing_its_live_parent_tree() {
    let (registry, graph, mut tree) = fixture(TreeLimits::default());
    let file = row(&tree, 3).id;
    let address = StoredLocation::new(
        ProviderId::new("test.container").unwrap(),
        NonZeroU32::MIN,
        Arc::from([]),
    );
    let first = registry
        .register(
            address.provider().clone(),
            Arc::new(Provider(graph.clone())),
        )
        .unwrap();
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    tree.attach_container(file, mount).unwrap();
    let old = tree.request(file, TreeRequest::Expand).unwrap();
    assert!(registry.unregister(address.provider(), first).unwrap());
    registry
        .register(address.provider().clone(), Arc::new(Provider(graph)))
        .unwrap();
    let replacement = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    tree.attach_container(file, replacement).unwrap();
    run(&mut tree, Some(file), TreeRequest::Expand);
    assert!(!tree.apply(old.run()));
    assert_eq!(tree.state(file).unwrap().disclosure, Disclosure::Expanded);
}

#[test]
fn failed_root_loading_is_distinct_from_a_confirmed_empty_root() {
    let (_registry, graph, tree) = fixture(TreeLimits::default());
    let (mount, _) = tree.navigation_target(tree.root()).unwrap();
    let mut tree = DirectoryTree::new(mount, TreeLimits::default()).unwrap();
    graph
        .0
        .lock()
        .unwrap()
        .insert(0, Err(ProviderError::Unavailable));
    run(&mut tree, None, TreeRequest::Expand);
    assert!(tree.rows().is_empty());
    assert_eq!(
        tree.state(tree.root()).unwrap().error,
        Some(ProviderError::Unavailable)
    );
    graph.set(0, Vec::new());
    run(&mut tree, None, TreeRequest::Expand);
    assert!(tree.rows().is_empty());
    assert_eq!(
        tree.state(tree.root()).unwrap().disclosure,
        Disclosure::Empty
    );
}
fn row(tree: &DirectoryTree, value: u64) -> TreeRow {
    tree.rows()
        .into_iter()
        .find(|row| row.entry.display_name == format!("entry-{value}"))
        .unwrap()
}
fn run(tree: &mut DirectoryTree, branch: Option<BranchId>, kind: TreeRequest) {
    let result = tree
        .request(branch.unwrap_or(tree.root()), kind)
        .unwrap()
        .run();
    assert!(tree.apply(result));
}

#[test]
fn several_branches_keep_root_files_but_insert_only_folders() {
    let (_registry, _graph, mut tree) = fixture(TreeLimits::default());
    assert_eq!(tree.rows().len(), 3);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Unknown);
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Expand);
    let second = row(&tree, 2).id;
    run(&mut tree, Some(second), TreeRequest::Expand);
    let rows = tree.rows();
    assert_eq!(
        rows.iter()
            .map(|row| row.entry.display_name.as_str())
            .collect::<Vec<_>>(),
        [
            "entry-1", "entry-4", "entry-6", "entry-2", "entry-7", "entry-3"
        ]
    );
    assert_eq!(
        rows.iter().map(|row| row.depth).collect::<Vec<_>>(),
        [0, 1, 1, 0, 1, 0]
    );
    assert_eq!(rows[1].entry.raw_name.as_ref(), &[0xff, 4]);
    assert_eq!(rows[2].disclosure, Disclosure::None);
    assert!(matches!(
        tree.request(rows[2].id, TreeRequest::Expand),
        Err(ProviderError::Unsupported)
    ));
    assert!(tree.collapse(first));
    assert_eq!(row(&tree, 2).disclosure, Disclosure::Expanded);
    assert_eq!(tree.rows().len(), 4);
}

#[test]
fn probe_distinguishes_known_children_empty_and_error_without_expanding() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Probe);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Collapsed);
    assert_eq!(tree.rows().len(), 3);
    graph.set(1, vec![entry(8, EntryKind::File)]);
    run(&mut tree, Some(first), TreeRequest::Probe);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Empty);
    graph
        .0
        .lock()
        .unwrap()
        .insert(1, Err(ProviderError::Failed("Access denied".into())));
    run(&mut tree, Some(first), TreeRequest::Probe);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Failed);
    assert_eq!(
        row(&tree, 1).error,
        Some(ProviderError::Failed("Access denied".into()))
    );
}

#[test]
fn collapse_cancels_inflight_and_old_result_cannot_reopen_branch() {
    let (_registry, _graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    let work = tree.request(first, TreeRequest::Expand).unwrap();
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Loading);
    assert!(tree.collapse(first));
    run(&mut tree, Some(first), TreeRequest::Expand);
    assert!(!tree.apply(work.run()));
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Expanded);
    assert_eq!(tree.rows().len(), 5);
}

#[test]
fn cancelled_worker_retains_admission_until_it_really_returns() {
    let limits = TreeLimits {
        requests: 1,
        ..TreeLimits::default()
    };
    let (_registry, _graph, mut tree) = fixture(limits);
    let first = row(&tree, 1).id;
    let work = tree.request(first, TreeRequest::Expand).unwrap();
    tree.collapse(first);
    assert!(matches!(
        tree.request(first, TreeRequest::Expand),
        Err(ProviderError::ResourceLimited)
    ));
    assert!(!tree.apply(work.run()));
    run(&mut tree, Some(first), TreeRequest::Expand);
}

#[test]
fn successful_refresh_keeps_surviving_occurrence_ids_and_cancels_removed_descendants() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Expand);
    let old_child = row(&tree, 4).id;
    let child_work = tree.request(old_child, TreeRequest::Expand).unwrap();
    graph.set(1, vec![directory(9), entry(6, EntryKind::DirectoryLink)]);
    let kept = row(&tree, 6).id;
    run(&mut tree, Some(first), TreeRequest::Refresh);
    assert_eq!(row(&tree, 6).id, kept);
    assert_ne!(row(&tree, 9).id, old_child);
    assert!(!tree.apply(child_work.run()));
    assert!(matches!(
        tree.request(old_child, TreeRequest::Expand),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn ancestor_cycle_is_an_error_and_failed_refresh_retains_previous_rows() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Expand);
    let child = row(&tree, 4).id;
    graph.set(4, vec![directory(0)]);
    run(&mut tree, Some(child), TreeRequest::Expand);
    assert_eq!(row(&tree, 4).disclosure, Disclosure::Failed);
    assert!(matches!(
        row(&tree, 4).error,
        Some(ProviderError::InvalidResponse(_))
    ));
    graph
        .0
        .lock()
        .unwrap()
        .insert(1, Err(ProviderError::Unavailable));
    run(&mut tree, Some(first), TreeRequest::Refresh);
    assert_eq!(tree.rows().len(), 5);
    assert!(row(&tree, 1).expanded);
    assert_eq!(row(&tree, 1).error, Some(ProviderError::Unavailable));
}

#[test]
fn node_and_depth_budgets_fail_atomically_and_do_not_turn_into_empty() {
    let limits = TreeLimits {
        nodes: 4,
        depth: 1,
        ..TreeLimits::default()
    };
    let (_registry, _graph, mut tree) = fixture(limits);
    let first = row(&tree, 1).id;
    assert!(matches!(
        tree.request(first, TreeRequest::Expand),
        Err(ProviderError::ResourceLimited)
    ));
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Unknown);
    let (_registry, _graph, mut tree) = fixture(TreeLimits {
        nodes: 4,
        ..TreeLimits::default()
    });
    let first = row(&tree, 1).id;
    run(&mut tree, Some(first), TreeRequest::Expand);
    assert_eq!(tree.rows().len(), 3);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Failed);
    assert_eq!(row(&tree, 1).error, Some(ProviderError::ResourceLimited));
}

#[test]
fn revoked_mount_rejects_result_and_new_tree_cannot_accept_old_request() {
    let (registry, _graph, mut tree) = fixture(TreeLimits::default());
    let first = row(&tree, 1).id;
    let result = tree.request(first, TreeRequest::Expand).unwrap().run();
    drop(registry);
    assert!(tree.apply(result));
    assert_eq!(row(&tree, 1).error, Some(ProviderError::StaleReference));
    let (_registry, _graph, mut other) = fixture(TreeLimits::default());
    assert!(matches!(
        other.request(first, TreeRequest::Expand),
        Err(ProviderError::StaleReference)
    ));
}
