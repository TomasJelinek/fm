use super::*;
use crate::transfer_leases::NAMESPACE_WEIGHT;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Notify, mpsc};

#[path = "archive_copy_tests.rs"]
mod archive;
#[path = "copy_cancel_queue_tests.rs"]
mod cancel_queue;
// Linux and Windows report a child that cannot be opened apart from its folder.
#[cfg(target_os = "linux")]
#[path = "copy_child_failure_tests.rs"]
mod child_failures;
#[cfg(windows)]
#[path = "copy_child_failure_windows_tests.rs"]
mod child_failures_windows;
#[path = "copy_completion_tests.rs"]
mod completion;
#[path = "copy_concurrency_regression_tests.rs"]
mod concurrency_regression;
#[path = "copy_conflict_display_tests.rs"]
mod conflict_display;
#[path = "copy_discovery_tests.rs"]
mod discovery;
#[path = "copy_error_tests.rs"]
mod errors;
#[path = "copy_eta_tests.rs"]
mod eta;
#[path = "copy_io_slot_tests.rs"]
mod io_slot;
#[path = "move_tests.rs"]
mod moves;
#[path = "move_native_tests.rs"]
mod native_moves;
#[path = "copy_pause_tests.rs"]
mod pause;
#[path = "copy_peer_refusal_tests.rs"]
mod peer_refusal;
#[path = "copy_preserve_tests.rs"]
mod preserve;
#[path = "copy_progress_discovery_tests.rs"]
mod progress_discovery;
#[path = "copy_progress_verification_tests.rs"]
mod progress_verification;
#[cfg(target_os = "linux")]
#[path = "move_protected_folder_tests.rs"]
mod protected_moves;
#[path = "copy_publication_tests.rs"]
mod publication;
#[path = "copy_queue_tests.rs"]
mod queue;
#[path = "copy_replacement_tests.rs"]
mod replacement;
#[path = "copy_scheduler_tests.rs"]
mod scheduler;
#[path = "copy_stage_retry_tests.rs"]
mod stage_retry;
#[path = "copy_streamed_tests.rs"]
mod streamed;
#[path = "copy_throughput_tests.rs"]
mod throughput;
// Manual timings on the native Linux adapter (ignored; procfs CPU time).
#[cfg(target_os = "linux")]
#[path = "copy_timing_tests.rs"]
mod timing;
#[path = "tree_copy_tests.rs"]
mod tree_copy;
#[path = "copy_unprotected_source_tests.rs"]
mod unprotected_source;
#[path = "copy_verification_tests.rs"]
mod verification;

#[tokio::test]
async fn native_drop_reservations_share_the_actual_copy_job_capacity() {
    let mut harness = Harness::without_service();
    let mut reservations: Vec<_> = (0..32)
        .map(|_| harness.controller.reserve().unwrap())
        .collect();
    assert!(harness.controller.reserve().is_err());
    let root = std::env::current_dir().unwrap();
    harness
        .controller
        .enqueue(request(1, &[root.join("source")], &root))
        .await;
    let full = harness.terminal(1).await;
    assert_eq!(full.errors[0].kind, std::io::ErrorKind::WouldBlock);
    let permit = reservations.remove(0);
    harness
        .controller
        .enqueue_reserved(request(2, &[root.join("source")], &root), permit)
        .await;
    let admitted = harness.terminal(2).await;
    assert_eq!(admitted.errors[0].kind, std::io::ErrorKind::Unsupported);
    harness.wait_capacity(1).await;
    let returned = harness
        .controller
        .reserve()
        .expect("terminal native copy releases its reserved slot");
    assert!(harness.controller.reserve().is_err());
    drop(returned);
    drop(reservations);
    harness.controller.shutdown();
    assert!(harness.controller.reserve().is_err());
    while harness.tasks.join_next().await.is_some() {}
}

/// A job-level wait fails after this even while heartbeat progress keeps arriving.
const JOB_DEADLINE: Duration = Duration::from_secs(30);

