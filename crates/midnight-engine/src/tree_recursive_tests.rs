use super::*;
use midnight_core::TreeEvent;

fn row_named(session: &Session, name: &str) -> TreeRowId {
    session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.display_name == name)
        .unwrap()
        .id
}

fn expand_recursive(fixture: &mut Fixture, name: &str, request: u64) {
    let view = fixture.session.projection().clone();
    assert!(fixture.session.commit(&view));
    let row = row_named(&fixture.session, name);
    fixture
        .session
        .branch(
            &view,
            TreeRequestId(request),
            row,
            BranchAction::ExpandRecursive,
        )
        .unwrap();
}

fn drain(fixture: &mut Fixture, limit: usize) -> usize {
    let mut completed = 0;
    loop {
        let view = fixture.session.projection().clone();
        assert!(fixture.session.commit(&view));
        assert!(fixture.session.queue.len() <= QUEUE_CAPACITY);
        let Some(job) = fixture.session.take_work().unwrap() else {
            break;
        };
        assert!(
            fixture.session.take_work().unwrap().is_none(),
            "one branch permit per pane"
        );
        complete_recursive(fixture, job, false);
        completed += 1;
        assert!(
            completed <= limit,
            "recursive work must not spin or revisit branches"
        );
    }
    completed
}

fn complete_recursive(fixture: &mut Fixture, job: BranchJob, elapsed: bool) -> bool {
    assert!(fixture.session.set_recursive_batch_elapsed(elapsed));
    fixture
        .session
        .apply(job.run(), &Cancellation::default())
        .unwrap()
}

fn wide_fixture() -> Fixture {
    let fixture = fixture();
    fixture.graph.rows.lock().unwrap().insert(
        1,
        (10..266)
            .map(|value| directory(value, &format!("child-{value}")))
            .collect(),
    );
    fixture
}

fn stage_unpublished(fixture: &mut Fixture, count: usize) {
    let published = fixture.session.projection().clone();
    for _ in 0..count {
        let job = fixture.session.take_work().unwrap().unwrap();
        assert!(!complete_recursive(fixture, job, false));
        assert!(Arc::ptr_eq(fixture.session.projection(), &published));
    }
}

fn one_batch(fixture: &mut Fixture) -> usize {
    let original = fixture.session.projection().clone();
    let mut results = 0;
    while Arc::ptr_eq(fixture.session.projection(), &original) {
        let Some(job) = fixture.session.take_work().unwrap() else {
            break;
        };
        complete_recursive(fixture, job, false);
        results += 1;
        assert!(
            results <= 128,
            "a batch must yield after at most 128 returned results"
        );
    }
    results
}

#[test]
fn recursive_expansion_walks_backend_children_depth_first_without_descendant_commands() {
    let mut fixture = fixture();
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![directory(4, "child"), directory(7, "keep")]);
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(4, vec![directory(6, "grandchild")]);
    expand_recursive(&mut fixture, "z-parent", 1);
    assert!(
        fixture.session.queue.is_empty(),
        "continuation is not a command per descendant"
    );
    assert_eq!(drain(&mut fixture, 8), 4);
    assert_eq!(*fixture.graph.listed.lock().unwrap(), [0, 1, 4, 6, 7]);
    for row in &*fixture.session.projection().rows {
        if ["z-parent", "child"].contains(&row.entry.display_name.as_str()) {
            assert!(row.expanded);
        }
    }
}

#[test]
fn ctrl_on_an_already_expanded_folder_still_expands_descendants() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let root = row_named(&fixture.session, "z-parent");
    fixture
        .session
        .branch(&view, TreeRequestId(1), root, BranchAction::Expand)
        .unwrap();
    let job = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(job.run(), &Cancellation::default())
        .unwrap();
    assert!(
        fixture
            .session
            .projection()
            .rows
            .iter()
            .find(|row| row.id == root)
            .unwrap()
            .expanded
    );
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(4, vec![directory(6, "grandchild")]);
    expand_recursive(&mut fixture, "z-parent", 2);
    drain(&mut fixture, 6);
    assert!(
        fixture
            .session
            .projection()
            .rows
            .iter()
            .any(|row| row.entry.display_name == "grandchild")
    );
}

