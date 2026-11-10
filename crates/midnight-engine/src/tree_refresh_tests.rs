use super::*;
use midnight_core::{TreeEvent, TreeRefreshRequest, TreeSelectionAction};

#[test]
fn unchanged_watch_failure_diagnostic_retains_snapshot_identity_for_coalescing() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    *fixture.graph.target_failure.lock().unwrap() = Some(4);
    let error = fixture
        .session
        .watch_scope()
        .err()
        .expect("scope resolution must fail");
    let diagnostic = fixture.session.watch_failure_diagnostic(&error);
    assert!(diagnostic.details().is_some());
    let calls = fixture.graph.target_calls.load(Ordering::SeqCst);
    for _ in 0..5 {
        fixture.session.commit(&view);
        let error = fixture
            .session
            .watch_scope()
            .err()
            .expect("scope resolution must fail");
        let repeated = fixture.session.watch_failure_diagnostic(&error);
        assert_eq!(
            diagnostic, repeated,
            "unchanged cached failure must not dirty the watch again"
        );
    }
    assert_eq!(fixture.graph.target_calls.load(Ordering::SeqCst), calls);
}

#[test]
fn refresh_reports_all_branch_failure_details_in_one_retained_source() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    branch(&mut fixture, "a-parent", BranchAction::Expand);
    fixture.graph.failures.lock().unwrap().extend([1, 2]);
    refresh(&mut fixture);
    let TreeEvent::Refreshed {
        result: Err(error), ..
    } = finish(&mut fixture)
    else {
        panic!("both failed branches must fail the refresh");
    };
    assert!(error.as_str().contains("2 refresh failures"));
    let source = error
        .details()
        .expect("refresh failure details are retained");
    let mut bytes = vec![0; source.length().try_into().unwrap()];
    let mut offset = 0;
    while offset < bytes.len() {
        let read = source.read_at(offset as u64, &mut bytes[offset..]).unwrap();
        assert!(read > 0);
        offset += read;
    }
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("z-parent"));
    assert!(text.contains("a-parent"));
    assert_eq!(text.matches("fixture listing denied").count(), 2);
}

#[test]
fn review_scope_failure_is_cached_until_listing_or_membership_changes() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    *fixture.graph.target_failure.lock().unwrap() = Some(4);
    assert!(fixture.session.watch_scope().is_err());
    let calls = fixture.graph.target_calls.load(Ordering::SeqCst);
    for _ in 0..5 {
        assert!(fixture.session.commit(&view));
        assert!(fixture.session.watch_scope().is_err());
    }
    assert_eq!(
        fixture.graph.target_calls.load(Ordering::SeqCst),
        calls,
        "unchanged failing Commit must not resolve or hash the scope again"
    );
    *fixture.graph.target_failure.lock().unwrap() = None;
    refresh(&mut fixture);
    finish(&mut fixture);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    assert_eq!(
        fixture
            .session
            .watch_scope()
            .unwrap()
            .unwrap()
            .targets
            .len(),
        2
    );
}

#[test]
fn watch_scope_retains_native_root_and_only_visible_expanded_branches() {
    let mut fixture = fixture();
    let initial = fixture.session.projection().clone();
    assert!(fixture.session.watch_scope().is_err());
    fixture.session.commit(&initial);
    let scope = fixture.session.watch_scope().unwrap().unwrap();
    assert_eq!(scope.targets.len(), 1);
    fixture.session.commit(&initial);
    let unchanged = fixture.session.watch_scope().unwrap().unwrap();
    assert!(
        Arc::ptr_eq(&scope.targets[0], &unchanged.targets[0]),
        "unchanged Commit must reuse accepted signatures and provider targets"
    );
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    let expanded = fixture.session.projection().clone();
    fixture.session.commit(&expanded);
    let scope = fixture.session.watch_scope().unwrap().unwrap();
    assert_eq!(scope.targets.len(), 2);
    assert!(scope.targets[1].path.ends_with("z-parent"));
    assert!(expanded.root.same_snapshot(&initial.root));
    branch(&mut fixture, "z-parent", BranchAction::Collapse);
    let collapsed = fixture.session.projection().clone();
    fixture.session.commit(&collapsed);
    let scope = fixture.session.watch_scope().unwrap().unwrap();
    assert_eq!(scope.targets.len(), 1);
}

