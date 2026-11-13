//! Real acknowledged occurrences and native prepared targets exercise Delete admission.
use super::*;
use midnight_core::{DeleteMode, MutationService, TreeDeleteRequest, TreeSelectionAction};
use std::fs;

fn request(
    session: &mut Session,
    fallback: Option<TreeRowId>,
    mode: DeleteMode,
) -> TreeDeleteRequest {
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    TreeDeleteRequest {
        scope: midnight_core::TreeClipboardScope::SelectionOrFallback,
        mode,
        view: publication.view,
        request: TreeRequestId(session.last_request.0 + 1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        fallback,
        cancel: Cancellation::default(),
    }
}

#[test]
fn tree_delete_suppresses_selected_descendants_before_native_preparation() {
    let mut f = fixture();
    let parent = f.session.projection().rows[1].id;
    let expanded = super::super::action_capture_tests::expand(&mut f.session, parent);
    let child = expanded.view.rows[2].id;
    super::super::action_capture_tests::select(&mut f.session, TreeSelectionAction::Toggle(child));
    super::super::action_capture_tests::select(&mut f.session, TreeSelectionAction::Toggle(parent));
    let req = request(&mut f.session, Some(TreeRowId(0)), DeleteMode::Permanent);
    let capacity = Arc::new(Semaphore::new(4));
    let capture = f
        .session
        .capture_delete(&req, &capacity, &Cancellation::default())
        .unwrap();
    assert_eq!(
        capture.root_count, 1,
        "selected parent covers its selected descendant"
    );
    assert_eq!(capture.sources.root_count(), 1);
    assert_eq!(capture.sources.stamp(), req.view.stamp);
    assert_eq!(capture.sources.request(), req.request);
    fs::create_dir_all(f.graph.root_path.join("z-parent/child")).unwrap();
    let service = midnight_platform::LocalFileSystem::new(None);
    let targets = capture
        .sources
        .prepare(&service, &Cancellation::default())
        .unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].item().path, f.graph.root_path.join("z-parent"));
    assert!(
        f.graph.root_path.join("z-parent/child").is_dir(),
        "preparation is read-only"
    );
    drop(targets);
    drop(capture);
    assert_eq!(capacity.available_permits(), 4);
}

#[test]
fn tree_delete_focused_descendant_retains_immutable_mode_after_navigation() {
    for mode in [DeleteMode::Trash, DeleteMode::Permanent] {
        let mut f = fixture();
        let parent = f.session.projection().rows[1].id;
        let expanded = super::super::action_capture_tests::expand(&mut f.session, parent);
        let child = expanded.view.rows[2].id;
        let req = request(&mut f.session, Some(child), mode);
        let capture = f
            .session
            .capture_delete(&req, &Arc::new(Semaphore::new(4)), &Cancellation::default())
            .unwrap();
        let expected = f.graph.root_path.join("z-parent/child");
        fs::create_dir_all(&expected).unwrap();
        drop(f.session);
        assert_eq!(capture.mode, mode);
        assert_eq!(capture.sources.mode(), mode);
        let targets = capture
            .sources
            .prepare(
                &midnight_platform::LocalFileSystem::new(None),
                &Cancellation::default(),
            )
            .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].item().path, expected);
    }
}

