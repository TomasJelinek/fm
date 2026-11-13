use midnight_core::*;
use std::sync::Arc;

#[test]
fn native_maintenance_actions_accept_new_files_and_reject_foreign_or_forged_snapshots() {
    let mut initial = row(1, "old.txt", EntryKind::File);
    initial.original_root_entry = Some(EntryId(10));
    let root = root_for(&[initial]);
    let mut current = row(2, "new.txt", EntryKind::File);
    current.original_root_entry = Some(EntryId(11));
    let actions = root_for(&[current.clone()]);
    let view = TreeProjection::new_with_root_actions(
        root.clone(),
        actions.clone(),
        stamp(),
        TreeLayoutRevision(2),
        vec![current.clone()],
    )
    .unwrap();
    assert!(view.root.same_snapshot(&root));
    assert!(view.root_actions.same_snapshot(&actions));
    assert_eq!(view.index.root_rows[&EntryId(11)], TreeRowId(2));
    assert!(!view.root.index.by_id.contains_key(&EntryId(11)));
    for mode in 0..9 {
        let mut forged = actions.clone();
        let mut rows = vec![current.clone()];
        match mode {
            0 => forged.navigation = NavigationId(99),
            1 => forged.pane = PaneId::Right,
            2 => forged.index = root.index.clone(),
            3 => rows[0].entry.path = std::env::current_dir().unwrap().join("substitution.txt"),
            4 => rows[0].entry.kind = EntryKind::Directory,
            5 => {
                rows[0].entry = Entry::new(
                    EntryId(102),
                    std::env::current_dir().unwrap().join("foreign/new.txt"),
                    EntryKind::File,
                );
                forged = root_for(&rows);
            }
            6 => {
                let mut entries = forged.entries.to_vec();
                entries[0].name = "forged basename".into();
                forged.entries = entries.into();
            }
            7 => {
                let mut index = (*forged.index).clone();
                index.by_id.reserve(TreeProjection::MAX_ROWS * 3);
                forged.index = Arc::new(index);
            }
            _ => rows[0].original_root_entry = None,
        }
        assert!(
            TreeProjection::new_with_root_actions(
                root.clone(),
                forged,
                stamp(),
                TreeLayoutRevision(2),
                rows
            )
            .is_err(),
            "forgery {mode}"
        );
    }
}

#[test]
fn initial_constructor_shares_root_action_allocations() {
    let view = projection(vec![row(1, "initial.txt", EntryKind::File)]);
    assert!(view.root.same_snapshot(&view.root_actions));
}

fn stamp() -> TreeViewStamp {
    TreeViewStamp {
        pane: PaneId::Left,
        navigation: NavigationId(7),
        session: TreeSessionId(9),
        revision: TreeRevision(1),
    }
}

fn row(id: u64, name: &str, kind: EntryKind) -> TreeDisplayRow {
    TreeDisplayRow {
        id: TreeRowId(id),
        entry: Entry::new(
            EntryId(id + 100),
            std::env::current_dir().unwrap().join(name),
            kind,
        ),
        parent: None,
        depth: 0,
        disclosure: TreeDisclosure::None,
        expanded: false,
        selectable: true,
        navigable: kind == EntryKind::Directory,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: None,
        prefix_end: 0,
        search_match: true,
    }
}

fn projection(rows: Vec<TreeDisplayRow>) -> Arc<TreeProjection> {
    Arc::new(checked_projection(rows).unwrap())
}

fn checked_projection(rows: Vec<TreeDisplayRow>) -> Result<TreeProjection, &'static str> {
    TreeProjection::new(root_for(&rows), stamp(), TreeLayoutRevision(1), rows)
}

fn root_for(rows: &[TreeDisplayRow]) -> TreeRootSnapshot {
    let entries: Arc<[Entry]> = rows
        .iter()
        .filter(|row| row.parent.is_none())
        .map(|row| {
            let mut entry = row.entry.clone();
            entry.id = row.original_root_entry.unwrap_or(entry.id);
            entry
        })
        .collect();
    TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(7),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    }
}