#[test]
fn watch_scope_keeps_filter_context_ancestors_but_removes_hidden_branches() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    fixture
        .session
        .reproject(
            ProjectionOptions {
                filter: midnight_core::PanelFilter {
                    enabled: true,
                    mode: midnight_core::FilterMode::Expression,
                    expression: "name = \"child\"".into(),
                    case_sensitive: true,
                },
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .unwrap();
    let view = fixture.session.projection().clone();
    assert!(view.rows[0].context_only);
    fixture.session.commit(&view);
    let scope = fixture.session.watch_scope().unwrap().unwrap();
    assert_eq!(scope.targets.len(), 2);
    fixture
        .session
        .reproject(
            ProjectionOptions {
                filter: midnight_core::PanelFilter {
                    enabled: true,
                    mode: midnight_core::FilterMode::Expression,
                    expression: "name = \"a-parent\"".into(),
                    case_sensitive: true,
                },
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .unwrap();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let scope = fixture.session.watch_scope().unwrap().unwrap();
    assert_eq!(scope.targets.len(), 1);
}

#[test]
fn blocked_maintenance_allows_explicit_different_branch_expand() {
    let mut fixture = fixture();
    let mutation = fixture.session.budget.mutation.clone();
    let held = mutation.clone().try_acquire_owned().unwrap();
    refresh(&mut fixture);
    let view = fixture.session.projection().clone();
    let row = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap()
        .id;
    let request = fixture.session.last_request.checked_next().unwrap();
    fixture
        .session
        .branch(&view, request, row, BranchAction::Expand)
        .unwrap();
    let job = fixture.session.take_work().unwrap();
    assert!(
        job.is_some(),
        "explicit expansion must run while maintenance waits for Copy"
    );
    let job = job.unwrap();
    assert!(!job.maintenance());
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
            .any(|row| row.entry.display_name == "child")
    );
    assert!(fixture.session.finish_refresh().is_none());
    drop(held);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
}

fn branch(fixture: &mut Fixture, name: &str, action: BranchAction) {
    let view = fixture.session.projection().clone();
    let row = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == name)
        .unwrap()
        .id;
    fixture.session.commit(&view);
    let request = fixture.session.last_request.checked_next().unwrap();
    fixture.session.branch(&view, request, row, action).unwrap();
    if action == BranchAction::Collapse {
        fixture
            .session
            .update_projection(&Cancellation::default())
            .unwrap();
    } else {
        let job = fixture.session.take_work().unwrap().unwrap();
        fixture
            .session
            .apply(job.run(), &Cancellation::default())
            .unwrap();
    }
}

fn refresh(fixture: &mut Fixture) -> TreeRefreshRequest {
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let request = TreeRefreshRequest {
        view,
        request: fixture.session.last_request.checked_next().unwrap(),
    };
    fixture
        .session
        .refresh_loaded(&request, &Cancellation::default())
        .unwrap();
    request
}

fn finish(fixture: &mut Fixture) -> TreeEvent {
    for _ in 0..TreeProjection::MAX_ROWS + 1 {
        if let Some(job) = fixture.session.take_work().unwrap() {
            fixture
                .session
                .apply(job.run(), &Cancellation::default())
                .unwrap();
        }
        if let Some(event) = fixture.session.finish_refresh() {
            return event;
        }
    }
    panic!("bounded maintenance did not finish")
}

fn select(fixture: &mut Fixture, name: &str) -> TreeRowId {
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let row = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == name)
        .unwrap()
        .id;
    fixture
        .session
        .select(&TreeSelectionRequest {
            view,
            revision: SelectionRevision(fixture.session.selection.revision.0 + 1),
            seed: None,
            seed_revision: fixture.session.selection.revision,
            action: TreeSelectionAction::Toggle(row),
        })
        .unwrap();
    row
}

#[test]
fn refresh_removes_nested_and_root_ghosts_retains_marks_expansion_and_admission() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    let marked = select(&mut fixture, "z-parent");
    let first = fixture.session.projection().clone();
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .retain(|row| row.token != token(3));
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![directory(7, "keep")]);
    let request = refresh(&mut fixture);
    assert!(fixture.session.finish_refresh().is_none());
    assert!(
        matches!(finish(&mut fixture), TreeEvent::Refreshed { stamp, request: id, result: Ok(()) }
        if stamp == request.view.stamp && id == request.request)
    );
    let view = fixture.session.projection();
    assert_eq!(view.stamp.session, first.stamp.session);
    assert!(view.root.same_snapshot(&first.root));
    assert_eq!(
        view.rows
            .iter()
            .map(|row| row.entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["a-parent", "z-parent", "keep"]
    );
    assert!(view.rows[1].expanded);
    assert_eq!(view.rows[1].id, marked);
    assert!(fixture.session.selection.snapshot.contains_row(marked));
}

