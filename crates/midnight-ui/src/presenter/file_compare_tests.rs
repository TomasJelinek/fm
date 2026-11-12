use super::*;

fn loaded() -> Presenter {
    let mut presenter = Presenter::new("left".into(), "right".into());
    presenter.viewer_plugins.startup_disabled = Some(Arc::from([]));
    for pane in &mut presenter.panes {
        pane.entries = vec![
            Entry::new(EntryId(0), pane.path.join("one"), EntryKind::File),
            Entry::new(EntryId(1), pane.path.join("two"), EntryKind::File),
            Entry::new(EntryId(2), pane.path.join("three"), EntryKind::File),
        ]
        .into();
        pane.index = Arc::new(DirectoryIndex::from_entries(&pane.entries));
        pane.cursor = Some(EntryId(0));
    }
    presenter
}

#[test]
fn captures_right_active_same_name_or_two_active_selections_without_expanding_them() {
    let mut presenter = loaded();
    let launch = presenter.file_compare_launch(PaneId::Right).unwrap();
    assert!(
        matches!(&launch.sources[..], [FileCompareSource::SameName { directory, .. }, FileCompareSource::Focused(right)] if directory == Path::new("left") && right == Path::new("right/one"))
    );
    let pane = &mut presenter.panes[1];
    pane.selected = Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![3]).unwrap());
    let launch = presenter.file_compare_launch(PaneId::Right).unwrap();
    assert!(
        matches!(&launch.sources[..], [FileCompareSource::Selected { selection, .. }] if Arc::ptr_eq(selection, &presenter.panes[1].selected))
    );
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[1],
        FileCompareSource::Unspecified
    ));
}

#[test]
fn pending_and_ambiguous_selection_offer_manual_choice_without_guessing() {
    let mut presenter = loaded();
    presenter.panes[0].selection_revision = SelectionRevision(1);
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
        FileCompareSource::Unspecified
    ));
    presenter.panes[0].selection_applied = SelectionRevision(1);
    let pane = &mut presenter.panes[0];
    pane.selected = Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![7]).unwrap());
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
        FileCompareSource::Unspecified
    ));
}

#[test]
fn ambiguous_active_selection_preserves_explicit_opposite_file() {
    for active in [PaneId::Left, PaneId::Right] {
        let mut presenter = loaded();
        for (index, pane) in presenter.panes.iter_mut().enumerate() {
            pane.selected = Arc::new(
                SelectionSnapshot::from_bits(
                    pane.index.clone(),
                    vec![if index == active.index() { 7 } else { 1 }],
                )
                .unwrap(),
            );
        }
        let launch = presenter.file_compare_launch(active).unwrap();
        assert!(matches!(
            launch.sources[active.index()],
            FileCompareSource::Unspecified
        ));
        assert!(
            matches!(&launch.sources[1-active.index()], FileCompareSource::Selected { selection, .. } if selection.len() == 1)
        );
    }
}

#[test]
fn same_name_capture_ignores_opposite_cursor_and_keeps_the_active_source() {
    let mut presenter = loaded();
    presenter.panes[1].cursor = Some(EntryId(2));
    let launch = presenter.file_compare_launch(PaneId::Left).unwrap();
    assert!(
        matches!(&launch.sources[0], FileCompareSource::Focused(path) if path == Path::new("left/one"))
    );
    assert!(
        matches!(&launch.sources[1], FileCompareSource::SameName { directory, entries, view } if directory == Path::new("right") && Arc::ptr_eq(entries, &presenter.panes[1].entries) && Arc::ptr_eq(view, presenter.panes[1].display_index()))
    );
}

#[test]
fn same_name_capture_preserves_one_active_selection_and_explicit_opposite_selection() {
    let mut presenter = loaded();
    let active = &mut presenter.panes[0];
    active.selected =
        Arc::new(SelectionSnapshot::from_bits(active.index.clone(), vec![2]).unwrap());
    let launch = presenter.file_compare_launch(PaneId::Left).unwrap();
    assert!(
        matches!(&launch.sources[0], FileCompareSource::Selected { selection, .. } if Arc::ptr_eq(selection, &presenter.panes[0].selected))
    );
    assert!(matches!(
        &launch.sources[1],
        FileCompareSource::SameName { .. }
    ));
    let opposite = &mut presenter.panes[1];
    opposite.selected =
        Arc::new(SelectionSnapshot::from_bits(opposite.index.clone(), vec![4]).unwrap());
    let launch = presenter.file_compare_launch(PaneId::Left).unwrap();
    assert!(
        matches!(&launch.sources[1], FileCompareSource::Selected { selection, .. } if Arc::ptr_eq(selection, &presenter.panes[1].selected))
    );
}

