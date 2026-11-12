//! Exact tree captures preserve native identity, overlap policy and worker ownership.
use super::*;
use crate::tree_rename::Retirement;
use midnight_core::*;
use std::{fs, path::Path};

fn request(session: &mut Session, fallback: Option<TreeRowId>) -> TreeMetadataRequest {
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    TreeMetadataRequest {
        view: publication.view,
        request: TreeRequestId(session.last_request.0 + 1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        fallback,
        cancel: Cancellation::default(),
    }
}
fn capture(f: &mut Fixture, req: &TreeMetadataRequest) -> Arc<dyn CapturedMetadataSources> {
    f.session
        .capture_metadata(
            req,
            &Arc::new(Semaphore::new(4)),
            &Retirement::testing(),
            &Cancellation::default(),
        )
        .unwrap()
}
fn expanded(f: &mut Fixture) -> (TreeRowId, TreeRowId) {
    let parent = f.session.projection().rows[1].id;
    let publication = super::super::action_capture_tests::expand(&mut f.session, parent);
    (parent, publication.view.rows[2].id)
}

#[test]
fn tree_metadata_capture_preserves_selected_overlap_in_deepest_first_native_order() {
    let mut f = fixture();
    let (parent, child) = expanded(&mut f);
    for row in [parent, child] {
        super::super::action_capture_tests::select(
            &mut f.session,
            TreeSelectionAction::Toggle(row),
        );
    }
    let req = request(&mut f.session, Some(TreeRowId(0)));
    let sources = capture(&mut f, &req);
    assert_eq!(sources.root_count(), 2);
    assert_eq!(sources.covered_roots(), [true, false]);
    assert_eq!(sources.stamp(), req.view.stamp);
    assert_eq!(sources.request(), req.request);
    fs::create_dir_all(f.graph.root_path.join("z-parent/child")).unwrap();
    let pins = sources
        .prepare(
            &midnight_platform::LocalFileSystem::new(None),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        pins.iter()
            .map(|pin| pin.item().path.clone())
            .collect::<Vec<_>>(),
        [
            f.graph.root_path.join("z-parent/child"),
            f.graph.root_path.join("z-parent")
        ]
    );
    assert!(f.graph.root_path.join("z-parent/child").is_dir());
}

#[test]
fn tree_metadata_capture_focused_descendant_survives_navigation_without_using_caption() {
    let mut f = fixture();
    let (_, child) = expanded(&mut f);
    let req = request(&mut f.session, Some(child));
    let sources = capture(&mut f, &req);
    let expected = f.graph.root_path.join("z-parent/child");
    fs::create_dir_all(&expected).unwrap();
    drop(f.session);
    let pins = sources
        .prepare(
            &midnight_platform::LocalFileSystem::new(None),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].item().path, expected);
    assert_eq!(sources.covered_roots(), [false]);
}

#[test]
fn tree_metadata_capture_rejects_forged_view_selection_parent_revision_and_replay() {
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let mut req = request(&mut f.session, Some(row));
    let capacity = Arc::new(Semaphore::new(4));
    let retirement = Retirement::testing();
    let exact = req.view.clone();
    req.view = Arc::new((*exact).clone());
    assert!(
        f.session
            .capture_metadata(&req, &capacity, &retirement, &Cancellation::default())
            .is_err()
    );
    req.view = exact;
    let selected = req.selection.clone();
    req.selection = Arc::new((*selected).clone());
    assert!(
        f.session
            .capture_metadata(&req, &capacity, &retirement, &Cancellation::default())
            .is_err()
    );
    req.selection = selected;
    req.fallback = Some(TreeRowId(0));
    assert!(
        f.session
            .capture_metadata(&req, &capacity, &retirement, &Cancellation::default())
            .is_err()
    );
    req.fallback = Some(row);
    req.selection_revision.0 += 1;
    assert!(
        f.session
            .capture_metadata(&req, &capacity, &retirement, &Cancellation::default())
            .is_err()
    );
    req.selection_revision.0 -= 1;
    let source = f
        .session
        .capture_metadata(&req, &capacity, &retirement, &Cancellation::default())
        .unwrap();
    assert!(
        f.session
            .capture_metadata(&req, &capacity, &retirement, &Cancellation::default())
            .is_err()
    );
    drop(source);
}