#[test]
fn refresh_visits_collapsed_cached_branches_but_not_probes_or_unopened_branches() {
    let mut fixture = fixture();
    branch(&mut fixture, "a-parent", BranchAction::Probe);
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    branch(&mut fixture, "child", BranchAction::Expand);
    branch(&mut fixture, "z-parent", BranchAction::Collapse);
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(4, vec![directory(6, "grandchild")]);
    fixture.graph.listed.lock().unwrap().clear();
    refresh(&mut fixture);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    assert_eq!(*fixture.graph.listed.lock().unwrap(), [0, 1, 4]);
    assert!(!fixture.session.projection().rows[1].expanded);
    assert!(
        fixture
            .session
            .tree
            .state(fixture.session.tree.loaded_native_branches()[2])
            .is_some()
    );
}

#[test]
fn refresh_skips_removed_descendants_without_failing_session() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    branch(&mut fixture, "child", BranchAction::Expand);
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .retain(|row| row.token != token(1));
    fixture.graph.listed.lock().unwrap().clear();
    refresh(&mut fixture);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    assert_eq!(*fixture.graph.listed.lock().unwrap(), [0]);
    assert_eq!(fixture.session.projection().rows.len(), 2);
}

#[test]
fn root_failure_reports_error_keeps_rows_and_marks_and_stops_descendants() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    let selected = select(&mut fixture, "child");
    let before = fixture.session.projection().clone();
    fixture.graph.failures.lock().unwrap().insert(0);
    fixture.graph.listed.lock().unwrap().clear();
    refresh(&mut fixture);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Err(_), .. }
    ));
    assert_eq!(*fixture.graph.listed.lock().unwrap(), [0]);
    assert!(before.same_structure(fixture.session.projection()));
    assert!(fixture.session.selection.snapshot.contains_row(selected));
}

#[test]
fn branch_failure_skips_its_descendants_and_continues_independent_loaded_branch() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    branch(&mut fixture, "child", BranchAction::Expand);
    branch(&mut fixture, "a-parent", BranchAction::Expand);
    fixture.graph.failures.lock().unwrap().insert(1);
    fixture.graph.listed.lock().unwrap().clear();
    refresh(&mut fixture);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Err(_), .. }
    ));
    assert_eq!(*fixture.graph.listed.lock().unwrap(), [0, 1, 2]);
    let parent = fixture
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap();
    assert!(parent.error.is_some());
    assert!(
        fixture
            .session
            .projection()
            .rows
            .iter()
            .any(|row| row.entry.display_name == "child")
    );
}

#[test]
fn maintenance_waits_for_mutation_gate_and_retains_it_through_apply() {
    let mut fixture = fixture();
    let selected = select(&mut fixture, "file.txt");
    let gate = fixture.session.budget.mutation.clone();
    let permit = gate.clone().try_acquire_owned().unwrap();
    let original = fixture.graph.rows.lock().unwrap().get(&0).unwrap().clone();
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .retain(|row| row.token != token(3));
    refresh(&mut fixture);
    assert!(fixture.session.take_work().unwrap().is_none());
    assert!(Arc::ptr_eq(&fixture.session.waiting_on(), &gate));
    assert_eq!(fixture.session.budget.listing.available_permits(), 2);
    fixture.graph.rows.lock().unwrap().insert(0, original);
    drop(permit);
    let job = fixture.session.take_work().unwrap().unwrap();
    assert!(gate.clone().try_acquire_owned().is_err());
    let completion = job.run();
    assert!(gate.clone().try_acquire_owned().is_err());
    fixture
        .session
        .apply(completion, &Cancellation::default())
        .unwrap();
    assert!(gate.clone().try_acquire_owned().is_ok());
    assert!(fixture.session.selection.snapshot.contains_row(selected));
    assert!(matches!(
        fixture.session.finish_refresh(),
        Some(TreeEvent::Refreshed { result: Ok(()), .. })
    ));
}