#[test]
fn tree_delete_rejects_unacknowledged_foreign_stale_and_parent_occurrences() {
    let mut f = fixture();
    let publication = f.session.publication().unwrap();
    let row = publication.view.rows[0].id;
    let mut req = TreeDeleteRequest {
        scope: midnight_core::TreeClipboardScope::SelectionOrFallback,
        mode: DeleteMode::Permanent,
        view: publication.view,
        request: TreeRequestId(1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        fallback: Some(row),
        cancel: Cancellation::default(),
    };
    let capacity = Arc::new(Semaphore::new(4));
    assert!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    assert!(f.session.commit(&req.view));
    let exact_view = req.view.clone();
    req.view = Arc::new((*exact_view).clone());
    assert!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    req.view = exact_view;
    let exact_selection = req.selection.clone();
    req.selection = Arc::new((*exact_selection).clone());
    assert!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    req.selection = exact_selection;
    req.fallback = Some(TreeRowId(0));
    assert!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    req.fallback = Some(row);
    req.selection_revision.0 += 1;
    assert!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    assert_eq!(capacity.available_permits(), 4);
}

#[test]
fn tree_delete_cancel_and_capacity_fail_before_native_preparation() {
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let req = request(&mut f.session, Some(row), DeleteMode::Permanent);
    let capacity = Arc::new(Semaphore::new(0));
    assert!(matches!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    let capacity = Arc::new(Semaphore::new(4));
    req.cancel.cancel();
    assert!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    assert_eq!(capacity.available_permits(), 4);
}

struct WrongTarget;
impl MutationService for WrongTarget {
    fn prepare(
        &self,
        _: &std::path::Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn midnight_core::PreparedMutation>, midnight_core::FsError> {
        panic!("cancelled preparation must not call the native service")
    }
}

#[test]
fn tree_delete_preparation_checks_cancellation_before_opening_targets() {
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let req = request(&mut f.session, Some(row), DeleteMode::Permanent);
    let capture = f
        .session
        .capture_delete(&req, &Arc::new(Semaphore::new(4)), &Cancellation::default())
        .unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    let result = capture.sources.prepare(&WrongTarget, &cancel);
    assert!(result.is_err());
}

#[test]
fn tree_delete_rejects_changed_covering_directory_before_preview() {
    let mut f = fixture();
    let parent = f.session.projection().rows[1].id;
    let expanded = super::super::action_capture_tests::expand(&mut f.session, parent);
    let child = expanded.view.rows[2].id;
    super::super::action_capture_tests::select(&mut f.session, TreeSelectionAction::Toggle(child));
    super::super::action_capture_tests::select(&mut f.session, TreeSelectionAction::Toggle(parent));
    let req = request(&mut f.session, None, DeleteMode::Permanent);
    let capture = f
        .session
        .capture_delete(&req, &Arc::new(Semaphore::new(4)), &Cancellation::default())
        .unwrap();
    let changed = f.graph.root_path.join("z-parent");
    fs::write(&changed, b"replacement is not the covering directory").unwrap();
    let result = capture.sources.prepare(
        &midnight_platform::LocalFileSystem::new(None),
        &Cancellation::default(),
    );
    assert!(
        result.is_err(),
        "suppression premise must hold during native preparation"
    );
    assert_eq!(
        fs::read(changed).unwrap(),
        b"replacement is not the covering directory"
    );
}

#[test]
fn tree_delete_retains_distinct_hard_link_names_as_two_targets() {
    let mut f = fixture();
    let first = f.graph.root_path.join("a-parent");
    let second = f.graph.root_path.join("z-parent");
    fs::write(&first, b"one inode, two selected namespace entries").unwrap();
    fs::hard_link(&first, &second).unwrap();
    let first_row = f.session.projection().rows[0].id;
    let second_row = f.session.projection().rows[1].id;
    super::super::action_capture_tests::select(
        &mut f.session,
        TreeSelectionAction::Toggle(first_row),
    );
    super::super::action_capture_tests::select(
        &mut f.session,
        TreeSelectionAction::Toggle(second_row),
    );
    let req = request(&mut f.session, None, DeleteMode::Permanent);
    let capture = f
        .session
        .capture_delete(&req, &Arc::new(Semaphore::new(4)), &Cancellation::default())
        .unwrap();
    assert_eq!(capture.root_count, 2);
    let targets = capture
        .sources
        .prepare(
            &midnight_platform::LocalFileSystem::new(None),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|target| target.item().path.clone())
            .collect::<Vec<_>>(),
        [first, second]
    );
}

#[test]
fn tree_delete_expanded_directory_link_prepares_only_its_logical_occurrence() {
    let mut f = fixture();
    f.graph.rows.lock().unwrap().get_mut(&1).unwrap()[0].kind = EntryKind::DirectoryLink;
    f.graph
        .rows
        .lock()
        .unwrap()
        .insert(4, vec![directory(6, "grandchild")]);
    let parent = f.session.projection().rows[1].id;
    let expanded = super::super::action_capture_tests::expand(&mut f.session, parent);
    let link = expanded.view.rows[2].id;
    assert_eq!(
        expanded.view.rows[2].disclosure,
        midnight_core::TreeDisclosure::Unknown
    );
    f.session
        .branch(&expanded.view, TreeRequestId(2), link, BranchAction::Expand)
        .unwrap();
    let job = f.session.take_work().unwrap().unwrap();
    f.session
        .apply(job.run(), &Cancellation::default())
        .unwrap();
    let view = f.session.projection();
    assert!(
        view.rows
            .iter()
            .find(|row| row.id == link)
            .unwrap()
            .expanded
    );
    assert!(
        view.rows
            .iter()
            .any(|row| row.entry.display_name == "grandchild")
    );

    // Browsing a link does not turn its visible children into Delete roots.
    let req = request(&mut f.session, Some(link), DeleteMode::Permanent);
    let captured = f
        .session
        .capture_delete(&req, &Arc::new(Semaphore::new(4)), &Cancellation::default())
        .unwrap();
    assert_eq!(captured.root_count, 1);
    assert_eq!(captured.sources.root_count(), 1);
    assert_eq!(captured.sources.stamp(), req.view.stamp);

    struct ObservePreparation(Mutex<Vec<PathBuf>>);
    impl MutationService for ObservePreparation {
        fn prepare(
            &self,
            path: &std::path::Path,
            _: &Cancellation,
        ) -> Result<Arc<dyn midnight_core::PreparedMutation>, midnight_core::FsError> {
            self.0.lock().unwrap().push(path.to_owned());
            Err(midnight_core::FsError::from_io(
                "observe Delete preparation",
                path,
                std::io::ErrorKind::Unsupported.into(),
            ))
        }
    }
    let service = ObservePreparation(Mutex::default());
    assert!(
        captured
            .sources
            .prepare(&service, &Cancellation::default())
            .is_err()
    );
    assert_eq!(
        *service.0.lock().unwrap(),
        [f.graph.root_path.join("z-parent/child")],
        "native preparation receives only the logical link, never its browsed descendants"
    );
}

#[test]
fn tree_delete_clicked_unselected_parent_preserves_child_marks_and_uses_only_clicked_root() {
    for clicked_parent in [true, false] {
        let mut f = fixture();
        let parent = f.session.projection().rows[1].id;
        let expanded = super::super::action_capture_tests::expand(&mut f.session, parent);
        let child = expanded.view.rows[2].id;
        super::super::action_capture_tests::select(
            &mut f.session,
            TreeSelectionAction::Toggle(child),
        );
        let mut req = request(&mut f.session, Some(child), DeleteMode::Trash);
        req.scope =
            midnight_core::TreeClipboardScope::Dragged(if clicked_parent { parent } else { child });
        let selection = req.selection.clone();
        let capacity = Arc::new(Semaphore::new(4));
        let capture = f
            .session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .unwrap();
        assert_eq!(capture.root_count, 1);
        fs::create_dir_all(f.graph.root_path.join("z-parent/child")).unwrap();
        let targets = capture
            .sources
            .prepare(
                &midnight_platform::LocalFileSystem::new(None),
                &Cancellation::default(),
            )
            .unwrap();
        let expected = f.graph.root_path.join(if clicked_parent {
            "z-parent"
        } else {
            "z-parent/child"
        });
        assert_eq!(targets[0].item().path, expected);
        let current = f.session.publication().unwrap();
        assert!(Arc::ptr_eq(&current.selection, &selection));
        assert!(current.selection.contains_row(child));
        assert!(!current.selection.contains_row(parent));
        assert!(
            f.graph.root_path.join("z-parent/child").is_dir(),
            "capture/preparation must not mutate the fixture"
        );
        drop(targets);
        drop(capture);
        assert_eq!(capacity.available_permits(), 4);
    }
}

#[test]
fn tree_delete_clicked_scope_does_not_fall_back_to_valid_marks_for_a_foreign_row() {
    let mut f = fixture();
    let selected = f.session.projection().rows[0].id;
    super::super::action_capture_tests::select(
        &mut f.session,
        TreeSelectionAction::Toggle(selected),
    );
    let mut req = request(&mut f.session, Some(selected), DeleteMode::Permanent);
    req.scope = midnight_core::TreeClipboardScope::Dragged(TreeRowId(0));
    let capacity = Arc::new(Semaphore::new(4));
    assert!(
        f.session
            .capture_delete(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    assert_eq!(capacity.available_permits(), 4);
}