fn mixed_rows() -> Vec<TreeDisplayRow> {
    let mut native = row(1, "東京.txt", EntryKind::File);
    native.entry.metadata.size = Some(7);
    let folder = row(2, "složka", EntryKind::Directory);
    let mut virtual_file = row(3, "same caption", EntryKind::File);
    virtual_file.entry = Entry::from_archive(ArchiveNode {
        modified: None,
        id: VirtualEntryId {
            mount: 44,
            entry: 1,
        },
        parent: 1,
        raw_name: Arc::from([0xff, 6]),
        display_name: "same caption".into(),
        kind: EntryKind::File,
        size: Some(11),
        readable: true,
    });
    virtual_file.entry.id = EntryId(103);
    vec![native, folder, virtual_file]
}

#[test]
fn tree_projection_keeps_native_and_virtual_identities_and_constant_time_indexes() {
    let view = projection(mixed_rows());
    assert_eq!(view.index.row_positions[&TreeRowId(3)], 2);
    assert_eq!(view.index.directory.by_id[&EntryId(103)], 2);
    assert_eq!(view.index.directory.by_path.len(), 2);
    assert!(view.rows[2].entry.native_path().is_none());
    assert_eq!(
        &*view.rows[2].entry.virtual_node.as_ref().unwrap().raw_name,
        &[0xff, 6]
    );
    assert!(view.rows[0].entry.path.ends_with("東京.txt"));
}

#[test]
fn search_index_contains_only_matching_projection_positions() {
    let mut rows = mixed_rows();
    rows[1].search_match = false;
    let view = projection(rows);
    assert_eq!(&*view.index.search_rows, &[0, 2]);
}

#[test]
fn root_focus_index_maps_only_visible_original_roots() {
    let mut parent = row(1, "東京 folder", EntryKind::Directory);
    parent.original_root_entry = Some(EntryId(90));
    parent.expanded = true;
    parent.disclosure = TreeDisclosure::Expanded;
    let mut omitted = row(2, "other folder", EntryKind::Directory);
    omitted.original_root_entry = Some(EntryId(91));
    let root = root_for(&[parent.clone(), omitted]);
    let mut child = row(3, "東京 folder/descendant", EntryKind::Directory);
    child.parent = Some(parent.id);
    child.depth = 1;
    let view =
        TreeProjection::new(root, stamp(), TreeLayoutRevision(1), vec![parent, child]).unwrap();

    assert_eq!(view.index.root_rows.len(), 1);
    assert_eq!(view.index.root_rows[&EntryId(90)], TreeRowId(1));
    assert!(!view.index.root_rows.contains_key(&EntryId(91)));
    assert!(!view.index.root_rows.contains_key(&EntryId(103)));
}

#[test]
fn tree_selection_counts_mixed_rows_without_native_path_aliases() {
    let view = projection(mixed_rows());
    let selection = TreeSelectionSnapshot::from_bits(view.clone(), vec![0b111]).unwrap();
    assert!(Arc::ptr_eq(selection.basis(), &view));
    assert_eq!(selection.len(), 3);
    assert_eq!(selection.file_count(), 2);
    assert_eq!(selection.folder_count(), 1);
    assert_eq!(selection.file_bytes(), Some(18));
    assert!(selection.contains_row(TreeRowId(3)));
    assert!(!selection.contains_row(TreeRowId(999)));
    assert!(!selection.contains_position(3));
    assert!(!selection.contains_position(usize::MAX));
    assert!(TreeSelectionSnapshot::from_bits(view.clone(), vec![8]).is_err());
    assert!(TreeSelectionSnapshot::from_bits(view, vec![]).is_err());
}