#[test]
fn recursive_expansion_waits_for_exact_current_commit_between_batches() {
    let mut fixture = fixture();
    expand_recursive(&mut fixture, "z-parent", 1);
    let job = fixture.session.take_work().unwrap().unwrap();
    complete_recursive(&mut fixture, job, true);
    assert!(fixture.session.take_work().unwrap().is_none());
    assert_eq!(
        fixture.session.queued(),
        0,
        "waiting for Commit must not busy-wait on a free semaphore"
    );
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    assert!(fixture.session.take_work().unwrap().is_some());
}

#[test]
fn the_real_fifty_ms_guard_yields_after_an_already_elapsed_deadline_without_sleeping() {
    let mut fixture = wide_fixture();
    let published = fixture.session.projection().clone();
    expand_recursive(&mut fixture, "z-parent", 1);
    stage_unpublished(&mut fixture, 32);
    let calls = fixture.graph.lists.load(Ordering::Relaxed);
    assert_eq!(calls, 33, "root listing plus 32 returned recursive calls");
    assert_eq!(fixture.session.budget.branch.available_permits(), 1);
    assert_eq!(fixture.session.budget.listing.available_permits(), 2);
    let job = fixture.session.take_work().unwrap().unwrap();
    assert!(fixture.session.expire_recursive_batch_deadline());
    assert!(Arc::ptr_eq(fixture.session.projection(), &published));
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), calls);
    assert_eq!(fixture.session.budget.branch.available_permits(), 0);
    assert_eq!(fixture.session.budget.listing.available_permits(), 1);
    assert!(fixture.session.take_work().unwrap().is_none());
    let completion = job.run();
    assert!(Arc::ptr_eq(fixture.session.projection(), &published));
    assert_eq!(fixture.session.budget.branch.available_permits(), 0);
    assert!(
        fixture
            .session
            .apply(completion, &Cancellation::default())
            .unwrap()
    );
    assert!(!Arc::ptr_eq(fixture.session.projection(), &published));
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), calls + 1);
    assert_eq!(fixture.session.budget.branch.available_permits(), 1);
    assert_eq!(fixture.session.budget.listing.available_permits(), 2);
    assert!(
        fixture.session.take_work().unwrap().is_none(),
        "the next batch still requires Commit"
    );
}

#[test]
fn recursive_results_share_one_checkpoint_and_projection_until_the_batch_finishes() {
    let mut fixture = fixture();
    let published = fixture.session.projection().clone();
    expand_recursive(&mut fixture, "z-parent", 1);
    let first = fixture.session.take_work().unwrap().unwrap();
    assert!(!complete_recursive(&mut fixture, first, false));
    assert!(
        Arc::ptr_eq(fixture.session.projection(), &published),
        "a folder result is staged without a Loading or completion projection"
    );
    let second = fixture.session.take_work().unwrap().unwrap();
    assert!(complete_recursive(&mut fixture, second, false));
    assert_eq!(
        fixture.session.projection().stamp.revision.0,
        published.stamp.revision.0 + 1
    );
    assert!(
        fixture
            .session
            .projection()
            .rows
            .iter()
            .all(|row| row.disclosure != midnight_core::TreeDisclosure::Loading)
    );
}

#[test]
fn a_batch_yields_after_128_results_and_requires_exact_current_commit_for_the_next_batch() {
    let mut fixture = wide_fixture();
    let previous = fixture.session.projection().clone();
    expand_recursive(&mut fixture, "z-parent", 1);
    let count = one_batch(&mut fixture);
    assert_eq!(count, 128, "257 callbacks cannot finish in the first batch");
    let calls = fixture.graph.lists.load(Ordering::Relaxed);
    assert_eq!(calls, 129, "root listing plus the capped batch");
    let published = fixture.session.projection().clone();
    assert!(!Arc::ptr_eq(&published, &previous));
    assert!(fixture.session.take_work().unwrap().is_none());
    assert!(!fixture.session.commit(&previous));
    assert!(fixture.session.take_work().unwrap().is_none());
    let copy = Arc::new((*published).clone());
    assert_eq!(copy.stamp, published.stamp);
    assert!(!fixture.session.commit(&copy));
    assert!(fixture.session.take_work().unwrap().is_none());
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), calls);
    assert!(fixture.session.commit(&published));
    let next = fixture.session.take_work().unwrap().unwrap();
    assert!(next.recursive());
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), calls);
    assert!(!complete_recursive(&mut fixture, next, false));
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), calls + 1);
}

