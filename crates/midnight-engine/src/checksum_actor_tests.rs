use super::*;
#[path = "checksum_capture_actor_tests.rs"]
mod captured;
use std::sync::{
    Condvar,
    atomic::{AtomicBool, Ordering},
};

struct Barrier {
    entered: tokio::sync::Notify,
    released: Mutex<bool>,
    wake: Condvar,
    returned: AtomicBool,
}
impl Barrier {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            released: Mutex::new(false),
            wake: Condvar::new(),
            returned: AtomicBool::new(false),
        })
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
struct ReleaseOnDrop(Arc<Barrier>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}
struct Provider(Option<Arc<Barrier>>);
impl TransferService for Provider {
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(invalid("No source in this export test."))
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        if let Some(barrier) = &self.0 {
            barrier.entered.notify_one();
            let mut released = barrier.released.lock().unwrap();
            while !*released {
                released = barrier.wake.wait(released).unwrap();
            }
            barrier.returned.store(true, Ordering::Release);
        }
        Err(invalid("Deterministic native boundary failure."))
    }
}
fn retained(path: std::path::PathBuf) -> Arc<ChecksumRun> {
    let retention = ChecksumRetention::new(());
    Arc::new(ChecksumRun {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(1),
        algorithms: ChecksumAlgorithms::default(),
        captured_roots: vec![path.clone()].into(),
        rows: vec![Arc::new(ChecksumRow {
            id: ChecksumRowId(1),
            path,
            identity: None,
            kind: Some(EntryKind::File),
            state: ChecksumRowState::Calculated,
            bytes_read: 3,
            digests: vec![ChecksumDigest {
                algorithm: ChecksumAlgorithm::Sha256,
                bytes: vec![1; 32],
            }],
            error: None,
            retention: retention.clone(),
        })]
        .into(),
        summary: ChecksumSummary {
            completion: ChecksumCompletion::Completed,
            progress: ChecksumProgress {
                discovered: 1,
                calculated: 1,
                ..Default::default()
            },
            reached_budgets: vec![],
            diagnostics: vec![],
        },
        retention,
    })
}
fn setup(
    gate: Arc<Semaphore>,
    barrier: Option<Arc<Barrier>>,
) -> (Controller, mpsc::Receiver<Event>, std::path::PathBuf) {
    let (sender, events) = mpsc::channel(32);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let controller = Controller::new(
        Retirement::testing(),
        Some(Arc::new(Provider(barrier))),
        outbox,
        gate,
    );
    let directory = std::env::current_dir().unwrap().join("_build/tmp");
    controller.state.lock().unwrap().jobs.insert(
        ChecksumJobId(1),
        Job {
            generation: ChecksumGeneration(1),
            cancel: controller.lifetime.child(),
            result: Some(retained(directory.join("source"))),
        },
    );
    (controller, events, directory)
}
fn request(directory: &Path, id: u64) -> ChecksumManifestRequest {
    ChecksumManifestRequest {
        algorithm: ChecksumAlgorithm::Sha256,
        destination: directory.join(format!("test-{id}.sha256")),
        acknowledge_partial: None,
    }
}
async fn terminal(
    events: &mut mpsc::Receiver<Event>,
    id: u64,
) -> Result<ChecksumManifestReceipt, FsError> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Event::ChecksumManifest(event)) = events.recv().await
                && event.save_id == ChecksumManifestId(id)
                && let ChecksumManifestUpdate::Finished(result) = event.update
            {
                return result;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn two_pending_gate_waiters_bound_admission_and_cancel_save_preserves_results() {
    let (controller, mut events, directory) = setup(Arc::new(Semaphore::new(0)), None);
    let key = (ChecksumJobId(1), ChecksumGeneration(1));
    controller
        .save(ChecksumManifestId(1), key, request(&directory, 1))
        .unwrap();
    controller
        .save(ChecksumManifestId(2), key, request(&directory, 2))
        .unwrap();
    assert_eq!(
        controller
            .save(ChecksumManifestId(3), key, request(&directory, 3))
            .unwrap_err()
            .kind,
        io::ErrorKind::WouldBlock
    );
    controller.cancel_save(ChecksumManifestId(1));
    assert_eq!(
        terminal(&mut events, 1).await.unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert!(
        controller.state.lock().unwrap().jobs[&key.0]
            .result
            .is_some()
    );
    controller
        .save(ChecksumManifestId(4), key, request(&directory, 4))
        .unwrap();
    assert!(
        controller
            .save(ChecksumManifestId(3), key, request(&directory, 3))
            .is_err()
    );
    assert!(
        controller
            .save(ChecksumManifestId(4), key, request(&directory, 4))
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), controller.shutdown())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancelled_native_export_keeps_mutation_permit_and_shutdown_waits_for_cleanup() {
    let gate = Arc::new(Semaphore::new(1));
    let barrier = Barrier::new();
    let _release = ReleaseOnDrop(barrier.clone());
    let (controller, _events, directory) = setup(gate.clone(), Some(barrier.clone()));
    controller
        .save(
            ChecksumManifestId(1),
            (ChecksumJobId(1), ChecksumGeneration(1)),
            request(&directory, 1),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), barrier.entered.notified())
        .await
        .unwrap();
    controller.cancel_save(ChecksumManifestId(1));
    assert_eq!(gate.available_permits(), 0);
    let shutdown = tokio::spawn(controller.shutdown());
    tokio::task::yield_now().await;
    assert!(!shutdown.is_finished());
    assert!(!barrier.returned.load(Ordering::Acquire));
    assert_eq!(gate.available_permits(), 0);
    barrier.release();
    tokio::time::timeout(Duration::from_secs(5), shutdown)
        .await
        .unwrap()
        .unwrap();
    assert!(barrier.returned.load(Ordering::Acquire));
    assert_eq!(gate.available_permits(), 1);
}

#[tokio::test]
async fn full_queue_rejects_whole_request_without_superseding_an_admitted_generation() {
    let retirement = Retirement::testing();
    let service = Service::new(retirement.service, None);
    let (input, _receiver) = mpsc::channel(32);
    let controller = Controller {
        input,
        state: Arc::default(),
        lifetime: Cancellation::default(),
        available: true,
        service,
        retirement: retirement.domain,
        intake: tokio::sync::Mutex::new(retirement.intake),
        task: None,
    };
    let path = std::env::current_dir().unwrap().join("_build/tmp/source");
    for id in 1..=32 {
        let mut request =
            ChecksumRequest::new(ChecksumJobId(id), ChecksumGeneration(1), vec![path.clone()]);
        request.limits.max_retained_bytes = 1;
        // This test fills this actor's channel, independently of process-wide
        // quota used concurrently by other tests. Global quota has its own tests.
        controller
            .admit_plain(
                Capture::Paths(request),
                (ChecksumJobId(id), ChecksumGeneration(1)),
                ChecksumRetention::new(()),
            )
            .unwrap();
    }
    let mut request = ChecksumRequest::new(ChecksumJobId(1), ChecksumGeneration(2), vec![path]);
    request.limits.max_retained_bytes = 1;
    assert_eq!(
        controller
            .admit_plain(
                Capture::Paths(request),
                (ChecksumJobId(1), ChecksumGeneration(2)),
                ChecksumRetention::new(()),
            )
            .unwrap_err()
            .kind,
        io::ErrorKind::WouldBlock
    );
    let state = controller.state.lock().unwrap();
    assert_eq!(state.jobs.len(), 32);
    assert_eq!(
        state.jobs[&ChecksumJobId(1)].generation,
        ChecksumGeneration(1)
    );
    assert!(!state.jobs[&ChecksumJobId(1)].cancel.is_cancelled());
}

#[test]
fn selected_capture_uses_captured_sizes_and_refuses_disappeared_entries_without_focus_fallback() {
    let directory = std::env::current_dir().unwrap().join("_build/tmp");
    let entries = ["first", "second"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut entry =
                Entry::new(EntryId(index as u64), directory.join(name), EntryKind::File);
            entry.metadata.size = Some(42);
            entry
        })
        .collect::<Vec<_>>();
    let original = Arc::new(DirectoryIndex::from_entries(&entries));
    let selected = Arc::new(SelectionSnapshot::from_bits(original.clone(), vec![3]).unwrap());
    let request = SelectedChecksumRequest {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(1),
        files: SelectedFiles {
            selection: selected,
            view: original,
            fallback: Some(directory.join("ignored-focus")),
        },
        algorithms: ChecksumAlgorithms::all(),
        recursive: false,
        limits: ChecksumLimits::default(),
    };
    let captured = resolve(Capture::Selected(request.clone()), &Cancellation::default()).unwrap();
    assert_eq!(captured.targets.len(), 2);
    assert_eq!(captured.targets[0].path, directory.join("first"));
    assert_eq!(captured.targets[0].size, Some(42));
    assert_eq!(captured.targets[0].identity, None);
    let mut stale = request;
    stale.files.view = Arc::new(DirectoryIndex::from_entries(&entries[..1]));
    assert!(resolve(Capture::Selected(stale), &Cancellation::default()).is_err());
}

