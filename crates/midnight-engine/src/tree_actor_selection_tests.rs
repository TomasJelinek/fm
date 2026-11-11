//! Selection and projection are a single publication at the actor boundary.
use super::{tests::fixture, *};
use midnight_core::{
    SelectionOperation, SelectionRanges, SelectionRevision, SelectionSnapshot, SelectionSpan,
    TreeRootSelection, TreeSelectionAction, TreeSelectionRequest,
};

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

fn expanded_fixture() -> super::tests::Fixture {
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

#[test]
fn select_all_only_marks_top_level_entries_and_clears_nested_marks() {
    let mut f = expanded_fixture();
    let view = f.session.projection().clone();
    let child = view.rows.iter().find(|row| row.depth > 0).unwrap().id;
    f.session
        .select(&request(&view, 1, TreeSelectionAction::Toggle(child)))
        .unwrap();
    assert!(
        f.session
            .publication()
            .unwrap()
            .selection
            .contains_row(child)
    );
    f.session
        .select(&request(
            &view,
            2,
            TreeSelectionAction::Bulk(SelectionOperation::All),
        ))
        .unwrap();
    let selected = f.session.publication().unwrap().selection;
    assert_eq!(selected.file_count(), 1);
    assert_eq!(selected.folder_count(), 2);
    assert!(!selected.contains_row(child));
}

#[test]
fn invert_all_changes_only_top_level_rows_and_preserves_nested_marks() {
    for nested_marked in [false, true] {
        let mut f = expanded_fixture();
        let view = f.session.projection().clone();
        let top = view.rows[0].id;
        let child = view.rows.iter().find(|row| row.depth > 0).unwrap().id;
        f.session
            .select(&request(&view, 1, TreeSelectionAction::Toggle(top)))
            .unwrap();
        if nested_marked {
            f.session
                .select(&request(&view, 2, TreeSelectionAction::Toggle(child)))
                .unwrap();
        }
        f.session
            .select(&request(
                &view,
                3,
                TreeSelectionAction::Bulk(SelectionOperation::InvertAll),
            ))
            .unwrap();
        let selected = f.session.publication().unwrap().selection;
        for row in view.rows.iter() {
            let expected = if row.depth == 0 {
                row.id != top
            } else {
                nested_marked && row.id == child
            };
            assert_eq!(
                selected.contains_row(row.id),
                expected,
                "{}",
                row.entry.display_name
            );
        }
        f.session
            .select(&request(
                &view,
                4,
                TreeSelectionAction::Bulk(SelectionOperation::InvertAll),
            ))
            .unwrap();
        let restored = f.session.publication().unwrap().selection;
        assert!(restored.contains_row(top));
        assert_eq!(restored.contains_row(child), nested_marked);
        assert_eq!(restored.len(), if nested_marked { 2 } else { 1 });
    }
}

#[test]
fn select_files_only_clears_selected_folders_in_expanded_tree() {
    let mut f = expanded_fixture();
    let view = f.session.projection().clone();
    f.session
        .select(&request(
            &view,
            1,
            TreeSelectionAction::Bulk(SelectionOperation::All),
        ))
        .unwrap();
    let all = f.session.publication().unwrap();
    assert!(all.selection.folder_count() > 0);
    assert_eq!(all.selection.file_count(), 1);
    f.session
        .select(&request(
            &view,
            2,
            TreeSelectionAction::Bulk(SelectionOperation::FilesOnly),
        ))
        .unwrap();
    let files = f.session.publication().unwrap();
    assert_eq!(files.selection.file_count(), 1);
    assert_eq!(files.selection.folder_count(), 0);
}

#[test]
fn tree_selection_publishes_root_migration_expansion_inversion_and_collapse_atomically() {
    let mut f = fixture();
    let first = f.session.projection().clone();
    // Root order is z-parent/a-parent/file, while the displayed tree is sorted.
    let selected =
        Arc::new(SelectionSnapshot::from_bits(first.root.index.clone(), vec![0b101]).unwrap());
    f.session
        .seed_selection(SelectionRevision(9), &TreeRootSelection::Native(selected))
        .unwrap();
    let publication = f.session.publication().unwrap();
    assert!(Arc::ptr_eq(
        &publication.view,
        publication.selection.basis()
    ));
    assert_eq!(publication.selection.bits(), &[0b110]);
    assert_eq!(publication.selection_revision, SelectionRevision(9));
    assert!(publication.can_adopt(&first.root, SelectionRevision(9)));
    assert!(!publication.can_adopt(&first.root, SelectionRevision(10)));
    assert!(f.session.commit(&first));
    assert!(
        f.session
            .seed_selection(SelectionRevision(10), &TreeRootSelection::Empty)
            .is_err()
    );
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
    let expanded = f.session.publication().unwrap();
    assert!(Arc::ptr_eq(&expanded.view, expanded.selection.basis()));
    assert_eq!(expanded.selection.len(), 2);
    assert!(!expanded.selection.contains_row(expanded.view.rows[2].id));
    f.session.commit(&expanded.view);
    f.session
        .select(&request(
            &expanded.view,
            10,
            TreeSelectionAction::Bulk(SelectionOperation::Invert),
        ))
        .unwrap();
    let inverted = f.session.publication().unwrap();
    assert_eq!(
        (
            inverted.selection.file_count(),
            inverted.selection.folder_count()
        ),
        (0, 1)
    );
    f.session
        .select(&request(
            &expanded.view,
            11,
            TreeSelectionAction::Toggle(expanded.view.rows[2].id),
        ))
        .unwrap();
    assert_eq!(f.session.publication().unwrap().selection.len(), 2);
    f.session
        .branch(
            &expanded.view,
            TreeRequestId(2),
            first.rows[1].id,
            BranchAction::Collapse,
        )
        .unwrap();
    f.session
        .reproject(ProjectionOptions::default(), &Cancellation::default())
        .unwrap();
    let collapsed = f.session.publication().unwrap();
    assert!(Arc::ptr_eq(&collapsed.view, collapsed.selection.basis()));
    assert_eq!(collapsed.selection.len(), 1);
    assert!(!collapsed.selection.contains_row(expanded.view.rows[2].id));
    assert_eq!(collapsed.selection_revision, SelectionRevision(11));
}

#[test]
fn tree_selection_rejects_foreign_views_replay_and_bad_ranges_without_partial_changes() {
    let mut f = fixture();
    let view = f.session.projection().clone();
    let foreign_index = Arc::new((*view.root.index).clone());
    let wrong = Arc::new(SelectionSnapshot::from_bits(foreign_index, vec![1]).unwrap());
    assert!(
        f.session
            .seed_selection(SelectionRevision(1), &TreeRootSelection::Native(wrong))
            .is_err()
    );
    assert!(f.session.publication().unwrap().selection.is_empty());
    f.session.commit(&view);
    f.session
        .select(&request(
            &view,
            1,
            TreeSelectionAction::Bulk(SelectionOperation::All),
        ))
        .unwrap();
    let before = f.session.publication().unwrap();
    let clone = Arc::new((*view).clone());
    let bad_ranges = SelectionRanges::new(2, vec![SelectionSpan { first: 0, last: 1 }]).unwrap();
    for rejected in [
        request(
            &clone,
            2,
            TreeSelectionAction::Bulk(SelectionOperation::Clear),
        ),
        request(
            &view,
            1,
            TreeSelectionAction::Bulk(SelectionOperation::Clear),
        ),
        request(&view, 2, TreeSelectionAction::Toggle(TreeRowId(u64::MAX))),
        request(
            &view,
            2,
            TreeSelectionAction::Ranges {
                ranges: bad_ranges,
                mode: midnight_core::SelectionMode::Replace,
            },
        ),
    ] {
        assert!(f.session.select(&rejected).is_err());
        let after = f.session.publication().unwrap();
        assert!(Arc::ptr_eq(&before.selection, &after.selection));
        assert_eq!(after.selection_revision, SelectionRevision(1));
    }
    f.session
        .select(&request(
            &view,
            2,
            TreeSelectionAction::Bulk(SelectionOperation::Invert),
        ))
        .unwrap();
    let after = f.session.publication().unwrap();
    assert_eq!(
        (after.selection.file_count(), after.selection.folder_count()),
        (0, 2)
    );
}

#[test]
fn tree_filter_context_is_never_selected_even_by_ranges_or_bulk_commands() {
    let mut f = fixture();
    let initial = f.session.projection().clone();
    f.session.commit(&initial);
    f.session
        .branch(
            &initial,
            TreeRequestId(1),
            initial.rows[1].id,
            BranchAction::Expand,
        )
        .unwrap();
    let work = f.session.take_work().unwrap().unwrap();
    f.session
        .apply(work.run(), &Cancellation::default())
        .unwrap();
    let view = f.session.projection().clone();
    f.session.commit(&view);
    f.session
        .select(&request(
            &view,
            1,
            TreeSelectionAction::Toggle(view.rows[2].id),
        ))
        .unwrap();
    f.session
        .reproject(
            ProjectionOptions {
                filter: midnight_core::PanelFilter {
                    enabled: true,
                    mode: midnight_core::FilterMode::Expression,
                    expression: "name = \"child\"".into(),
                    case_sensitive: true,
                },
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .unwrap();
    let filtered = f.session.publication().unwrap();
    assert_eq!(filtered.selection.len(), 1);
    assert!(!filtered.selection.contains_position(0));
    f.session.commit(&filtered.view);
    let context = filtered.view.rows[0].id;
    let child = filtered.view.rows[1].id;
    assert!(
        f.session
            .select(&request(
                &filtered.view,
                2,
                TreeSelectionAction::Toggle(context)
            ))
            .is_err()
    );
    for (revision, action) in [
        (
            2,
            TreeSelectionAction::Range {
                anchor: context,
                target: child,
                additive: false,
            },
        ),
        (3, TreeSelectionAction::Bulk(SelectionOperation::All)),
        (
            4,
            TreeSelectionAction::Ranges {
                ranges: SelectionRanges::new(2, vec![SelectionSpan { first: 0, last: 1 }]).unwrap(),
                mode: midnight_core::SelectionMode::Replace,
            },
        ),
    ] {
        let selects_child = !matches!(action, TreeSelectionAction::Bulk(SelectionOperation::All));
        f.session
            .select(&request(&filtered.view, revision, action))
            .unwrap();
        let publication = f.session.publication().unwrap();
        assert_eq!(publication.selection.len(), usize::from(selects_child));
        assert!(!publication.selection.contains_row(context));
        assert_eq!(publication.selection.contains_row(child), selects_child);
    }
}

#[test]
fn selection_seeds_are_exact_ordered_and_fail_atomically() {
    use midnight_core::TreeSelectionSnapshot;

    let mut f = fixture();
    let view = f.session.projection().clone();
    let seed = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0b001]).unwrap());
    let mut action = request(&view, 5, TreeSelectionAction::Toggle(view.rows[2].id));
    action.seed_revision = SelectionRevision(4);
    action.seed = Some(seed.clone());
    assert!(
        f.session.select(&action).is_err(),
        "unacknowledged views cannot select"
    );
    f.session.commit(&view);
    let before = f.session.publication().unwrap();
    let foreign =
        Arc::new(TreeSelectionSnapshot::from_bits(Arc::new((*view).clone()), vec![1]).unwrap());
    for (revision, seed) in [(4, None), (5, Some(seed.clone())), (4, Some(foreign))] {
        action.seed_revision = SelectionRevision(revision);
        action.seed = seed;
        assert!(f.session.select(&action).is_err());
        assert!(Arc::ptr_eq(
            &before.selection,
            &f.session.publication().unwrap().selection
        ));
    }
    action.seed_revision = SelectionRevision(4);
    action.seed = Some(seed);
    f.session.select(&action).unwrap();
    assert_eq!(f.session.publication().unwrap().selection.bits(), &[0b101]);

    // A late snapshot must not overwrite a selection the worker already accepted.
    action.revision = SelectionRevision(6);
    action.action = TreeSelectionAction::Toggle(view.rows[1].id);
    f.session.select(&action).unwrap();
    assert_eq!(f.session.publication().unwrap().selection.bits(), &[0b111]);
    action.revision = SelectionRevision(7);
    action.seed_revision = SelectionRevision(6);
    action.seed = None;
    action.action = TreeSelectionAction::Bulk(SelectionOperation::Clear);
    f.session.select(&action).unwrap();
    assert!(f.session.publication().unwrap().selection.is_empty());
}