#[test]
fn explicit_input_rolls_back_unpublished_occurrences_without_reusing_their_ids() {
    let mut fixture = wide_fixture();
    let published = fixture.session.projection().clone();
    let initial_rows = fixture.session.tree.rows().len();
    expand_recursive(&mut fixture, "z-parent", 1);
    stage_unpublished(&mut fixture, 32);
    let abandoned = fixture
        .session
        .tree
        .rows()
        .into_iter()
        .find(|row| row.entry.display_name == "child-10")
        .unwrap()
        .id;
    let delayed = fixture.session.take_work().unwrap().unwrap();
    let abandoned_work = delayed.pending.version;
    let next_work = fixture.session.next_work;
    let root = row_named(&fixture.session, "z-parent");
    fixture
        .session
        .branch(&published, TreeRequestId(2), root, BranchAction::Collapse)
        .unwrap();
    assert!(!fixture.session.recursive_batch_active());
    assert!(Arc::ptr_eq(fixture.session.projection(), &published));
    assert_eq!(fixture.session.tree.rows().len(), initial_rows);
    assert_eq!(fixture.session.next_work, next_work);
    assert_eq!(fixture.session.budget.branch.available_permits(), 0);
    assert_eq!(fixture.session.budget.listing.available_permits(), 1);
    assert!(fixture.session.take_work().unwrap().is_none());
    let target = fixture
        .session
        .capture_target(
            &published,
            row_named(&fixture.session, "z-parent"),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(
        matches!(target.target.value().unwrap(), ProviderTarget::Native(path) if path == &fixture.graph.root_path.join("z-parent"))
    );
    let completion = delayed.run();
    assert_eq!(fixture.session.budget.branch.available_permits(), 0);
    assert!(
        !fixture
            .session
            .apply(completion, &Cancellation::default())
            .unwrap(),
        "a canceled unpublished callback cannot reopen rolled-back occurrences"
    );
    assert!(Arc::ptr_eq(fixture.session.projection(), &published));
    assert_eq!(fixture.session.tree.rows().len(), initial_rows);
    assert_eq!(fixture.session.next_work, next_work);
    assert_eq!(fixture.session.budget.branch.available_permits(), 1);
    assert_eq!(fixture.session.budget.listing.available_permits(), 2);
    expand_recursive(&mut fixture, "z-parent", 3);
    let retry = fixture.session.take_work().unwrap().unwrap();
    assert!(retry.pending.version > abandoned_work);
    complete_recursive(&mut fixture, retry, false);
    let replacement = fixture
        .session
        .tree
        .rows()
        .into_iter()
        .find(|row| row.entry.display_name == "child-10")
        .unwrap()
        .id;
    assert_ne!(
        replacement, abandoned,
        "unpublished branch IDs remain consumed after rollback"
    );
}

#[test]
fn reordered_inserted_and_removed_children_pause_the_occurrence_cursor_after_refresh() {
    for change in 0..3 {
        let mut fixture = fixture();
        let original = (10..266)
            .map(|value| directory(value, &format!("child-{value}")))
            .collect::<Vec<_>>();
        fixture
            .graph
            .rows
            .lock()
            .unwrap()
            .insert(1, original.clone());
        expand_recursive(&mut fixture, "z-parent", 1);
        assert!(one_batch(&mut fixture) >= 1);
        let mut changed = original;
        match change {
            0 => changed.reverse(),
            1 => changed.insert(0, directory(1000, "inserted")),
            _ => {
                changed.remove(0);
            }
        }
        fixture.graph.rows.lock().unwrap().insert(1, changed);
        let view = fixture.session.projection().clone();
        fixture.session.commit(&view);
        let root = row_named(&fixture.session, "z-parent");
        fixture
            .session
            .branch(&view, TreeRequestId(2), root, BranchAction::Refresh)
            .unwrap();
        let refresh = fixture.session.take_work().unwrap().unwrap();
        fixture
            .session
            .apply(refresh.run(), &Cancellation::default())
            .unwrap();
        let refreshed = fixture.session.projection().clone();
        fixture.session.commit(&refreshed);
        let calls = fixture.graph.lists.load(Ordering::Relaxed);
        assert!(fixture.session.take_work().unwrap().is_none());
        assert_eq!(
            fixture.graph.lists.load(Ordering::Relaxed),
            calls,
            "stale child cursor must not request another occurrence"
        );
        let TreeEvent::Rejected { error, .. } = fixture.session.take_recursive_notice().unwrap()
        else {
            panic!("changed branch notice");
        };
        assert!(error.as_str().contains("changed"));
        assert!(Arc::ptr_eq(fixture.session.projection(), &refreshed));
    }
}

#[test]
fn recursive_cyclic_and_denied_branches_leave_readable_siblings_running() {
    let mut fixture = fixture();
    let mut link = directory(4, "child");
    link.kind = EntryKind::DirectoryLink;
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![link, directory(7, "keep"), directory(10, "denied")]);
    fixture.graph.browse_identities.lock().unwrap().insert(
        4,
        Ok(Some(midnight_core::SizeDirectoryIdentity {
            device: 1,
            object: 1_u128.to_le_bytes(),
        })),
    );
    fixture.graph.failures.lock().unwrap().insert(10);
    expand_recursive(&mut fixture, "z-parent", 1);
    drain(&mut fixture, 6);
    assert!(
        !fixture.graph.listed.lock().unwrap().contains(&4),
        "cycle stops before provider enumeration"
    );
    assert!(fixture.graph.listed.lock().unwrap().contains(&7));
    assert!(
        fixture
            .session
            .projection()
            .rows
            .iter()
            .filter(|row| row.error.is_some())
            .count()
            >= 2
    );
}