/// Wait until `ready()` holds; another thread or task makes it true. Spin on `yield_now`
/// first: the common case finishes within microseconds and the yield lets the tasks of
/// a current-thread runtime (such as the admission consumer) run. Then back off to 1 ms
/// sleeps, so a slower wait does not burn a core. Fails only after 4 s.
async fn eventually(mut ready: impl FnMut() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    let mut spins = 0u32;
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        if spins < 1_000 {
            spins += 1;
            tokio::task::yield_now().await;
        } else {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-copy-queue/_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("queue-")
        .tempdir_in(root)
        .unwrap()
}
fn request(id: u64, sources: &[PathBuf], destination: &Path) -> CopyRequest {
    CopyRequest {
        start_policy: midnight_core::TransferStartPolicy::Immediate,
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(id),
        clipboard: FileClipboard {
            paths: sources.into(),
            intent: FileClipboardIntent::Copy,
        },
        destination: destination.into(),
    }
}
struct Harness {
    controller: Controller,
    events: mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    mutation: Arc<Semaphore>,
    io: Arc<IoBudget>,
    terminals: std::collections::HashMap<OperationId, Arc<CopyProgress>>,
}
impl Harness {
    fn new<T: midnight_core::AuditedSynchronousTransferService + 'static>(
        service: Option<Arc<T>>,
    ) -> Self {
        Self::with_slots(service, 1)
    }
    fn with_slots<T: midnight_core::AuditedSynchronousTransferService + 'static>(
        service: Option<Arc<T>>,
        slots: u8,
    ) -> Self {
        Self::configured(
            service.map(crate::transfer_composition::QualifiedTransferInstallation::audited),
            slots,
        )
    }
    fn without_service() -> Self {
        Self::configured(None, 1)
    }
    fn unqualified(service: Arc<dyn TransferService>) -> Self {
        Self::configured(
            Some(crate::transfer_composition::QualifiedTransferInstallation::unqualified(service)),
            1,
        )
    }
    fn configured(
        service: Option<crate::transfer_composition::QualifiedTransferInstallation>,
        slots: u8,
    ) -> Self {
        Self::configured_with_events(service, slots, 256)
    }
    /// The native service, with the UI event channel the application uses (32).
    fn native_with_events(capacity: usize) -> Self {
        Self::configured_with_events(
            Some(
                crate::transfer_composition::QualifiedTransferInstallation::audited(Arc::new(
                    midnight_platform::LocalFileSystem::new(None),
                )),
            ),
            1,
            capacity,
        )
    }
    fn configured_with_events(
        service: Option<crate::transfer_composition::QualifiedTransferInstallation>,
        slots: u8,
        capacity: usize,
    ) -> Self {
        let (sender, events) = mpsc::channel(capacity);
        let mut tasks = JoinSet::new();
        let mutation = crate::transfer_leases::namespace_gate();
        let io = IoBudget::new(Arc::new(midnight_work_budget::Gate::new(slots)));
        let controller = Controller::new(
            crate::worker_retirement::ActorRetirement::testing(),
            service,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            mutation.clone(),
            io.clone(),
            &mut tasks,
        );
        Self {
            controller,
            events,
            tasks,
            mutation,
            io,
            terminals: std::collections::HashMap::new(),
        }
    }
    fn native() -> Self {
        Self::new(Some(Arc::new(midnight_platform::LocalFileSystem::new(
            None,
        ))))
    }
    async fn next(&mut self) -> CopyEvent {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Event::Copy(event) =
                    self.events.recv().await.expect("copy event stream closed")
                {
                    return event;
                }
            }
        })
        .await
        .expect("copy event timed out")
    }
    async fn wait_capacity(&self, available: usize) {
        let capacity = self.controller.capacity();
        eventually(
            || capacity.available_permits() == available,
            "checked retirement and delivery must return the expected admission capacity",
        )
        .await;
    }
    async fn reserve_ready(&self) -> Reservation {
        let mut reservation = None;
        eventually(
            || match self.controller.reserve() {
                Ok(ready) => {
                    reservation = Some(ready);
                    true
                }
                Err(error) if error.kind == io::ErrorKind::WouldBlock => false,
                Err(error) => panic!("Copy admission unexpectedly refused: {error}"),
            },
            "actual Copy admission must become ready",
        )
        .await;
        reservation.expect("admission became ready")
    }
    /// Wait until the job's live status satisfies `ready`.
    async fn wait_status(&self, id: u64, ready: impl Fn(&CopyProgress) -> bool) -> CopyProgress {
        let mut matched = None;
        eventually(
            || {
                let registry = self.controller.registry.lock().unwrap();
                let status = registry
                    .get(&OperationId(id))
                    .map(|job| snapshot(&job.status));
                matched = status.filter(|status| ready(status));
                matched.is_some()
            },
            "the job must reach the expected live status",
        )
        .await;
        matched.expect("status matched")
    }
    async fn enqueue_reserved_input(&self, input: CopyInput, reservation: Reservation) {
        let (input, sender) = reservation.attach_input(input);
        self.controller.enqueue_input_reserved(input, sender).await;
    }
    async fn terminal(&mut self, id: u64) -> Arc<CopyProgress> {
        if let Some(progress) = self.terminals.remove(&OperationId(id)) {
            return progress;
        }
        // Heartbeats keep `next` from timing out while a broken job never finishes.
        let deadline = std::time::Instant::now() + JOB_DEADLINE;
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "job {id} did not finish"
            );
            match self.next().await {
                CopyEvent::Progress(progress)
                    if progress.id == OperationId(id) && progress.state.is_terminal() =>
                {
                    return progress;
                }
                CopyEvent::Progress(progress) if progress.state.is_terminal() => {
                    assert!(
                        self.terminals.insert(progress.id, progress).is_none(),
                        "duplicate terminal event"
                    );
                }
                // Existing terminal-failure tests explicitly decline recovery. New
                // recovery tests consume Failure themselves and exercise Retry.
                CopyEvent::Failure(failure) if failure.id == OperationId(id) => {
                    self.controller
                        .decide(CopyDecision {
                            id: failure.id,
                            version: failure.version,
                            choice: if failure.can_skip {
                                CopyChoice::Skip
                            } else {
                                CopyChoice::Cancel
                            },
                        })
                        .await;
                }
                _ => {}
            }
        }
    }
    async fn conflict(&mut self, id: u64) -> Arc<CopyConflict> {
        assert!(
            !self.terminals.contains_key(&OperationId(id)),
            "operation already finished"
        );
        let deadline = std::time::Instant::now() + JOB_DEADLINE;
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "job {id} raised no conflict"
            );
            match self.next().await {
                CopyEvent::Conflict(conflict) if conflict.id == OperationId(id) => return conflict,
                CopyEvent::Progress(progress) if progress.id == OperationId(id) => assert!(
                    !progress.state.is_terminal(),
                    "unexpected terminal copy: {progress:?}"
                ),
                CopyEvent::Progress(progress) if progress.state.is_terminal() => {
                    assert!(
                        self.terminals.insert(progress.id, progress).is_none(),
                        "duplicate terminal event"
                    );
                }
                _ => {}
            }
        }
    }
    async fn wait_shutdown(&mut self) {
        tokio::time::timeout(Duration::from_secs(4), async {
            while self.tasks.join_next().await.is_some() {}
        })
        .await
        .unwrap();
    }
    async fn close(mut self) {
        self.controller.shutdown();
        self.wait_shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recursive_multisource_copy_verifies_bytes_and_preserves_sources() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("nested/文档.bin"), vec![29; 700_003]).unwrap();
    let other = temp.path().join("other");
    fs::write(&other, b"payload").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[source.clone(), other.clone()], &destination))
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!(done.files_completed, 2);
    assert_eq!(done.directories_created, 2);
    assert_eq!(done.copied_bytes, 700_010);
    assert_eq!(done.verified_bytes, 700_010);
    assert_eq!(done.destination_directory, destination);
    assert_eq!(
        fs::read(destination.join("source/nested/文档.bin")).unwrap(),
        vec![29; 700_003]
    );
    assert_eq!(fs::read(destination.join("other")).unwrap(), b"payload");
    assert!(source.join("nested/文档.bin").is_file());
    assert_eq!(fs::read(other).unwrap(), b"payload");
    h.close().await;
}

