//! Captured clipboard scopes use real session admission and retained provider authority.
use super::*;
use midnight_core::{FileClipboardIntent, TreeClipboardRequest, TreeSelectionAction};

fn request(session: &mut Session, fallback: Option<TreeRowId>) -> TreeClipboardRequest {
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    TreeClipboardRequest {
        scope: midnight_core::TreeClipboardScope::SelectionOrFallback,
        intent: FileClipboardIntent::Copy,
        view: publication.view,
        request: TreeRequestId(session.last_request.0 + 1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        fallback,
        cancel: Cancellation::default(),
    }
}

#[test]
fn clipboard_keeps_ordered_expanded_native_targets_and_copy_cut_intent() {
    for intent in [FileClipboardIntent::Copy, FileClipboardIntent::Move] {
        let mut fixture = tests::fixture();
        let parent = fixture.session.projection().rows[1].id;
        let expected = fixture.session.projection().rows[1].entry.path.clone();
        let expanded = action_capture_tests::expand(&mut fixture.session, parent);
        let child = expanded.view.rows[2].id;
        action_capture_tests::select(&mut fixture.session, TreeSelectionAction::Toggle(child));
        action_capture_tests::select(&mut fixture.session, TreeSelectionAction::Toggle(parent));
        let mut request = request(&mut fixture.session, Some(TreeRowId(u64::MAX)));
        request.intent = intent;
        let source = fixture
            .session
            .capture_clipboard(
                &request,
                &Arc::new(Semaphore::new(4)),
                &Cancellation::default(),
            )
            .unwrap();
        assert_eq!(source.intent(), intent);
        let files = source.resolve(&Cancellation::default()).unwrap();
        assert_eq!(files.intent, intent);
        assert_eq!(files.paths, [expected.clone(), expected.join("child")]);
    }
}

#[test]
fn clipboard_without_marks_uses_exact_focused_row_and_refuses_parent_navigation() {
    let mut fixture = tests::fixture();
    let row = fixture.session.projection().rows[2].id;
    let expected = fixture.session.projection().rows[2].entry.path.clone();
    let capacity = Arc::new(Semaphore::new(4));
    let missing = request(&mut fixture.session, None);
    assert!(matches!(
        fixture
            .session
            .capture_clipboard(&missing, &capacity, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
    let foreign = request(&mut fixture.session, Some(TreeRowId(0)));
    assert!(matches!(
        fixture
            .session
            .capture_clipboard(&foreign, &capacity, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    let valid = request(&mut fixture.session, Some(row));
    let source = fixture
        .session
        .capture_clipboard(&valid, &capacity, &Cancellation::default())
        .unwrap();
    assert_eq!(
        source.resolve(&Cancellation::default()).unwrap().paths,
        [expected]
    );
}

#[test]
fn dragged_unselected_occurrence_preserves_marks_and_captures_only_that_row() {
    let mut fixture = tests::fixture();
    let selected = fixture.session.projection().rows[1].id;
    let dragged = fixture.session.projection().rows[2].id;
    let expected = fixture.session.projection().rows[2].entry.path.clone();
    action_capture_tests::select(&mut fixture.session, TreeSelectionAction::Toggle(selected));
    let mut request = request(&mut fixture.session, Some(selected));
    request.scope = midnight_core::TreeClipboardScope::Dragged(dragged);
    let marks = request.selection.clone();
    let source = fixture
        .session
        .capture_clipboard(
            &request,
            &Arc::new(Semaphore::new(4)),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        source.resolve(&Cancellation::default()).unwrap().paths,
        [expected]
    );
    let after = fixture.session.publication().unwrap();
    assert_eq!(
        source.tree_copy_occurrence(0),
        Some(midnight_core::TreeCopyOccurrence {
            stamp: request.view.stamp,
            row: dragged
        })
    );
    assert_eq!(source.tree_copy_occurrence(1), None);
    assert!(Arc::ptr_eq(&marks, &after.selection));
    assert_eq!(after.selection.len(), 1);
}

#[test]
fn clipboard_rejects_foreign_selection_and_replayed_requests() {
    let mut fixture = tests::fixture();
    let row = fixture.session.projection().rows[0].id;
    let capacity = Arc::new(Semaphore::new(4));
    let valid = request(&mut fixture.session, Some(row));
    let mut foreign = valid.clone();
    foreign.selection = Arc::new((*foreign.selection).clone());
    assert!(matches!(
        fixture
            .session
            .capture_clipboard(&foreign, &capacity, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    let mut foreign = valid.clone();
    foreign.view = Arc::new((*foreign.view).clone());
    assert!(matches!(
        fixture
            .session
            .capture_clipboard(&foreign, &capacity, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
    fixture
        .session
        .capture_clipboard(&valid, &capacity, &Cancellation::default())
        .unwrap();
    assert!(matches!(
        fixture
            .session
            .capture_clipboard(&valid, &capacity, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn clipboard_capacity_is_shared_and_retained_through_last_source_owner() {
    let mut fixture = tests::fixture();
    let row = fixture.session.projection().rows[0].id;
    let capacity = Arc::new(Semaphore::new(1));
    let first = request(&mut fixture.session, Some(row));
    let source = fixture
        .session
        .capture_clipboard(&first, &capacity, &Cancellation::default())
        .unwrap();
    let second = request(&mut fixture.session, Some(row));
    assert!(matches!(
        fixture
            .session
            .capture_clipboard(&second, &capacity, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    let retained = source.clone();
    drop(source);
    assert_eq!(capacity.available_permits(), 0);
    drop(retained);
    assert_eq!(capacity.available_permits(), 1);
    fixture
        .session
        .capture_clipboard(&second, &capacity, &Cancellation::default())
        .unwrap();
}

#[test]
fn clipboard_cancellation_blocks_export_after_capture_and_preserves_provider_lease() {
    for which in 0..3 {
        let mut fixture = tests::fixture();
        let row = fixture.session.projection().rows[0].id;
        let request = request(&mut fixture.session, Some(row));
        let generation = Cancellation::default();
        let export = Cancellation::default();
        let registration = Arc::downgrade(&fixture.session.registration);
        let source = fixture
            .session
            .capture_clipboard(&request, &Arc::new(Semaphore::new(4)), &generation)
            .unwrap();
        drop(fixture.session);
        assert!(registration.upgrade().is_some());
        match which {
            0 => request.cancel.cancel(),
            1 => generation.cancel(),
            _ => export.cancel(),
        }
        assert!(
            matches!(source.resolve(&export), Err(error) if error.kind == std::io::ErrorKind::Interrupted)
        );
        drop(source);
        assert!(registration.upgrade().is_none());
    }
}

#[test]
fn clipboard_rechecks_provider_context_even_without_a_navigation_cancel() {
    let mut fixture = tests::fixture();
    let row = fixture.session.projection().rows[0].id;
    let request = request(&mut fixture.session, Some(row));
    let source = fixture
        .session
        .capture_clipboard(
            &request,
            &Arc::new(Semaphore::new(4)),
            &Cancellation::default(),
        )
        .unwrap();
    let registration = &fixture.session.registration;
    assert!(
        registration
            .registry
            .unregister(&registration.provider, registration.instance)
            .unwrap()
    );
    assert!(source.resolve(&Cancellation::default()).is_err());
}

#[test]
fn internal_drop_captures_only_exact_native_folders_and_retains_bounded_authority() {
    let mut fixture = tests::fixture();
    let publication = fixture.session.publication().unwrap();
    assert!(fixture.session.commit(&publication.view));
    let view = publication.view;
    let folder = view.rows[1].id;
    let file = view.rows[2].id;
    let expected = view.rows[1].entry.path.clone();
    let capacity = Arc::new(Semaphore::new(1));
    let cancel = Cancellation::default();
    let request = TreeRequestId(fixture.session.last_request.0 + 1);
    assert!(matches!(
        fixture.session.capture_drop_directory(
            &Arc::new((*view).clone()),
            folder,
            request,
            &capacity,
            &cancel
        ),
        Err(ProviderError::StaleReference)
    ));
    assert!(matches!(
        fixture
            .session
            .capture_drop_directory(&view, file, request, &capacity, &cancel),
        Err(ProviderError::Unsupported)
    ));
    let registration = Arc::downgrade(&fixture.session.registration);
    let target = fixture
        .session
        .capture_drop_directory(&view, folder, request, &capacity, &cancel)
        .unwrap();
    assert_eq!(target.stamp(), view.stamp);
    assert_eq!(target.row(), folder);
    assert_eq!(target.resolve(&Cancellation::default()).unwrap(), expected);
    assert_eq!(capacity.available_permits(), 0);
    assert!(matches!(
        fixture.session.capture_drop_directory(
            &view,
            folder,
            TreeRequestId(request.0 + 1),
            &capacity,
            &cancel
        ),
        Err(ProviderError::ResourceLimited)
    ));
    drop(fixture.session);
    assert!(registration.upgrade().is_some());
    cancel.cancel();
    assert_eq!(
        target.resolve(&Cancellation::default()).unwrap_err().kind,
        std::io::ErrorKind::Interrupted
    );
    drop(target);
    assert_eq!(capacity.available_permits(), 1);
    assert!(registration.upgrade().is_none());
}