#[test]
fn maintenance_holds_no_mutation_gate_while_listing_capacity_is_unavailable() {
    let mut fixture = fixture();
    let listing = fixture.session.budget.listing.clone();
    let permits = listing.clone().try_acquire_many_owned(2).unwrap();
    refresh(&mut fixture);
    assert!(fixture.session.take_work().unwrap().is_none());
    assert!(
        fixture
            .session
            .budget
            .mutation
            .clone()
            .try_acquire_owned()
            .is_ok()
    );
    assert!(Arc::ptr_eq(&fixture.session.waiting_on(), &listing));
    drop(permits);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
}

#[test]
fn new_root_files_receive_current_exact_action_aliases_and_removed_aliases_never_return() {
    let mut fixture = fixture();
    let original = fixture.session.projection().clone();
    let old = original
        .rows
        .iter()
        .find(|row| row.entry.kind == EntryKind::File)
        .unwrap();
    let old_alias = old.original_root_entry.unwrap();
    let old_row = old.id;
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .retain(|row| row.token != token(3));
    refresh(&mut fixture);
    finish(&mut fixture);
    let mut file = directory(5, "new.txt");
    file.kind = EntryKind::File;
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .get_mut(&0)
        .unwrap()
        .push(file);
    refresh(&mut fixture);
    finish(&mut fixture);
    let view = fixture.session.projection();
    let added = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "new.txt")
        .unwrap();
    let alias = added.original_root_entry.unwrap();
    assert_ne!(alias, old_alias);
    assert_ne!(added.id, old_row);
    assert!(!view.root.index.by_id.contains_key(&alias));
    assert_eq!(
        view.root_actions.entries[view.root_actions.index.by_id[&alias]].path,
        fixture.graph.root_path.join("new.txt")
    );
    assert!(view.root.same_snapshot(&original.root));
    fixture.session.commit(&view.clone());
}

#[test]
fn large_loaded_scope_streams_one_job_and_never_fills_user_queue() {
    let mut fixture = fixture_with_extra_directories(40);
    for index in 0..40 {
        branch(
            &mut fixture,
            &format!("b-folder-{index:03}"),
            BranchAction::Expand,
        );
    }
    fixture.graph.listed.lock().unwrap().clear();
    refresh(&mut fixture);
    assert!(fixture.session.queue.is_empty());
    assert_eq!(fixture.session.queued(), 41);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    assert_eq!(fixture.graph.listed.lock().unwrap().len(), 41);
    assert!(fixture.session.queue.is_empty());
}

