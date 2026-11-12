use super::*;
use crate::providers::{MountedFilesystem, ProviderEntry, ProviderTarget};
use midnight_core::{EntryId, EntryKind, EntryMetadata, NavigationId, NodeToken, PaneId};
use std::{collections::HashMap, path::PathBuf, sync::atomic::AtomicUsize};

#[path = "tree_checksum_capture_tests.rs"]
mod checksum_capture_tests;
#[path = "tree_delete_capture_tests.rs"]
mod delete_capture_tests;
#[path = "tree_editor_capture_tests.rs"]
mod editor_capture_tests;
#[path = "tree_rename_capture_tests.rs"]
mod rename_capture_tests;

#[path = "tree_refresh_tests.rs"]
mod refresh_tests;

struct Graph {
    root_path: PathBuf,
    lists: AtomicUsize,
    rows: Mutex<HashMap<u64, Vec<ProviderEntry>>>,
    listed: Mutex<Vec<u64>>,
    failures: Mutex<std::collections::HashSet<u64>>,
    target_calls: AtomicUsize,
    target_failure: Mutex<Option<u64>>,
    blocked: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
}

fn token(value: u64) -> NodeToken {
    NodeToken::new(&value.to_le_bytes()).unwrap()
}

fn directory(value: u64, name: &str) -> ProviderEntry {
    ProviderEntry {
        token: token(value),
        raw_name: Arc::from(name.as_bytes()),
        display_name: name.into(),
        kind: EntryKind::Directory,
        metadata: EntryMetadata::default(),
    }
}

impl MountedFilesystem for Graph {
    fn list(
        &self,
        node: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        self.lists.fetch_add(1, Ordering::Relaxed);
        let value = u64::from_le_bytes(node.as_bytes().try_into().unwrap());
        self.listed.lock().unwrap().push(value);
        if self.failures.lock().unwrap().contains(&value) {
            return Err(ProviderError::Failed("fixture listing denied".into()));
        }
        if value == 1
            && let Some((entered, release)) = self.blocked.lock().unwrap().as_ref()
        {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
        for row in self.rows.lock().unwrap().get(&value).into_iter().flatten() {
            if !emit(row.clone()) {
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
        self.target_calls.fetch_add(1, Ordering::SeqCst);
        let value = u64::from_le_bytes(node.as_bytes().try_into().unwrap());
        if *self.target_failure.lock().unwrap() == Some(value) {
            return Err(ProviderError::ResourceLimited);
        }
        let path = match value {
            0 => self.root_path.clone(),
            1 => self.root_path.join("z-parent"),
            2 => self.root_path.join("a-parent"),
            3 => self.root_path.join("file.txt"),
            4 => self.root_path.join("z-parent").join("child"),
            5 => self.root_path.join("new.txt"),
            6 => self
                .root_path
                .join("z-parent")
                .join("child")
                .join("grandchild"),
            7 => self.root_path.join("z-parent").join("keep"),
            value if value >= 10 => self.root_path.join(format!("b-folder-{:03}", value - 10)),
            _ => return Err(ProviderError::StaleReference),
        };
        Ok(ProviderTarget::Native(path))
    }
}

pub(super) struct Fixture {
    pub(super) session: Session,
    graph: Arc<Graph>,
    _directory: tempfile::TempDir,
}

pub(super) fn fixture() -> Fixture {
    fixture_with_extra_directories(0)
}

fn fixture_with_extra_directories(extra: usize) -> Fixture {
    fixture_with_directory_kind(extra, EntryKind::Directory)
}

pub(super) fn fixture_with_directory_kind(extra: usize, kind: EntryKind) -> Fixture {
    let directory_root = tempfile::tempdir().unwrap();
    let mut file = directory(3, "file.txt");
    file.kind = EntryKind::File;
    let mut roots = vec![directory(1, "z-parent"), directory(2, "a-parent"), file];
    roots[0].kind = kind;
    roots.extend((0..extra).map(|i| directory(i as u64 + 10, &format!("b-folder-{i:03}"))));
    let entries: Arc<[midnight_core::Entry]> = roots
        .iter()
        .map(|row| {
            midnight_core::Entry::new(
                EntryId(u64::from_le_bytes(row.token.as_bytes().try_into().unwrap())),
                directory_root.path().join(&row.display_name),
                row.kind,
            )
        })
        .collect::<Vec<_>>()
        .into();
    let graph = Arc::new(Graph {
        root_path: directory_root.path().to_owned(),
        lists: AtomicUsize::new(0),
        rows: Mutex::new(HashMap::from([
            (0, roots),
            (1, vec![directory(4, "child")]),
        ])),
        blocked: Mutex::new(None),
        listed: Mutex::default(),
        failures: Mutex::default(),
        target_calls: AtomicUsize::new(0),
        target_failure: Mutex::default(),
    });
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(7),
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
        crate::ReportArena::testing(),
    )
    .unwrap();
    Fixture {
        session,
        graph,
        _directory: directory_root,
    }
}

#[test]
fn captured_root_is_consumed_once_preserves_flat_arc_and_sorts_siblings() {
    let mut fixture = fixture();
    let first = fixture.session.projection().clone();
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), 1);
    assert_eq!(
        first
            .rows
            .iter()
            .map(|row| row.entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["a-parent", "z-parent", "file.txt"]
    );
    assert_eq!(first.root.entries[0].display_name, "z-parent");
    fixture
        .session
        .reproject(ProjectionOptions::default(), &Cancellation::default())
        .unwrap();
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), 1);
    assert!(Arc::ptr_eq(
        &first.root.entries,
        &fixture.session.projection().root.entries
    ));
}