fn selected_request(id: u64, entries: &[Entry], destination: &Path) -> SelectedCopyRequest {
    let view = Arc::new(DirectoryIndex::from_entries(entries));
    SelectedCopyRequest {
        start_policy: midnight_core::TransferStartPolicy::Immediate,
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(id),
        files: SelectedFiles {
            selection: Arc::new(SelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap()),
            view,
            fallback: entries.last().map(|entry| entry.path.clone()),
        },
        destination: destination.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_copy_uses_the_captured_view_and_never_falls_back_to_other_files() {
    let temp = fixture();
    let destination = temp.path().join("target");
    fs::create_dir(&destination).unwrap();
    let entries: Vec<_> = ["文档.bin", "not selected.bin"]
        .into_iter()
        .enumerate()
        .map(|(id, name)| {
            let path = temp.path().join(name);
            fs::write(&path, b"verified bytes").unwrap();
            Entry::new(EntryId(id as u64), path, EntryKind::File)
        })
        .collect();
    let mut h = Harness::native();
    h.controller
        .enqueue_selected(selected_request(1, &entries, &destination))
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.verified_bytes, 14);
    assert_eq!(
        fs::read(destination.join("文档.bin")).unwrap(),
        b"verified bytes"
    );
    assert!(!destination.join("not selected.bin").exists());

    let mut filtered = selected_request(2, &entries, &destination);
    filtered.files.view = Arc::new(DirectoryIndex::from_entries(&entries[1..]));
    h.controller.enqueue_selected(filtered).await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Failed);
    assert!(!destination.join("not selected.bin").exists());
    h.close().await;
}

