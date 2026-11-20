//! Select and deselect by mask in an inline tree: expanded descendants take part,
//! folders only when chosen, and the published selection is atomic.
use super::{tests::fixture, *};
use midnight_core::{MaskSelection, SelectionRevision, TreeSelectionAction, TreeSelectionRequest};

fn mask(pattern: &str, select: bool, folders: bool) -> TreeSelectionAction {
    TreeSelectionAction::Mask(Arc::new(MaskSelection {
        pattern: pattern.into(),
        select,
        folders,
    }))
}

fn request(
    view: &Arc<TreeProjection>,
    revision: u64,
    action: TreeSelectionAction,
) -> TreeSelectionRequest {
    TreeSelectionRequest {
        view: view.clone(),
        revision: SelectionRevision(revision),
        seed: None,
        seed_revision: SelectionRevision(0),
        action,
    }
}

fn expanded() -> super::tests::Fixture {
    let mut f = fixture();
    let first = f.session.projection().clone();
    f.session.commit(&first);
    f.session
        .branch(
            &first,
            TreeRequestId(1),
            first.rows[1].id,
            BranchAction::Expand,
        )
        .unwrap();
    let work = f.session.take_work().unwrap().unwrap();
    f.session
        .apply(work.run(), &Cancellation::default())
        .unwrap();
    let view = f.session.projection().clone();
    f.session.commit(&view);
    f
}

fn selected(f: &mut super::tests::Fixture) -> Vec<String> {
    let publication = f.session.publication().unwrap();
    let mut names: Vec<_> = publication
        .view
        .rows
        .iter()
        .filter(|row| publication.selection.contains_row(row.id))
        .map(|row| row.entry.display_name.clone())
        .collect();
    names.sort();
    names
}

#[test]
fn tree_masks_reach_expanded_descendants_and_folders_only_when_chosen() {
    let mut f = expanded();
    let view = f.session.projection().clone();
    assert!(view.rows.iter().any(|row| row.depth == 1));
    f.session
        .select(&request(&view, 1, mask("*.TXT", true, true)))
        .unwrap();
    assert_eq!(selected(&mut f), ["file.txt"]);
    f.session
        .select(&request(&view, 2, mask("*parent;ch*", true, false)))
        .unwrap();
    assert_eq!(
        selected(&mut f),
        ["file.txt"],
        "folders need the folder choice"
    );
    f.session
        .select(&request(&view, 3, mask("*parent;ch*", true, true)))
        .unwrap();
    assert_eq!(
        selected(&mut f),
        ["a-parent", "child", "file.txt", "z-parent"],
        "a nested folder of an expanded branch is selected too"
    );
    f.session
        .select(&request(&view, 4, mask("*|z*", false, true)))
        .unwrap();
    assert_eq!(selected(&mut f), ["z-parent"]);
}

#[test]
fn an_invalid_tree_mask_changes_nothing() {
    let mut f = expanded();
    let view = f.session.projection().clone();
    f.session
        .select(&request(&view, 1, mask("*", true, true)))
        .unwrap();
    let before = selected(&mut f);
    assert!(
        f.session
            .select(&request(&view, 2, mask("a|b|c", false, true)))
            .is_err()
    );
    assert_eq!(selected(&mut f), before);
}
