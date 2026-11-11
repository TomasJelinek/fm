use super::*;
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
struct Files(Arc<Gate>);
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
        emit(Entry::new(
            EntryId(1),
            path.join("retained.txt"),
            EntryKind::File,
        ));
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
    let mutation = Arc::new(Semaphore::new(1));
    let held = hold_mutation.then(|| mutation.clone().try_acquire_owned().unwrap());
    let listing = Arc::new(Semaphore::new(1));
    let native = NativeProvider::new(Arc::new(Files(gate.clone())));
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
        Arc::default(),
        native,
        None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_expand_completes_on_worker_while_maintenance_waits_for_copy_gate() {
    let (mut fixture, held) = fixture_with_gate(true);
    assert!(!fixture.worker.inflight);
    fixture.receiver.try_recv().unwrap();
    fixture.worker.command(TreeCommand::Branch {
        view: fixture.view.clone(),
        request: TreeRequestId(2),
        row: fixture.view.rows[0].id,
        action: TreeBranchAction::Expand,
    });
    fixture.worker.pump();
    tokio::time::timeout(Duration::from_secs(2), fixture.gate.entered.notified())
        .await
        .unwrap();
    assert!(!fixture.worker.maintenance_inflight);
    fixture.gate.release();
    let completion = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Input::Completion(completion)) = fixture.lane.pop() {
                break completion;
            }
            fixture.lane.wake.notified().await;
        }
    })
    .await
    .unwrap();
    let event = fixture.worker.complete(completion).unwrap();
    let TreeEvent::Published(publication) = event else {
        panic!("Expand must publish its completed listing");
    };
    assert_eq!(publication.view.rows[0].disclosure, TreeDisclosure::Empty);
    let session = fixture.worker.session.as_mut().unwrap();
    assert!(session.commit(&publication.view));
    assert_eq!(
        session.watch_scope().unwrap().unwrap().targets.len(),
        2,
        "confirmed empty directory retains its expansion intent and watch"
    );
    assert_eq!(
        fixture.mutation.available_permits(),
        0,
        "Copy gate remains held through explicit Expand completion"
    );
    assert!(
        fixture
            .worker
            .session
            .as_mut()
            .unwrap()
            .finish_refresh()
            .is_none()
    );
    fixture.receiver.try_recv().unwrap();
    drop(held);
    fixture.worker.pump();
    assert!(fixture.worker.maintenance_inflight);
    let completion = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Input::Completion(completion)) = fixture.lane.pop() {
                break completion;
            }
            fixture.lane.wake.notified().await;
        }
    })
    .await
    .unwrap();
    fixture.worker.complete(completion);
    fixture.worker.pump();
    assert!(
        matches!(fixture.receiver.try_recv().unwrap(), Event::Tree(event) if matches!(*event, TreeEvent::Refreshed { result: Ok(()), .. }))
    );
    assert_eq!(fixture.mutation.available_permits(), 1);
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
    let released =
        tokio::time::timeout(Duration::from_secs(1), mutation.clone().acquire_owned()).await;
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
    assert!(mutation.clone().try_acquire_owned().is_err());
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
    assert_eq!(mutation.available_permits(), 1);
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
