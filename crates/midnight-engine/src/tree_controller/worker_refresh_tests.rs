use super::*;
use crate::transfer_leases::{NAMESPACE_WEIGHT, namespace_gate};
use midnight_core::*;
use std::{
    path::{Path, PathBuf},
    sync::{Condvar, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

#[derive(Default)]
struct Gate {
    entered: Notify,
    released: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
struct Files(Arc<Gate>, bool);
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.0.entered.notify_one();
        let result = self
            .0
            .changed
            .wait_timeout_while(
                self.0.released.lock().unwrap(),
                Duration::from_secs(5),
                |released| !*released,
            )
            .unwrap();
        assert!(!result.1.timed_out(), "fixture provider must be released");
        // Keep the directory stable if maintenance runs before explicit Expand.
        if self.1 && path.ends_with("retained.txt") {
            return Ok(());
        }
        emit(Entry::new(
            EntryId(1),
            path.join("retained.txt"),
            if self.1 {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
        ));
        if !self.1 {
            // A changed root listing, so maintenance publishes a new projection.
            emit(Entry::new(
                EntryId(2),
                path.join("added.txt"),
                EntryKind::File,
            ));
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("complete listing metadata")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 0,
            available: 0,
        })
    }
}

struct Fixture {
    worker: Worker,
    lane: Arc<Mailbox>,
    mutation: Arc<Semaphore>,
    gate: Arc<Gate>,
    receiver: mpsc::Receiver<Event>,
    view: Arc<TreeProjection>,
}
fn fixture() -> Fixture {
    fixture_with_gate(false).0
}
fn fixture_with_gate(hold_mutation: bool) -> (Fixture, Option<OwnedSemaphorePermit>) {
    let lane = Arc::new(Mailbox::default());
    let gate = Arc::new(Gate::default());
    let mutation = namespace_gate();
    let held = hold_mutation.then(|| mutation.clone().try_acquire_owned().unwrap());
    let listing = Arc::new(Semaphore::new(1));
    let native = NativeProvider::new(Arc::new(Files(gate.clone(), hold_mutation)));
    let path: PathBuf = std::env::current_dir()
        .unwrap()
        .join("_build/worker-refresh-fixture");
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        path.join("retained.txt"),
        if hold_mutation {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
    )]
    .into();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let opened = native
        .capture_root(&path, &root.entries, &Cancellation::default())
        .unwrap();
    let (sender, receiver) = mpsc::channel(1);
    let budget = WorkerBudget::new(listing.clone()).with_mutation_gate(mutation.clone());
    let mut session = Session::new(
        root,
        opened,
        Arc::default(),
        budget.clone(),
        ProjectionOptions::default(),
        &Cancellation::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    let view = session.projection().clone();
    session.commit(&view);
    let generation = Generation {
        navigation: NavigationId(1),
        sort: SortSpec::default(),
        cancel: Cancellation::default(),
    };
    lane.reset(generation.clone());
    lane.pop();
    lane.publish(&view);
    let mut worker = Worker::new(
        lane.clone(),
        crate::test_snapshots(Default::default()),
        Services {
            native,
            locale: None,
            reports: crate::ReportArena::testing(),
            rename: crate::tree_rename::Retirement::testing(),
        },
        budget,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        Arc::new(Semaphore::new(4)),
    );
    worker.generation = Some(generation);
    worker.session = Some(session);
    worker
        .session
        .as_mut()
        .unwrap()
        .refresh_loaded(
            &TreeRefreshRequest {
                view: view.clone(),
                request: TreeRequestId(1),
            },
            &Cancellation::default(),
        )
        .unwrap();
    let filler = worker.session.as_mut().unwrap().publication().unwrap();
    worker
        .outbox
        .sender
        .try_send(Event::Tree(Box::new(TreeEvent::Published(filler))))
        .unwrap();
    worker.pump();
    (
        Fixture {
            worker,
            lane,
            mutation,
            gate,
            receiver,
            view,
        },
        held,
    )
}

async fn next_completion(lane: &Mailbox) -> crate::tree_actor::BranchCompletion {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Input::Completion(completion)) = lane.pop() {
                break completion;
            }
            lane.wake.notified().await;
        }
    })
    .await
    .unwrap()
}

