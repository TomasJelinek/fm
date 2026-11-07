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
fn captures_two_focused_files_or_active_selection_without_expanding_it() {
    let mut presenter = loaded();
    let launch = presenter.file_compare_launch(PaneId::Right).unwrap();
    assert!(
        matches!(&launch.sources[..], [FileCompareSource::Focused(left), FileCompareSource::Focused(right)] if left == Path::new("left/one") && right == Path::new("right/one"))
    );
    let pane = &mut presenter.panes[1];
    pane.selected = Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![3]).unwrap());
    let launch = presenter.file_compare_launch(PaneId::Right).unwrap();
    assert!(
        matches!(&launch.sources[..], [FileCompareSource::Selected { selection, .. }] if Arc::ptr_eq(selection, &presenter.panes[1].selected))
    );
    assert!(presenter.file_compare_launch(PaneId::Left).is_err());
}

#[test]
fn rejects_pending_and_ambiguous_selection() {
    let mut presenter = loaded();
    presenter.panes[0].selection_revision = SelectionRevision(1);
    assert!(presenter.file_compare_launch(PaneId::Left).is_err());
    presenter.panes[0].selection_applied = SelectionRevision(1);
    let pane = &mut presenter.panes[0];
    pane.selected = Arc::new(SelectionSnapshot::from_bits(pane.index.clone(), vec![7]).unwrap());
    assert!(presenter.file_compare_launch(PaneId::Left).is_err());
}