#[test]
fn same_name_capture_does_not_require_an_opposite_cursor_but_requires_a_ready_view() {
    let mut presenter = loaded();
    presenter.panes[1].cursor = None;
    presenter.panes[1].parent_cursor = true;
    assert!(presenter.file_compare_launch(PaneId::Left).is_ok());
    presenter.panes[1].loading = true;
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[1],
        FileCompareSource::Unspecified
    ));
    presenter.panes[1].loading = false;
    presenter.panes[1].search.panel_filter.enabled = true;
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[1],
        FileCompareSource::Unspecified
    ));
    presenter.panes[1].search.panel_filter.enabled = false;
    presenter.panes[1].selection_rebase_pending = true;
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[1],
        FileCompareSource::Unspecified
    ));
}

#[test]
fn explicit_opposite_selection_and_active_selection_do_not_require_cursors() {
    let mut presenter = loaded();
    for (index, pane) in presenter.panes.iter_mut().enumerate() {
        pane.cursor = None;
        pane.parent_cursor = true;
        pane.selected =
            Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![1 << index]).unwrap());
    }
    let launch = presenter.file_compare_launch(PaneId::Right).unwrap();
    assert!(
        matches!(&launch.sources[..], [FileCompareSource::Selected { selection: left, .. }, FileCompareSource::Selected { selection: right, .. }] if Arc::ptr_eq(left, &presenter.panes[0].selected) && Arc::ptr_eq(right, &presenter.panes[1].selected))
    );
}

#[test]
fn parent_directory_and_missing_cursor_offer_manual_file_choice() {
    let mut presenter = loaded();
    for state in 0..3 {
        presenter.panes[0].parent_cursor = state == 0;
        presenter.panes[0].cursor = (state != 1).then_some(EntryId(0));
        if state == 2 {
            Arc::make_mut(&mut presenter.panes[0].entries)[0].kind = EntryKind::Directory;
        }
        assert!(matches!(
            presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
            FileCompareSource::Unspecified
        ));
    }
}

#[test]
fn file_choice_waits_for_disabled_plugin_preferences() {
    let mut presenter = loaded();
    presenter.viewer_plugins.startup_disabled = None;
    assert!(presenter.file_compare_launch(PaneId::Left).is_err());
}

#[test]
fn network_discovery_never_supplies_native_counterpart_directory_or_old_selection() {
    let mut presenter = loaded();
    let pane = &mut presenter.panes[1];
    pane.network = Some(network_neighborhood::NetworkPane::restored(
        NetworkAddress {
            provider: "network.test".into(),
            opaque: Arc::from([1]),
            display: "Server".into(),
        },
        pane.path.clone(),
    ));
    for selected in [false, true] {
        if selected {
            let pane = &mut presenter.panes[1];
            pane.selected =
                Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![1]).unwrap());
        }
        let launch = presenter.file_compare_launch(PaneId::Left).unwrap();
        assert!(matches!(launch.sources[1], FileCompareSource::Unspecified));
    }
}

