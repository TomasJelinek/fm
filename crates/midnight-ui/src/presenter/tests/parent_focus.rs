use super::*;

fn child_view() -> (Presenter, PathBuf, PathBuf) {
    let parent = test_root().join("parent");
    let child = parent.join("Žluťoučký child. ");
    let mut p = Presenter::new(child.clone(), test_root());
    let nav = begin(&mut p, PaneId::Left, child.to_str().unwrap());
    finish(&mut p, PaneId::Left, nav, vec![]);
    (p, parent, child)
}

fn parent_entries(parent: &Path, child: &Path) -> Vec<Entry> {
    vec![
        Entry::new(EntryId(10), parent.join("a"), EntryKind::Directory),
        Entry::new(EntryId(20), child.to_owned(), EntryKind::Directory),
    ]
}

#[test]
fn parent_focus_returns_to_raw_child_after_streaming_and_new_sorted_ids() {
    let (mut p, parent, child) = child_view();
    assert!(p.up(PaneId::Left, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![parent_entries(&parent, &child).remove(0)],
    });
    assert_eq!(
        p.panes[0].display_len(),
        1,
        "ancestor navigation still streams"
    );
    assert_eq!(
        p.panes[0].cursor, None,
        "do not focus an unrelated early row"
    );
    assert!(p.panes[0].parent_cursor);
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: vec![Entry::new(EntryId(99), child.clone(), EntryKind::Directory)],
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(99)));
    assert!(!p.panes[0].parent_cursor);
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        parent_entries(&parent, &child),
    );
    assert_eq!(p.panes[0].cursor_index(), Some(2));
    assert_eq!(p.panes[0].cursor, Some(EntryId(20)));
    assert!(p.panes[0].selected.is_empty());
    assert!(!p.panes[0].preserve_viewport);
}

#[test]
fn parent_focus_manual_cursor_wins_and_missing_child_is_not_fabricated() {
    let (mut p, parent, child) = child_view();
    assert!(p.up(PaneId::Left, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: parent_entries(&parent, &child),
    });
    p.focus_display_row(PaneId::Left, 1);
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        parent_entries(&parent, &child),
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(10)));

    let (mut p, parent, _) = child_view();
    assert!(p.up(PaneId::Left, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![Entry::new(
            EntryId(10),
            parent.join("a"),
            EntryKind::Directory,
        )],
    );
    assert_eq!(p.panes[0].rows().len(), 1);
    assert_eq!(p.panes[0].cursor_index(), Some(0));
}

#[test]
fn parent_focus_ancestor_breadcrumb_chooses_immediate_child_and_rejects_stale_result() {
    let (mut p, parent, _) = child_view();
    let root = test_root();
    let token = p
        .path_parts(PaneId::Left)
        .into_iter()
        .find(|part| part.prefix == root.to_string_lossy())
        .unwrap()
        .identity;
    assert!(p.navigate_path_part(PaneId::Left, &token, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: NavigationId(navigation.0 - 1),
        entries: vec![Entry::new(
            EntryId(55),
            parent.clone(),
            EntryKind::Directory,
        )],
    });
    assert!(p.panes[0].streaming.is_empty());
    finish(&mut p, PaneId::Left, NavigationId(navigation.0 - 1), vec![]);
    assert!(p.panes[0].loading);
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        vec![
            Entry::new(EntryId(66), root.join("a"), EntryKind::Directory),
            Entry::new(EntryId(55), parent, EntryKind::Directory),
        ],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(55)));
    assert!(!p.panes[0].parent_cursor);
}

#[test]
fn parent_focus_deliberate_parent_row_focus_wins_over_late_child() {
    let (mut p, parent, child) = child_view();
    assert!(p.up(PaneId::Left, &|_| true));
    let navigation = NavigationId(p.panes[0].generation);
    p.focus_display_row(PaneId::Left, 0);
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation,
        entries: parent_entries(&parent, &child),
    });
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        parent_entries(&parent, &child),
    );
    assert_eq!(p.panes[0].cursor_index(), Some(0));
}

#[test]
fn parent_focus_persistent_filter_preserves_only_a_visible_child() {
    for visible in [false, true] {
        let (mut p, parent, child) = child_view();
        p.preferences.values.panel_filters[0].enabled = true;
        assert!(p.up(PaneId::Left, &|_| true));
        let navigation = NavigationId(p.panes[0].generation);
        finish(
            &mut p,
            PaneId::Left,
            navigation,
            parent_entries(&parent, &child),
        );
        let pane = &p.panes[0];
        let row = usize::from(visible);
        let source = pane.entries.clone();
        let projection = Arc::new(DirectoryIndex::from_entries(&[source[row].clone()]));
        let matches: Arc<[SearchMatch]> = vec![SearchMatch {
            source_row: row,
            display_prefix_end: 0,
        }]
        .into();
        p.apply(Event::SearchReady {
            pane: PaneId::Left,
            navigation,
            revision: pane.search.revision,
            result: Arc::new(SearchResult {
                source,
                matches: matches.clone(),
                filtered: matches,
                filtered_index: projection.clone(),
                index: projection,
            }),
        });
        assert_eq!(p.panes[0].display_len(), 1);
        assert_eq!(
            p.panes[0].cursor,
            Some(EntryId(if visible { 20 } else { 10 }))
        );
        assert_eq!(p.panes[0].cursor_index(), Some(1));
    }
}
