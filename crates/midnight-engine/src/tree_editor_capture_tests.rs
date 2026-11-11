//! Real session admission; provider tokens, not display captions, select the file.
use super::*;
use midnight_core::{CapturedEditorFile, TreeEditorRequest};

fn request(session: &mut Session, row: TreeRowId) -> TreeEditorRequest {
    let view = session.publication().unwrap().view;
    assert!(session.commit(&view));
    TreeEditorRequest {
        view,
        request: TreeRequestId(session.last_request.0 + 1),
        row,
        cancel: Cancellation::default(),
    }
}
fn source(f: &mut Fixture, request: &TreeEditorRequest) -> Arc<dyn CapturedEditorFile> {
    f.session
        .capture_editor(
            request,
            &Arc::new(Semaphore::new(4)),
            &Cancellation::default(),
        )
        .unwrap()
}

#[test]
fn tree_editor_capture_resolves_focused_native_root_file_after_folder_expansion() {
    let mut f = fixture();
    let parent = f
        .session
        .projection()
        .rows
        .iter()
        .find(|r| r.entry.display_name == "z-parent")
        .unwrap()
        .id;
    let publication = super::super::action_capture_tests::expand(&mut f.session, parent);
    assert!(publication.view.rows.iter().any(|row| row.depth > 0));
    super::super::action_capture_tests::select(
        &mut f.session,
        midnight_core::TreeSelectionAction::Toggle(parent),
    );
    let file = f
        .session
        .projection()
        .rows
        .iter()
        .find(|r| r.entry.kind == EntryKind::File)
        .unwrap()
        .id;
    let req = request(&mut f.session, file);
    let captured = source(&mut f, &req);
    assert_eq!(
        captured.resolve(&Cancellation::default()).unwrap(),
        f.graph.root_path.join("file.txt")
    );
    assert_eq!(captured.stamp(), req.view.stamp);
    assert_eq!(captured.row(), file);
}

#[test]
fn tree_editor_capture_rejects_unacknowledged_forged_stale_and_parent_rows() {
    let mut f = fixture();
    let view = f.session.projection().clone();
    let row = view
        .rows
        .iter()
        .find(|r| r.entry.kind == EntryKind::File)
        .unwrap()
        .id;
    let mut req = TreeEditorRequest {
        view,
        request: TreeRequestId(1),
        row,
        cancel: Cancellation::default(),
    };
    let capacity = Arc::new(Semaphore::new(4));
    assert!(
        f.session
            .capture_editor(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    assert!(f.session.commit(&req.view));
    let original = req.view.clone();
    req.view = Arc::new((*original).clone());
    assert!(
        f.session
            .capture_editor(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    req.view = original;
    req.row = TreeRowId(0);
    assert!(
        f.session
            .capture_editor(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    req.row = row;
    let capture = f
        .session
        .capture_editor(&req, &capacity, &Cancellation::default())
        .unwrap();
    assert!(
        f.session
            .capture_editor(&req, &capacity, &Cancellation::default())
            .is_err(),
        "duplicate request rejected"
    );
    assert_eq!(capacity.available_permits(), 3);
    drop(capture);
    assert_eq!(capacity.available_permits(), 4);
}

pub(super) fn one_root(kind: EntryKind) -> Fixture {
    let directory_root = tempfile::tempdir().unwrap();
    let mut row = directory(3, "file.txt");
    row.kind = kind;
    let entries: Arc<[midnight_core::Entry]> = Arc::from([midnight_core::Entry::new(
        EntryId(3),
        directory_root.path().join("file.txt"),
        kind,
    )]);
    let graph = Arc::new(Graph {
        root_path: directory_root.path().to_owned(),
        lists: AtomicUsize::new(0),
        rows: Mutex::new(HashMap::from([(0, vec![row])])),
        listed: Mutex::default(),
        failures: Mutex::default(),
        target_calls: AtomicUsize::new(0),
        target_failure: Mutex::default(),
        blocked: Mutex::default(),
    });
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(8),
        index: Arc::new(midnight_core::DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let session = Session::new(
        root,
        OpenedLocation {
            mount: graph.clone(),
            root: token(0),
        },
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
    )
    .unwrap();
    Fixture {
        session,
        graph,
        _directory: directory_root,
    }
}

#[test]
fn tree_editor_capture_rejects_folders_links_and_special_files() {
    for kind in [
        EntryKind::Directory,
        EntryKind::DirectoryLink,
        EntryKind::Symlink,
        EntryKind::Other,
    ] {
        let mut f = one_root(kind);
        let row = f.session.projection().rows[0].id;
        let req = request(&mut f.session, row);
        assert!(
            matches!(
                f.session.capture_editor(
                    &req,
                    &Arc::new(Semaphore::new(4)),
                    &Cancellation::default()
                ),
                Err(ProviderError::Unsupported)
            ),
            "{kind:?}"
        );
    }
}

#[test]
fn tree_editor_capture_cancellation_navigation_and_provider_revocation_reject_resolution() {
    for cause in 0..3 {
        let mut f = fixture();
        let row = f
            .session
            .projection()
            .rows
            .iter()
            .find(|r| r.entry.kind == EntryKind::File)
            .unwrap()
            .id;
        let req = request(&mut f.session, row);
        let generation = Cancellation::default();
        let captured = f
            .session
            .capture_editor(&req, &Arc::new(Semaphore::new(4)), &generation)
            .unwrap();
        match cause {
            0 => req.cancel.cancel(),
            1 => generation.cancel(),
            _ => {
                let registration = &f.session.registration;
                registration
                    .registry
                    .unregister(&registration.provider, registration.instance)
                    .unwrap();
            }
        }
        assert!(captured.resolve(&Cancellation::default()).is_err());
    }
    let mut f = fixture();
    let row = f
        .session
        .projection()
        .rows
        .iter()
        .find(|r| r.entry.kind == EntryKind::File)
        .unwrap()
        .id;
    let req = request(&mut f.session, row);
    req.cancel.cancel();
    let capacity = Arc::new(Semaphore::new(1));
    assert!(
        f.session
            .capture_editor(&req, &capacity, &Cancellation::default())
            .is_err()
    );
    assert_eq!(capacity.available_permits(), 1);
}

#[test]
fn tree_editor_capture_retains_shared_capacity_and_provider_until_last_owner() {
    let mut f = fixture();
    let row = f
        .session
        .projection()
        .rows
        .iter()
        .find(|r| r.entry.kind == EntryKind::File)
        .unwrap()
        .id;
    let capacity = Arc::new(Semaphore::new(1));
    let req = request(&mut f.session, row);
    let captured = f
        .session
        .capture_editor(&req, &capacity, &Cancellation::default())
        .unwrap();
    let another = request(&mut f.session, row);
    assert!(matches!(
        f.session
            .capture_editor(&another, &capacity, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    let cloned = captured.clone();
    drop(captured);
    assert_eq!(capacity.available_permits(), 0);
    drop(f.session);
    assert!(
        cloned.resolve(&Cancellation::default()).is_ok(),
        "retained registration survives session destruction"
    );
    drop(cloned);
    assert_eq!(capacity.available_permits(), 1);
}
