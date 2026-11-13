//! Exercise retained branch failures through actual provider work and retry.
use super::super::tree::tests::assert_payload_accounting;
use super::*;
use midnight_core::GeneratedTextSource;

fn text(source: &dyn GeneratedTextSource) -> String {
    let mut bytes = vec![0; source.length().try_into().unwrap()];
    let mut offset = 0;
    while offset < bytes.len() {
        let end = (offset + 16 * 1024).min(bytes.len());
        let read = source
            .read_at(offset as u64, &mut bytes[offset..end])
            .unwrap();
        assert!(read > 0);
        offset += read;
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn full_failure_details_survive_bounded_summary_retry_and_tree_destruction() {
    let (_registry, graph, mut tree) = fixture(TreeLimits::default());
    let branch = row(&tree, 1).id;
    let reason = format!(
        "Denied {} unique-final-reason",
        "東京 long failure ".repeat(8_000)
    );
    graph
        .0
        .lock()
        .unwrap()
        .insert(1, Err(ProviderError::Failed(reason.clone())));
    run(&mut tree, Some(branch), TreeRequest::Expand);
    let failed = row(&tree, 1);
    assert_eq!(failed.disclosure, Disclosure::Failed);
    assert!(failed.error.as_ref().unwrap().to_string().len() <= 4096);
    let details = failed
        .error_details
        .expect("failed listing retains full details");
    let original = text(details.as_ref());
    assert!(original.contains(&reason));
    assert!(!original.contains("Directory size warnings"));
    assert!(original.contains("Observed issues: 1"));
    graph.0.lock().unwrap().insert(
        2,
        Err(ProviderError::Failed("later sibling failure".into())),
    );
    let second = row(&tree, 2).id;
    run(&mut tree, Some(second), TreeRequest::Expand);
    let sibling = row(&tree, 2).error_details.unwrap();
    let sibling_text = text(sibling.as_ref());
    assert!(sibling_text.contains("later sibling failure"));
    assert!(!sibling_text.contains("unique-final-reason"));
    assert!(!text(details.as_ref()).contains("later sibling failure"));
    graph.set(1, vec![directory(4)]);
    let work = tree.request(branch, TreeRequest::Expand).unwrap();
    assert!(row(&tree, 1).error_details.is_none());
    assert!(tree.apply(work.run()));
    assert!(row(&tree, 1).error.is_none());
    assert!(row(&tree, 1).error_details.is_none());
    drop(tree);
    assert_eq!(text(details.as_ref()), original);
}

fn budget_entry(value: u64) -> ProviderEntry {
    let mut child = directory(value);
    child.display_name = String::with_capacity(5120);
    child.display_name.push_str(&format!("entry-{value}"));
    child
}

fn run_checked(tree: &mut DirectoryTree, branch: Option<BranchId>, kind: TreeRequest) {
    assert_payload_accounting(tree);
    let result = tree.request(branch.unwrap_or(tree.root()), kind).unwrap();
    assert_payload_accounting(tree);
    assert!(tree.apply(result.run()));
    assert_payload_accounting(tree);
}

#[test]
fn payload_budget_reclaims_replaced_entries_and_removed_descendants() {
    let (_registry, graph, mut tree) = fixture(TreeLimits {
        bytes: 8192,
        ..TreeLimits::default()
    });
    let first = row(&tree, 1).id;
    let second = row(&tree, 2).id;
    graph.set(1, vec![budget_entry(4)]);
    graph.set(2, vec![budget_entry(7)]);
    run_checked(&mut tree, Some(first), TreeRequest::Expand);
    let retained = row(&tree, 4).id;
    assert!(row(&tree, 1).error.is_none());
    assert!(tree.collapse(first));
    assert_payload_accounting(&tree);
    run_checked(&mut tree, Some(first), TreeRequest::Expand);
    run_checked(&mut tree, Some(second), TreeRequest::Expand);
    assert_eq!(row(&tree, 2).error, Some(ProviderError::ResourceLimited));

    graph.set(1, vec![directory(4)]);
    run_checked(&mut tree, Some(first), TreeRequest::Refresh);
    assert_eq!(row(&tree, 4).id, retained);
    run_checked(&mut tree, Some(second), TreeRequest::Expand);
    assert!(row(&tree, 2).error.is_none());
    graph.set(1, vec![budget_entry(4)]);
    run_checked(&mut tree, Some(first), TreeRequest::Refresh);
    assert_eq!(row(&tree, 1).error, Some(ProviderError::ResourceLimited));
    assert_eq!(row(&tree, 4).id, retained);

    graph.set(2, Vec::new());
    run_checked(&mut tree, Some(second), TreeRequest::Refresh);
    run_checked(&mut tree, Some(first), TreeRequest::Refresh);
    assert!(row(&tree, 1).error.is_none());
    graph.set(1, Vec::new());
    run_checked(&mut tree, Some(first), TreeRequest::Refresh);
    graph.set(2, vec![budget_entry(7)]);
    run_checked(&mut tree, Some(second), TreeRequest::Expand);
    assert!(row(&tree, 2).error.is_none());
    assert_eq!(tree.rows().len(), 4);
}

#[test]
fn payload_budget_releases_error_summaries_on_retry_and_bounds_new_failures() {
    let (_registry, graph, mut tree) = fixture(TreeLimits {
        bytes: 8192,
        ..TreeLimits::default()
    });
    let first = row(&tree, 1).id;
    let second = row(&tree, 2).id;
    graph
        .0
        .lock()
        .unwrap()
        .insert(1, Err(ProviderError::Failed("x".repeat(4096))));
    graph.set(2, vec![budget_entry(7)]);
    run_checked(&mut tree, Some(first), TreeRequest::Expand);
    run_checked(&mut tree, Some(second), TreeRequest::Expand);
    assert_eq!(row(&tree, 2).error, Some(ProviderError::ResourceLimited));
    drop(tree.request(first, TreeRequest::Expand).unwrap());
    assert_payload_accounting(&tree);
    run_checked(&mut tree, Some(second), TreeRequest::Expand);
    assert!(row(&tree, 2).error.is_none());
    let mut summaries = Vec::new();
    for _ in 0..3 {
        run_checked(&mut tree, Some(first), TreeRequest::Expand);
        let Some(ProviderError::Failed(message)) = row(&tree, 1).error else {
            panic!("expected a bounded failure summary");
        };
        assert!(!message.is_empty());
        assert!(message.len() < 4096);
        summaries.push(message.len());
    }
    assert_eq!(summaries, vec![summaries[0]; 3]);
    graph.set(1, Vec::new());
    run_checked(&mut tree, Some(first), TreeRequest::Probe);
    assert_eq!(row(&tree, 1).disclosure, Disclosure::Empty);
}

#[test]
fn payload_budget_rollback_restores_capacity_without_reusing_occurrence_ids() {
    let (_registry, graph, mut tree) = fixture(TreeLimits {
        bytes: 8192,
        ..TreeLimits::default()
    });
    let first = row(&tree, 1).id;
    let second = row(&tree, 2).id;
    let checkpoint = tree.checkpoint();
    graph.set(1, vec![budget_entry(4)]);
    run_checked(&mut tree, Some(first), TreeRequest::Expand);
    let unpublished = row(&tree, 4).id;
    let pending = tree.request(unpublished, TreeRequest::Expand).unwrap();
    tree.rollback_unpublished(checkpoint);
    assert_payload_accounting(&tree);
    assert!(!tree.apply(pending.run()));
    assert_eq!(tree.rows().len(), 3);
    graph.set(2, vec![budget_entry(7)]);
    run_checked(&mut tree, Some(second), TreeRequest::Expand);
    assert!(row(&tree, 2).error.is_none());
    let replacement = row(&tree, 7).id;
    assert_ne!(replacement, unpublished);
    assert!(tree.state(unpublished).is_none());
}

#[test]
fn payload_budget_restore_replaces_the_saved_error_charge() {
    let (_registry, graph, mut tree) = fixture(TreeLimits {
        bytes: 8192,
        ..TreeLimits::default()
    });
    let first = row(&tree, 1).id;
    let second = row(&tree, 2).id;
    graph
        .0
        .lock()
        .unwrap()
        .insert(1, Err(ProviderError::Failed("x".repeat(4096))));
    run_checked(&mut tree, Some(first), TreeRequest::Expand);
    let checkpoint = tree.checkpoint();
    graph.set(1, vec![budget_entry(4)]);
    run_checked(&mut tree, Some(first), TreeRequest::Expand);
    assert!(row(&tree, 1).error.is_none());
    tree.restore(
        checkpoint,
        first,
        ProviderError::Failed("restored failure".into()),
    );
    assert_payload_accounting(&tree);
    assert_eq!(tree.rows().len(), 3);
    assert_eq!(
        row(&tree, 1).error,
        Some(ProviderError::Failed("restored failure".into()))
    );
    graph.set(2, vec![budget_entry(7)]);
    run_checked(&mut tree, Some(second), TreeRequest::Expand);
    assert!(row(&tree, 2).error.is_none());
}

#[test]
fn payload_budget_container_replacement_releases_target_and_descendant_charges() {
    let (registry, graph, mut tree) = fixture(TreeLimits {
        bytes: 8192,
        ..TreeLimits::default()
    });
    let file = row(&tree, 3).id;
    let first = row(&tree, 1).id;
    let mount = |length| {
        let address = StoredLocation::new(
            ProviderId::new("test.tree").unwrap(),
            NonZeroU32::MIN,
            Arc::from(vec![0; length]),
        );
        registry
            .resolve(&address, &Cancellation::default())
            .unwrap()
    };
    tree.attach_container(file, mount(4096)).unwrap();
    assert_payload_accounting(&tree);
    run_checked(&mut tree, Some(file), TreeRequest::Expand);
    let child = tree
        .rows()
        .iter()
        .find(|row| row.parent == Some(file))
        .unwrap()
        .id;
    graph
        .0
        .lock()
        .unwrap()
        .insert(1, Err(ProviderError::Failed("x".repeat(512))));
    run_checked(&mut tree, Some(child), TreeRequest::Expand);
    assert!(tree.state(child).unwrap().error.is_some());
    tree.attach_container(file, mount(0)).unwrap();
    assert_payload_accounting(&tree);
    assert!(tree.state(child).is_none());
    graph.set(1, vec![budget_entry(4)]);
    run_checked(&mut tree, Some(first), TreeRequest::Expand);
    assert!(row(&tree, 1).error.is_none());
    assert_eq!(tree.rows().len(), 4);
    assert_eq!(
        tree.attach_container(file, mount(4096)),
        Err(ProviderError::ResourceLimited)
    );
    assert_payload_accounting(&tree);
}

#[test]
fn payload_budget_includes_the_root_mount_address() {
    let (registry, graph, _tree) = fixture(TreeLimits::default());
    graph.set(1, vec![budget_entry(4)]);
    for address_bytes in [0, 4096] {
        let address = StoredLocation::new(
            ProviderId::new("test.tree").unwrap(),
            NonZeroU32::MIN,
            Arc::from(vec![0; address_bytes]),
        );
        let mount = registry
            .resolve(&address, &Cancellation::default())
            .unwrap();
        let mut tree = DirectoryTree::new(
            mount,
            TreeLimits {
                bytes: 8192,
                ..TreeLimits::default()
            },
            crate::ReportArena::testing(),
        )
        .unwrap();
        run_checked(&mut tree, None, TreeRequest::Expand);
        let first = row(&tree, 1).id;
        run_checked(&mut tree, Some(first), TreeRequest::Expand);
        if address_bytes == 0 {
            assert!(row(&tree, 1).error.is_none());
        } else {
            assert_eq!(row(&tree, 1).error, Some(ProviderError::ResourceLimited));
        }
    }
}
