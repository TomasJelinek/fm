//! Defensive policy for future provider shapes; current projections forbid expanded links.
use super::*;

#[test]
fn tree_delete_ancestry_rejects_selected_roots_overlapping_through_links() {
    for kind in [EntryKind::DirectoryLink, EntryKind::Symlink] {
        for selected in [HashSet::from([TreeRowId(1)]), HashSet::from([TreeRowId(2)])] {
            let result = covering_chain(Some(TreeRowId(2)), &selected, |id| match id.0 {
                2 => Ok((kind, Some(TreeRowId(1)))),
                1 => Ok((EntryKind::Directory, None)),
                _ => Err(ProviderError::StaleReference),
            });
            assert!(
                matches!(result, Err(ProviderError::Failed(message)) if message.contains("link"))
            );
        }
    }
}

#[test]
fn tree_delete_ancestry_keeps_ordinary_parent_coverage_and_independent_link_descendants() {
    let selected = HashSet::from([TreeRowId(1)]);
    assert_eq!(
        covering_chain(Some(TreeRowId(2)), &selected, |id| {
            Ok((EntryKind::Directory, (id.0 == 2).then_some(TreeRowId(1))))
        })
        .unwrap(),
        Some(TreeRowId(1))
    );
    assert_eq!(
        covering_chain(Some(TreeRowId(2)), &HashSet::new(), |id| {
            Ok((
                EntryKind::DirectoryLink,
                (id.0 == 2).then_some(TreeRowId(1)),
            ))
        })
        .unwrap(),
        None
    );
}