#[test]
fn nested_collapse_rejects_late_results_and_continues_other_recursive_siblings() {
    let mut fixture = fixture();
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![directory(4, "child"), directory(7, "keep")]);
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(4, vec![directory(6, "grandchild")]);
    expand_recursive(&mut fixture, "z-parent", 1);
    let root_job = fixture.session.take_work().unwrap().unwrap();
    complete_recursive(&mut fixture, root_job, true);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let child = row_named(&fixture.session, "child");
    let delayed = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .branch(&view, TreeRequestId(2), child, BranchAction::Collapse)
        .unwrap();
    fixture
        .session
        .update_projection(&Cancellation::default())
        .unwrap();
    assert!(
        !fixture
            .session
            .apply(delayed.run(), &Cancellation::default())
            .unwrap()
    );
    drain(&mut fixture, 5);
    assert!(
        !fixture
            .session
            .projection()
            .rows
            .iter()
            .any(|row| row.entry.display_name == "grandchild")
    );
    assert!(fixture.graph.listed.lock().unwrap().contains(&7));
    assert!(
        !fixture
            .session
            .projection()
            .rows
            .iter()
            .find(|row| row.id == child)
            .unwrap()
            .expanded
    );
}

#[test]
fn a_collapsed_unvisited_child_is_excluded_for_the_remaining_walk() {
    let mut fixture = fixture();
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![directory(4, "child"), directory(7, "keep")]);
    expand_recursive(&mut fixture, "z-parent", 1);
    let job = fixture.session.take_work().unwrap().unwrap();
    complete_recursive(&mut fixture, job, true);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let keep = row_named(&fixture.session, "keep");
    fixture
        .session
        .branch(&view, TreeRequestId(2), keep, BranchAction::Collapse)
        .unwrap();
    fixture
        .session
        .update_projection(&Cancellation::default())
        .unwrap();
    drain(&mut fixture, 5);
    assert!(!fixture.graph.listed.lock().unwrap().contains(&7));
}

#[test]
fn recursive_width_exceeding_command_capacity_uses_one_continuation() {
    let mut fixture = fixture();
    fixture.graph.rows.lock().unwrap().insert(
        1,
        (10..74)
            .map(|value| directory(value, &format!("child-{value}")))
            .collect(),
    );
    expand_recursive(&mut fixture, "z-parent", 1);
    assert_eq!(drain(&mut fixture, 70), 65);
    assert!(fixture.session.queue.is_empty());
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), 66);
}