#[test]
fn stale_forged_and_duplicate_refresh_requests_are_rejected_without_listing() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let forged = TreeRefreshRequest {
        view: Arc::new((*view).clone()),
        request: TreeRequestId(1),
    };
    assert!(matches!(
        fixture
            .session
            .refresh_loaded(&forged, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    let request = refresh(&mut fixture);
    let busy = TreeRefreshRequest {
        view,
        request: request.request.checked_next().unwrap(),
    };
    assert!(matches!(
        fixture
            .session
            .refresh_loaded(&busy, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    finish(&mut fixture);
    assert!(matches!(
        fixture
            .session
            .refresh_loaded(&request, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn failed_action_materialization_rolls_back_root_membership_and_selection() {
    let mut fixture = fixture();
    let selected = select(&mut fixture, "file.txt");
    let before = fixture.session.projection().clone();
    // Listing succeeds, but this new opaque token cannot resolve to native authority.
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(0, vec![directory(8, "unresolvable")]);
    refresh(&mut fixture);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Err(_), .. }
    ));
    assert!(before.same_structure(fixture.session.projection()));
    assert!(
        fixture
            .session
            .projection()
            .root_actions
            .same_snapshot(&before.root_actions)
    );
    assert!(fixture.session.selection.snapshot.contains_row(selected));
}

#[test]
fn later_mutation_job_blocks_the_next_loaded_branch_step() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    let child = select(&mut fixture, "child");
    refresh(&mut fixture);
    let root_job = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(root_job.run(), &Cancellation::default())
        .unwrap();
    let gate = fixture.session.budget.mutation.clone();
    let second_job = gate.clone().try_acquire_owned().unwrap();
    fixture.graph.rows.lock().unwrap().insert(1, vec![]);
    assert!(fixture.session.take_work().unwrap().is_none());
    assert!(Arc::ptr_eq(&fixture.session.waiting_on(), &gate));
    fixture
        .graph
        .rows
        .lock()
        .unwrap()
        .insert(1, vec![directory(4, "child")]);
    drop(second_job);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    assert!(fixture.session.selection.snapshot.contains_row(child));
}

#[test]
fn retired_maintenance_completion_cannot_modify_a_replacement_session() {
    let mut old = fixture();
    refresh(&mut old);
    let budget = old.session.budget.clone();
    let job = old.session.take_work().unwrap().unwrap();
    old.session.lifetime.cancel();
    drop(old.session);
    assert_eq!(budget.mutation.available_permits(), 0);
    let mut current = fixture();
    current.session.budget = budget.clone();
    let before = current.session.projection().clone();
    assert!(
        !current
            .session
            .apply(job.run(), &Cancellation::default())
            .unwrap()
    );
    assert!(Arc::ptr_eq(current.session.projection(), &before));
    assert!(current.session.finish_refresh().is_none());
    assert_eq!(budget.mutation.available_permits(), 1);
}

#[test]
fn empty_native_admission_can_publish_new_file_action_authority() {
    let mut fixture = fixture();
    fixture.graph.rows.lock().unwrap().insert(0, vec![]);
    let original = fixture.session.projection().root.clone();
    fixture.session = Session::new(
        TreeRootSnapshot {
            pane: original.pane,
            navigation: original.navigation,
            entries: Arc::from([]),
            index: Arc::new(midnight_core::DirectoryIndex::from_entries(&[])),
        },
        OpenedLocation {
            mount: fixture.graph.clone(),
            root: token(0),
        },
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(1))),
        ProjectionOptions::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let mut added = directory(5, "new.txt");
    added.kind = EntryKind::File;
    fixture.graph.rows.lock().unwrap().insert(0, vec![added]);
    refresh(&mut fixture);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    let view = fixture.session.projection();
    assert!(view.root.entries.is_empty());
    assert_eq!(view.rows[0].original_root_entry, Some(EntryId(1)));
    assert_eq!(
        view.root_actions.entries[0].path,
        fixture.graph.root_path.join("new.txt")
    );
}

#[test]
fn collapsing_during_a_sweep_retries_cancelled_step_without_reopening_branch() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    refresh(&mut fixture);
    let root_job = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(root_job.run(), &Cancellation::default())
        .unwrap();
    let child_job = fixture.session.take_work().unwrap().unwrap();
    branch(&mut fixture, "z-parent", BranchAction::Collapse);
    fixture
        .session
        .apply(child_job.run(), &Cancellation::default())
        .unwrap();
    fixture.graph.rows.lock().unwrap().insert(1, vec![]);
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    let parent = fixture
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap();
    assert!(!parent.expanded);
    assert_eq!(parent.disclosure, midnight_core::TreeDisclosure::Empty);
}

#[test]
fn maintenance_does_not_consume_the_version_of_a_queued_explicit_expansion() {
    let mut fixture = fixture();
    branch(&mut fixture, "z-parent", BranchAction::Expand);
    branch(&mut fixture, "a-parent", BranchAction::Expand);
    branch(&mut fixture, "z-parent", BranchAction::Collapse);
    refresh(&mut fixture);
    let root_job = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(root_job.run(), &Cancellation::default())
        .unwrap();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let x = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap()
        .id;
    let y = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "a-parent")
        .unwrap()
        .id;
    let request = fixture.session.last_request.checked_next().unwrap();
    fixture
        .session
        .branch(&view, request, y, BranchAction::Refresh)
        .unwrap();
    fixture
        .session
        .branch(
            &view,
            request.checked_next().unwrap(),
            x,
            BranchAction::Expand,
        )
        .unwrap();
    let user_y = fixture.session.take_work().unwrap().unwrap();
    assert_eq!(user_y.row(), y);
    fixture
        .session
        .apply(user_y.run(), &Cancellation::default())
        .unwrap();
    assert!(matches!(
        finish(&mut fixture),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    while let Some(job) = fixture.session.take_work().unwrap() {
        fixture
            .session
            .apply(job.run(), &Cancellation::default())
            .unwrap();
    }
    let current = fixture.session.projection();
    assert!(
        current.rows.iter().any(|row| row.id == x && row.expanded),
        "explicit queued Expand must survive maintenance"
    );
    assert!(
        current
            .rows
            .iter()
            .any(|row| row.entry.display_name == "child")
    );
}