#[test]
fn acknowledgments_and_expand_require_the_exact_current_published_basis() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    let row = view.rows[0].id;
    assert!(
        fixture
            .session
            .branch(&view, TreeRequestId(1), row, BranchAction::Expand)
            .is_err()
    );
    assert!(!fixture.session.commit(&Arc::new((*view).clone())));
    assert!(fixture.session.commit(&view));
    assert!(
        fixture
            .session
            .branch(
                &Arc::new((*view).clone()),
                TreeRequestId(1),
                row,
                BranchAction::Expand
            )
            .is_err()
    );
    fixture
        .session
        .branch(&view, TreeRequestId(1), row, BranchAction::Expand)
        .unwrap();
    fixture
        .session
        .reproject(ProjectionOptions::default(), &Cancellation::default())
        .unwrap();
    assert!(!fixture.session.commit(&view));
    assert!(
        fixture
            .session
            .branch(&view, TreeRequestId(2), row, BranchAction::Expand)
            .is_err()
    );
}

#[test]
fn session_identity_exhaustion_never_wraps_or_reuses_a_value() {
    let ids = AtomicU64::new(u64::MAX - 1);
    assert_eq!(next_session(&ids).unwrap().0, u64::MAX - 1);
    assert_eq!(next_session(&ids), Err(ProviderError::ResourceLimited));
    assert_eq!(next_session(&ids), Err(ProviderError::ResourceLimited));
}

#[test]
fn branch_queue_coalesces_and_explicit_expansion_precedes_viewport_probe() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let probe = view.rows[0].id;
    let expand = view.rows[1].id;
    fixture
        .session
        .viewport(&view, &[probe, expand, probe])
        .unwrap();
    fixture
        .session
        .branch(&view, TreeRequestId(1), expand, BranchAction::Expand)
        .unwrap();
    assert_eq!(fixture.session.queued(), 2);
    let work = fixture.session.take_work().unwrap().unwrap();
    assert_eq!(work.row(), expand);
    assert_eq!(work.request(), TreeRequestId(1));
    assert!(fixture.session.take_work().unwrap().is_none());
    assert!(
        fixture
            .session
            .apply(work.run(), &Cancellation::default())
            .unwrap()
    );
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), 2);
    let names = fixture
        .session
        .projection()
        .rows
        .iter()
        .map(|row| row.entry.display_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["a-parent", "z-parent", "child", "file.txt"]);
}

#[test]
fn viewport_larger_than_queue_completes_in_bounded_batches() {
    let mut fixture = fixture_with_extra_directories(40);
    let mut completed = 0;
    loop {
        let view = fixture.session.projection().clone();
        fixture.session.commit(&view);
        let visible = view.rows.iter().map(|row| row.id).collect::<Vec<_>>();
        fixture.session.viewport(&view, &visible).unwrap();
        assert!(fixture.session.queued() <= QUEUE_CAPACITY);
        let Some(job) = fixture.session.take_work().unwrap() else {
            break;
        };
        fixture
            .session
            .apply(job.run(), &Cancellation::default())
            .unwrap();
        completed += 1;
        assert!(
            completed <= 42,
            "each visible directory is probed only once"
        );
    }
    assert_eq!(completed, 42);
    assert!(fixture.session.projection().rows.iter().all(|row| {
        row.disclosure != midnight_core::TreeDisclosure::Unknown
            && row.disclosure != midnight_core::TreeDisclosure::Failed
    }));
}