#[tokio::test]
async fn stale_generation_is_rechecked_before_a_backpressured_event_can_be_published() {
    let (sender, mut events) = mpsc::channel(1);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let state = Arc::new(Mutex::new(State::default()));
    state.lock().unwrap().jobs.insert(
        ChecksumJobId(1),
        Job {
            generation: ChecksumGeneration(1),
            cancel: Cancellation::default(),
            result: None,
        },
    );
    let event = ChecksumEvent {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(1),
        update: ChecksumUpdate::Queued,
    };
    outbox
        .sender
        .try_send(Event::Checksum(Box::new(event.clone())))
        .unwrap();
    let shared = state.clone();
    let pending = tokio::spawn(async move {
        send_guarded(
            &outbox,
            Event::Checksum(Box::new(event)),
            &Cancellation::default(),
            Some((&shared, (ChecksumJobId(1), ChecksumGeneration(1)))),
        )
        .await
    });
    tokio::task::yield_now().await;
    state
        .lock()
        .unwrap()
        .jobs
        .get_mut(&ChecksumJobId(1))
        .unwrap()
        .generation = ChecksumGeneration(2);
    let _ = events.recv().await;
    assert!(
        !tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .unwrap()
            .unwrap()
    );
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn verification_admission_fits_alongside_retained_calculation_or_verification() {
    let _quota = checksum::test_quota_fixture().await;
    for previous_verification in [false, true] {
        let (controller, _events, directory) = setup(Arc::new(Semaphore::new(1)), None);
        let held = if previous_verification {
            checksum::reserve_verification(&ChecksumVerifyRequest {
                job: ChecksumJobId(1),
                generation: ChecksumGeneration(1),
                manifest: directory.join("previous.md5"),
                limits: ChecksumLimits::default(),
            })
            .unwrap()
        } else {
            checksum::reserve(8 * 1024 * 1024, 8 * 1024 * 1024).unwrap()
        };
        let mut previous = (*retained(directory.join("source"))).clone();
        previous.retention = held.clone();
        controller
            .state
            .lock()
            .unwrap()
            .jobs
            .get_mut(&ChecksumJobId(1))
            .unwrap()
            .result = Some(Arc::new(previous));
        controller
            .verify(ChecksumVerifyRequest {
                job: ChecksumJobId(1),
                generation: ChecksumGeneration(2),
                manifest: directory.join("captured.md5"),
                limits: ChecksumLimits::default(),
            })
            .unwrap();
        assert_eq!(
            controller.state.lock().unwrap().jobs[&ChecksumJobId(1)].generation,
            ChecksumGeneration(2)
        );
        controller.cancel(ChecksumJobId(1), ChecksumGeneration(1));
        assert!(
            !controller.state.lock().unwrap().jobs[&ChecksumJobId(1)]
                .cancel
                .is_cancelled()
        );
        controller.release(ChecksumJobId(1), ChecksumGeneration(2));
        assert!(
            controller
                .verify(ChecksumVerifyRequest {
                    job: ChecksumJobId(1),
                    generation: ChecksumGeneration(3),
                    manifest: directory.join("other.md5"),
                    limits: ChecksumLimits::default(),
                })
                .is_err()
        );
        controller.shutdown().await;
        drop(held);
    }
}
