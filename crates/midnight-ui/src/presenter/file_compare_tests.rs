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
