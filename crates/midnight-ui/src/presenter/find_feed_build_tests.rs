use super::super::find::FindForm;
use super::*;
use std::cell::RefCell;
fn searched(p: &mut Presenter, pane: PaneId, name: &str) -> FindSessionId {
    let id = p.create_find(pane).unwrap();
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    let result = Arc::new(FindResult {
        id: FindResultId(1),
        entry: FindEntry {
            path: std::env::temp_dir().join("feed-initial-worker").join(name),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(3),
                ..Default::default()
            },
            identity: TransferIdentity {
                device: 1,
                object: [1; 16],
            },
            revision: FindRevision([0; 6]),
        },
        duplicate_group: None,
        duplicate_group_size: 0,
        archive: None,
    });
    for update in [
        FindUpdate::Batch(vec![result].into()),
        FindUpdate::Finished(FindSummary {
            completion: FindCompletion::Completed,
            progress: FindProgress::default(),
            reached_budgets: vec![],
            diagnostics: vec![],
        }),
    ] {
        p.apply(Event::Find(Box::new(FindEvent {
            session: id,
            generation: FindGeneration(1),
            update,
        })));
    }
    id
}
fn presenter() -> Presenter {
    Presenter::new(std::env::temp_dir(), std::env::temp_dir())
}
fn answer(p: &mut Presenter, request: &FindFeedBuildRequest) {
    p.apply(Event::FindFeedBuilt(Box::new(FindFeedBuildResult {
        request: request.stamp(),
        result: request.prepare(),
    })));
}
fn record(requests: &RefCell<Vec<Arc<FindFeedBuildRequest>>>, command: Command) -> bool {
    if let Command::BuildFindFeed(request) = command {
        requests.borrow_mut().push(request);
    }
    true
}
#[test]
fn busy_queue_and_close_admission_never_replace_the_existing_feed_early() {
    let mut p = presenter();
    let id = searched(&mut p, PaneId::Left, "old.txt");
    p.feed_find_results(id).unwrap();
    let old = p.feed(PaneId::Left).unwrap().entries.clone();
    let session = p.finds.sessions.get_mut(&id).unwrap();
    Arc::make_mut(&mut session.results[0]).entry.path =
        std::env::temp_dir().join("feed-initial-worker/new.txt");
    assert!(p.request_feed_find_results(id, &|_| false));
    assert!(p.finds.sessions[&id].feeding);
    assert!(Arc::ptr_eq(&p.feed(PaneId::Left).unwrap().entries, &old));
    let requests = RefCell::new(vec![]);
    p.flush_find_feed_builds(&|command| record(&requests, command));
    let request = requests.borrow_mut().pop().unwrap();
    answer(&mut p, &request);
    p.flush_find_feed_builds(&|command| !matches!(command, Command::CloseFind(_)));
    assert!(p.finds.sessions.contains_key(&id));
    assert!(Arc::ptr_eq(&p.feed(PaneId::Left).unwrap().entries, &old));
    p.flush_find_feed_builds(&|_| true);
    assert!(!p.finds.sessions.contains_key(&id));
    assert_eq!(
        p.feed(PaneId::Left).unwrap().entries[0].path,
        request.results[0].entry.path
    );
    assert_eq!(p.finds.feed_focus.get(), Some(PaneId::Left));
}
#[test]
fn newer_gesture_cancels_old_work_unfreezes_old_window_and_keeps_latest_results() {
    let mut p = presenter();
    let first = searched(&mut p, PaneId::Left, "first.txt");
    let second = searched(&mut p, PaneId::Left, "second.txt");
    let requests = RefCell::new(vec![]);
    p.request_feed_find_results(first, &|command| record(&requests, command));
    let old = requests.borrow_mut().pop().unwrap();
    p.request_feed_find_results(second, &|command| record(&requests, command));
    let latest = requests.borrow_mut().pop().unwrap();
    assert!(old.cancel.is_cancelled());
    assert!(!p.finds.sessions[&first].feeding);
    assert!(p.finds.sessions[&second].feeding);
    assert!(latest.id > old.id);
    answer(&mut p, &old);
    assert!(p.feed(PaneId::Left).is_none());
    answer(&mut p, &latest);
    p.flush_find_feed_builds(&|_| true);
    assert_eq!(
        p.feed(PaneId::Left).unwrap().entries[0].path,
        latest.results[0].entry.path
    );
    assert!(p.finds.sessions.contains_key(&first));
}
#[test]
fn closed_restarted_and_navigated_targets_ignore_worker_tails() {
    for change in 0..4 {
        let mut p = presenter();
        let id = searched(&mut p, PaneId::Left, "stale.txt");
        if change == 3 {
            p.feed_find_results(id).unwrap();
        }
        let requests = RefCell::new(vec![]);
        p.request_feed_find_results(id, &|command| record(&requests, command));
        let request = requests.borrow_mut().pop().unwrap();
        match change {
            0 => {
                assert!(p.close_find(id, &|_| true));
            }
            1 => {
                assert!(p.start_find(id, FindForm::default(), &|_| true));
            }
            2 => {
                assert!(p.navigate(
                    PaneId::Left,
                    std::env::temp_dir().join("moved-feed-pane"),
                    None,
                    &|_| true
                ));
            }
            _ => p.feed_action(
                PaneId::Left,
                super::super::find::feed::FeedAction::Leave,
                &|_| true,
            ),
        }
        assert!(request.cancel.is_cancelled());
        answer(&mut p, &request);
        p.flush_find_feed_builds(&|_| true);
        assert!(p.feed(PaneId::Left).is_none());
        assert!(p.finds.feed_focus.get().is_none());
        if let Some(session) = p.finds.sessions.get(&id) {
            assert!(!session.feeding);
        }
    }
}
#[test]
fn failed_preparation_unfreezes_source_and_retains_existing_panel_rows() {
    let mut p = presenter();
    let id = searched(&mut p, PaneId::Left, "keep.txt");
    p.feed_find_results(id).unwrap();
    let old = p.feed(PaneId::Left).unwrap().entries.clone();
    let requests = RefCell::new(vec![]);
    p.request_feed_find_results(id, &|command| record(&requests, command));
    let request = requests.borrow_mut().pop().unwrap();
    p.apply(Event::FindFeedBuilt(Box::new(FindFeedBuildResult {
        request: request.stamp(),
        result: Err(FsError::from_io(
            "worker",
            Path::new(""),
            std::io::ErrorKind::InvalidData.into(),
        )),
    })));
    p.flush_find_feed_builds(&|_| true);
    assert!(!p.finds.sessions[&id].feeding);
    assert!(!p.finds.sessions[&id].error.is_empty());
    assert!(Arc::ptr_eq(&p.feed(PaneId::Left).unwrap().entries, &old));
}

