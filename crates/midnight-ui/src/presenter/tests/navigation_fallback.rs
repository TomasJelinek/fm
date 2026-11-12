use super::*;

fn fallback(pane: PaneId, navigation: NavigationId, failed: &Path, path: PathBuf) -> Event {
    Event::ListingFallback {
        pane,
        navigation,
        path,
        error: FsError::from_io("list", failed, io::ErrorKind::NotFound.into()),
    }
}

#[test]
fn recovered_navigation_commits_actual_path_history_tab_and_fresh_generation() {
    let root = test_root();
    let previous = root.join("previous");
    let recovered = root.join("reachable");
    let missing = recovered.join("missing");
    let mut p = Presenter::new(previous.clone(), root.clone());
    let first = begin(&mut p, PaneId::Left, previous.to_str().unwrap());
    finish(&mut p, PaneId::Left, first, vec![entry(1, "old")]);
    let failed = begin(&mut p, PaneId::Left, missing.to_str().unwrap());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        fallback(PaneId::Left, failed, &missing, recovered.clone()),
        &|c| {
            sent.borrow_mut().push(c);
            true
        },
    );
    let next = sent
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::Navigate {
                navigation, path, ..
            } if path == &recovered => Some(*navigation),
            _ => None,
        })
        .expect("the parent is retried automatically");
    assert!(next > failed);
    assert_eq!(
        p.panes[0].path, previous,
        "recovery is provisional until listed"
    );
    finish(&mut p, PaneId::Left, next, vec![entry(2, "new")]);
    assert_eq!(p.panes[0].path, recovered);
    assert_eq!(p.panes[0].history, [previous, recovered.clone()]);
    assert_eq!(p.tabs(PaneId::Left)[0].path, recovered);
    assert_eq!(p.preferences.values.tabs[0].paths, [recovered]);
    assert!(
        p.panes[0].parent_cursor,
        "recovery starts with the parent row focused"
    );
    assert_eq!(p.panes[0].cursor, None);
    assert!(p.panes[0].error.is_empty());
}

#[test]
fn stale_fallback_cannot_replace_a_later_navigation_or_completed_directory() {
    let root = test_root();
    let missing = root.join("missing");
    let later = root.join("later");
    let mut p = Presenter::new(root.clone(), root.clone());
    let failed = begin(&mut p, PaneId::Left, missing.to_str().unwrap());
    let current = begin(&mut p, PaneId::Left, later.to_str().unwrap());
    p.apply_with_sender(
        fallback(PaneId::Left, failed, &missing, root.clone()),
        &|_| panic!("an obsolete fallback cannot enqueue navigation"),
    );
    finish(&mut p, PaneId::Left, current, vec![]);
    p.apply_with_sender(fallback(PaneId::Left, current, &later, root), &|_| {
        panic!("a completed request cannot be restarted by a late fallback")
    });
    assert_eq!(p.panes[0].path, later);
}

#[test]
fn rejected_fallback_retains_last_directory_and_reports_failure() {
    let root = test_root();
    let previous = root.join("previous");
    let missing = root.join("missing/child");
    let mut p = Presenter::new(previous.clone(), root.clone());
    let first = begin(&mut p, PaneId::Left, previous.to_str().unwrap());
    finish(&mut p, PaneId::Left, first, vec![entry(1, "old")]);
    p.panes[0].cursor = Some(EntryId(1));
    p.panes[0].parent_cursor = false;
    let failed = begin(&mut p, PaneId::Left, missing.to_str().unwrap());
    p.apply_with_sender(fallback(PaneId::Left, failed, &missing, root), &|_| false);
    assert_eq!(p.panes[0].path, previous);
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
    assert!(!p.panes[0].loading);
    assert!(!p.panes[0].error.is_empty());
}

#[test]
fn automatic_recovery_has_a_finite_budget_even_if_a_backend_repeats_targets() {
    let root = test_root();
    let alternate = root.join("alternate");
    let mut p = Presenter::new(root.clone(), root.clone());
    let first = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    p.apply_with_sender(
        fallback(PaneId::Left, first, &root, alternate.clone()),
        &|_| true,
    );
    let next = NavigationId(p.panes[0].generation);
    assert!(next > first);
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        fallback(PaneId::Left, next, &alternate, root.clone()),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(
        !sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::Navigate { .. } | Command::Refresh { .. }))
    );
    assert_eq!(p.panes[0].generation, next.0);
    assert!(!p.panes[0].loading);
    assert!(!p.panes[0].error.is_empty());
}

#[test]
fn recovery_discards_partial_entries_and_late_storage_for_the_failed_path() {
    let root = test_root();
    let missing = root.join("missing");
    let mut p = Presenter::new(root.clone(), root.clone());
    let first = begin(&mut p, PaneId::Left, missing.to_str().unwrap());
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: first,
        entries: vec![entry(77, "partial")],
    });
    p.apply_with_sender(
        fallback(PaneId::Left, first, &missing, root.clone()),
        &|_| true,
    );
    let next = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, next, vec![entry(1, "real")]);
    p.apply(Event::SpaceUpdated {
        pane: PaneId::Left,
        navigation: first,
        space: SpaceInfo {
            available: 1,
            total: 2,
        },
    });
    finish(&mut p, PaneId::Left, first, vec![entry(77, "late")]);
    assert_eq!(p.panes[0].path, root);
    assert_eq!(p.panes[0].entries[0].id, EntryId(1));
    assert!(p.panes[0].space.is_none());
}

#[test]
fn failed_back_destination_commits_its_recovered_parent_as_a_new_history_entry() {
    let root = test_root();
    let recovered = root.join("old");
    let deleted = recovered.join("deleted");
    let current = root.join("current");
    let mut p = Presenter::new(deleted.clone(), root);
    for path in [&deleted, &current] {
        let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
        finish(&mut p, PaneId::Left, navigation, vec![]);
    }
    assert!(p.history(PaneId::Left, -1, &|_| true));
    let failed = NavigationId(p.panes[0].generation);
    p.apply_with_sender(
        fallback(PaneId::Left, failed, &deleted, recovered.clone()),
        &|_| true,
    );
    let next = NavigationId(p.panes[0].generation);
    finish(&mut p, PaneId::Left, next, vec![]);
    assert_eq!(p.panes[0].path, recovered);
    assert_eq!(p.panes[0].history, [deleted, current, recovered]);
    assert_eq!(p.panes[0].history_index, Some(2));
}
