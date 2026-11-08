use super::*;
use crate::providers::{
    MountedLocation, MountedTarget, ProviderTarget,
    tree::{DirectoryTree, TreeLimits, TreeRequest},
};

fn mount(root: &Path) -> (ProviderRegistry, MountedLocation) {
    let registry = ProviderRegistry::default();
    let address = native_location(root).unwrap();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(Files))),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    (registry, location)
}

fn target_path(target: &MountedTarget) -> &Path {
    let ProviderTarget::Native(path) = target.value().unwrap() else {
        panic!("expected native fixture target");
    };
    path
}

fn expand(tree: &mut DirectoryTree, branch: crate::providers::tree::BranchId) {
    let result = tree.request(branch, TreeRequest::Expand).unwrap().run();
    assert!(tree.apply(result));
}

#[test]
fn tree_entry_targets_keep_nested_paths_with_repeated_listing_ids_and_names() {
    let root = path().join("東京 Žluťoučký");
    let (_registry, location) = mount(&root);
    let mut tree = DirectoryTree::new(location, TreeLimits::default()).unwrap();
    let root_id = tree.root();
    expand(&mut tree, root_id);
    let folder = tree
        .rows()
        .into_iter()
        .find(|row| row.entry.kind == EntryKind::Directory)
        .unwrap();
    expand(&mut tree, folder.id);
    let child = tree.rows().into_iter().find(|row| row.depth == 1).unwrap();
    let target = tree
        .resolve_entry_target(child.id, &Cancellation::default())
        .unwrap();
    assert_eq!(target_path(&target), root.join("folder").join("folder"));
    assert_ne!(child.node, folder.node);
    assert_ne!(child.id, folder.id);
}

#[test]
fn container_source_and_entered_root_are_distinct_even_after_container_revocation() {
    let root = path().join("source");
    let (_registry, location) = mount(&root);
    let mut tree = DirectoryTree::new(location, TreeLimits::default()).unwrap();
    let root_id = tree.root();
    expand(&mut tree, root_id);
    let file = tree
        .rows()
        .into_iter()
        .find(|row| row.entry.kind == EntryKind::File)
        .unwrap();
    let container_root = path().join("container");
    let (container_registry, container) = mount(&container_root);
    tree.attach_container(file.id, container).unwrap();
    expand(&mut tree, file.id);
    let child = tree
        .rows()
        .into_iter()
        .find(|row| row.parent == Some(file.id))
        .unwrap();
    let source = tree
        .resolve_entry_target(file.id, &Cancellation::default())
        .unwrap();
    assert_eq!(target_path(&source), root.join("same name"));
    let (entered, node) = tree.navigation_target(file.id).unwrap();
    assert_eq!(
        target_path(
            &entered
                .resolve_target(&node, &Cancellation::default())
                .unwrap()
        ),
        container_root
    );
    assert_eq!(
        target_path(
            &tree
                .resolve_entry_target(child.id, &Cancellation::default())
                .unwrap()
        ),
        container_root.join("folder")
    );
    drop(container_registry);
    assert_eq!(
        target_path(
            &tree
                .resolve_entry_target(file.id, &Cancellation::default())
                .unwrap()
        ),
        root.join("same name")
    );
    assert!(matches!(
        tree.navigation_target(file.id),
        Err(ProviderError::StaleReference)
    ));
    assert!(matches!(
        tree.resolve_entry_target(child.id, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn hidden_root_and_collapsed_descendants_never_resolve_as_action_entries() {
    let (_registry, location) = mount(&path());
    let mut tree = DirectoryTree::new(location, TreeLimits::default()).unwrap();
    let root_id = tree.root();
    expand(&mut tree, root_id);
    assert!(matches!(
        tree.resolve_entry_target(root_id, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
    let folder = tree
        .rows()
        .into_iter()
        .find(|row| row.entry.kind == EntryKind::Directory)
        .unwrap();
    expand(&mut tree, folder.id);
    let child = tree.rows().into_iter().find(|row| row.depth == 1).unwrap();
    assert!(tree.collapse(folder.id));
    assert!(matches!(
        tree.resolve_entry_target(child.id, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    assert!(
        tree.resolve_entry_target(folder.id, &Cancellation::default())
            .is_ok()
    );
}

#[test]
fn foreign_and_cancelled_tree_entry_targets_are_rejected() {
    let (_registry, location) = mount(&path());
    let mut tree = DirectoryTree::new(location.clone(), TreeLimits::default()).unwrap();
    let root_id = tree.root();
    expand(&mut tree, root_id);
    let entry = tree.rows().remove(0);
    let other_tree = DirectoryTree::new(location, TreeLimits::default()).unwrap();
    assert!(matches!(
        other_tree.resolve_entry_target(entry.id, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(matches!(
        tree.resolve_entry_target(entry.id, &cancelled),
        Err(ProviderError::Cancelled)
    ));
}

#[test]
fn retained_descendant_target_rejects_loss_of_its_enclosing_provider() {
    let (registry, location) = mount(&path().join("source"));
    let mut tree = DirectoryTree::new(location, TreeLimits::default()).unwrap();
    let root_id = tree.root();
    expand(&mut tree, root_id);
    let file = tree
        .rows()
        .into_iter()
        .find(|row| row.entry.kind == EntryKind::File)
        .unwrap();
    let (_container_registry, container) = mount(&path().join("container"));
    tree.attach_container(file.id, container).unwrap();
    expand(&mut tree, file.id);
    let child = tree
        .rows()
        .into_iter()
        .find(|row| row.parent == Some(file.id))
        .unwrap();
    let target = tree
        .resolve_entry_target(child.id, &Cancellation::default())
        .unwrap();
    assert!(target.value().is_ok());
    drop(registry);
    assert!(matches!(target.value(), Err(ProviderError::StaleReference)));
}
