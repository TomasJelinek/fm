use super::super::action_capture_tests::{expand, select};
use super::*;
use midnight_core::{TreeSelectionAction, TreeUserCommandRequest};

fn request(session: &mut Session, focus: Option<TreeRowId>) -> TreeUserCommandRequest {
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    TreeUserCommandRequest {
        view: publication.view,
        request: TreeRequestId(session.last_request.0 + 1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        focus,
        cancel: Cancellation::default(),
    }
}

#[test]
fn tree_user_command_capture_keeps_selected_descendants_separate_from_focus() {
    let mut f = fixture();
    let parent = f
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap()
        .id;
    let expanded = expand(&mut f.session, parent);
    let child = expanded
        .view
        .rows
        .iter()
        .find(|row| row.depth == 1)
        .unwrap()
        .id;
    let file = expanded
        .view
        .rows
        .iter()
        .find(|row| row.entry.kind == EntryKind::File)
        .unwrap()
        .id;
    select(&mut f.session, TreeSelectionAction::Toggle(child));
    let req = request(&mut f.session, Some(file));
    let sources = f
        .session
        .capture_user_command(
            &req,
            &Arc::new(Semaphore::new(1)),
            &crate::tree_rename::Retirement::testing(),
            &Cancellation::default(),
        )
        .unwrap();
    let resolved = sources.resolve(&Cancellation::default()).unwrap();
    assert_eq!(resolved.directory, f.graph.root_path);
    assert_eq!(resolved.selected.len(), 1);
    assert_eq!(
        resolved.selected[0].path,
        f.graph.root_path.join("z-parent/child")
    );
    assert_eq!(resolved.selected[0].kind, EntryKind::Directory);
    sources
        .revalidate_for(
            &[resolved.selected[0].id, resolved.focus.as_ref().unwrap().id],
            &Cancellation::default(),
        )
        .unwrap();
    assert!(
        sources
            .revalidate_for(&[EntryId(u64::MAX)], &Cancellation::default())
            .is_err()
    );

    assert_eq!(
        resolved.focus.unwrap().path,
        f.graph.root_path.join("file.txt")
    );
}

#[tokio::test]
async fn tree_user_command_capture_allows_location_only_once_and_keeps_unmarked_focus_unselected() {
    let mut f = fixture();
    let file = f
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.kind == EntryKind::File)
        .unwrap()
        .id;
    for focus in [None, Some(file)] {
        let req = request(&mut f.session, focus);
        let capacity = Arc::new(Semaphore::new(1));
        let sources = f
            .session
            .capture_user_command(
                &req,
                &capacity,
                &crate::tree_rename::Retirement::testing(),
                &Cancellation::default(),
            )
            .unwrap();
        let resolved = sources.resolve(&Cancellation::default()).unwrap();
        assert!(resolved.selected.is_empty());
        assert_eq!(resolved.focus.is_some(), focus.is_some());
        assert_eq!(capacity.available_permits(), 0);
        drop(sources);
        wait_retired(&capacity).await;
    }
}