#[tokio::test]
async fn selected_copy_shares_admission_and_cancellation_with_paste_and_drop() {
    let temp = fixture();
    let entries = [Entry::new(
        EntryId(1),
        temp.path().join("source"),
        EntryKind::File,
    )];
    let mut h = Harness::native();
    let reservations: Vec<_> = (0..32).map(|_| h.controller.reserve().unwrap()).collect();
    h.controller
        .enqueue_selected(selected_request(1, &entries, temp.path()))
        .await;
    assert_eq!(
        h.terminal(1).await.errors[0].kind,
        io::ErrorKind::WouldBlock
    );
    drop(reservations);
    h.wait_capacity(MAX_JOBS).await;
    let gate = h
        .mutation
        .clone()
        .acquire_many_owned(crate::transfer_leases::NAMESPACE_WEIGHT)
        .await
        .unwrap();
    let ready = h.reserve_ready().await;
    h.enqueue_reserved_input(selected_request(2, &entries, temp.path()).into(), ready)
        .await;
    h.controller.cancel(OperationId(2));
    drop(gate);
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Cancelled);
    h.close().await;
}

#[tokio::test]
async fn selected_copy_rejects_relative_destination_before_opening_any_sources() {
    let mut h = Harness::native();
    let entries = [Entry::new(
        EntryId(1),
        PathBuf::from("missing"),
        EntryKind::File,
    )];
    h.controller
        .enqueue_selected(selected_request(1, &entries, Path::new("relative")))
        .await;
    assert_eq!(
        h.terminal(1).await.errors[0].kind,
        io::ErrorKind::InvalidInput
    );
    h.close().await;
}

/// A pending user decision keeps the namespace share but returns the only disk slot; the
/// controller accepts more work meanwhile and stale decisions stay ignored (the later
/// exact-version Rename is still accepted).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conflict_wait_releases_the_disk_slot_and_rejects_stale_decisions() {
    let temp = fixture();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::write(&first, b"new bytes").unwrap();
    fs::write(&second, b"next").unwrap();
    fs::write(destination.join("first"), b"keep").unwrap();
    fs::write(destination.join("taken"), b"also keep").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[first], &destination))
        .await;
    let conflict = h.conflict(1).await;
    assert_eq!(conflict.source.metadata.size, Some(9));
    assert_eq!(conflict.target.metadata.size, Some(4));
    assert_eq!(h.io.available_permits(), 1);
    assert_eq!(
        h.mutation.available_permits(),
        NAMESPACE_WEIGHT as usize - 1
    );
    h.controller
        .enqueue(request(2, &[second], &destination))
        .await;
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: conflict.version + 1,
            choice: CopyChoice::Skip,
        })
        .await;
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: conflict.version,
            choice: CopyChoice::Rename("taken".into()),
        })
        .await;
    let newer = h.conflict(1).await;
    assert!(newer.version > conflict.version);
    assert_eq!(newer.target.path, destination.join("taken"));
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: newer.version,
            choice: CopyChoice::Rename("renamed".into()),
        })
        .await;
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Completed);
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Completed);
    assert_eq!(fs::read(destination.join("second")).unwrap(), b"next");
    assert_eq!(fs::read(destination.join("first")).unwrap(), b"keep");
    assert_eq!(fs::read(destination.join("taken")).unwrap(), b"also keep");
    assert_eq!(fs::read(destination.join("renamed")).unwrap(), b"new bytes");
    h.close().await;
}

