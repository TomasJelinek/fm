use super::*;

fn loaded(count: usize) -> Presenter {
    let root = test_root().join("selected-navigation");
    let mut p = Presenter::new(root.clone(), root.clone());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        (0..count)
            .map(|row| {
                Entry::new(
                    EntryId(row as u64 + 1),
                    root.join(format!("{row} 文档")),
                    if row % 2 == 0 {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                )
            })
            .collect(),
    );
    p
}

fn select_positions(p: &mut Presenter, positions: &[usize]) {
    let mut bits = vec![0; p.panes[0].entries.len().div_ceil(64)];
    for position in positions {
        bits[position / 64] |= 1 << (position % 64);
    }
    p.panes[0].selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), bits).unwrap());
}

#[test]
fn selected_navigation_skips_current_includes_folders_preserves_marks_and_never_wraps() {
    let mut p = loaded(8);
    select_positions(&mut p, &[0, 3, 6]);
    let selection = p.panes[0].selected.clone();
    let offset = p.panes[0].parent_offset();
    p.focus_display_row(PaneId::Left, offset);
    for (key, expected) in [(14, 4), (14, 7), (14, 7), (13, 4), (13, 1), (13, 1)] {
        p.move_cursor(PaneId::Left, key, &|_| {
            panic!("focus must not submit selection or I/O")
        });
        assert_eq!(p.panes[0].cursor, Some(EntryId(expected)));
        assert!(Arc::ptr_eq(&selection, &p.panes[0].selected));
    }
    p.focus_display_row(PaneId::Left, 0);
    p.move_cursor(PaneId::Left, 13, &|_| false);
    assert!(p.panes[0].parent_cursor);
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
    assert!(!p.panes[0].parent_cursor);
    p.active = PaneId::Right;
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(1)),
        "inactive panel is unchanged"
    );
}

#[test]
fn selected_navigation_sparse_far_selection_uses_display_positions_not_entry_ids() {
    let mut p = loaded(100_003);
    select_positions(&mut p, &[1, 100_001]);
    let offset = p.panes[0].parent_offset();
    p.focus_display_row(PaneId::Left, 1 + offset);
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(100_002)));
    p.move_cursor(PaneId::Left, 13, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
}

#[test]
fn selected_navigation_honors_pending_marks_and_rejects_loading_or_filename_search() {
    let mut p = loaded(8);
    select_positions(&mut p, &[1, 5]);
    let path = p.panes[0].entries[3].path.clone();
    assert!(p.send_selection(PaneId::Left, SelectionAction::Toggle(path), &|_| true));
    let offset = p.panes[0].parent_offset();
    p.focus_display_row(PaneId::Left, 1 + offset);
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(4)),
        "unacknowledged visible mark participates"
    );
    p.panes[0].search.query = "x".into();
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(4)));
    p.panes[0].search.query.clear();
    p.panes[0].loading = true;
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(4)));
}

#[test]
fn selected_navigation_resolves_reordered_and_filtered_selection_basis_by_raw_identity() {
    let mut p = loaded(8);
    let source = p.panes[0].entries.clone();
    let mut reversed = source.to_vec();
    reversed.reverse();
    let basis = Arc::new(DirectoryIndex::from_entries(&reversed));
    p.panes[0].selected = Arc::new(SelectionSnapshot::from_bits(basis, vec![0b1010_0010]).unwrap());
    // Original positions 0, 2 and 6 are selected. Hide position 2 in the displayed filter.
    let rows = [0, 1, 3, 6, 7];
    let projected: Vec<_> = rows.iter().map(|row| source[*row].clone()).collect();
    p.panes[0].search.panel_filter.enabled = true;
    p.panes[0].search.result = Some(Arc::new(SearchResult {
        source,
        filtered: rows
            .iter()
            .map(|row| SearchMatch {
                source_row: *row,
                display_prefix_end: 0,
            })
            .collect::<Vec<_>>()
            .into(),
        filtered_index: Arc::new(DirectoryIndex::from_entries(&projected)),
        matches: Arc::from([]),
        index: Arc::new(DirectoryIndex::default()),
    }));
    p.focus_display_row(PaneId::Left, p.panes[0].parent_offset());
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
    p.move_cursor(PaneId::Left, 13, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
}

#[test]
fn selected_navigation_inline_tree_uses_visible_occurrences_and_pending_marks() {
    let (mut p, root) = tree::fixture();
    let publication = tree::publication(root, 1, true, true);
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    let offset = p.panes[0].parent_offset();
    p.focus_display_row(PaneId::Left, offset + 1);
    p.move_cursor(PaneId::Left, 13, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
    p.select_row_modified(PaneId::Left, offset + 1, true, false, &|_| true);
    p.focus_display_row(PaneId::Left, offset);
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(11)),
        "nested marked occurrence participates"
    );
    p.move_cursor(PaneId::Left, 14, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(11)));
    p.move_cursor(PaneId::Left, 13, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(EntryId(7)));
}

#[test]
fn selected_navigation_archive_uses_opaque_ids_instead_of_duplicate_display_names() {
    let mut p = archives::archive_pane();
    let folder = p.panes[0].entries[0].id;
    let last = p.panes[0].entries[2].id;
    p.panes[0].archive.as_mut().unwrap().selected = Arc::new(HashSet::from([folder, last]));
    let offset = p.panes[0].parent_offset();
    p.focus_display_row(PaneId::Left, offset);
    p.move_cursor(PaneId::Left, 14, &|_| panic!("no provider call for focus"));
    assert_eq!(p.panes[0].cursor, Some(last));
    p.move_cursor(PaneId::Left, 13, &|_| false);
    assert_eq!(p.panes[0].cursor, Some(folder));
}