#[test]
fn ranges_keep_modifiers_and_reordering_keeps_selected_occurrences() {
    use midnight_core::SelectionMode;

    let mut f = fixture();
    let view = f.session.projection().clone();
    f.session.commit(&view);
    f.session
        .select(&request(
            &view,
            1,
            TreeSelectionAction::Toggle(view.rows[0].id),
        ))
        .unwrap();
    f.session
        .select(&request(
            &view,
            2,
            TreeSelectionAction::Range {
                anchor: view.rows[2].id,
                target: view.rows[1].id,
                additive: true,
            },
        ))
        .unwrap();
    assert_eq!(f.session.publication().unwrap().selection.bits(), &[0b111]);
    let ranges = SelectionRanges::new(3, vec![SelectionSpan { first: 1, last: 2 }]).unwrap();
    for (revision, mode, expected) in [
        (3, SelectionMode::Toggle, 0b001),
        (4, SelectionMode::Add, 0b111),
        (5, SelectionMode::Replace, 0b110),
    ] {
        f.session
            .select(&request(
                &view,
                revision,
                TreeSelectionAction::Ranges {
                    ranges: ranges.clone(),
                    mode,
                },
            ))
            .unwrap();
        assert_eq!(
            f.session.publication().unwrap().selection.bits(),
            &[expected]
        );
    }
    f.session
        .reproject(
            ProjectionOptions {
                sort: midnight_core::SortSpec {
                    descending: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .unwrap();
    let sorted = f.session.publication().unwrap();
    assert_eq!(sorted.selection.bits(), &[0b101]);
    assert!(sorted.selection.contains_row(view.rows[1].id));
    assert!(sorted.selection.contains_row(view.rows[2].id));
    assert!(
        f.session
            .select(&request(&view, 6, TreeSelectionAction::Rebase))
            .is_err()
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        f.session
            .reproject(ProjectionOptions::default(), &cancel)
            .is_err()
    );
    let after = f.session.publication().unwrap();
    assert!(Arc::ptr_eq(&sorted.view, &after.view));
    assert!(Arc::ptr_eq(&sorted.selection, &after.selection));
}

#[test]
fn archive_root_migration_uses_original_ids_and_preserves_directory_links_on_invert() {
    use midnight_core::{ArchiveNode, DirectoryIndex, Entry, EntryId, EntryKind, VirtualEntryId};
    use std::collections::HashSet;

    let f = fixture();
    let native = f.session.projection();
    let entries: Arc<[Entry]> = native
        .root
        .entries
        .iter()
        .map(|entry| {
            Entry::from_archive(ArchiveNode {
                modified: None,
                id: VirtualEntryId {
                    mount: 17,
                    entry: entry.id.0,
                },
                parent: 0,
                raw_name: Arc::from(entry.display_name.as_bytes()),
                display_name: entry.display_name.clone(),
                kind: if entry.display_name == "z-parent" {
                    EntryKind::DirectoryLink
                } else {
                    entry.kind
                },
                size: entry.metadata.size,
                readable: true,
            })
        })
        .collect();
    let root = TreeRootSnapshot {
        entries: entries.clone(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        ..native.root.clone()
    };
    let rows = native
        .rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            let alias = row.entry.id;
            row.entry = entries[root.index.by_id[&row.original_root_entry.unwrap()]].clone();
            row.entry.id = alias;
            if row.entry.kind == EntryKind::DirectoryLink {
                row.disclosure = midnight_core::TreeDisclosure::None;
            }
            row
        })
        .collect();
    let view =
        Arc::new(TreeProjection::new(root, native.stamp, native.layout_revision, rows).unwrap());
    let selected = TreeRootSelection::Archive(Arc::new(HashSet::from([EntryId(1), EntryId(3)])));
    let mut state = Selection::from_root(view.clone(), SelectionRevision(8), &selected).unwrap();
    assert_eq!(state.snapshot.bits(), &[0b110]);
    state
        .apply(&request(
            &view,
            9,
            TreeSelectionAction::Bulk(SelectionOperation::Invert),
        ))
        .unwrap();
    assert_eq!(state.snapshot.bits(), &[0b010]);
    assert_eq!(state.snapshot.folder_count(), 1);
    let foreign = TreeRootSelection::Archive(Arc::new(HashSet::from([EntryId(99)])));
    assert!(Selection::from_root(view, SelectionRevision(10), &foreign).is_err());
}

#[test]
fn repeated_root_seed_cannot_replace_a_publication_with_conflicting_equal_revision() {
    let mut f = fixture();
    let view = f.session.projection().clone();
    let original = TreeRootSelection::Native(Arc::new(
        SelectionSnapshot::from_bits(view.root.index.clone(), vec![0b001]).unwrap(),
    ));
    f.session
        .seed_selection(SelectionRevision(0), &original)
        .unwrap();
    let published = f.session.publication().unwrap();
    // The migration's initial revision may be zero. Retrying it is idempotent.
    f.session
        .seed_selection(SelectionRevision(0), &original)
        .unwrap();
    assert!(Arc::ptr_eq(
        &published.selection,
        &f.session.publication().unwrap().selection
    ));
    assert!(
        f.session
            .seed_selection(SelectionRevision(0), &TreeRootSelection::Empty)
            .is_err()
    );
    assert!(f.session.commit(&published.view));
    assert_eq!(f.session.publication().unwrap().selection.bits(), &[0b010]);
}

#[test]
fn published_default_selection_cannot_be_reseeded_differently_at_the_same_revision() {
    let mut f = fixture();
    let published = f.session.publication().unwrap();
    let different = TreeRootSelection::Native(Arc::new(
        SelectionSnapshot::from_bits(published.view.root.index.clone(), vec![1]).unwrap(),
    ));
    assert!(
        f.session
            .seed_selection(SelectionRevision(0), &different)
            .is_err()
    );
    assert!(Arc::ptr_eq(
        &published.selection,
        &f.session.publication().unwrap().selection
    ));
}

#[test]
fn clear_files_only_in_expanded_tree_preserves_explicit_nested_folder_marks() {
    let mut f = expanded_fixture();
    let view = f.session.projection().clone();
    for (position, row) in view.rows.iter().enumerate() {
        f.session
            .select(&request(
                &view,
                position as u64 + 1,
                TreeSelectionAction::Toggle(row.id),
            ))
            .unwrap();
    }
    f.session
        .select(&request(
            &view,
            view.rows.len() as u64 + 1,
            TreeSelectionAction::Bulk(SelectionOperation::ClearFilesOnly),
        ))
        .unwrap();
    let selected = f.session.publication().unwrap().selection;
    assert_eq!(selected.file_count(), 0);
    assert_eq!(
        selected.folder_count(),
        3,
        "preserve both top-level folders and the marked descendant"
    );
    for row in view.rows.iter() {
        assert_eq!(
            selected.contains_row(row.id),
            matches!(
                row.entry.kind,
                midnight_core::EntryKind::Directory | midnight_core::EntryKind::DirectoryLink
            )
        );
    }
}
