//! Browse identities stop only the cyclic occurrence, never sibling aliases.
use super::super::registry::check_cancel;
use super::*;
use midnight_core::SizeDirectoryIdentity;

#[test]
fn loaded_maintenance_scope_does_not_cross_an_attached_provider_mount() {
    let (registry, _graph, mut tree) = fixture(TreeLimits::default());
    let native = row(&tree, 1).id;
    run(&mut tree, Some(native), TreeRequest::Expand);
    let file = row(&tree, 3).id;
    let address = StoredLocation::new(
        ProviderId::new("test.tree").unwrap(),
        NonZeroU32::MIN,
        Arc::from([]),
    );
    let container = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    tree.attach_container(file, container).unwrap();
    run(&mut tree, Some(file), TreeRequest::Expand);
    let mounted_child = tree
        .rows()
        .into_iter()
        .find(|row| row.parent == Some(file))
        .unwrap()
        .id;
    run(&mut tree, Some(mounted_child), TreeRequest::Expand);
    assert_eq!(tree.loaded_native_branches(), [tree.root(), native]);
    assert!(!tree.loaded_native_branch(file));
    assert!(!tree.loaded_native_branch(mounted_child));
}

struct IdentityGraph {
    rows: HashMap<u64, Vec<ProviderEntry>>,
    identities: Mutex<HashMap<u64, u64>>,
    identity_errors: Mutex<HashMap<u64, ProviderError>>,
    identity_sequences: Mutex<HashMap<u64, std::collections::VecDeque<u64>>>,
    listed: Mutex<Vec<u64>>,
}
struct IdentityProvider(Arc<IdentityGraph>);

fn value(node: &NodeToken) -> u64 {
    u64::from_le_bytes(node.as_bytes().try_into().unwrap())
}