#[test]
fn recursive_sibling_aliases_are_both_visited_without_a_global_seen_set() {
    let mut fixture = fixture();
    let mut first = directory(4, "child");
    first.kind = EntryKind::DirectoryLink;
    let mut second = directory(7, "keep");
    second.kind = EntryKind::DirectoryLink;
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![first, second]);
    let identity = midnight_core::SizeDirectoryIdentity {
        device: 1,
        object: 55_u128.to_le_bytes(),
    };
    for value in [4, 7] {
        fixture
            .graph
            .browse_identities
            .lock()
            .unwrap()
            .insert(value, Ok(Some(identity)));
    }
    expand_recursive(&mut fixture, "z-parent", 1);
    assert_eq!(drain(&mut fixture, 5), 3);
    assert!(fixture.graph.listed.lock().unwrap().contains(&4));
    assert!(fixture.graph.listed.lock().unwrap().contains(&7));
}

#[test]
fn replacing_a_navigation_cancels_old_recursive_work_without_reopening_new_rows() {
    let mut previous = fixture();
    expand_recursive(&mut previous, "z-parent", 1);
    let root = previous.session.take_work().unwrap().unwrap();
    complete_recursive(&mut previous, root, false);
    let view = previous.session.projection().clone();
    previous.session.commit(&view);
    let delayed = previous.session.take_work().unwrap().unwrap();
    let graph = previous.graph.clone();
    let mut replacement = fixture();
    let replacement_view = replacement.session.projection().clone();
    drop(previous);
    assert!(
        !replacement
            .session
            .apply(delayed.run(), &Cancellation::default())
            .unwrap()
    );
    assert!(Arc::ptr_eq(
        replacement.session.projection(),
        &replacement_view
    ));
    assert!(!graph.listed.lock().unwrap().contains(&4));
}

#[test]
fn wide_node_capacity_keeps_the_last_valid_rows_and_stops_automatic_work() {
    let mut fixture = fixture();
    let original = fixture.session.projection().clone();
    fixture.graph.rows.lock().unwrap().insert(
        1,
        (10..10_010)
            .map(|value| directory(value, &format!("wide-{value}")))
            .collect(),
    );
    expand_recursive(&mut fixture, "z-parent", 1);
    assert_eq!(drain(&mut fixture, 3), 1);
    assert_eq!(fixture.session.projection().rows.len(), original.rows.len());
    assert_eq!(
        fixture.session.projection().stamp.session,
        original.stamp.session
    );
    assert!(fixture.session.take_recursive_notice().is_some());
    assert_eq!(fixture.session.queued(), 0);
}

#[test]
fn projection_capacity_rolls_back_new_children_and_reports_incomplete_without_session_loss() {
    let mut fixture = fixture();
    let original = fixture.session.projection().clone();
    *fixture.graph.target_failure.lock().unwrap() = Some(4);
    expand_recursive(&mut fixture, "z-parent", 1);
    assert!(drain(&mut fixture, 16) <= 16);
    assert_eq!(
        fixture.session.projection().stamp.session,
        original.stamp.session
    );
    assert!(Arc::ptr_eq(
        &fixture.session.projection().root.entries,
        &original.root.entries
    ));
    assert!(
        !fixture
            .session
            .projection()
            .rows
            .iter()
            .any(|row| row.entry.display_name == "child")
    );
    assert!(fixture.session.take_recursive_notice().is_some());
    assert!(fixture.session.take_work().unwrap().is_none());
}