fn tree_files(presenter: &mut Presenter, id: PaneId, bits: u64) -> Arc<TreeProjection> {
    let pane = &mut presenter.panes[id.index()];
    pane.path = std::env::temp_dir().join(if id == PaneId::Left { "left" } else { "right" });
    let entries: Arc<[Entry]> = ["one", "two", "three", "folder"]
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(
                EntryId(index as u64),
                pane.path.join(name),
                if index == 3 {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
            )
        })
        .collect::<Vec<_>>()
        .into();
    let root = TreeRootSnapshot {
        pane: id,
        navigation: NavigationId(pane.generation),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let mut rows: Vec<_> = root
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut displayed = entry.clone();
            displayed.id = EntryId(index as u64 + 100);
            TreeDisplayRow {
                id: TreeRowId(index as u64 + 10),
                entry: displayed,
                parent: None,
                depth: 0,
                disclosure: if index == 3 {
                    TreeDisclosure::Expanded
                } else {
                    TreeDisclosure::None
                },
                expanded: index == 3,
                selectable: true,
                navigable: index == 3,
                context_only: false,
                error: None,
                error_details: None,
                original_root_entry: Some(entry.id),
                prefix_end: 0,
                search_match: false,
            }
        })
        .collect();
    rows.push(TreeDisplayRow {
        id: TreeRowId(14),
        entry: Entry::new(
            EntryId(104),
            pane.path.join("folder/child"),
            EntryKind::Directory,
        ),
        parent: Some(TreeRowId(13)),
        depth: 1,
        disclosure: TreeDisclosure::Collapsed,
        expanded: false,
        selectable: true,
        navigable: true,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: None,
        prefix_end: 0,
        search_match: false,
    });
    let view = Arc::new(
        TreeProjection::new(
            root,
            TreeViewStamp {
                pane: id,
                navigation: NavigationId(pane.generation),
                session: TreeSessionId(1),
                revision: TreeRevision(1),
            },
            TreeLayoutRevision(1),
            rows,
        )
        .unwrap(),
    );
    pane.tree.publication = Some(
        TreePublication::new(
            view.clone(),
            SelectionRevision(0),
            Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![bits]).unwrap()),
        )
        .unwrap(),
    );
    pane.cursor = Some(EntryId(100));
    pane.parent_cursor = false;
    view
}

#[test]
fn adopted_tree_file_comparison_uses_exact_root_aliases_and_two_selected_order() {
    let mut presenter = loaded();
    let view = tree_files(&mut presenter, PaneId::Left, 3);
    // The old flat identities deliberately differ from the current backend root actions.
    let launch = presenter.file_compare_launch(PaneId::Left).unwrap();
    assert!(
        matches!(&launch.sources[..], [FileCompareSource::Focused(a), FileCompareSource::Focused(b)]
        if a == &view.root_actions.entries[0].path && b == &view.root_actions.entries[1].path)
    );
    let view = tree_files(&mut presenter, PaneId::Left, 0);
    let other = tree_files(&mut presenter, PaneId::Right, 0);
    let launch = presenter.file_compare_launch(PaneId::Left).unwrap();
    assert!(
        matches!(&launch.sources[0], FileCompareSource::Focused(path) if path == &view.root_actions.entries[0].path)
    );
    assert!(
        matches!(&launch.sources[1], FileCompareSource::RootSameName { entries, visible, .. }
        if Arc::ptr_eq(entries, &other.root_actions.entries) && Arc::ptr_eq(visible, &other.index.native_root_files))
    );
}

#[test]
fn adopted_tree_comparison_refuses_marked_folders_ambiguous_or_unsettled_selection() {
    let mut presenter = loaded();
    for bits in [8, 16, 17, 7] {
        tree_files(&mut presenter, PaneId::Left, bits);
        assert!(matches!(
            presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
            FileCompareSource::Unspecified
        ));
    }
    tree_files(&mut presenter, PaneId::Left, 1);
    presenter.panes[0].tree.revision = SelectionRevision(1);
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
        FileCompareSource::Unspecified
    ));
    presenter.panes[0].tree.resolved = SelectionRevision(1);
    presenter.panes[0].tree.preview.push((
        SelectionRevision(1),
        TreeSelectionAction::Toggle(TreeRowId(10)),
    ));
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
        FileCompareSource::Unspecified
    ));
}

#[test]
fn adopted_tree_comparison_refuses_errored_root_file_and_parent_focus() {
    let mut presenter = loaded();
    let old = tree_files(&mut presenter, PaneId::Left, 0);
    presenter.panes[0].parent_cursor = true;
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
        FileCompareSource::Unspecified
    ));
    presenter.panes[0].parent_cursor = false;
    let mut rows = old.rows.to_vec();
    rows[0].error = Some("unavailable".into());
    let view = Arc::new(
        TreeProjection::new(old.root.clone(), old.stamp, old.layout_revision, rows).unwrap(),
    );
    presenter.panes[0].tree.publication = Some(
        TreePublication::new(
            view.clone(),
            SelectionRevision(0),
            Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap()),
        )
        .unwrap(),
    );
    assert!(matches!(
        presenter.file_compare_launch(PaneId::Left).unwrap().sources[0],
        FileCompareSource::Unspecified
    ));
    assert!(!view.index.native_root_files.contains(&EntryId(0)));
}