/// A "different name" the destination cannot use asks again (fresh version, reason in
/// the job details) instead of recording a failure and dropping the entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_rename_asks_again_and_keeps_the_entry() {
    let temp = fixture();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let source = temp.path().join("item");
    fs::write(&source, b"new bytes").unwrap();
    fs::write(destination.join("item"), b"keep").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let conflict = h.conflict(1).await;
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: conflict.version,
            choice: CopyChoice::Rename("nested/name".into()),
        })
        .await;
    let again = h.conflict(1).await;
    assert!(again.version > conflict.version);
    assert_eq!(again.destination, conflict.destination);
    let message = snapshot(&h.controller.registry.lock().unwrap()[&OperationId(1)].status)
        .message
        .clone();
    assert!(message.contains("cannot be used here"), "{message}");
    // One lexically valid component that the destination itself refuses (longer than
    // any native basename on both platforms): the probe asks again before publication.
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: again.version,
            choice: CopyChoice::Rename("n".repeat(300).into()),
        })
        .await;
    let overlong = h.conflict(1).await;
    assert!(overlong.version > again.version);
    assert_eq!(overlong.destination, conflict.destination);
    let message = snapshot(&h.controller.registry.lock().unwrap()[&OperationId(1)].status)
        .message
        .clone();
    assert!(message.contains("cannot be used here"), "{message}");
    h.controller
        .decide(CopyDecision {
            id: OperationId(1),
            version: overlong.version,
            choice: CopyChoice::Rename("renamed".into()),
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(
        done.errors.is_empty() && done.message.is_empty(),
        "{done:?}"
    );
    assert_eq!(fs::read(destination.join("renamed")).unwrap(), b"new bytes");
    assert_eq!(fs::read(destination.join("item")).unwrap(), b"keep");
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skip_all_is_scoped_to_one_job_and_existing_directories_never_merge() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(destination.join("source")).unwrap();
    fs::write(source.join("child"), b"source child").unwrap();
    let other = temp.path().join("other");
    fs::write(&other, b"source other").unwrap();
    fs::write(destination.join("other"), b"existing").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[source.clone(), other.clone()], &destination))
        .await;
    let conflict = h.conflict(1).await;
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::SkipAll,
        })
        .await;
    let done = h.terminal(1).await;
    // Skipping existing names is the user's choice, not an error.
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!((done.skipped, done.failed), (2, 0));
    assert_eq!(done.files_completed, 0);
    assert!(!destination.join("source/child").exists());
    h.controller
        .enqueue(request(2, &[other], &destination))
        .await;
    let conflict = h.conflict(2).await;
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Cancelled);
    assert_eq!(fs::read(source.join("child")).unwrap(), b"source child");
    assert_eq!(fs::read(destination.join("other")).unwrap(), b"existing");
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn move_clipboard_is_rejected_without_any_source_removal() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"source").unwrap();
    fs::create_dir(&destination).unwrap();
    let mut job = request(1, std::slice::from_ref(&source), &destination);
    job.clipboard.intent = FileClipboardIntent::Move;
    let mut h = Harness::native();
    h.controller.enqueue(job).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Failed);
    assert!(done.message.contains("Move"));
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_is_bounded_and_the_overflow_job_is_rejected() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"source").unwrap();
    let mut h = Harness::native();
    let held = h
        .mutation
        .clone()
        .acquire_many_owned(crate::transfer_leases::NAMESPACE_WEIGHT)
        .await
        .unwrap();
    for id in 1..=33 {
        h.controller
            .enqueue(request(id, std::slice::from_ref(&source), temp.path()))
            .await;
    }
    let rejected = h.terminal(33).await;
    assert_eq!(rejected.state, CopyOperationState::Failed);
    assert!(rejected.message.contains("32"));
    for id in 1..=32 {
        h.controller.cancel(OperationId(id));
    }
    drop(held);
    let mut terminals = std::collections::HashSet::new();
    while terminals.len() < 32 {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.state.is_terminal()
        {
            assert_eq!(progress.state, CopyOperationState::Cancelled);
            assert!(terminals.insert(progress.id));
        }
    }
    h.close().await;
}

