//! The disk slot is held only while a transfer executes native steps. Pause, user
//! decisions and namespace waits return it, and a running transfer yields it at a step
//! boundary to queued call-scoped requests, so exclusive and later shared requests on
//! the same process-wide gate never wait for a whole transfer.
use super::*;
use midnight_work_budget::{Admission, Gate};

/// A call-scoped consumer (checksum, thumbnail, folder size) queued on a real gate.
/// Dropping it withdraws a still-queued request, so a failed assertion cannot hang.
struct Consumer {
    entered: std::sync::mpsc::Receiver<()>,
    release: Option<std::sync::mpsc::Sender<()>>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Consumer {
    fn queue(gate: &Arc<Gate>, admission: Admission) -> Self {
        let gate = gate.clone();
        let (entered_sender, entered) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let worker = std::thread::spawn(move || {
            if let Some(permit) = gate.acquire(admission, || cancelled.load(Ordering::Acquire)) {
                let _ = entered_sender.send(());
                let _ = released.recv();
                drop(permit);
            }
        });
        Self {
            entered,
            release: Some(release),
            cancel,
            worker: Some(worker),
        }
    }
    async fn admitted(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        loop {
            match self.entered.try_recv() {
                Ok(()) => return,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "a queued call-scoped request waited for a transfer"
                    );
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(error) => panic!("consumer ended: {error}"),
            }
        }
    }
    fn waiting(&self) -> bool {
        matches!(
            self.entered.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        )
    }
    fn finish(mut self) {
        self.release.take();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl Drop for Consumer {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.release.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
async fn queued(gate: &Gate, count: usize) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while gate.snapshot().queued != count {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("request did not queue");
}
fn blocking(read_before_block: bool) -> (Arc<Notify>, Arc<AtomicBool>, BlockingService) {
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let service = BlockingService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
        prepared: Arc::new(AtomicUsize::new(0)),
        read_before_block,
    };
    (entered, released, service)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decision_wait_lets_queued_exclusive_and_later_shared_requests_run() {
    let temp = fixture();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let source = temp.path().join("item");
    fs::write(&source, b"new bytes").unwrap();
    fs::write(destination.join("item"), b"keep").unwrap();
    let mut h = Harness::native();
    let gate = h.io.gate().clone();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let conflict = h.conflict(1).await;
    let exclusive = Consumer::queue(&gate, Admission::Exclusive);
    exclusive.admitted().await;
    let shared = Consumer::queue(&gate, Admission::Shared);
    queued(&gate, 1).await;
    exclusive.finish();
    shared.admitted().await;
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: conflict.version,
            choice: CopyChoice::Rename("renamed".into()),
        })
        .await;
    // The decided job readmits its slot through the FIFO queue behind the shared reader.
    queued(&gate, 1).await;
    shared.finish();
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(fs::read(destination.join("renamed")).unwrap(), b"new bytes");
    assert_eq!(fs::read(destination.join("item")).unwrap(), b"keep");
    assert_eq!(gate.snapshot().active_shared, 0);
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paused_transfer_lets_queued_exclusive_and_later_shared_requests_run() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let bytes = vec![42; 600_000];
    fs::write(&source, &bytes).unwrap();
    fs::create_dir(&destination).unwrap();
    let (entered, released, service) = blocking(false);
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(service)));
    let gate = h.io.gate().clone();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    h.controller.set_paused(OperationId(1), true).await;
    released.store(true, Ordering::Release);
    loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.id == OperationId(1)
            && progress.paused
        {
            break;
        }
    }
    assert_eq!(
        gate.snapshot().active_shared,
        0,
        "a paused job holds no slot"
    );
    let exclusive = Consumer::queue(&gate, Admission::Exclusive);
    exclusive.admitted().await;
    let shared = Consumer::queue(&gate, Admission::Shared);
    queued(&gate, 1).await;
    exclusive.finish();
    shared.admitted().await;
    h.controller.set_paused(OperationId(1), false).await;
    queued(&gate, 1).await;
    shared.finish();
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(fs::read(destination.join("source")).unwrap(), bytes);
    h.close().await;
}

/// A job paused while it waits for capacity takes no slot when one frees up: its queued
/// request is withdrawn and it waits for resume holding neither lease.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paused_job_waiting_for_capacity_does_not_take_a_freed_slot() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"waits for resume").unwrap();
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    let gate = h.io.gate().clone();
    let holder = Consumer::queue(&gate, Admission::Exclusive);
    holder.admitted().await;
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    queued(&gate, 1).await;
    h.controller.set_paused(OperationId(1), true).await;
    loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.id == OperationId(1)
            && progress.paused
        {
            break;
        }
    }
    assert_eq!(
        gate.snapshot().queued,
        0,
        "a paused job withdraws its request"
    );
    assert_eq!(
        h.mutation.available_permits(),
        crate::transfer_leases::NAMESPACE_WEIGHT as usize,
        "a paused job holds no namespace share either"
    );
    holder.finish();
    assert_eq!(gate.snapshot().active_shared, 0);
    assert!(!gate.snapshot().exclusive);
    h.controller.set_paused(OperationId(1), false).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        fs::read(destination.join("source")).unwrap(),
        b"waits for resume"
    );
    h.close().await;
}

