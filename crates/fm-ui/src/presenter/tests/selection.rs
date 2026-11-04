use super::*;

#[test]
fn sorting_preserves_cursor_and_selection_by_identity() {
    let mut p = Presenter::new("left".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![entry(1, "a"), entry(2, "b")],
    );
    p.panes[0].cursor = Some(EntryId(2));
    set_selected(&mut p, PaneId::Left, &[EntryId(1)]);
    let sorted = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        sorted,
        vec![entry(2, "b"), entry(1, "a")],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(1)));
}
#[test]
fn keyboard_selection_and_movement_keep_identity_and_clamp_at_boundaries() {
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![entry(1, "a"), entry(2, "b"), entry(3, "c")],
    );
    move_cursor(&mut p, PaneId::Left, 9);
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(1)));
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    move_cursor(&mut p, PaneId::Left, 5);
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)));
    move_cursor(&mut p, PaneId::Left, 6);
    move_cursor(&mut p, PaneId::Left, 8);
    assert!(p.panes[0].selected.is_empty());
    move_cursor(&mut p, PaneId::Left, 2);
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
}
#[test]
fn refreshed_identities_preserve_selection_by_raw_path() {
    let mut p = Presenter::new("a".into(), "right".into());
    let first = begin(&mut p, PaneId::Left, "a");
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![entry(1, "a"), entry(2, "b")],
    );
    p.panes[0].cursor = Some(EntryId(2));
    set_selected(&mut p, PaneId::Left, &[EntryId(1)]);
    let second = begin(&mut p, PaneId::Left, "a");
    finish(
        &mut p,
        PaneId::Left,
        second,
        vec![entry(50, "b"), entry(51, "a")],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(50)));
    assert_eq!(selected_ids(&p, PaneId::Left), HashSet::from([EntryId(51)]));
}
#[test]
fn streaming_cursor_and_selection_survive_the_final_sorted_snapshot() {
    let mut p = Presenter::new("a".into(), "right".into());
    let id = begin(&mut p, PaneId::Left, "a");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: id,
        entries: vec![entry(1, "a"), entry(2, "b")],
    });
    p.panes[0].cursor = Some(EntryId(2));
    set_selected(&mut p, PaneId::Left, &[EntryId(1)]);
    finish(&mut p, PaneId::Left, id, vec![entry(2, "b"), entry(1, "a")]);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(1)));
}
#[test]
fn new_location_streams_immediately_and_failure_restores_old_row_state() {
    let mut p = Presenter::new("a".into(), "right".into());
    let a = begin(&mut p, PaneId::Left, "a");
    finish(&mut p, PaneId::Left, a, vec![entry(1, "a/old")]);
    set_selected(&mut p, PaneId::Left, &[EntryId(1)]);
    p.panes[0].cursor = Some(EntryId(1));
    let b = begin(&mut p, PaneId::Left, "b");
    assert_eq!(p.panes[0].display_path(), std::path::Path::new("b"));
    assert!(p.panes[0].rows().is_empty());
    assert!(p.panes[0].selected.is_empty());
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: b,
        entries: vec![entry(1, "b/new")],
    });
    assert_eq!(p.panes[0].rows()[0].path, std::path::Path::new("b/new"));
    assert!(p.panes[0].selected.is_empty());
    set_selected(&mut p, PaneId::Left, &[EntryId(1)]);
    assert!(!p.open(PaneId::Left, 0, &|c| {
        assert!(
            matches!(c, Command::Navigate { path, .. } if path == std::path::Path::new("b/new"))
        );
        false
    }));
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: b,
        error: FsError::from_io(
            "list",
            std::path::Path::new("b"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
    });
    assert_eq!(p.panes[0].display_path(), std::path::Path::new("a"));
    assert_eq!(p.panes[0].rows()[0].path, std::path::Path::new("a/old"));
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(1)));
    assert_eq!(p.panes[0].history, [PathBuf::from("a")]);
}
#[test]
fn progressive_selection_uses_the_new_generation_and_survives_final_sort() {
    let mut p = Presenter::new("a".into(), "right".into());
    let a = begin(&mut p, PaneId::Left, "a");
    finish(&mut p, PaneId::Left, a, vec![entry(1, "old")]);
    set_selected(&mut p, PaneId::Left, &[EntryId(1)]);
    let b = begin(&mut p, PaneId::Left, "b");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: b,
        entries: vec![entry(1, "new-a"), entry(2, "new-b")],
    });
    assert!(p.panes[0].selected.is_empty());
    set_selected(&mut p, PaneId::Left, &[EntryId(2)]);
    p.panes[0].cursor = Some(EntryId(2));
    finish(
        &mut p,
        PaneId::Left,
        b,
        vec![entry(2, "new-b"), entry(1, "new-a")],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert_eq!(selected_ids(&p, PaneId::Left), HashSet::from([EntryId(2)]));
}