/// Through the production admission path (a published reservation filled through
/// `enqueue_input_reserved`, as native drops are) and with free capacity: a duplicate of
/// a running operation's ID retires without a second preparation or terminal event, and
/// the original keeps its controls.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_id_with_free_capacity_never_runs_or_takes_over_the_original() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, vec![3; 300_000]).unwrap();
    fs::create_dir(&destination).unwrap();
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let prepared = Arc::new(AtomicUsize::new(0));
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(BlockingService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
        prepared: prepared.clone(),
        read_before_block: false,
    })));
    let original = h.reserve_ready().await;
    h.enqueue_reserved_input(
        request(1, std::slice::from_ref(&source), &destination).into(),
        original,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .expect("the original runs");
    let duplicate = h.reserve_ready().await;
    h.enqueue_reserved_input(
        request(1, std::slice::from_ref(&source), &destination).into(),
        duplicate,
    )
    .await;
    // The duplicate's admission returns while the original still holds its own.
    h.wait_capacity(MAX_JOBS - 1).await;
    assert_eq!(
        prepared.load(Ordering::Acquire),
        1,
        "a duplicate ID never prepares its sources"
    );
    // The registered controls still belong to the original.
    h.controller.cancel(OperationId(1));
    released.store(true, Ordering::Release);
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    h.wait_capacity(MAX_JOBS).await;
    h.controller.shutdown();
    h.wait_shutdown().await;
    while let Ok(event) = h.events.try_recv() {
        if let Event::Copy(CopyEvent::Progress(progress)) = event {
            assert!(
                !(progress.id == OperationId(1) && progress.state.is_terminal()),
                "a second terminal event for the duplicate ID: {progress:?}"
            );
        }
    }
    assert_eq!(prepared.load(Ordering::Acquire), 1);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    assert_eq!(fs::metadata(&source).unwrap().len(), 300_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cancellation_finishes_while_another_mutation_still_holds_the_gate() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"source").unwrap();
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    let held = h
        .mutation
        .clone()
        .acquire_many_owned(crate::transfer_leases::NAMESPACE_WEIGHT)
        .await
        .unwrap();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let waiting = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let CopyEvent::Progress(progress) = h.next().await {
                assert!(!progress.state.is_terminal(), "{progress:?}");
                if progress.wait_reason == Some(TransferWaitReason::OtherOperation) {
                    break progress;
                }
            }
        }
    })
    .await
    .expect("held mutation capacity must be reported as a resource wait");
    assert_eq!(waiting.state, CopyOperationState::Queued);
    assert_eq!(waiting.discovery.state, DiscoveryState::NotStarted);
    assert_eq!(waiting.verification_pass_id, 0);
    assert!(waiting.verification_throughput.samples.is_empty());
    h.controller.cancel(OperationId(1));
    let cancelled = h.terminal(1).await;
    assert_eq!(cancelled.state, CopyOperationState::Cancelled);
    assert_eq!(cancelled.wait_reason, None);
    assert_eq!(h.mutation.available_permits(), 0);
    assert_eq!(h.io.available_permits(), 1);
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    assert_eq!(fs::read(source).unwrap(), b"source");
    drop(held);
    h.close().await;
}

