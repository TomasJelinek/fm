use super::*;

#[test]
fn watch_acknowledges_only_committed_snapshots_and_validates_hint_generation() {
    let mut p = Presenter::new("left".into(), "right".into());
    assert!(p.visible_directory(PaneId::Left).is_none());
    let generation = begin(&mut p, PaneId::Left, "left");
    assert!(p.visible_directory(PaneId::Left).is_none());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(7, "left/a"), entry(8, "left/b")],
    );
    select_row(&mut p, PaneId::Left, 1, true);
    let visible = p.visible_directory(PaneId::Left).unwrap();
    assert_eq!(visible.navigation, generation);
    assert_eq!(visible.path, PathBuf::from("left"));
    assert!(Arc::ptr_eq(&visible.entries, &p.panes[0].entries));
    assert!(
        !p.refresh_if_changed(PaneId::Left, NavigationId(generation.0 - 1), &|_| panic!(
            "stale hints must not send"
        ))
    );
    assert!(!p.refresh_if_changed(PaneId::Left, generation, &|_| false));
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(8)));
    assert!(Arc::ptr_eq(
        &visible.entries,
        &p.visible_directory(PaneId::Left).unwrap().entries
    ));
    let sent = RefCell::new(None);
    assert!(p.refresh_if_changed(PaneId::Left, generation, &|command| {
        *sent.borrow_mut() = Some(command);
        true
    }));
    assert!(
        matches!(sent.into_inner().unwrap(), Command::Refresh { pane: PaneId::Left, path, .. } if path == Path::new("left"))
    );
    assert!(p.visible_directory(PaneId::Left).is_none());
    assert!(!p.refresh_if_changed(PaneId::Left, generation, &|_| panic!(
        "pending refresh must not restart"
    )));
    assert_eq!(p.panes[0].rows().len(), 2);
    p.apply(Event::DirectoryChanged {
        pane: PaneId::Left,
        navigation: generation,
    });
    assert!(p.panes[0].loading);
}

#[test]
fn failed_same_path_refresh_keeps_interactions_and_restores_old_ack_generation() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(7, "left/a"), entry(8, "left/b")],
    );
    let visible = p.visible_directory(PaneId::Left).unwrap();
    let refresh = begin(&mut p, PaneId::Left, "left");
    select_row(&mut p, PaneId::Left, 1, true);
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: refresh,
        error: FsError::from_io(
            "list",
            Path::new("left"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        ),
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert!(selected_ids(&p, PaneId::Left).contains(&EntryId(8)));
    let restored = p.visible_directory(PaneId::Left).unwrap();
    assert_eq!(restored.navigation, generation);
    assert!(Arc::ptr_eq(&visible.entries, &restored.entries));
}

#[test]
fn leaving_a_staged_refresh_then_failing_restores_the_latest_committed_selection() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(7, "left/a"), entry(8, "left/b")],
    );
    begin(&mut p, PaneId::Left, "left");
    select_row(&mut p, PaneId::Left, 1, true);
    let other = begin(&mut p, PaneId::Left, "other");
    p.apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation: other,
        error: FsError::from_io(
            "list",
            Path::new("other"),
            io::Error::from(io::ErrorKind::NotFound),
        ),
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(8)));
    assert_eq!(selected_ids(&p, PaneId::Left), HashSet::from([EntryId(8)]));
    assert_eq!(
        p.visible_directory(PaneId::Left).unwrap().navigation,
        generation
    );
}
