use super::*;

fn side(pane: PaneId, size: u64) -> CompareSide {
    let path = std::env::current_dir()
        .unwrap()
        .join(if pane == PaneId::Left {
            "left"
        } else {
            "right"
        });
    let folder = Entry::new(EntryId(4), path.join("folder"), EntryKind::Directory);
    let mut file = Entry::new(EntryId(8), path.join("same.txt"), EntryKind::File);
    file.metadata.size = Some(size);
    let entries: Arc<[Entry]> = vec![folder, file].into();
    let root = TreeRootSnapshot {
        pane,
        navigation: NavigationId(1),
        entries: entries.clone(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
    };
    let row = |mut entry: Entry, id, parent: Option<TreeRowId>, original| {
        entry.id = EntryId(id);
        TreeDisplayRow {
            id: TreeRowId(id),
            entry,
            parent,
            depth: usize::from(parent.is_some()),
            disclosure: TreeDisclosure::None,
            expanded: false,
            selectable: true,
            navigable: original == Some(EntryId(4)) || parent.is_some(),
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: original,
            prefix_end: 0,
            search_match: false,
        }
    };
    let mut parent = row(entries[0].clone(), 40, None, Some(EntryId(4)));
    parent.expanded = true;
    parent.disclosure = TreeDisclosure::Expanded;
    let child = row(
        Entry::new(
            EntryId(9),
            path.join("folder").join(if pane == PaneId::Left {
                "only-left-child"
            } else {
                "only-right-child"
            }),
            EntryKind::Directory,
        ),
        90,
        Some(TreeRowId(40)),
        None,
    );
    let file = row(entries[1].clone(), 80, None, Some(EntryId(8)));
    let tree = Arc::new(
        TreeProjection::new(
            root,
            TreeViewStamp {
                pane,
                navigation: NavigationId(1),
                session: TreeSessionId(3 + pane.index() as u64),
                revision: TreeRevision(1),
            },
            TreeLayoutRevision(1),
            vec![parent, child, file],
        )
        .unwrap(),
    );
    CompareSide {
        source: VisibleDirectory {
            pane,
            navigation: NavigationId(1),
            path,
            entries,
        },
        view: tree.index.directory.clone(),
        tree: Some(tree),
        view_revision: 0,
        selection_revision: SelectionRevision(0),
    }
}
fn request() -> Arc<CompareRequest> {
    Arc::new(CompareRequest {
        id: CompareId(1),
        sides: [side(PaneId::Left, 10), side(PaneId::Right, 20)],
        options: CompareOptions {
            by_modified: false,
            unmatched_directories: true,
            ..Default::default()
        },
    })
}

#[test]
fn comparison_uses_root_rows_and_returns_occurrence_selections() {
    let request = request();
    let result = compute(request.clone(), &|| false).unwrap();
    assert_eq!(
        result
            .selections
            .each_ref()
            .map(|selection| selection.len()),
        [0, 1]
    );
    for slot in 0..2 {
        let selection = result.tree_replacements[slot].as_ref().unwrap();
        assert!(selection.matches_origin(request.sides[slot].tree.as_ref().unwrap()));
        assert!(
            !selection.contains_position(1, TreeRowId(90)),
            "nested folders are not root comparison entries"
        );
        assert_eq!(selection.contains_position(2, TreeRowId(80)), slot == 1);
    }
}

#[test]
fn comparison_selection_accepts_metadata_refresh_but_rejects_changed_layout() {
    let request = request();
    let result = compute(request.clone(), &|| false).unwrap();
    let source = result.tree_replacements[1].as_ref().unwrap();
    let basis = request.sides[1].tree.as_ref().unwrap();
    for changed in [false, true] {
        let mut rows = basis.rows.to_vec();
        rows[1].entry.metadata.size = Some(123);
        if changed {
            rows[1].selectable = false;
        }
        let mut stamp = basis.stamp;
        stamp.revision = TreeRevision(2);
        let view = Arc::new(
            TreeProjection::new(basis.root.clone(), stamp, basis.layout_revision, rows).unwrap(),
        );
        let mut current = crate::tree_selection::Selection::from_root(
            view.clone(),
            SelectionRevision(0),
            &TreeRootSelection::Empty,
        )
        .unwrap();
        let applied = current.apply(&TreeSelectionRequest {
            view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Replace(source.clone()),
        });
        if changed {
            assert!(applied.is_err());
            assert!(current.snapshot.is_empty());
        } else {
            applied.unwrap();
            assert!(current.snapshot.contains_row(TreeRowId(80)));
        }
    }
}

#[test]
fn comparison_tree_source_must_be_the_registered_native_root() {
    let request = request();
    let snapshots = super::tests::sources(&request);
    assert!(!source_matches(&snapshots, &request));
    {
        let mut states = snapshots.lock().unwrap();
        for (slot, side) in request.sides.iter().enumerate() {
            states[slot].remember_native_tree_root(
                side.tree.as_ref().unwrap().root.clone(),
                side.source.path.clone(),
            );
        }
    }
    assert!(source_matches(&snapshots, &request));
    snapshots.lock().unwrap()[1].pending = Some(NavigationId(2));
    assert!(!source_matches(&snapshots, &request));
}