struct BlockedSource {
    inner: Box<dyn TransferSource>,
    entered: Arc<Notify>,
    released: Arc<AtomicBool>,
    read_before_block: bool,
}
impl TransferSource for BlockedSource {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        if self.read_before_block {
            self.read_before_block = false;
            return self.inner.read_chunk(buffer, cancel);
        }
        self.entered.notify_one();
        while !self.released.load(Ordering::Acquire) {
            std::thread::park_timeout(Duration::from_millis(5));
        }
        self.inner.read_chunk(buffer, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner.next_child(cancel)
    }
}
struct BlockingService {
    inner: midnight_platform::LocalFileSystem,
    entered: Arc<Notify>,
    released: Arc<AtomicBool>,
    prepared: Arc<AtomicUsize>,
    read_before_block: bool,
}
struct ReleaseOnDrop(Arc<AtomicBool>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
// Native delegates and fixture state are destroyed synchronously; no retirement queues are hidden.
impl midnight_core::AuditedSynchronousTransferService for BlockingService {}
impl TransferService for BlockingService {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.prepared.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(BlockedSource {
            inner: self.inner.prepare_transfer_source(path, cancel)?,
            entered: self.entered.clone(),
            released: self.released.clone(),
            read_before_block: self.read_before_block,
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_transfer_destination(path, cancel)
    }
}

/// A Move of more than the 64 selected items it once took moves them all, as a Copy of
/// the same items copies them all: such a selection streams (each item is prepared for
/// Move only when the job takes it).
#[tokio::test]
async fn a_move_of_65_items_moves_them_all_as_a_copy_copies_them_all() {
    let (_temp, root) = moves::protected_fixture();
    let copied_to = root.join("copied");
    let moved_to = root.join("moved");
    fs::create_dir(&copied_to).unwrap();
    fs::create_dir(&moved_to).unwrap();
    let paths: Vec<_> = (0..65u8)
        .map(|index| {
            let path = root.join(format!("file-{index:02}"));
            fs::write(&path, [index]).unwrap();
            path
        })
        .collect();
    let mut h = Harness::native();
    h.controller.enqueue(request(1, &paths, &copied_to)).await;
    let copied = h.terminal(1).await;
    assert_eq!(copied.state, CopyOperationState::Completed, "{copied:?}");
    assert_eq!(copied.files_completed, 65, "{copied:?}");
    assert_eq!(fs::read_dir(&copied_to).unwrap().count(), 65);
    h.controller
        .enqueue_move(request(2, &paths, &moved_to))
        .await;
    let moved = h.terminal(2).await;
    assert_eq!(moved.state, CopyOperationState::Completed, "{moved:?}");
    assert_eq!(moved.completed_roots, 65, "{moved:?}");
    assert!(paths.iter().all(|path| !path.exists()));
    assert_eq!(fs::read_dir(&moved_to).unwrap().count(), 65);
    h.close().await;
}

#[tokio::test]
async fn erased_service_installation_cannot_borrow_an_audited_native_witness() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"preserved").unwrap();
    fs::create_dir(&destination).unwrap();
    let prepared = Arc::new(AtomicUsize::new(0));
    let service: Arc<dyn TransferService> = Arc::new(BlockingService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: Arc::new(Notify::new()),
        released: Arc::new(AtomicBool::new(true)),
        prepared: prepared.clone(),
        read_before_block: false,
    });
    let mut h = Harness::unqualified(service);
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let done = h.terminal(1).await;
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Failed);
    assert!(
        done.errors
            .iter()
            .any(|error| error.kind == io::ErrorKind::Unsupported)
    );
    assert_eq!(prepared.load(Ordering::Acquire), 0);
    assert_eq!(fs::read(source).unwrap(), b"preserved");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_cannot_start_the_next_copy_before_native_io_returns() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let next_source = temp.path().join("next-source");
    fs::write(&source, vec![1; 500_000]).unwrap();
    fs::write(&next_source, b"next queued copy").unwrap();
    fs::create_dir(&destination).unwrap();
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let prepared = Arc::new(AtomicUsize::new(0));
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(BlockingService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
        prepared: prepared.clone(),
        read_before_block: false,
    })));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    h.controller
        .enqueue(request(2, std::slice::from_ref(&next_source), &destination))
        .await;
    h.controller.cancel(OperationId(1));
    // The queued job holds its namespace share while it waits for the disk slot,
    // queued behind the cancelled job, whose native I/O keeps that slot.
    eventually(
        || {
            h.mutation.available_permits() == NAMESPACE_WEIGHT as usize - 2
                && h.io.gate().snapshot().queued == 1
        },
        "the queued job takes its namespace share and waits for the disk slot",
    )
    .await;
    assert_eq!(h.io.available_permits(), 0);
    assert_eq!(prepared.load(Ordering::Acquire), 1);
    assert_eq!(
        h.controller
            .registry
            .lock()
            .unwrap()
            .get(&OperationId(2))
            .unwrap()
            .status
            .lock()
            .unwrap()
            .state,
        CopyOperationState::Queued
    );
    released.store(true, Ordering::Release);
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    let completed = h.terminal(2).await;
    assert_eq!(completed.state, CopyOperationState::Completed);
    assert_eq!(completed.verified_bytes, 16);
    assert_eq!(prepared.load(Ordering::Acquire), 2);
    assert_eq!(h.io.available_permits(), 1);
    assert_eq!(h.mutation.available_permits(), NAMESPACE_WEIGHT as usize);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    assert_eq!(
        fs::read(destination.join("next-source")).unwrap(),
        b"next queued copy"
    );
    assert_eq!(fs::read(next_source).unwrap(), b"next queued copy");
    assert_eq!(fs::metadata(source).unwrap().len(), 500_000);
    h.close().await;
}
#[path = "tree_move_tests.rs"]
mod tree_move;