impl MountedFilesystem for IdentityGraph {
    fn browse_directory_identity(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryIdentity>, ProviderError> {
        check_cancel(cancel)?;
        if let Some(error) = self.identity_errors.lock().unwrap().get(&value(node)) {
            return Err(error.clone());
        }
        if let Some(value) = self
            .identity_sequences
            .lock()
            .unwrap()
            .get_mut(&value(node))
            .and_then(|sequence| sequence.pop_front())
        {
            return Ok(Some(SizeDirectoryIdentity {
                device: 7,
                object: u128::from(value).to_le_bytes(),
            }));
        }
        Ok(self
            .identities
            .lock()
            .unwrap()
            .get(&value(node))
            .map(|value| SizeDirectoryIdentity {
                device: 7,
                object: u128::from(*value).to_le_bytes(),
            }))
    }

    fn list(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        check_cancel(cancel)?;
        let value = value(node);
        self.listed.lock().unwrap().push(value);
        for row in self.rows.get(&value).into_iter().flatten() {
            if !emit(row.clone()) {
                break;
            }
        }
        check_cancel(cancel)
    }

    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
}

impl FilesystemProvider for IdentityProvider {
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

fn identity_fixture() -> (ProviderRegistry, Arc<IdentityGraph>, DirectoryTree) {
    let graph = Arc::new(IdentityGraph {
        rows: HashMap::from([
            (0, vec![directory(1), directory(2)]),
            (1, vec![entry(3, EntryKind::DirectoryLink), directory(4)]),
            (
                2,
                vec![
                    entry(5, EntryKind::DirectoryLink),
                    entry(6, EntryKind::DirectoryLink),
                    entry(10, EntryKind::DirectoryLink),
                ],
            ),
            (3, vec![directory(90)]),
            (4, vec![directory(7)]),
            (5, vec![directory(8)]),
            (6, vec![directory(9)]),
        ]),
        identities: Mutex::new(HashMap::from([
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 0),
            (4, 4),
            (5, 55),
            (6, 55),
            (10, 10),
        ])),
        identity_errors: Mutex::default(),
        identity_sequences: Mutex::default(),
        listed: Mutex::default(),
    });
    let address = StoredLocation::new(
        ProviderId::new("test.tree.identity").unwrap(),
        NonZeroU32::MIN,
        Arc::from([]),
    );
    let registry = ProviderRegistry::default();
    registry
        .register(
            address.provider().clone(),
            Arc::new(IdentityProvider(graph.clone())),
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
    (registry, graph, tree)
}

#[test]
fn an_identity_cycle_stops_before_listing_and_readable_sibling_stays_expandable() {
    let (_registry, graph, mut tree) = identity_fixture();
    let parent = row(&tree, 1).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let cycle = row(&tree, 3).id;
    run(&mut tree, Some(cycle), TreeRequest::Expand);
    assert_eq!(row(&tree, 3).disclosure, Disclosure::Failed);
    assert!(!graph.listed.lock().unwrap().contains(&3));
    let sibling = row(&tree, 4).id;
    run(&mut tree, Some(sibling), TreeRequest::Expand);
    assert!(
        tree.rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-7")
    );
    assert!(
        !tree
            .rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-90")
    );
}

#[test]
fn sibling_directory_aliases_both_expand_their_logical_occurrences() {
    let (_registry, graph, mut tree) = identity_fixture();
    let parent = row(&tree, 2).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    for value in [5, 6] {
        let branch = row(&tree, value).id;
        run(&mut tree, Some(branch), TreeRequest::Expand);
        assert_eq!(row(&tree, value).disclosure, Disclosure::Expanded);
    }
    assert!(
        tree.rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-8")
    );
    assert!(
        tree.rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-9")
    );
    assert!(graph.listed.lock().unwrap().contains(&5));
    assert!(graph.listed.lock().unwrap().contains(&6));
}

#[test]
fn a_known_empty_link_has_the_same_empty_disclosure_as_an_ordinary_folder() {
    let (_registry, _graph, mut tree) = identity_fixture();
    let parent = row(&tree, 2).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let link = row(&tree, 10).id;
    run(&mut tree, Some(link), TreeRequest::Probe);
    assert_eq!(row(&tree, 10).disclosure, Disclosure::Empty);
    assert!(!row(&tree, 10).expanded);
}

#[test]
fn ordinary_folders_still_browse_when_a_provider_has_no_identity_hook() {
    let (_registry, _graph, mut tree) = fixture(TreeLimits::default());
    let parent = row(&tree, 1).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    assert!(
        tree.rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-4")
    );
}

#[test]
fn inspected_non_directory_unsupported_and_denied_links_remain_distinct() {
    let (_registry, graph, mut tree) = identity_fixture();
    let parent = row(&tree, 2).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let link = row(&tree, 10).id;
    graph.identities.lock().unwrap().remove(&10);
    run(&mut tree, Some(link), TreeRequest::Probe);
    assert!(
        matches!(row(&tree, 10).error, Some(ProviderError::Failed(ref message)) if message.contains("no longer a directory"))
    );
    graph
        .identity_errors
        .lock()
        .unwrap()
        .insert(10, ProviderError::Unsupported);
    run(&mut tree, Some(link), TreeRequest::Probe);
    assert_eq!(row(&tree, 10).error, Some(ProviderError::Unsupported));
    graph
        .identity_errors
        .lock()
        .unwrap()
        .insert(10, ProviderError::Failed("Permission denied".into()));
    run(&mut tree, Some(link), TreeRequest::Probe);
    assert_eq!(
        row(&tree, 10).error,
        Some(ProviderError::Failed("Permission denied".into()))
    );
    assert!(!graph.listed.lock().unwrap().contains(&10));
}

#[test]
fn a_directory_target_retargeted_during_listing_never_publishes_children_under_the_old_identity() {
    let (_registry, graph, mut tree) = identity_fixture();
    let parent = row(&tree, 2).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    graph
        .identity_sequences
        .lock()
        .unwrap()
        .insert(5, [55, 77].into());
    let link = row(&tree, 5).id;
    run(&mut tree, Some(link), TreeRequest::Expand);
    assert!(
        matches!(row(&tree, 5).error, Some(ProviderError::Failed(ref message)) if message.contains("changed while listing"))
    );
    assert!(
        !tree
            .rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-8")
    );
    let sibling = row(&tree, 6).id;
    run(&mut tree, Some(sibling), TreeRequest::Expand);
    assert!(
        tree.rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-9")
    );
    graph
        .identity_sequences
        .lock()
        .unwrap()
        .insert(5, [77, 77].into());
    run(&mut tree, Some(link), TreeRequest::Expand);
    assert!(
        tree.rows()
            .iter()
            .any(|row| row.entry.display_name == "entry-8")
    );
}

#[test]
fn refreshing_a_loaded_link_rechecks_identity_and_retains_its_logical_occurrence() {
    let (_registry, graph, mut tree) = identity_fixture();
    let parent = row(&tree, 2).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let link = row(&tree, 5).id;
    run(&mut tree, Some(link), TreeRequest::Expand);
    let original_node = row(&tree, 5).node;
    let original_child = row(&tree, 8).id;
    graph
        .identity_sequences
        .lock()
        .unwrap()
        .insert(5, [55, 77].into());
    run(&mut tree, Some(link), TreeRequest::Refresh);
    assert!(matches!(
        row(&tree, 5).error,
        Some(ProviderError::Failed(ref message)) if message.contains("changed while listing")
    ));
    assert_eq!(row(&tree, 8).id, original_child);
    graph.identities.lock().unwrap().insert(5, 77);
    run(&mut tree, Some(link), TreeRequest::Refresh);
    assert!(row(&tree, 5).error.is_none());
    assert_eq!(row(&tree, 5).node, original_node);
    assert_eq!(row(&tree, 8).id, original_child);
}

#[test]
fn a_probe_does_not_replace_the_identity_basis_of_a_cached_listing() {
    let (_registry, graph, mut tree) = identity_fixture();
    let parent = row(&tree, 2).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let link = row(&tree, 5).id;
    run(&mut tree, Some(link), TreeRequest::Expand);
    let identity = tree.loaded_native_identity(link).unwrap();
    let version = tree.watch_listing(link).unwrap().0;
    assert!(tree.collapse(link));
    graph.identities.lock().unwrap().insert(5, 77);
    run(&mut tree, Some(link), TreeRequest::Probe);
    assert_eq!(tree.loaded_native_identity(link).unwrap(), identity);
    assert_eq!(tree.watch_listing(link).unwrap().0, version);
}

#[test]
fn rejected_refresh_installation_does_not_replace_the_listing_identity_basis() {
    let (registry, graph, mut tree) = identity_fixture();
    let parent = row(&tree, 2).id;
    run(&mut tree, Some(parent), TreeRequest::Expand);
    let link = row(&tree, 5).id;
    run(&mut tree, Some(link), TreeRequest::Expand);
    let identity = tree.loaded_native_identity(link).unwrap();
    let version = tree.watch_listing(link).unwrap().0;
    graph.identities.lock().unwrap().insert(5, 77);
    let completed = tree.request(link, TreeRequest::Refresh).unwrap().run();
    drop(registry);
    assert!(tree.apply(completed));
    assert_eq!(row(&tree, 5).error, Some(ProviderError::StaleReference));
    assert_eq!(tree.loaded_native_identity(link).unwrap(), identity);
    assert_eq!(tree.watch_listing(link).unwrap().0, version);
}