#[test]
fn cumulative_legitimate_long_paths_hit_the_real_projection_guard_without_losing_the_session() {
    // Each component is below native basename limits and each absolute path is
    // below 4096 bytes. Their combined retained projection exceeds 16 MiB.
    let mut root = std::env::temp_dir().join("midnight-recursive-long-projection");
    for _ in 0..185 {
        root.push("valid-segment-1234");
    }
    assert!(root.as_os_str().len() < 3800);
    let mut fixture = fixture_with_root_path(0, EntryKind::Directory, Some(root.clone()));
    let mut children = Vec::new();
    for value in 10..2510 {
        let name = format!("child-{value}");
        children.push(directory(value, &name));
        fixture
            .graph
            .target_paths
            .lock()
            .unwrap()
            .insert(value, root.join("z-parent").join(&name));
    }
    fixture.graph.rows.lock().unwrap().insert(1, children);
    assert!(
        fixture.graph.target_failure.lock().unwrap().is_none(),
        "no injected resource failure"
    );
    let original = fixture.session.projection().clone();
    expand_recursive(&mut fixture, "z-parent", 1);
    assert!(drain(&mut fixture, 128) <= 128);
    assert!(
        fixture.graph.target_calls.load(Ordering::Relaxed) >= 2500,
        "real projection materialization reached its cumulative guard"
    );
    assert_eq!(
        fixture.session.projection().stamp.session,
        original.stamp.session
    );
    assert_eq!(fixture.session.projection().rows.len(), original.rows.len());
    assert!(fixture.session.take_recursive_notice().is_some());
    assert_eq!(fixture.session.queued(), 0);
    assert!(Arc::ptr_eq(fixture.session.projection(), &original));
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![directory(10, "child-10")]);
    expand_recursive(&mut fixture, "z-parent", 2);
    drain(&mut fixture, 3);
    let child = row_named(&fixture.session, "child-10");
    assert!(
        child.0 > 2500,
        "failed unpublished projection row IDs remain consumed"
    );
}

#[test]
fn explicit_tree_navigation_cancels_a_recursive_continuation_immediately() {
    let mut fixture = fixture();
    expand_recursive(&mut fixture, "z-parent", 1);
    let job = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(job.run(), &Cancellation::default())
        .unwrap();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let delayed = fixture.session.take_work().unwrap().unwrap();
    let destination = row_named(&fixture.session, "a-parent");
    fixture
        .session
        .navigate(
            &view,
            TreeRequestId(2),
            destination,
            NavigationId(8),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(
        !fixture
            .session
            .apply(delayed.run(), &Cancellation::default())
            .unwrap()
    );
    assert!(fixture.session.take_work().unwrap().is_none());
}

#[test]
fn recursive_work_counter_exhaustion_is_terminal_for_the_walk_without_busy_retry() {
    let mut fixture = fixture();
    expand_recursive(&mut fixture, "z-parent", 1);
    fixture.session.next_work = u64::MAX;
    assert!(fixture.session.take_work().unwrap().is_none());
    assert!(
        fixture.session.take_recursive_publication().is_none(),
        "capacity preserves the published view"
    );
    let view = fixture.session.projection().clone();
    assert!(
        fixture
            .session
            .capture_target(
                &view,
                row_named(&fixture.session, "a-parent"),
                &Cancellation::default()
            )
            .is_ok()
    );
    assert!(fixture.session.take_recursive_notice().is_some());
    assert_eq!(fixture.session.queued(), 0);
    assert!(fixture.session.take_work().unwrap().is_none());
}

#[test]
fn depth_capacity_pauses_with_visible_incomplete_status_and_retains_the_session() {
    let mut fixture = fixture();
    let mut parent = 1;
    for value in 10..80 {
        fixture
            .graph
            .rows
            .lock()
            .unwrap()
            .insert(parent, vec![directory(value, &format!("deep-{value}"))]);
        parent = value;
    }
    let session = fixture.session.projection().stamp.session;
    expand_recursive(&mut fixture, "z-parent", 1);
    drain(&mut fixture, 70);
    assert_eq!(fixture.session.projection().stamp.session, session);
    assert!(fixture.session.projection().rows.len() <= TreeProjection::MAX_ROWS);
    let TreeEvent::Rejected { error, .. } = fixture.session.take_recursive_notice().unwrap() else {
        panic!("incomplete notice");
    };
    assert!(error.as_str().contains("incomplete"));
    assert!(fixture.session.take_work().unwrap().is_none());
    assert!(fixture.session.take_work().unwrap().is_none());
}

#[test]
fn recursive_identity_resolution_never_replaces_captured_logical_action_paths() {
    let mut fixture = fixture_with_directory_kind(0, EntryKind::DirectoryLink);
    expand_recursive(&mut fixture, "z-parent", 1);
    drain(&mut fixture, 5);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let child = row_named(&fixture.session, "child");
    let capture = fixture
        .session
        .capture_target(&view, child, &Cancellation::default())
        .unwrap();
    let ProviderTarget::Native(path) = capture.target.value().unwrap() else {
        panic!("native target");
    };
    assert_eq!(
        path,
        &fixture.graph.root_path.join("z-parent").join("child")
    );
}