#[test]
fn tree_metadata_capture_bound_cancel_and_capacity_fail_before_provider_capture() {
    let mut f = fixture_with_extra_directories(MAX_METADATA_TARGETS);
    let view = f.session.publication().unwrap().view;
    f.session.commit(&view);
    super::super::action_capture_tests::select(
        &mut f.session,
        TreeSelectionAction::Bulk(SelectionOperation::All),
    );
    let req = request(&mut f.session, None);
    let before = f.graph.target_calls.load(Ordering::SeqCst);
    let capacity = Arc::new(Semaphore::new(4));
    assert!(
        f.session
            .capture_metadata(
                &req,
                &capacity,
                &Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    assert_eq!(f.graph.target_calls.load(Ordering::SeqCst), before);
    assert_eq!(capacity.available_permits(), 4);
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let req = request(&mut f.session, Some(row));
    assert!(matches!(
        f.session.capture_metadata(
            &req,
            &Arc::new(Semaphore::new(0)),
            &Retirement::testing(),
            &Cancellation::default()
        ),
        Err(ProviderError::ResourceLimited)
    ));
    req.cancel.cancel();
    assert!(matches!(
        f.session.capture_metadata(
            &req,
            &capacity,
            &Retirement::testing(),
            &Cancellation::default()
        ),
        Err(ProviderError::Cancelled)
    ));
}

struct NoPreparation;
impl MutationService for NoPreparation {
    fn prepare(&self, _: &Path, _: &Cancellation) -> Result<Arc<dyn PreparedMutation>, FsError> {
        panic!("cancelled metadata must not enter native preparation");
    }
}
#[test]
fn tree_metadata_capture_cancelled_preparation_does_not_open_native_targets() {
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let req = request(&mut f.session, Some(row));
    let sources = capture(&mut f, &req);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        sources.prepare(&NoPreparation, &cancel).err().unwrap().kind,
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn tree_metadata_capture_retains_distinct_hard_link_names() {
    let mut f = fixture();
    let first = f.graph.root_path.join("a-parent");
    let second = f.graph.root_path.join("z-parent");
    fs::write(&first, b"same inode with distinct namespace entries").unwrap();
    fs::hard_link(&first, &second).unwrap();
    for row in [
        f.session.projection().rows[0].id,
        f.session.projection().rows[1].id,
    ] {
        super::super::action_capture_tests::select(
            &mut f.session,
            TreeSelectionAction::Toggle(row),
        );
    }
    let req = request(&mut f.session, None);
    let sources = capture(&mut f, &req);
    assert_eq!(sources.covered_roots(), [false, false]);
    let pins = sources
        .prepare(
            &midnight_platform::LocalFileSystem::new(None),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        pins.iter()
            .map(|pin| pin.item().path.clone())
            .collect::<Vec<_>>(),
        [first, second]
    );
}
#[path = "tree_metadata_retirement_tests.rs"]
mod retirement;

#[test]
fn tree_metadata_capture_context_ancestor_is_not_a_mutation_target() {
    let mut f = fixture();
    let (parent, _) = expanded(&mut f);
    f.session
        .reproject(
            ProjectionOptions {
                filter: PanelFilter {
                    enabled: true,
                    mode: FilterMode::Expression,
                    expression: "name = \"child\"".into(),
                    case_sensitive: true,
                },
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .unwrap();
    let req = request(&mut f.session, Some(parent));
    assert!(req.view.rows[0].context_only);
    let capacity = Arc::new(Semaphore::new(1));
    assert!(matches!(
        f.session.capture_metadata(
            &req,
            &capacity,
            &Retirement::testing(),
            &Cancellation::default()
        ),
        Err(ProviderError::Unsupported)
    ));
    assert_eq!(capacity.available_permits(), 1);
}

#[test]
fn tree_metadata_provider_resource_refusal_is_terminal_not_admission_retry() {
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let req = request(&mut f.session, Some(row));
    *f.graph.target_failure.lock().unwrap() = Some(2);
    let capacity = Arc::new(Semaphore::new(1));
    assert!(
        matches!(
            f.session.capture_metadata(
                &req,
                &capacity,
                &Retirement::testing(),
                &Cancellation::default()
            ),
            Err(ProviderError::Failed(_))
        ),
        "provider/target budgets require explicit review, not a permanent capture retry loop"
    );
    assert_eq!(capacity.available_permits(), 1);
}