/// No pause and no decision: the running transfer still yields at its next step boundary
/// while call-scoped requests are queued, then rejoins the queue behind them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_transfer_yields_its_slot_to_queued_exclusive_and_later_shared_requests() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let bytes: Vec<_> = (0..700_000).map(|index| (index % 251) as u8).collect();
    fs::write(&source, &bytes).unwrap();
    fs::create_dir(&destination).unwrap();
    let (entered, released, service) = blocking(false);
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(service)));
    let gate = h.io.gate().clone();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    let exclusive = Consumer::queue(&gate, Admission::Exclusive);
    queued(&gate, 1).await;
    let shared = Consumer::queue(&gate, Admission::Shared);
    queued(&gate, 2).await;
    assert!(exclusive.waiting() && shared.waiting());
    released.store(true, Ordering::Release);
    exclusive.admitted().await;
    assert!(
        shared.waiting(),
        "FIFO keeps the later shared request behind"
    );
    // The exclusive request ran mid-job: the transfer now waits for its slot.
    let progress = snapshot(&h.controller.registry.lock().unwrap()[&OperationId(1)].status);
    assert!(!progress.state.is_terminal(), "{progress:?}");
    assert_eq!(progress.files_completed, 0);
    exclusive.finish();
    shared.admitted().await;
    shared.finish();
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(fs::read(destination.join("source")).unwrap(), bytes);
    h.close().await;
}

/// A job refused a namespace grant keeps its share and planned record but returns its
/// disk slot between attempts: a call-scoped request is admitted although the granted
/// peer holds the only other slot for as long as its native call blocks. The fixture has
/// no read-only destination probe, so both jobs take the opaque lane and the second says
/// that their folders cannot be confirmed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn namespace_grant_wait_returns_the_disk_slot() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, vec![7; 300_000]).unwrap();
    fs::create_dir(&destination).unwrap();
    let (entered, released, service) = blocking(false);
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::with_slots(Some(Arc::new(service)), 2);
    let gate = h.io.gate().clone();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    h.controller
        .enqueue(request(2, std::slice::from_ref(&source), &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let CopyEvent::Progress(progress) = h.next().await
                && progress.id == OperationId(2)
                && progress.wait_reason == Some(TransferWaitReason::UnconfirmedFolders)
                && progress.state == CopyOperationState::Preparing
            {
                break;
            }
        }
    })
    .await
    .expect("the second job waits for the conflicting grant");
    let reader = Consumer::queue(&gate, Admission::Shared);
    reader.admitted().await;
    reader.finish();
    released.store(true, Ordering::Release);
    let (mut first, mut conflict) = (None, None);
    while first.is_none() || conflict.is_none() {
        match h.next().await {
            CopyEvent::Progress(progress)
                if progress.id == OperationId(1) && progress.state.is_terminal() =>
            {
                first = Some(progress);
            }
            CopyEvent::Conflict(event) if event.id == OperationId(2) => conflict = Some(event),
            _ => {}
        }
    }
    assert_eq!(first.unwrap().state, CopyOperationState::Completed);
    h.controller
        .decide(CopyDecision {
            id: OperationId(2),
            version: conflict.unwrap().version,
            choice: CopyChoice::Rename("again".into()),
        })
        .await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Completed);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
    h.close().await;
}

/// A pause requested after the pre-check but before the checkpoint is acknowledged by the
/// checkpoint itself; the slot is returned before that paused state can be published, so
/// a paused job never holds a disk slot.
#[test]
fn pause_acknowledged_by_the_checkpoint_returns_the_slot_first() {
    let gate = Arc::new(Gate::new(1));
    let permit = gate
        .try_acquire_retained(Admission::Shared)
        .expect("free slot");
    let cancel = Cancellation::default().pausable_child();
    BEFORE_PAUSE_CHECKPOINT.with(|hook| {
        hook.set(Some(|cancel: &Cancellation| {
            cancel.set_pause_requested(true)
        }))
    });
    assert!(!pause_checkpoint(&cancel, Some(&permit)));
    assert!(cancel.is_paused());
    assert!(!permit.is_held(), "an acknowledged pause holds no slot");
    assert_eq!(gate.snapshot().active_shared, 0);
    // Without a pause the checkpoint keeps the slot.
    cancel.set_pause_requested(false);
    assert!(permit.try_reacquire());
    assert!(pause_checkpoint(&cancel, Some(&permit)));
    assert!(permit.is_held());
}

/// An archive copy can complete more roots than the notice queue holds. While the UI
/// takes no notice, announcing one more root waits for room in that queue, and the job
/// waits without its disk slot: a request queued on the slot is admitted meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_root_announcement_waiting_for_the_ui_returns_the_disk_slot() {
    let gate = Arc::new(Gate::new(1));
    let io = Arc::new(
        gate.try_acquire_retained(Admission::Shared)
            .expect("free slot"),
    );
    // The job's first progress was never delivered, so no notice reaches the UI.
    let (sender, _ui) = mpsc::channel(1);
    let outbox = CopyOutbox::pending(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let root = |entry| CopyEvent::RootsCompleted {
        id: OperationId(1),
        sources: vec![CompletedCopySource::Archive(VirtualEntryId {
            mount: 7,
            entry,
        })],
    };
    for entry in 0..MAX_NATIVE_TRANSFER_ROOTS as u64 + 2 {
        outbox.send(Event::Copy(root(entry))).await;
    }
    let reader = Consumer::queue(&gate, Admission::Shared);
    let announcement = tokio::spawn({
        let (outbox, io) = (outbox.clone(), io.clone());
        async move { announce_root(&outbox, &io, root(u64::MAX)).await }
    });
    // A bounded wait for the reader, not a sleep: it fails if the slot stays held.
    let admitted =
        tokio::task::block_in_place(|| reader.entered.recv_timeout(Duration::from_secs(4)).is_ok());
    assert!(admitted, "a queued request waited for the job's UI");
    assert!(!io.is_held());
    assert!(
        !announcement.is_finished(),
        "the full notice queue had room"
    );
    announcement.abort();
    reader.finish();
}