#[test]
fn tree_user_command_capture_requires_exact_acknowledged_view_selection_and_request() {
    let mut f = fixture();
    let mut req = request(&mut f.session, None);
    let capacity = Arc::new(Semaphore::new(1));
    let original = req.view.clone();
    req.view = Arc::new((*original).clone());
    assert!(
        f.session
            .capture_user_command(
                &req,
                &capacity,
                &crate::tree_rename::Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    req.view = original;
    let original = req.selection.clone();
    req.selection = Arc::new((*original).clone());
    assert!(
        f.session
            .capture_user_command(
                &req,
                &capacity,
                &crate::tree_rename::Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    req.selection = original;
    req.focus = Some(TreeRowId(u64::MAX));
    assert!(
        f.session
            .capture_user_command(
                &req,
                &capacity,
                &crate::tree_rename::Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    assert_eq!(capacity.available_permits(), 1);
    req.focus = None;
    let sources = f
        .session
        .capture_user_command(
            &req,
            &capacity,
            &crate::tree_rename::Retirement::testing(),
            &Cancellation::default(),
        )
        .unwrap();
    drop(sources);
    assert!(matches!(
        f.session.capture_user_command(
            &req,
            &capacity,
            &crate::tree_rename::Retirement::testing(),
            &Cancellation::default()
        ),
        Err(ProviderError::StaleReference)
    ));
}

#[tokio::test]
async fn tree_user_command_capture_retains_provider_after_navigation_and_rejects_cancellation_or_revocation()
 {
    for revoke in [false, true] {
        let mut f = fixture();
        let req = request(&mut f.session, None);
        let capacity = Arc::new(Semaphore::new(1));
        let navigation = Cancellation::default();
        let sources = f
            .session
            .capture_user_command(
                &req,
                &capacity,
                &crate::tree_rename::Retirement::testing(),
                &navigation,
            )
            .unwrap();
        let next = request(&mut f.session, None);
        assert!(matches!(
            f.session.capture_user_command(
                &next,
                &capacity,
                &crate::tree_rename::Retirement::testing(),
                &navigation
            ),
            Err(ProviderError::ResourceLimited)
        ));
        navigation.cancel();
        if revoke {
            let registration = &f.session.registration;
            registration
                .registry
                .unregister(&registration.provider, registration.instance)
                .unwrap();
            assert!(sources.revalidate(&Cancellation::default()).is_err());
        } else {
            drop(f.session);
            assert!(sources.resolve(&Cancellation::default()).is_ok());
            req.cancel.cancel();
            assert!(sources.resolve(&Cancellation::default()).is_err());
        }
        drop(sources);
        wait_retired(&capacity).await;
    }
}

#[test]
fn tree_user_command_capture_rejects_filtered_context_focus_without_exposing_native_values() {
    let mut f = fixture();
    let parent = f
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap()
        .id;
    expand(&mut f.session, parent);
    f.session
        .reproject(
            ProjectionOptions {
                filter: midnight_core::PanelFilter {
                    enabled: true,
                    mode: midnight_core::FilterMode::Expression,
                    expression: "name = \"child\"".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .unwrap();
    let req = request(&mut f.session, Some(parent));
    assert!(
        req.view
            .rows
            .iter()
            .any(|row| row.id == parent && row.context_only)
    );
    assert!(
        f.session
            .capture_user_command(
                &req,
                &Arc::new(Semaphore::new(1)),
                &crate::tree_rename::Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
}

#[test]
fn tree_user_command_provider_resource_refusal_is_terminal_not_admission_retry() {
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let req = request(&mut f.session, Some(row));
    *f.graph.target_failure.lock().unwrap() = Some(2);
    let capacity = Arc::new(Semaphore::new(1));
    assert!(matches!(
        f.session.capture_user_command(
            &req,
            &capacity,
            &crate::tree_rename::Retirement::testing(),
            &Cancellation::default()
        ),
        Err(ProviderError::Failed(_))
    ));
    assert_eq!(capacity.available_permits(), 1);
}

async fn wait_retired(capacity: &Semaphore) {
    let _permit = tokio::time::timeout(std::time::Duration::from_secs(5), capacity.acquire())
        .await
        .expect("capture admission remains held until worker retirement")
        .unwrap();
}

#[test]
fn tree_user_command_cleanup_capacity_refuses_before_provider_capture() {
    let mut f = fixture();
    let req = request(&mut f.session, None);
    let capacity = Arc::new(Semaphore::new(1));
    let retirement = crate::tree_rename::Retirement {
        domain: crate::retirement::RetirementDomain::try_start(1, 0).unwrap(),
    };
    let _occupied = retirement.domain.try_reserve().unwrap();
    let calls = f.graph.target_calls.load(Ordering::SeqCst);
    assert!(matches!(
        f.session
            .capture_user_command(&req, &capacity, &retirement, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    assert_eq!(f.graph.target_calls.load(Ordering::SeqCst), calls);
    assert_eq!(capacity.available_permits(), 1);
}