#[test]
fn shutdown_cancels_pending_and_ready_builds_and_retires_both_feeds_on_a_worker() {
    let mut p = presenter();
    let left = searched(&mut p, PaneId::Left, "left.txt");
    let right = searched(&mut p, PaneId::Right, "right.txt");
    p.feed_find_results(left).unwrap();
    p.feed_find_results(right).unwrap();
    let references_before_view = Arc::strong_count(&p.feed(PaneId::Left).unwrap().entries);
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    p.feed_action(
        PaneId::Left,
        super::super::find::feed::FeedAction::View(false),
        &|_| panic!("view capture submits no folder command"),
    );
    assert!(
        Arc::strong_count(&p.feed(PaneId::Left).unwrap().entries) > references_before_view,
        "viewer staging must retain its own feed rows"
    );
    let left_rows = Arc::downgrade(&p.feed(PaneId::Left).unwrap().entries);
    let right_rows = Arc::downgrade(&p.feed(PaneId::Right).unwrap().entries);
    let requests = RefCell::new(vec![]);
    p.request_feed_find_results(left, &|command| record(&requests, command));
    let pending = requests.borrow_mut().pop().unwrap();
    let pending_cancel = pending.cancel.clone();
    let pending_inputs = Arc::downgrade(&pending.results);
    p.request_feed_find_results(right, &|command| record(&requests, command));
    let ready = requests.borrow_mut().pop().unwrap();
    let ready_cancel = ready.cancel.clone();
    answer(&mut p, &ready);
    let ready_rows = Arc::downgrade(
        &p.finds.feed_builds[1]
            .as_ref()
            .unwrap()
            .ready
            .as_ref()
            .unwrap()
            .entries,
    );
    drop(pending);
    drop(ready);
    let earlier = p.take_retired();
    std::thread::spawn(move || drop(earlier)).join().unwrap();

    p.retire_all();
    assert!(pending_cancel.is_cancelled());
    assert!(ready_cancel.is_cancelled());
    assert!(p.finds.feed_builds.iter().all(Option::is_none));
    assert!(p.finds.feeds.iter().all(Option::is_none));
    assert!(p.finds.sessions.is_empty());
    assert!(
        p.take_find_viewer().is_none(),
        "pending viewer was detached for reclamation"
    );
    assert!(
        pending_inputs.upgrade().is_some(),
        "pending input belongs to deferred reclamation"
    );
    assert!(
        ready_rows.upgrade().is_some(),
        "prepared output belongs to deferred reclamation"
    );
    assert!(left_rows.upgrade().is_some());
    assert!(right_rows.upgrade().is_some());

    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(pending_inputs.upgrade().is_none());
    assert!(ready_rows.upgrade().is_none());
    assert!(left_rows.upgrade().is_none());
    assert!(right_rows.upgrade().is_none());
}