#[test]
fn context_ancestors_are_retained_for_navigation_but_cannot_be_selected() {
    let mut parent = row(1, "ancestor", EntryKind::Directory);
    parent.context_only = true;
    parent.selectable = false;
    parent.search_match = false;
    parent.expanded = true;
    parent.disclosure = TreeDisclosure::Expanded;
    let mut child = row(2, "ancestor/match", EntryKind::Directory);
    child.parent = Some(parent.id);
    child.depth = 1;
    let view = projection(vec![parent, child]);
    assert!(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).is_err());
    assert_eq!(
        TreeSelectionSnapshot::from_bits(view, vec![2])
            .unwrap()
            .folder_count(),
        1
    );
}

#[test]
fn projection_rejects_duplicate_ids_wrong_parent_order_and_hidden_descendants() {
    for case in 0..7 {
        let mut first = row(1, "a", EntryKind::Directory);
        let mut second = row(2, "b", EntryKind::Directory);
        match case {
            0 => second.id = first.id,
            1 => second.entry.id = first.entry.id,
            2 => {
                second.parent = Some(TreeRowId(99));
                second.depth = 1;
            }
            3 => {
                second.parent = Some(first.id);
                second.depth = 1;
            }
            4 => first.id = TreeRowId(0),
            5 => first.context_only = true,
            _ => second.depth = TreeProjection::MAX_DEPTH + 1,
        }
        assert!(
            checked_projection(vec![first, second]).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn utf8_highlights_and_projection_payload_are_bounded_without_max_path() {
    let mut text = row(1, "東京", EntryKind::File);
    text.prefix_end = 1;
    assert!(checked_projection(vec![text.clone()]).is_err());
    text.prefix_end = 3;
    assert!(checked_projection(vec![text]).is_ok());
    assert!(checked_projection(vec![row(1, &"x".repeat(70_000), EntryKind::File)]).is_ok());
    let mut large = row(1, "small", EntryKind::File);
    large.entry.display_name = String::with_capacity(TreeProjection::MAX_BYTES + 1);
    large.entry.display_name.push('a');
    assert!(checked_projection(vec![large]).is_err());
    let mut large_error = row(1, "small", EntryKind::Directory);
    large_error.error = Some("x".repeat(4097));
    assert!(checked_projection(vec![large_error]).is_err());
}

#[test]
fn selection_never_invents_known_bytes_for_unknown_or_overflowing_sizes() {
    let mut rows = mixed_rows();
    rows[0].entry.metadata.size = None;
    assert_eq!(
        TreeSelectionSnapshot::from_bits(projection(rows), vec![1])
            .unwrap()
            .file_bytes(),
        None
    );
    let mut rows = mixed_rows();
    rows[0].entry.metadata.size = Some(u64::MAX);
    assert_eq!(
        TreeSelectionSnapshot::from_bits(projection(rows), vec![5])
            .unwrap()
            .file_bytes(),
        None
    );
    let empty = projection(Vec::new());
    assert!(
        TreeSelectionSnapshot::from_bits(empty, vec![])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn projection_rejects_contradictory_disclosure_and_context_flags() {
    for disclosure in [
        TreeDisclosure::Unknown,
        TreeDisclosure::Collapsed,
        TreeDisclosure::Empty,
    ] {
        let mut parent = row(1, "folder", EntryKind::Directory);
        parent.expanded = true;
        parent.disclosure = disclosure;
        assert!(checked_projection(vec![parent]).is_err());
    }
    let mut context = row(1, "context", EntryKind::Directory);
    context.context_only = true;
    context.selectable = false;
    assert!(checked_projection(vec![context]).is_err());
    let mut unavailable = row(1, "file", EntryKind::File);
    unavailable.disclosure = TreeDisclosure::Failed;
    assert!(checked_projection(vec![unavailable]).is_err());
}

#[test]
fn directory_links_share_coherent_folder_disclosure_validation() {
    for disclosure in [
        TreeDisclosure::Unknown,
        TreeDisclosure::Loading,
        TreeDisclosure::Collapsed,
        TreeDisclosure::Expanded,
        TreeDisclosure::Empty,
        TreeDisclosure::Failed,
    ] {
        let mut link = row(1, "link", EntryKind::DirectoryLink);
        link.navigable = true;
        link.expanded = disclosure == TreeDisclosure::Expanded;
        link.disclosure = disclosure;
        assert!(
            checked_projection(vec![link.clone()]).is_ok(),
            "{disclosure:?}"
        );
        link.navigable = false;
        assert!(checked_projection(vec![link]).is_err());
    }
}

#[test]
fn projection_rejects_fabricated_virtual_paths_and_mismatched_virtual_kinds() {
    let mut virtual_row = mixed_rows().remove(2);
    virtual_row.entry.path = std::env::current_dir().unwrap().join("pretend-native-path");
    assert!(checked_projection(vec![virtual_row]).is_err());
    let mut virtual_row = mixed_rows().remove(2);
    virtual_row.entry.kind = EntryKind::Directory;
    assert!(checked_projection(vec![virtual_row]).is_err());
}

#[test]
fn projection_rejects_descendant_root_aliases_and_duplicate_root_aliases() {
    let mut first = row(1, "folder", EntryKind::Directory);
    first.original_root_entry = Some(EntryId(90));
    let mut second = row(2, "sibling", EntryKind::Directory);
    second.original_root_entry = first.original_root_entry;
    assert!(checked_projection(vec![first.clone(), second.clone()]).is_err());
    first.expanded = true;
    first.disclosure = TreeDisclosure::Expanded;
    second.parent = Some(first.id);
    second.depth = 1;
    second.original_root_entry = Some(EntryId(91));
    assert!(checked_projection(vec![first, second]).is_err());
}

#[test]
fn projection_budget_includes_indexes_and_bounds_diagnostic_allocations() {
    let mut near_limit = mixed_rows().remove(2);
    near_limit.entry.name = Default::default();
    let node = near_limit.entry.virtual_node.as_mut().unwrap();
    node.raw_name = Arc::from([]);
    node.display_name = String::new();
    near_limit.entry.display_name = String::with_capacity(
        TreeProjection::MAX_BYTES - std::mem::size_of::<TreeDisplayRow>() - 8,
    );
    near_limit.entry.display_name.push('x');
    assert!(checked_projection(vec![near_limit]).is_err());

    let mut diagnostic = row(1, "folder", EntryKind::Directory);
    diagnostic.disclosure = TreeDisclosure::Failed;
    diagnostic.error = Some(String::with_capacity(8192));
    diagnostic.error.as_mut().unwrap().push_str("failed");
    assert!(checked_projection(vec![diagnostic]).is_err());
}

#[test]
fn context_row_must_lead_to_a_visible_match() {
    let mut context = row(1, "context", EntryKind::Directory);
    context.context_only = true;
    context.selectable = false;
    context.search_match = false;
    context.expanded = true;
    context.disclosure = TreeDisclosure::Expanded;
    assert!(checked_projection(vec![context]).is_err());
}

#[test]
fn filter_only_context_keeps_matching_descendants_without_a_prefix_query() {
    let mut context = row(1, "context", EntryKind::Directory);
    context.context_only = true;
    context.selectable = false;
    context.search_match = false;
    context.expanded = true;
    context.disclosure = TreeDisclosure::Expanded;
    let mut child = row(2, "context/filtered", EntryKind::Directory);
    child.parent = Some(context.id);
    child.depth = 1;
    child.search_match = false;
    let view = checked_projection(vec![context, child]).unwrap();
    assert!(!view.rows[1].search_match);
    assert!(view.rows[1].selectable);
}

#[test]
fn root_binding_requires_the_exact_entries_and_index_and_scope() {
    let rows = mixed_rows();
    let root = root_for(&rows);
    assert!(root.same_snapshot(&root.clone()));
    let mut substitute = root.clone();
    substitute.entries = root.entries.to_vec().into();
    assert!(!root.same_snapshot(&substitute));
    let mut substitute = root.clone();
    substitute.index = Arc::new((*root.index).clone());
    assert!(!root.same_snapshot(&substitute));
    let mut substitute = root.clone();
    substitute.navigation = NavigationId(99);
    assert!(!root.same_snapshot(&substitute));
    assert!(TreeProjection::new(substitute, stamp(), TreeLayoutRevision(1), rows).is_err());
}

#[test]
fn metadata_changes_preserve_layout_but_selection_rebases_to_exact_new_view() {
    let view = projection(mixed_rows());
    let selected = TreeSelectionSnapshot::from_bits(view.clone(), vec![5]).unwrap();
    let mut rows = view.rows.to_vec();
    rows[0].entry.metadata.size = Some(50);
    rows[1].disclosure = TreeDisclosure::Empty;
    let mut next_stamp = stamp();
    next_stamp.revision = TreeRevision(2);
    let next = Arc::new(
        TreeProjection::new(view.root.clone(), next_stamp, TreeLayoutRevision(1), rows).unwrap(),
    );
    assert!(view.same_structure(&next));
    let rebased = selected.rebase(next.clone()).unwrap();
    assert!(Arc::ptr_eq(rebased.basis(), &next));
    assert!(rebased.contains_row(TreeRowId(1)));
    assert!(rebased.contains_row(TreeRowId(3)));
    assert_eq!(rebased.file_bytes(), Some(61));
    let mut rows = next.rows.to_vec();
    rows.swap(0, 1);
    next_stamp.revision = TreeRevision(3);
    let reordered =
        TreeProjection::new(view.root.clone(), next_stamp, TreeLayoutRevision(2), rows).unwrap();
    assert!(!next.same_structure(&reordered));
}

#[test]
fn selection_rebase_drops_hidden_rows_and_rejects_foreign_sessions() {
    let view = projection(mixed_rows());
    let selected = TreeSelectionSnapshot::from_bits(view.clone(), vec![5]).unwrap();
    let mut next_stamp = stamp();
    next_stamp.revision = TreeRevision(2);
    let next = Arc::new(
        TreeProjection::new(
            view.root.clone(),
            next_stamp,
            TreeLayoutRevision(2),
            vec![view.rows[2].clone()],
        )
        .unwrap(),
    );
    let rebased = selected.rebase(next).unwrap();
    assert_eq!(rebased.len(), 1);
    assert!(rebased.contains_row(TreeRowId(3)));
    next_stamp.session = TreeSessionId(10);
    let foreign = Arc::new(
        TreeProjection::new(
            view.root.clone(),
            next_stamp,
            TreeLayoutRevision(1),
            view.rows.to_vec(),
        )
        .unwrap(),
    );
    assert!(selected.rebase(foreign).is_err());
}

#[test]
fn tree_identity_counters_never_wrap_or_reuse_zero() {
    assert_eq!(TreeSessionId(0).checked_next(), Some(TreeSessionId(1)));
    assert_eq!(TreeSessionId(u64::MAX).checked_next(), None);
    assert_eq!(TreeRevision(u64::MAX).checked_next(), None);
    assert_eq!(TreeLayoutRevision(u64::MAX).checked_next(), None);
    assert_eq!(TreeRowId(u64::MAX).checked_next(), None);
    assert_eq!(TreeRequestId(u64::MAX).checked_next(), None);
}

#[test]
fn publication_requires_exact_view_selection_and_current_selection_revision() {
    let view = projection(mixed_rows());
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap());
    let publication =
        TreePublication::new(view.clone(), SelectionRevision(7), selection.clone()).unwrap();
    assert!(publication.can_adopt(&view.root, SelectionRevision(7)));
    assert!(!publication.can_adopt(&view.root, SelectionRevision(8)));
    assert!(!publication.can_adopt(&view.root, SelectionRevision(6)));
    let impostor = Arc::new((*view).clone());
    assert!(TreePublication::new(impostor, SelectionRevision(7), selection).is_err());
    let mut root = view.root.clone();
    root.index = Arc::new((*root.index).clone());
    assert!(!publication.can_adopt(&root, SelectionRevision(7)));
}

#[test]
fn root_selection_requires_its_exact_native_index_or_archive_membership() {
    let rows = vec![
        row(1, "one", EntryKind::File),
        row(2, "two", EntryKind::Directory),
    ];
    let view = projection(rows);
    let selected =
        Arc::new(SelectionSnapshot::from_bits(view.root.index.clone(), vec![1]).unwrap());
    assert!(
        TreeRootSelection::Native(selected)
            .validate_for(&view.root)
            .is_ok()
    );
    let selected = Arc::new(
        SelectionSnapshot::from_bits(Arc::new((*view.root.index).clone()), vec![1]).unwrap(),
    );
    assert!(
        TreeRootSelection::Native(selected)
            .validate_for(&view.root)
            .is_err()
    );
    let view = projection(vec![mixed_rows().remove(2)]);
    let selected = Arc::new([EntryId(103)].into_iter().collect());
    assert!(
        TreeRootSelection::Archive(selected)
            .validate_for(&view.root)
            .is_ok()
    );
    let foreign = Arc::new([EntryId(999)].into_iter().collect());
    assert!(
        TreeRootSelection::Archive(foreign)
            .validate_for(&view.root)
            .is_err()
    );
}

#[test]
fn viewport_and_diagnostics_are_bounded_and_keep_unicode() {
    let view = projection(mixed_rows());
    assert_eq!(
        TreeViewport::new(&view, vec![TreeRowId(1), TreeRowId(3)])
            .unwrap()
            .rows(),
        &[TreeRowId(1), TreeRowId(3)]
    );
    assert!(TreeViewport::new(&view, vec![TreeRowId(1); 257]).is_err());
    assert!(TreeViewport::new(&view, vec![TreeRowId(999)]).is_err());
    assert!(TreeViewport::new(&view, vec![TreeRowId(1); 2]).is_err());
    let error = TreeDiagnostic::new(&"東京".repeat(1000));
    assert!(error.as_str().len() <= 4096);
    assert!(error.as_str().ends_with('東') || error.as_str().ends_with('京'));
    assert!(TreeDiagnostic::new("slow provider").as_str() == "slow provider");
}

#[test]
fn file_only_bulk_policy_never_selects_expanded_descendants_and_clear_preserves_folders() {
    for depth in [0, 1, 2] {
        for kind in [
            EntryKind::Directory,
            EntryKind::DirectoryLink,
            EntryKind::File,
            EntryKind::Symlink,
            EntryKind::Other,
        ] {
            let mut item = row(7, "marked", kind);
            item.depth = depth;
            let directory = matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink);
            for selected in [false, true] {
                assert_eq!(
                    item.bulk_selected(SelectionOperation::FilesOnly, selected),
                    depth == 0 && !directory,
                    "{kind:?} at depth {depth}"
                );
                assert_eq!(
                    item.bulk_selected(SelectionOperation::ClearFilesOnly, selected),
                    directory && selected,
                    "{kind:?} at depth {depth}"
                );
                assert!(!item.bulk_selected(SelectionOperation::Clear, selected));
            }
            item.context_only = true;
            assert!(!item.bulk_selected(SelectionOperation::FilesOnly, false));
            assert!(!item.bulk_selected(SelectionOperation::ClearFilesOnly, true));
        }
    }
}

#[test]
fn replacement_action_does_not_retain_its_source_projection() {
    let view = projection(vec![row(1, "source.txt", EntryKind::File)]);
    let retained = Arc::downgrade(&view);
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap());
    let action = TreeSelectionAction::Replace(Arc::new(
        TreeSelectionReplacement::from_snapshot(&selection).unwrap(),
    ));
    drop(selection);
    drop(view);
    assert!(
        retained.upgrade().is_none(),
        "a queued selection replacement must retain no source projection"
    );
    drop(action);
}

#[test]
fn replacement_payload_caps_shape_and_fences_preview_positions() {
    let view = projection(vec![
        row(1, "one.txt", EntryKind::File),
        row(2, "two.txt", EntryKind::File),
    ]);
    let selection = TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap();
    let replacement = TreeSelectionReplacement::from_snapshot(&selection).unwrap();
    assert!(replacement.within_payload_budget());
    assert!(replacement.matches_origin(&view));
    assert!(replacement.contains_position(0, TreeRowId(1)));
    assert!(!replacement.contains_position(0, TreeRowId(2)));
    assert!(!replacement.contains_position(1, TreeRowId(2)));
    assert!(!replacement.contains_position(usize::MAX, TreeRowId(1)));
    let mut oversized = (*view).clone();
    oversized.rows = vec![view.rows[0].clone(); TreeProjection::MAX_ROWS + 1].into();
    let oversized = Arc::new(oversized);
    let snapshot = TreeSelectionSnapshot::from_bits(
        oversized.clone(),
        vec![0; oversized.rows.len().div_ceil(64)],
    )
    .unwrap();
    assert!(TreeSelectionReplacement::from_snapshot(&snapshot).is_err());
}

#[test]
fn replacement_payload_accepts_metadata_but_rejects_scope_and_occurrence_changes() {
    let view = projection(vec![
        row(1, "one.txt", EntryKind::File),
        row(2, "two.txt", EntryKind::File),
    ]);
    let selection = TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap();
    let replacement = TreeSelectionReplacement::from_snapshot(&selection).unwrap();
    for change in 0..9 {
        let mut next = (*view).clone();
        next.stamp.revision = TreeRevision(2);
        let mut rows = view.rows.to_vec();
        rows[0].entry.metadata.size = Some(123);
        match change {
            0 => {}
            1 => next.stamp.pane = PaneId::Right,
            2 => next.stamp.session = TreeSessionId(77),
            3 => next.stamp.navigation = NavigationId(77),
            4 => next.layout_revision = TreeLayoutRevision(77),
            5 => rows.swap(0, 1),
            6 => rows[0].id = TreeRowId(77),
            7 => rows[0].selectable = false,
            8 => rows[0].original_root_entry = Some(EntryId(77)),
            _ => unreachable!(),
        }
        next.rows = rows.into();
        assert_eq!(
            replacement.matches_view(&next),
            change == 0,
            "change {change}"
        );
        assert!(!replacement.matches_origin(&next));
    }
}

#[test]
fn small_selection_and_visible_native_root_file_cache_are_exact_and_bounded() {
    let mut rows = vec![
        row(1, "one", EntryKind::File),
        row(2, "two", EntryKind::File),
        row(3, "three", EntryKind::File),
    ];
    for row in &mut rows {
        row.original_root_entry = Some(row.entry.id);
    }
    rows[1].error = Some("unavailable".into());
    let view = projection(rows);
    assert_eq!(
        view.index.native_root_files.as_ref(),
        &[EntryId(101), EntryId(103)]
    );
    for (bits, expected) in [
        (0, Some(vec![])),
        (4, Some(vec![TreeRowId(3)])),
        (5, Some(vec![TreeRowId(1), TreeRowId(3)])),
        (7, None),
    ] {
        let selection = TreeSelectionSnapshot::from_bits(view.clone(), vec![bits]).unwrap();
        assert_eq!(selection.small_selection(), expected.as_deref());
    }
}