fn tree_event(receiver: &mut mpsc::Receiver<Event>) -> TreeEvent {
    match receiver.try_recv().unwrap() {
        Event::Tree(event) => *event,
        other => panic!("tree event expected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_expand_completes_while_maintenance_waits_for_a_copy_share() {
    let (mut fixture, held) = fixture_with_gate(true);
    assert!(!fixture.worker.inflight, "maintenance needs every share");
    assert!(matches!(
        fixture.worker.session.as_ref().unwrap().waiting_on(),
        Wait::Namespace
    ));
    fixture.receiver.try_recv().unwrap();
    fixture.worker.command(TreeCommand::Branch {
        view: fixture.view.clone(),
        request: TreeRequestId(2),
        row: fixture.view.rows[0].id,
        action: TreeBranchAction::Expand,
    });
    fixture.worker.pump();
    assert!(
        fixture.worker.inflight,
        "browsing does not wait for a transfer"
    );
    assert!(!fixture.worker.maintenance_inflight);
    assert_eq!(
        fixture.mutation.available_permits(),
        NAMESPACE_WEIGHT as usize - 1,
        "explicit expansion takes no transfer share"
    );
    let TreeEvent::Published(loading) = tree_event(&mut fixture.receiver) else {
        panic!("explicit expansion publishes its Loading row");
    };
    assert_eq!(loading.view.rows[0].disclosure, TreeDisclosure::Loading);
    tokio::time::timeout(Duration::from_secs(2), fixture.gate.entered.notified())
        .await
        .unwrap();
    fixture.gate.release();
    let completion = next_completion(&fixture.lane).await;
    let Some(TreeEvent::Published(publication)) = fixture.worker.complete(completion) else {
        panic!("completed listing must publish");
    };
    assert_eq!(publication.view.rows[0].disclosure, TreeDisclosure::Empty);
    let session = fixture.worker.session.as_mut().unwrap();
    assert!(session.commit(&publication.view));
    assert_eq!(
        session.watch_scope().unwrap().unwrap().targets.len(),
        2,
        "confirmed empty directory retains its expansion intent and watch"
    );
    fixture.worker.pump();
    assert!(
        !fixture.worker.inflight,
        "maintenance still waits for the copy"
    );
    let second_transfer = fixture.mutation.clone().try_acquire_owned();
    assert!(
        second_transfer.is_ok(),
        "a waiting tree must not reserve namespace shares from later transfers"
    );
    drop(second_transfer);
    assert!(
        fixture
            .worker
            .session
            .as_mut()
            .unwrap()
            .finish_refresh()
            .is_none()
    );
    drop(held);
    fixture.worker.pump();
    assert!(fixture.worker.maintenance_inflight);
    assert_eq!(fixture.mutation.available_permits(), 0);
    let completion = next_completion(&fixture.lane).await;
    if let Some(event) = fixture.worker.complete(completion) {
        assert!(matches!(event, TreeEvent::Published(_)), "{event:?}");
    }
    assert_eq!(
        fixture.mutation.available_permits(),
        NAMESPACE_WEIGHT as usize
    );
    fixture.worker.pump();
    assert!(matches!(
        tree_event(&mut fixture.receiver),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
}

/// Watch probes observe an empty directory; the fixture's gated provider and
/// its `entry_metadata` guard stay private to the model worker.
struct Unobserved;
impl DirectoryService for Unobserved {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Unobserved {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Unobserved {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 0,
            available: 0,
        })
    }
}

/// A production watch controller (15 s polling); nobody reads its hints. Keep the
/// returned resident pool owner alive while the controller runs: dropping it seals
/// the pool that runs the watch worker.
fn watch_controller() -> (
    midnight_work_budget::resident::Owner,
    crate::watch::Controller,
    tokio::task::JoinSet<()>,
    mpsc::Receiver<Event>,
) {
    let domain =
        crate::retirement::RetirementDomain::try_start(crate::watch::RETIREMENT_CAPACITY, 0)
            .unwrap();
    let retirement = crate::watch::Retirement {
        panes: std::array::from_fn(|_| domain.try_reserve().unwrap()),
    };
    let (sender, hints) = mpsc::channel(8);
    let mut tasks = tokio::task::JoinSet::new();
    let (owner, resident) =
        midnight_work_budget::resident::Owner::start("tree-refresh-watch-test", 2).unwrap();
    let watch = crate::watch::Controller::new(
        &resident,
        retirement,
        crate::task_lifetime::TaskLifetime::testing(()),
        Arc::new(Unobserved),
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    (owner, watch, tasks, hints)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unchanged_refresh_loaded_publishes_nothing_and_settles_the_watch_without_commit() {
    // The held share parks the fixture's pass; the root lists unchanged.
    let (mut fixture, held) = fixture_with_gate(true);
    fixture.receiver.try_recv().unwrap(); // Output filler.
    fixture.gate.release();
    drop(held);
    // Finish the fixture's own pass, which started without a size swap.
    fixture.worker.pump();
    assert!(fixture.worker.maintenance_inflight);
    let completion = next_completion(&fixture.lane).await;
    assert!(fixture.worker.complete(completion).is_none());
    fixture.worker.pump();
    assert!(matches!(
        tree_event(&mut fixture.receiver),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    let (_owner, watch, mut tasks, _hints) = watch_controller();
    let handle = watch.handle();
    fixture.worker.watch = Some(handle.clone());
    // The exact Commit installs watch authority for the acknowledged view.
    fixture
        .worker
        .command(TreeCommand::Commit(fixture.view.clone()));
    let session = fixture.worker.session.as_ref().unwrap();
    let (stamp, eligible) = (session.projection().stamp, session.watch_eligibility());
    // A scope diagnostic persists until a complete scope is installed again.
    let failure = TreeDiagnostic::new("fixture scope failure");
    assert!(handle.scope_failed(stamp, eligible.clone(), failure.clone()));
    assert!(!handle.scope_failed(stamp, eligible.clone(), failure.clone()));
    // The production command also installs a new, still empty, size lookup.
    fixture
        .worker
        .command(TreeCommand::RefreshLoaded(TreeRefreshRequest {
            view: fixture.view.clone(),
            request: TreeRequestId(2),
        }));
    fixture.worker.pump();
    assert!(fixture.worker.maintenance_inflight);
    let completion = next_completion(&fixture.lane).await;
    assert!(
        fixture.worker.complete(completion).is_none(),
        "an unchanged listing neither rebuilds nor publishes"
    );
    fixture.worker.pump();
    assert!(matches!(
        tree_event(&mut fixture.receiver),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
    assert!(
        fixture.receiver.try_recv().is_err(),
        "neither a publication nor a watch failure follows"
    );
    assert!(Arc::ptr_eq(
        fixture.worker.session.as_ref().unwrap().projection(),
        &fixture.view
    ));
    // No Commit followed, yet the finished pass installed a complete scope for
    // the acknowledged view after closing its ticket: that installation settles
    // the refresh fence, and it also cleared the diagnostic.
    assert!(
        handle.scope_failed(stamp, eligible, failure),
        "the finished pass reinstalled the complete watch scope"
    );
    watch.shutdown();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("watch workers stop");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observations_after_terminal_session_failure_are_ignored() {
    let mut fixture = fixture();
    fixture.gate.release();
    fixture.receiver.try_recv().unwrap();
    fixture
        .worker
        .fail_session(ProviderError::Failed("fixture failure".into()));
    assert!(matches!(
        tree_event(&mut fixture.receiver),
        TreeEvent::Unavailable { .. }
    ));
    // The UI may still observe the last adopted publication. Answering with a
    // retryable rejection made it commit again in an endless loop.
    for _ in 0..3 {
        let rows = TreeViewport::new(&fixture.view, vec![fixture.view.rows[0].id]).unwrap();
        for observation in [
            TreeCommand::Commit(fixture.view.clone()),
            TreeCommand::Viewport {
                view: fixture.view.clone(),
                rows,
            },
        ] {
            fixture.worker.command(observation);
            assert!(matches!(
                fixture.receiver.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
    }
    // Explicit actions still get their terminal, non-retryable answer.
    fixture.worker.command(selection(fixture.view.clone()));
    assert!(matches!(
        tree_event(&mut fixture.receiver),
        TreeEvent::SelectionFailed { .. }
    ));
}

fn selection(view: Arc<TreeProjection>) -> TreeCommand {
    TreeCommand::Select(TreeSelectionRequest {
        revision: SelectionRevision(1),
        seed: None,
        seed_revision: SelectionRevision(0),
        action: TreeSelectionAction::Toggle(view.rows[0].id),
        view,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_refresh_after_fatal_session_failure_is_terminal() {
    let mut fixture = fixture();
    fixture.gate.release();
    fixture.worker.session = None;
    fixture.receiver.try_recv().unwrap();
    fixture
        .worker
        .command(TreeCommand::RefreshLoaded(TreeRefreshRequest {
            view: fixture.view.clone(),
            request: TreeRequestId(2),
        }));
    assert!(
        matches!(fixture.receiver.try_recv().unwrap(), Event::Tree(event)
        if matches!(*event, TreeEvent::RefreshRejected { retryable: false, .. })),
        "a queued watch hint after fatal teardown must not enter a refresh retry loop"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_outbox_still_applies_maintenance_and_releases_gate_before_selection_reply_drains() {
    let Fixture {
        mut worker,
        lane,
        mutation,
        gate,
        mut receiver,
        view,
    } = fixture();
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    let command = selection(view.clone());
    let handled = tokio::task::spawn_blocking(move || {
        worker.command(command);
        worker
    });
    gate.release();
    let released = tokio::time::timeout(
        Duration::from_secs(1),
        mutation.clone().acquire_many_owned(NAMESPACE_WEIGHT),
    )
    .await;
    let released_before_drain = released.is_ok();
    drop(released);
    // Drain even on RED so no test worker or permit survives the assertion.
    let mut output = Vec::new();
    while !handled.is_finished() {
        if let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), receiver.recv()).await
        {
            output.push(event);
        }
        if !released_before_drain {
            break;
        }
    }
    while let Ok(event) = receiver.try_recv() {
        output.push(event);
    }
    drop(receiver);
    let mut worker = tokio::time::timeout(Duration::from_secs(2), handled)
        .await
        .unwrap()
        .unwrap();
    if let Some(Input::Completion(completion)) = lane.pop() {
        worker.handle(Input::Completion(completion));
    }
    assert!(
        released_before_drain,
        "completed native listing must release the gate without draining unrelated output"
    );
    assert!(
        worker
            .session
            .as_mut()
            .unwrap()
            .publication()
            .unwrap()
            .selection
            .contains_row(view.rows[0].id)
    );
    let changed = output.iter().position(|event| matches!(event, Event::Tree(event) if matches!(event.as_ref(), TreeEvent::SelectionChanged { .. }))).unwrap();
    let refreshed = output.iter().rposition(|event| matches!(event, Event::Tree(event) if matches!(event.as_ref(), TreeEvent::Published(publication) if publication.view.stamp.revision.0 > view.stamp.revision.0))).unwrap();
    assert!(
        changed < refreshed,
        "accepted selection response precedes maintenance publication"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_unblocks_full_output_but_running_provider_keeps_gate_until_return() {
    let Fixture {
        mut worker,
        lane,
        mutation,
        gate,
        receiver,
        view,
    } = fixture();
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    let handled = tokio::task::spawn_blocking(move || {
        worker.command(selection(view));
        worker
    });
    lane.reset(Generation {
        navigation: NavigationId(2),
        sort: SortSpec::default(),
        cancel: Cancellation::default(),
    });
    let mut worker = tokio::time::timeout(Duration::from_secs(2), handled)
        .await
        .unwrap()
        .unwrap();
    assert!(
        mutation
            .clone()
            .try_acquire_many_owned(NAMESPACE_WEIGHT)
            .is_err()
    );
    worker.handle(lane.pop().unwrap());
    assert!(worker.session.is_none());
    gate.release();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Input::Completion(completion)) = lane.pop() {
                worker.handle(Input::Completion(completion));
                break;
            }
            lane.wake.notified().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(mutation.available_permits(), NAMESPACE_WEIGHT as usize);
    assert!(worker.session.is_none());
    drop(receiver);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_observations_superseded_by_completion_do_not_report_errors() {
    let mut fixture = fixture();
    fixture.receiver.try_recv().unwrap(); // Drain the fixture output barrier.
    fixture
        .lane
        .push(TreeCommand::Commit(fixture.view.clone()))
        .unwrap();
    let rows = TreeViewport::new(&fixture.view, vec![fixture.view.rows[0].id]).unwrap();
    fixture
        .lane
        .push(TreeCommand::Viewport {
            view: fixture.view.clone(),
            rows,
        })
        .unwrap();
    fixture.gate.release();
    let completion = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(completion) = fixture.lane.take_completion() {
                break completion;
            }
            fixture.lane.wake.notified().await;
        }
    })
    .await
    .unwrap();
    fixture.worker.complete(completion);
    let newer = fixture
        .worker
        .session
        .as_ref()
        .unwrap()
        .projection()
        .clone();
    assert!(newer.stamp.revision.0 > fixture.view.stamp.revision.0);
    assert!(fixture.lane.publish(&newer));
    // Completion is taken before queued commands; admission-time validation
    // alone cannot prevent an obsolete observation from reaching the worker.
    for _ in 0..2 {
        let Some(Input::Command(command)) = fixture.lane.pop() else {
            panic!("queued observation")
        };
        tokio::task::block_in_place(|| fixture.worker.command(command));
        assert!(matches!(
            fixture.receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
    tokio::task::block_in_place(|| fixture.worker.command(TreeCommand::Commit(newer)));
    assert!(matches!(
        fixture.receiver.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

fn transfer(publication: &TreePublication, request: u64) -> TreeCommand {
    TreeCommand::Transfer(TreeTransferRequest {
        intent: TransferIntent::Copy,
        view: publication.view.clone(),
        request: TreeRequestId(request),
        selection_revision: publication.selection_revision,
        selection: publication.selection.clone(),
        fallback: Some(publication.view.rows[0].id),
        cancel: Cancellation::default(),
    })
}

/// The mailbox admitted F5 for the current view, but a completion taken first
/// published a newer projection: that publication precedes the refusal, which
/// the presenter retries on it. A refusal of the current view is reported, even
/// when only the shared capture pool is busy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfer_refusal_is_retryable_only_when_a_newer_projection_replaced_its_view() {
    let mut fixture = fixture();
    fixture.receiver.try_recv().unwrap(); // Drain the fixture output barrier.
    let admitted = fixture
        .worker
        .session
        .as_mut()
        .unwrap()
        .publication()
        .unwrap();
    fixture.gate.release();
    let completion = next_completion(&fixture.lane).await;
    tokio::task::block_in_place(|| fixture.worker.handle(Input::Completion(completion)));
    let TreeEvent::Published(newer) = tree_event(&mut fixture.receiver) else {
        panic!("the changed listing publishes a newer projection");
    };
    assert!(newer.view.stamp.revision.0 > admitted.view.stamp.revision.0);
    tokio::task::block_in_place(|| fixture.worker.command(transfer(&admitted, 2)));
    let refusal = tree_event(&mut fixture.receiver);
    assert!(
        matches!(&refusal, TreeEvent::Transfer { stamp, retryable: true, result: Err(_), .. }
            if *stamp == admitted.view.stamp),
        "{refusal:?}"
    );
    // The presenter acknowledges the newer view, then sends F5 for it.
    tokio::task::block_in_place(|| {
        fixture
            .worker
            .command(TreeCommand::Commit(newer.view.clone()))
    });
    let pool = fixture.worker.copy_capacity.clone();
    let permits = u32::try_from(pool.available_permits()).unwrap();
    let busy = pool.try_acquire_many_owned(permits).unwrap();
    tokio::task::block_in_place(|| fixture.worker.command(transfer(&newer, 3)));
    let refusal = tree_event(&mut fixture.receiver);
    assert!(
        matches!(&refusal, TreeEvent::Transfer { stamp, retryable: false, result: Err(_), .. }
            if *stamp == newer.view.stamp),
        "{refusal:?}"
    );
    drop(busy);
}

/// The session ended before the worker took F5 for its last view. That refusal
/// is final, unless the panel had already asked for a new session for a changed
/// listing: the new session replaces the view, so the refusal is retryable and
/// the presenter drops the capture silently once that session is shown.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfer_after_the_session_ended_is_retryable_only_while_a_new_one_is_prepared() {
    for re_enabled in [false, true] {
        let mut fixture = fixture();
        fixture.gate.release();
        fixture.receiver.try_recv().unwrap(); // Drain the fixture output barrier.
        let last = fixture
            .worker
            .session
            .as_mut()
            .unwrap()
            .publication()
            .unwrap();
        if re_enabled {
            let root = &last.view.root;
            let relisted: Arc<[Entry]> = root.entries.to_vec().into();
            let enable = TreeCommand::Enable {
                root: TreeRootSnapshot {
                    index: Arc::new(DirectoryIndex::from_entries(&relisted)),
                    entries: relisted,
                    ..root.clone()
                },
                selection_revision: SelectionRevision(0),
                selected: TreeRootSelection::Empty,
            };
            tokio::task::block_in_place(|| fixture.worker.command(enable));
            assert!(
                fixture.worker.pending.is_some(),
                "a new session is prepared"
            );
        }
        tokio::task::block_in_place(|| {
            fixture
                .worker
                .fail_session(ProviderError::Failed("fixture failure".into()))
        });
        assert!(matches!(
            tree_event(&mut fixture.receiver),
            TreeEvent::Unavailable { .. }
        ));
        tokio::task::block_in_place(|| fixture.worker.command(transfer(&last, 2)));
        let refusal = tree_event(&mut fixture.receiver);
        assert!(
            matches!(&refusal, TreeEvent::Transfer { stamp, retryable, result: Err(_), .. }
                if *stamp == last.view.stamp && *retryable == re_enabled),
            "re-enabled {re_enabled}: {refusal:?}"
        );
    }
}

/// Closes the lane even when an assertion fails, so a running worker returns
/// and the test runtime can shut down.
struct CloseOnDrop(Arc<Mailbox>);
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_running_worker_retries_maintenance_without_reserving_transfer_shares() {
    // A transfer holds one namespace share while the fixture's pass is pending.
    let (mut fixture, held) = fixture_with_gate(true);
    fixture.receiver.try_recv().unwrap(); // Output filler.
    fixture.gate.release();
    let retries = Arc::new(Notify::new());
    fixture.worker.namespace_retries = Some(retries.clone());
    let Fixture {
        worker,
        lane,
        mutation,
        mut receiver,
        ..
    } = fixture;
    let closing = CloseOnDrop(lane);
    let running = tokio::task::spawn_blocking(move || worker.run());
    // Without any input the worker keeps retrying admission after bounded waits.
    // A waiter queued on the fair gate would block (after at most one stored
    // lane wakeup) until this very share is released, holding every other share
    // meanwhile.
    for _ in 0..3 {
        tokio::time::timeout(Duration::from_secs(2), retries.notified())
            .await
            .expect("the waiting worker retries namespace admission");
    }
    assert_eq!(
        mutation.available_permits(),
        NAMESPACE_WEIGHT as usize - 1,
        "a waiting tree holds no namespace share"
    );
    let later = mutation.clone().try_acquire_owned();
    assert!(
        later.is_ok(),
        "a later transfer starts while maintenance waits"
    );
    drop(later);
    drop(held);
    let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("maintenance runs once every share is free")
        .unwrap();
    assert!(
        matches!(&event, Event::Tree(tree)
            if matches!(**tree, TreeEvent::Refreshed { result: Ok(()), .. })),
        "{event:?}"
    );
    assert_eq!(mutation.available_permits(), NAMESPACE_WEIGHT as usize);
    drop(closing);
    tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .unwrap()
        .unwrap();
}

/// A pass whose last listing changed a branch publishes that view only after
/// the UI acknowledged the previous one. Its Refreshed reply must not overtake
/// the parked view: the UI ends a file job's input hold on the reply and moves
/// the cursor to a renamed entry only once a view shows it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refreshed_never_overtakes_a_publication_of_its_pass_parked_behind_a_commit() {
    let mut fixture = fixture();
    fixture.receiver.try_recv().unwrap(); // Output filler.
    fixture.gate.release();
    let completion = next_completion(&fixture.lane).await;
    let Some(TreeEvent::Published(publication)) = fixture.worker.complete(completion) else {
        panic!("the changed root listing publishes");
    };
    let renamed = publication.view.clone();
    // The UI has not acknowledged the view emitted before this pass's last
    // listing, so that listing's view waits behind it.
    fixture.worker.awaiting_commit = Some(fixture.view.clone());
    fixture.worker.emit(TreeEvent::Published(publication));
    assert!(fixture.worker.pending_publication.is_some(), "parked");
    // The pass is complete; a notice emitted with a full outbox can reach this
    // point of `pump` while the view is still parked.
    fixture.worker.finish_refresh_pass();
    assert!(
        matches!(
            fixture.receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ),
        "the reply waits for the parked view"
    );
    // Acknowledging the earlier view sends the parked one; the next pump replies.
    fixture
        .worker
        .command(TreeCommand::Commit(fixture.view.clone()));
    assert!(
        matches!(tree_event(&mut fixture.receiver), TreeEvent::Published(published)
            if Arc::ptr_eq(&published.view, &renamed)),
        "the parked view is sent first"
    );
    fixture.worker.pump();
    assert!(matches!(
        tree_event(&mut fixture.receiver),
        TreeEvent::Refreshed { result: Ok(()), .. }
    ));
}
