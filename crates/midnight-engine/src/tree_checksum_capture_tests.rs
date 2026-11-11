use super::super::action_capture_tests::{expand, select};
use super::*;
use midnight_core::{TreeChecksumRequest, TreeSelectionAction};

fn request(session: &mut Session, fallback: Option<TreeRowId>) -> TreeChecksumRequest {
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    TreeChecksumRequest {
        view: publication.view,
        request: TreeRequestId(session.last_request.0 + 1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        fallback,
        cancel: Cancellation::default(),
    }
}
fn file(f: &Fixture) -> TreeRowId {
    f.session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.kind == EntryKind::File)
        .unwrap()
        .id
}

#[test]
fn checksum_capture_selected_nested_folder_and_root_file_preserves_provider_order() {
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
    let file = file(&f);
    select(&mut f.session, TreeSelectionAction::Toggle(child));
    select(&mut f.session, TreeSelectionAction::Toggle(file));
    let req = request(&mut f.session, Some(TreeRowId(u64::MAX)));
    let captured = f
        .session
        .capture_checksums(&req, &Arc::new(Semaphore::new(4)), &Cancellation::default())
        .unwrap();
    assert!(
        captured.fallback.is_none(),
        "marks override even an invalid fallback"
    );
    assert_eq!(captured.directory, f.graph.root_path);
    let targets = captured.sources.resolve(&Cancellation::default()).unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|t| (&t.path, t.kind))
            .collect::<Vec<_>>(),
        vec![
            (
                &f.graph.root_path.join("z-parent/child"),
                Some(EntryKind::Directory)
            ),
            (&f.graph.root_path.join("file.txt"), Some(EntryKind::File)),
        ]
    );
}

#[test]
fn checksum_capture_fallback_is_regular_file_only_and_requires_exact_acknowledgement() {
    let mut f = fixture();
    let row = file(&f);
    let mut req = request(&mut f.session, Some(row));
    let capacity = Arc::new(Semaphore::new(1));
    let original = req.view.clone();
    req.view = Arc::new((*original).clone());
    assert!(
        f.session
            .capture_checksums(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    req.view = original;
    let original_selection = req.selection.clone();
    req.selection = Arc::new((*original_selection).clone());
    assert!(
        f.session
            .capture_checksums(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    req.selection = original_selection;
    for fallback in [None, Some(TreeRowId(0)), Some(req.view.rows[0].id)] {
        req.fallback = fallback;
        assert!(
            f.session
                .capture_checksums(&req, &capacity, &Cancellation::default())
                .is_err()
        );
        assert_eq!(capacity.available_permits(), 1);
    }
    req.fallback = Some(row);
    let captured = f
        .session
        .capture_checksums(&req, &capacity, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.fallback, Some(f.graph.root_path.join("file.txt")));
    assert!(
        f.session
            .capture_checksums(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    drop(captured);
    assert_eq!(capacity.available_permits(), 1);
}

#[test]
fn checksum_capture_rejects_links_special_rows_and_filtered_context() {
    for kind in [
        EntryKind::DirectoryLink,
        EntryKind::Symlink,
        EntryKind::Other,
    ] {
        let mut f = super::editor_capture_tests::one_root(kind);
        let row = f.session.projection().rows[0].id;
        select(&mut f.session, TreeSelectionAction::Toggle(row));
        let req = request(&mut f.session, Some(row));
        assert!(matches!(
            f.session.capture_checksums(
                &req,
                &Arc::new(Semaphore::new(1)),
                &Cancellation::default()
            ),
            Err(ProviderError::Unsupported)
        ));
    }
    let mut f = fixture();
    let parent = f.session.projection().rows[1].id;
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
    assert!(req.view.rows[0].context_only);
    assert!(matches!(
        f.session
            .capture_checksums(&req, &Arc::new(Semaphore::new(1)), &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn checksum_capture_survives_navigation_but_not_cancel_or_provider_revocation() {
    for revoke in [false, true] {
        let mut f = fixture();
        let row = file(&f);
        let req = request(&mut f.session, Some(row));
        let capacity = Arc::new(Semaphore::new(1));
        let navigation = Cancellation::default();
        let captured = f
            .session
            .capture_checksums(&req, &capacity, &navigation)
            .unwrap();
        let next = request(&mut f.session, Some(row));
        assert!(matches!(
            f.session.capture_checksums(&next, &capacity, &navigation),
            Err(ProviderError::ResourceLimited)
        ));
        navigation.cancel();
        if revoke {
            let registration = &f.session.registration;
            registration
                .registry
                .unregister(&registration.provider, registration.instance)
                .unwrap();
            assert!(captured.sources.resolve(&Cancellation::default()).is_err());
        } else {
            drop(f.session);
            assert!(captured.sources.resolve(&Cancellation::default()).is_ok());
            req.cancel.cancel();
            assert!(captured.sources.resolve(&Cancellation::default()).is_err());
        }
        assert_eq!(capacity.available_permits(), 0);
        drop(captured);
        assert_eq!(capacity.available_permits(), 1);
    }
}
