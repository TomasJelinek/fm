//! Exercise retained branch failures through actual provider work and retry.
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