#[test]
fn expansion_displaces_a_background_probe_when_the_queue_is_full() {
    let mut fixture = fixture_with_extra_directories(20);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let visible = view
        .rows
        .iter()
        .take(QUEUE_CAPACITY)
        .map(|row| row.id)
        .collect::<Vec<_>>();
    fixture.session.viewport(&view, &visible).unwrap();
    let row = view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap()
        .id;
    fixture
        .session
        .branch(&view, TreeRequestId(1), row, BranchAction::Expand)
        .unwrap();
    assert_eq!(fixture.session.queued(), QUEUE_CAPACITY);
    let job = fixture.session.take_work().unwrap().unwrap();
    assert_eq!(job.row(), row);
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
}

#[test]
fn viewport_checks_do_not_displace_explicit_work_or_expand_the_queue_limit() {
    let mut fixture = fixture_with_extra_directories(20);
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    for (i, row) in view.rows.iter().take(QUEUE_CAPACITY).enumerate() {
        fixture
            .session
            .branch(
                &view,
                TreeRequestId(i as u64 + 1),
                row.id,
                BranchAction::Expand,
            )
            .unwrap();
    }
    let visible = view.rows.iter().map(|row| row.id).collect::<Vec<_>>();
    fixture.session.viewport(&view, &visible).unwrap();
    assert_eq!(fixture.session.queued(), QUEUE_CAPACITY);
    assert_eq!(
        fixture.session.branch(
            &view,
            TreeRequestId(9),
            view.rows[8].id,
            BranchAction::Expand
        ),
        Err(ProviderError::ResourceLimited)
    );
    assert_eq!(
        fixture.session.take_work().unwrap().unwrap().row(),
        view.rows[0].id
    );
}

#[test]
fn admitted_target_lease_survives_session_navigation_then_revokes_on_release() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let capture = fixture
        .session
        .capture_target(&view, view.rows[0].id, &Cancellation::default())
        .unwrap();
    drop(fixture.session);
    assert!(capture.target.value().is_ok());
    let target = capture.target;
    drop(capture._registration);
    assert!(matches!(target.value(), Err(ProviderError::StaleReference)));
}

#[tokio::test]
async fn cancelled_waiter_and_session_keep_worker_slots_until_the_blocked_call_finishes() {
    let mut fixture = fixture();
    let budget = fixture.session.budget.clone();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    fixture
        .session
        .branch(
            &view,
            TreeRequestId(1),
            view.rows[1].id,
            BranchAction::Expand,
        )
        .unwrap();
    let work = fixture.session.take_work().unwrap().unwrap();
    let (entered_send, entered) = std::sync::mpsc::channel();
    let (release, release_receive) = std::sync::mpsc::channel();
    *fixture.graph.blocked.lock().unwrap() = Some((entered_send, release_receive));
    let worker = tokio::task::spawn_blocking(move || work.run());
    let waiter = tokio::spawn(worker);
    entered
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    waiter.abort();
    drop(fixture.session);
    assert_eq!(
        budget.listing.available_permits(),
        1,
        "navigation retains its other slot"
    );
    assert!(
        budget.acquire(false).unwrap().is_none(),
        "cancelled call must retain the branch slot"
    );
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if budget.listing.available_permits() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(budget.acquire(false).unwrap().is_some());
}

#[test]
fn collapse_discards_a_late_branch_reply_and_prunes_its_queue() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    let row = view.rows[1].id;
    fixture
        .session
        .branch(&view, TreeRequestId(1), row, BranchAction::Expand)
        .unwrap();
    let work = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .branch(&view, TreeRequestId(2), row, BranchAction::Collapse)
        .unwrap();
    assert!(
        !fixture
            .session
            .apply(work.run(), &Cancellation::default())
            .unwrap()
    );
    fixture
        .session
        .reproject(ProjectionOptions::default(), &Cancellation::default())
        .unwrap();
    assert_eq!(fixture.session.projection().rows.len(), 3);
    assert!(!fixture.session.projection().rows[1].expanded);
    assert_eq!(fixture.session.queued(), 0);
}

