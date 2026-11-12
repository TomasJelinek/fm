//! Action admission is exercised through the session's real selection and provider boundaries.
use super::*;
use crate::providers::{MountedFilesystem, ProviderEntry, ProviderTarget};
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, EntryMetadata, NavigationId, NodeToken, PaneId,
    PanelFilter, SelectionOperation, TreeSelectionAction,
};
use std::{path::PathBuf, sync::atomic::AtomicUsize};

pub(super) fn select(session: &mut Session, action: TreeSelectionAction) -> TreePublication {
    let current = session.publication().unwrap();
    assert!(session.commit(&current.view));
    session
        .select(&TreeSelectionRequest {
            view: current.view,
            revision: SelectionRevision(current.selection_revision.0 + 1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action,
        })
        .unwrap();
    session.publication().unwrap()
}

fn capture(
    session: &Session,
    publication: &TreePublication,
    fallback: Option<TreeRowId>,
    cancel: &Cancellation,
) -> Result<CapturedSelection, ProviderError> {
    session.capture_selection(
        &publication.view,
        publication.selection_revision,
        &publication.selection,
        fallback,
        cancel,
    )
}

pub(super) fn expand(session: &mut Session, row: TreeRowId) -> TreePublication {
    let view = session.projection().clone();
    assert!(session.commit(&view));
    session
        .branch(&view, TreeRequestId(1), row, BranchAction::Expand)
        .unwrap();
    let job = session.take_work().unwrap().unwrap();
    assert!(session.apply(job.run(), &Cancellation::default()).unwrap());
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    publication
}

#[test]
fn selected_native_ancestor_and_descendant_remain_exact_ordered_occurrences() {
    let mut fixture = tests::fixture();
    let parent = fixture.session.projection().rows[1].id;
    let expanded = expand(&mut fixture.session, parent);
    let child = expanded.view.rows[2].id;
    select(&mut fixture.session, TreeSelectionAction::Toggle(child));
    let selected = select(&mut fixture.session, TreeSelectionAction::Toggle(parent));
    let batch = capture(&fixture.session, &selected, None, &Cancellation::default()).unwrap();
    assert!(Arc::ptr_eq(&batch.view, &selected.view));
    assert!(Arc::ptr_eq(&batch._selection, &selected.selection));
    assert_eq!(
        batch
            .targets
            .iter()
            .map(|item| item.row)
            .collect::<Vec<_>>(),
        [parent, child]
    );
    let ProviderTarget::Native(parent_path) = batch.targets[0].captured.target.value().unwrap()
    else {
        panic!("native parent expected");
    };
    let ProviderTarget::Native(child_path) = batch.targets[1].captured.target.value().unwrap()
    else {
        panic!("native child expected");
    };
    assert_eq!(child_path, &parent_path.join("child"));
}

#[test]
fn capture_requires_acknowledged_exact_view_selection_and_revision() {
    let mut fixture = tests::fixture();
    let publication = fixture.session.publication().unwrap();
    let row = publication.view.rows[0].id;
    let cancel = Cancellation::default();
    assert!(matches!(
        capture(&fixture.session, &publication, Some(row), &cancel),
        Err(ProviderError::StaleReference)
    ));
    fixture.session.commit(&publication.view);
    let foreign_view = TreePublication {
        view: Arc::new((*publication.view).clone()),
        ..publication.clone()
    };
    assert!(matches!(
        capture(&fixture.session, &foreign_view, Some(row), &cancel),
        Err(ProviderError::StaleReference)
    ));
    let foreign_selection = TreePublication {
        selection: Arc::new((*publication.selection).clone()),
        ..publication.clone()
    };
    assert!(matches!(
        capture(&fixture.session, &foreign_selection, Some(row), &cancel),
        Err(ProviderError::StaleReference)
    ));
    let foreign_revision = TreePublication {
        selection_revision: SelectionRevision(1),
        ..publication.clone()
    };
    assert!(matches!(
        capture(&fixture.session, &foreign_revision, Some(row), &cancel),
        Err(ProviderError::StaleReference)
    ));
    select(&mut fixture.session, TreeSelectionAction::Toggle(row));
    assert!(matches!(
        capture(&fixture.session, &publication, Some(row), &cancel),
        Err(ProviderError::StaleReference)
    ));
    // Equal bits after another selection change are not the admitted allocation or revision.
    select(&mut fixture.session, TreeSelectionAction::Toggle(row));
    assert!(matches!(
        capture(&fixture.session, &publication, Some(row), &cancel),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn fallback_applies_only_to_empty_selection_and_must_be_visible() {
    let mut fixture = tests::fixture();
    let publication = fixture.session.publication().unwrap();
    fixture.session.commit(&publication.view);
    let row = publication.view.rows[0].id;
    let cancel = Cancellation::default();
    assert!(matches!(
        capture(&fixture.session, &publication, None, &cancel),
        Err(ProviderError::Unsupported)
    ));
    assert!(matches!(
        capture(
            &fixture.session,
            &publication,
            Some(TreeRowId(u64::MAX)),
            &cancel
        ),
        Err(ProviderError::StaleReference)
    ));
    let batch = capture(&fixture.session, &publication, Some(row), &cancel).unwrap();
    assert_eq!(batch.targets.len(), 1);
    assert_eq!(batch.targets[0].row, row);
    assert!(batch._selection.is_empty());
    let selected = select(&mut fixture.session, TreeSelectionAction::Toggle(row));
    let batch = capture(
        &fixture.session,
        &selected,
        Some(TreeRowId(u64::MAX)),
        &cancel,
    )
    .unwrap();
    assert_eq!(batch.targets.len(), 1);
    assert_eq!(batch.targets[0].row, row);
}

#[test]
fn collapsed_descendant_and_context_only_ancestor_are_refused() {
    let mut fixture = tests::fixture();
    let parent = fixture.session.projection().rows[1].id;
    let expanded = expand(&mut fixture.session, parent);
    let child = expanded.view.rows[2].id;
    fixture
        .session
        .branch(
            &expanded.view,
            TreeRequestId(2),
            parent,
            BranchAction::Collapse,
        )
        .unwrap();
    // Even before the display is rebuilt, model visibility blocks a collapsed descendant.
    assert!(matches!(
        capture(
            &fixture.session,
            &expanded,
            Some(child),
            &Cancellation::default()
        ),
        Err(ProviderError::StaleReference)
    ));
    fixture
        .session
        .reproject(ProjectionOptions::default(), &Cancellation::default())
        .unwrap();
    let collapsed = fixture.session.publication().unwrap();
    fixture.session.commit(&collapsed.view);
    assert!(matches!(
        capture(
            &fixture.session,
            &collapsed,
            Some(child),
            &Cancellation::default()
        ),
        Err(ProviderError::StaleReference)
    ));

    let mut fixture = tests::fixture();
    let parent = fixture.session.projection().rows[1].id;
    expand(&mut fixture.session, parent);
    fixture
        .session
        .reproject(
            ProjectionOptions {
                filter: PanelFilter {
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
    let filtered = fixture.session.publication().unwrap();
    fixture.session.commit(&filtered.view);
    assert!(filtered.view.rows[0].context_only);
    assert!(matches!(
        capture(
            &fixture.session,
            &filtered,
            Some(parent),
            &Cancellation::default()
        ),
        Err(ProviderError::Unsupported)
    ));
    assert!(
        capture(
            &fixture.session,
            &filtered,
            Some(filtered.view.rows[1].id),
            &Cancellation::default()
        )
        .is_ok()
    );
}

#[test]
fn admitted_batch_keeps_original_context_after_session_drop_and_releases_last_lease() {
    let mut fixture = tests::fixture();
    let selected = select(
        &mut fixture.session,
        TreeSelectionAction::Bulk(SelectionOperation::All),
    );
    let batch = capture(&fixture.session, &selected, None, &Cancellation::default()).unwrap();
    let old_registration = Arc::downgrade(&fixture.session.registration);
    drop(fixture.session);
    let next = tests::fixture();
    assert_ne!(
        batch.view.stamp.session,
        next.session.projection().stamp.session
    );
    for target in &batch.targets {
        assert!(target.captured.target.value().is_ok());
    }
    drop(batch);
    assert!(old_registration.upgrade().is_none());
}

struct BatchMount {
    root: PathBuf,
    count: usize,
    calls: AtomicUsize,
    fail_at: AtomicUsize,
    cancel_at: Mutex<Option<(usize, Cancellation)>>,
    large_at: AtomicUsize,
}

fn token(value: u64) -> NodeToken {
    NodeToken::new(&value.to_le_bytes()).unwrap()
}

impl MountedFilesystem for BatchMount {
    fn list(
        &self,
        _: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        for index in 1..=self.count {
            let name = format!("item-{index:04}");
            if !emit(ProviderEntry {
                token: token(index as u64),
                raw_name: Arc::from(name.as_bytes()),
                display_name: name,
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            }) {
                break;
            }
        }
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
    fn resolve_target(
        &self,
        node: &NodeToken,
        _: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        if call == self.fail_at.load(Ordering::Relaxed) {
            return Err(ProviderError::Failed("target disappeared".into()));
        }
        if let Some((at, cancel)) = &*self.cancel_at.lock().unwrap()
            && *at == call
        {
            cancel.cancel();
        }
        let index = u64::from_le_bytes(node.as_bytes().try_into().unwrap());
        let mut path = self.root.join(format!("item-{index:04}"));
        if call == self.large_at.load(Ordering::Relaxed) {
            path.reserve(4 * 1024 * 1024);
        }
        Ok(ProviderTarget::Native(path))
    }
}

fn batch_fixture(count: usize) -> (Session, Arc<BatchMount>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let mount = Arc::new(BatchMount {
        root: directory.path().to_owned(),
        count,
        calls: AtomicUsize::new(0),
        fail_at: AtomicUsize::new(0),
        cancel_at: Mutex::new(None),
        large_at: AtomicUsize::new(0),
    });
    let entries: Arc<[Entry]> = (1..=count)
        .map(|index| {
            Entry::new(
                EntryId(index as u64),
                directory.path().join(format!("item-{index:04}")),
                EntryKind::File,
            )
        })
        .collect();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(19),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let session = Session::new(
        root,
        OpenedLocation {
            mount: mount.clone(),
            root: token(0),
        },
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    (session, mount, directory)
}

#[test]
fn failure_drops_previously_captured_targets_without_publishing_a_partial_batch() {
    let (mut session, mount, _directory) = batch_fixture(3);
    let selected = select(
        &mut session,
        TreeSelectionAction::Bulk(SelectionOperation::All),
    );
    let owners = Arc::strong_count(&session.registration);
    mount.fail_at.store(2, Ordering::Relaxed);
    assert!(matches!(
        capture(&session, &selected, None, &Cancellation::default()),
        Err(ProviderError::Failed(_))
    ));
    assert_eq!(mount.calls.load(Ordering::Relaxed), 2);
    assert_eq!(Arc::strong_count(&session.registration), owners);
    assert!(Arc::ptr_eq(
        &session.publication().unwrap().selection,
        &selected.selection
    ));
}

#[test]
fn cancellation_before_during_and_after_last_resolution_releases_every_target() {
    for at in [0, 2, 3] {
        let (mut session, mount, _directory) = batch_fixture(3);
        let selected = select(
            &mut session,
            TreeSelectionAction::Bulk(SelectionOperation::All),
        );
        let owners = Arc::strong_count(&session.registration);
        let cancel = Cancellation::default();
        if at == 0 {
            cancel.cancel();
        } else {
            *mount.cancel_at.lock().unwrap() = Some((at, cancel.clone()));
        }
        assert!(matches!(
            capture(&session, &selected, None, &cancel),
            Err(ProviderError::Cancelled)
        ));
        assert_eq!(mount.calls.load(Ordering::Relaxed), at);
        assert_eq!(Arc::strong_count(&session.registration), owners);
    }
}

#[test]
fn admission_accepts_4096_targets_and_rejects_4097_before_provider_resolution() {
    for count in [4096, 4097] {
        let (mut session, mount, _directory) = batch_fixture(count);
        let selected = select(
            &mut session,
            TreeSelectionAction::Bulk(SelectionOperation::All),
        );
        let result = capture(&session, &selected, None, &Cancellation::default());
        if count == 4096 {
            assert_eq!(result.unwrap().targets.len(), count);
            assert_eq!(mount.calls.load(Ordering::Relaxed), count);
        } else {
            assert!(matches!(result, Err(ProviderError::ResourceLimited)));
            assert_eq!(mount.calls.load(Ordering::Relaxed), 0);
        }
    }
}

#[test]
fn target_allocation_budget_counts_owned_path_capacity_and_releases_prior_targets() {
    let (mut session, mount, _directory) = batch_fixture(3);
    let selected = select(
        &mut session,
        TreeSelectionAction::Bulk(SelectionOperation::All),
    );
    let owners = Arc::strong_count(&session.registration);
    mount.large_at.store(2, Ordering::Relaxed);
    assert!(matches!(
        capture(&session, &selected, None, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    assert_eq!(mount.calls.load(Ordering::Relaxed), 2);
    assert_eq!(Arc::strong_count(&session.registration), owners);
}

#[path = "tree_action_archive_tests.rs"]
mod archive_tests;

#[test]
fn copy_capture_is_global_bounded_cancelled_and_monotonic() {
    let mut fixture = tests::fixture();
    let publication = fixture.session.publication().unwrap();
    fixture.session.commit(&publication.view);
    let capacity = Arc::new(tokio::sync::Semaphore::new(4));
    let mut captures = Vec::new();
    for id in 1..=4 {
        let request = midnight_core::TreeTransferRequest {
            intent: midnight_core::TransferIntent::Copy,
            view: publication.view.clone(),
            request: TreeRequestId(id),
            selection_revision: publication.selection_revision,
            selection: publication.selection.clone(),
            fallback: Some(publication.view.rows[0].id),
            cancel: Cancellation::default(),
        };
        captures.push(
            fixture
                .session
                .capture_transfer(&request, &capacity, &Cancellation::default())
                .unwrap(),
        );
        assert!(
            fixture
                .session
                .capture_transfer(&request, &capacity, &Cancellation::default())
                .is_err()
        );
    }
    let request = midnight_core::TreeTransferRequest {
        intent: midnight_core::TransferIntent::Copy,
        view: publication.view.clone(),
        request: TreeRequestId(5),
        selection_revision: publication.selection_revision,
        selection: publication.selection.clone(),
        fallback: Some(publication.view.rows[0].id),
        cancel: Cancellation::default(),
    };
    assert!(
        fixture
            .session
            .capture_transfer(&request, &capacity, &Cancellation::default())
            .is_err()
    );
    captures.pop();
    request.cancel.cancel();
    assert!(
        fixture
            .session
            .capture_transfer(&request, &capacity, &Cancellation::default())
            .is_err()
    );
    let request = midnight_core::TreeTransferRequest {
        intent: midnight_core::TransferIntent::Copy,
        cancel: Cancellation::default(),
        ..request
    };
    captures.push(
        fixture
            .session
            .capture_transfer(&request, &capacity, &Cancellation::default())
            .unwrap(),
    );
    assert_eq!(capacity.available_permits(), 0);
    drop(captures);
    assert_eq!(capacity.available_permits(), 4);
}

#[test]
fn copied_nested_native_roots_keep_occurrences_and_leases_after_navigation() {
    let mut fixture = tests::fixture();
    let parent = fixture.session.projection().rows[1].id;
    let expanded = expand(&mut fixture.session, parent);
    let child = expanded
        .view
        .rows
        .iter()
        .find(|row| row.depth == 1)
        .unwrap()
        .id;
    let sibling = expanded
        .view
        .rows
        .iter()
        .find(|row| row.depth == 0 && row.id != parent && row.entry.kind == EntryKind::Directory)
        .unwrap()
        .id;
    select(&mut fixture.session, TreeSelectionAction::Toggle(child));
    let selected = select(&mut fixture.session, TreeSelectionAction::Toggle(sibling));
    let capacity = Arc::new(Semaphore::new(4));
    let captured = fixture
        .session
        .capture_transfer(
            &midnight_core::TreeTransferRequest {
                intent: midnight_core::TransferIntent::Copy,
                view: selected.view.clone(),
                request: TreeRequestId(2),
                selection_revision: selected.selection_revision,
                selection: selected.selection.clone(),
                fallback: None,
                cancel: Cancellation::default(),
            },
            &capacity,
            &Cancellation::default(),
        )
        .unwrap();
    let original = capture(&fixture.session, &selected, None, &Cancellation::default()).unwrap();
    for target in &original.targets {
        let ProviderTarget::Native(path) = target.captured.target.value().unwrap() else {
            panic!()
        };
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("payload"), b"nested bytes").unwrap();
    }
    drop(original);
    let registration = Arc::downgrade(&fixture.session.registration);
    let service = midnight_platform::LocalFileSystem::new(None);
    let midnight_core::PreparedCapturedTransfer::Copy(mut roots) = captured
        .sources
        .prepare(&service, &Cancellation::default())
        .unwrap()
    else {
        panic!("expected Copy")
    };
    assert_eq!(captured.root_count, 2);
    assert!(!captured.metadata_available);
    drop(captured);
    drop(fixture.session);
    assert!(registration.upgrade().is_some());
    assert_eq!(capacity.available_permits(), 3);
    let mut descendants = Vec::new();
    while let Some(mut root) = roots.pop_front() {
        assert!(
            root.tree_copy_occurrence()
                .is_some_and(|occurrence| [child, sibling].contains(&occurrence.row))
        );
        let mut child_source = root.next_child(&Cancellation::default()).unwrap().unwrap();
        assert!(child_source.tree_copy_occurrence().is_none());
        let mut bytes = [0; 32];
        let count = child_source
            .read_chunk(&mut bytes, &Cancellation::default())
            .unwrap();
        assert_eq!(&bytes[..count], b"nested bytes");
        descendants.push(child_source);
    }
    assert_eq!(capacity.available_permits(), 3);
    drop(descendants);
    assert_eq!(capacity.available_permits(), 4);
    assert!(registration.upgrade().is_none());
}

#[test]
fn capture_transfer_requires_exact_acknowledged_selection_and_view() {
    let mut fixture = tests::fixture();
    let publication = fixture.session.publication().unwrap();
    let capacity = Arc::new(Semaphore::new(4));
    let request = midnight_core::TreeTransferRequest {
        intent: midnight_core::TransferIntent::Copy,
        view: publication.view.clone(),
        request: TreeRequestId(1),
        selection_revision: publication.selection_revision,
        selection: publication.selection.clone(),
        fallback: Some(publication.view.rows[0].id),
        cancel: Cancellation::default(),
    };
    assert!(
        fixture
            .session
            .capture_transfer(&request, &capacity, &Cancellation::default())
            .is_err()
    );
    fixture.session.commit(&publication.view);
    let foreign = midnight_core::TreeTransferRequest {
        intent: midnight_core::TransferIntent::Copy,
        view: Arc::new((*publication.view).clone()),
        ..request.clone()
    };
    assert!(
        fixture
            .session
            .capture_transfer(&foreign, &capacity, &Cancellation::default())
            .is_err()
    );
    let foreign = midnight_core::TreeTransferRequest {
        intent: midnight_core::TransferIntent::Copy,
        selection: Arc::new((*publication.selection).clone()),
        ..request.clone()
    };
    assert!(
        fixture
            .session
            .capture_transfer(&foreign, &capacity, &Cancellation::default())
            .is_err()
    );
    let foreign = midnight_core::TreeTransferRequest {
        intent: midnight_core::TransferIntent::Copy,
        selection_revision: SelectionRevision(9),
        ..request.clone()
    };
    assert!(
        fixture
            .session
            .capture_transfer(&foreign, &capacity, &Cancellation::default())
            .is_err()
    );
    assert_eq!(capacity.available_permits(), 4);
    assert!(
        fixture
            .session
            .capture_transfer(&request, &capacity, &Cancellation::default())
            .is_ok()
    );
}

#[test]
fn completion_selection_removal_is_bounded_exact_and_idempotent() {
    let mut fixture = tests::fixture();
    let parent = fixture.session.projection().rows[1].id;
    let expanded = expand(&mut fixture.session, parent);
    let child = expanded.view.rows[2].id;
    select(&mut fixture.session, TreeSelectionAction::Toggle(parent));
    select(&mut fixture.session, TreeSelectionAction::Toggle(child));
    let once = select(
        &mut fixture.session,
        TreeSelectionAction::Remove(Arc::from([child])),
    );
    assert_eq!(once.selection.len(), 1);
    assert!(once.selection.contains_row(parent));
    let twice = select(
        &mut fixture.session,
        TreeSelectionAction::Remove(Arc::from([child])),
    );
    assert_eq!(twice.selection.len(), 1);
    for rows in [vec![parent, TreeRowId(u64::MAX)], vec![parent; 65]] {
        assert!(
            fixture
                .session
                .select(&TreeSelectionRequest {
                    view: twice.view.clone(),
                    revision: SelectionRevision(twice.selection_revision.0 + 1),
                    seed: None,
                    seed_revision: SelectionRevision(0),
                    action: TreeSelectionAction::Remove(rows.into()),
                })
                .is_err()
        );
        let current = fixture.session.publication().unwrap();
        assert_eq!(current.selection_revision, twice.selection_revision);
        assert!(Arc::ptr_eq(&current.selection, &twice.selection));
    }
}