#[test]
fn disclosure_update_preserves_layout_while_expansion_changes_it() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    fixture
        .session
        .branch(
            &view,
            TreeRequestId(1),
            view.rows[1].id,
            BranchAction::Probe,
        )
        .unwrap();
    let probe = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(probe.run(), &Cancellation::default())
        .unwrap();
    let collapsed = fixture.session.projection().clone();
    assert_eq!(collapsed.layout_revision, view.layout_revision);
    assert_eq!(
        collapsed.rows[1].disclosure,
        midnight_core::TreeDisclosure::Collapsed
    );
    fixture.session.commit(&collapsed);
    fixture
        .session
        .branch(
            &collapsed,
            TreeRequestId(2),
            collapsed.rows[1].id,
            BranchAction::Expand,
        )
        .unwrap();
    let expand = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(expand.run(), &Cancellation::default())
        .unwrap();
    assert_ne!(
        fixture.session.projection().layout_revision,
        collapsed.layout_revision
    );
}

#[test]
fn branch_result_cannot_cross_to_a_replacement_session_with_the_same_navigation() {
    let mut old = fixture();
    let view = old.session.projection().clone();
    old.session.commit(&view);
    old.session
        .branch(
            &view,
            TreeRequestId(1),
            view.rows[1].id,
            BranchAction::Expand,
        )
        .unwrap();
    let work = old.session.take_work().unwrap().unwrap();
    let mut replacement = fixture();
    assert_ne!(
        replacement.session.projection().stamp.session,
        view.stamp.session
    );
    assert!(
        !replacement
            .session
            .apply(work.run(), &Cancellation::default())
            .unwrap()
    );
    assert_eq!(replacement.session.projection().rows.len(), 3);
}

#[test]
fn expanding_an_empty_directory_publishes_confirmed_empty_without_a_false_expanded_affordance() {
    let mut fixture = fixture();
    let view = fixture.session.projection().clone();
    fixture.session.commit(&view);
    fixture
        .session
        .branch(
            &view,
            TreeRequestId(1),
            view.rows[0].id,
            BranchAction::Expand,
        )
        .unwrap();
    let work = fixture.session.take_work().unwrap().unwrap();
    assert!(
        fixture
            .session
            .apply(work.run(), &Cancellation::default())
            .unwrap()
    );
    let row = &fixture.session.projection().rows[0];
    assert_eq!(row.disclosure, midnight_core::TreeDisclosure::Empty);
    assert!(!row.expanded);
}

#[test]
fn tree_limit_rejection_preserves_the_complete_flat_listing_before_provider_work() {
    let fixture = fixture();
    let entries: Arc<[_]> = (0..10_000)
        .map(|id| {
            midnight_core::Entry::new(
                EntryId(id),
                fixture._directory.path().join(format!("file-{id}")),
                EntryKind::File,
            )
        })
        .collect::<Vec<_>>()
        .into();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(8),
        index: Arc::new(midnight_core::DirectoryIndex::from_entries(&entries)),
        entries: entries.clone(),
    };
    let retained = root.clone();
    let result = Session::new(
        root,
        OpenedLocation {
            mount: fixture.graph.clone(),
            root: token(0),
        },
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
        crate::ReportArena::testing(),
    );
    assert!(matches!(result, Err(ProviderError::ResourceLimited)));
    assert_eq!(fixture.graph.lists.load(Ordering::Relaxed), 1);
    assert_eq!(retained.entries.len(), 10_000);
    assert!(Arc::ptr_eq(&retained.entries, &entries));
}

#[test]
fn filtering_retains_unselectable_ancestor_context_and_rebases_selection_by_occurrence() {
    let mut fixture = fixture();
    let first = fixture.session.projection().clone();
    fixture.session.commit(&first);
    fixture
        .session
        .branch(
            &first,
            TreeRequestId(1),
            first.rows[1].id,
            BranchAction::Expand,
        )
        .unwrap();
    let work = fixture.session.take_work().unwrap().unwrap();
    fixture
        .session
        .apply(work.run(), &Cancellation::default())
        .unwrap();
    let expanded = fixture.session.projection().clone();
    let selected =
        midnight_core::TreeSelectionSnapshot::from_bits(expanded.clone(), vec![0b1111]).unwrap();
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
    let filtered = fixture.session.projection().clone();
    assert_eq!(
        filtered
            .rows
            .iter()
            .map(|row| row.entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["z-parent", "child"]
    );
    assert!(filtered.rows[0].context_only);
    assert!(!filtered.rows[0].selectable);
    assert!(filtered.rows[1].selectable);
    assert_ne!(expanded.layout_revision, filtered.layout_revision);
    let selected = selected.rebase(filtered.clone()).unwrap();
    assert_eq!(selected.len(), 1);
    assert!(selected.contains_row(filtered.rows[1].id));
    fixture.session.commit(&filtered);
    assert!(matches!(
        fixture
            .session
            .capture_target(&filtered, filtered.rows[0].id, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}
