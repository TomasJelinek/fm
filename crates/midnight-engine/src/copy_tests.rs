use super::*;
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
#[path = "copy_completion_tests.rs"]
mod completion;
#[path = "copy_error_tests.rs"]
mod errors;
#[path = "move_tests.rs"]
mod moves;
#[path = "move_native_tests.rs"]
mod native_moves;
#[path = "copy_queue_tests.rs"]
mod queue;
#[path = "copy_replacement_tests.rs"]
mod replacement;
#[path = "copy_throughput_tests.rs"]
mod throughput;
#[path = "tree_copy_tests.rs"]
mod tree_copy;
#[path = "copy_verification_tests.rs"]
mod verification;

#[tokio::test]
async fn native_drop_reservations_share_the_actual_copy_job_capacity() {
    let mut harness = Harness::new(None);
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
    io: Arc<Semaphore>,
}
impl Harness {
    fn new(service: Option<Arc<dyn TransferService>>) -> Self {
        let (sender, events) = mpsc::channel(256);
        let mut tasks = JoinSet::new();
        let mutation = Arc::new(Semaphore::new(1));
        let io = Arc::new(Semaphore::new(1));
        let controller = Controller::new(
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
    async fn terminal(&mut self, id: u64) -> Arc<CopyProgress> {
        loop {
            match self.next().await {
                CopyEvent::Progress(progress)
                    if progress.id == OperationId(id) && progress.state.is_terminal() =>
                {
                    return progress;
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
        loop {
            match self.next().await {
                CopyEvent::Conflict(conflict) if conflict.id == OperationId(id) => return conflict,
                CopyEvent::Progress(progress) if progress.id == OperationId(id) => assert!(
                    !progress.state.is_terminal(),
                    "unexpected terminal copy: {progress:?}"
                ),
                _ => {}
            }
        }
    }
    async fn close(mut self) {
        self.controller.shutdown();
        tokio::time::timeout(Duration::from_secs(4), async {
            while self.tasks.join_next().await.is_some() {}
        })
        .await
        .unwrap();
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
    let gate = h.mutation.clone().acquire_owned().await.unwrap();
    h.controller
        .enqueue_selected(selected_request(2, &entries, temp.path()))
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conflict_wait_accepts_more_work_and_rejects_stale_decisions() {
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
    assert_eq!(h.mutation.available_permits(), 0);
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
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!destination.join("second").exists());
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
    assert_eq!(fs::read(destination.join("first")).unwrap(), b"keep");
    assert_eq!(fs::read(destination.join("taken")).unwrap(), b"also keep");
    assert_eq!(fs::read(destination.join("renamed")).unwrap(), b"new bytes");
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
    assert_eq!(done.state, CopyOperationState::CompletedWithErrors);
    assert_eq!(done.skipped, 2);
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
async fn queue_is_bounded_and_duplicate_ids_cannot_execute_twice() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"source").unwrap();
    let mut h = Harness::native();
    let held = h.mutation.clone().acquire_owned().await.unwrap();
    for id in 1..=33 {
        h.controller
            .enqueue(request(id, std::slice::from_ref(&source), temp.path()))
            .await;
    }
    h.controller
        .enqueue(request(1, &[source], temp.path()))
        .await;
    let rejected = h.terminal(33).await;
    assert_eq!(rejected.state, CopyOperationState::Failed);
    assert!(rejected.message.contains("32"));
    h.controller.shutdown();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cancellation_finishes_while_another_mutation_still_holds_the_gate() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"source").unwrap();
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::native();
    let held = h.mutation.clone().acquire_owned().await.unwrap();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    h.controller.cancel(OperationId(1));
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
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
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(h.io.available_permits(), 0);
    assert_eq!(h.mutation.available_permits(), 0);
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
    assert_eq!(h.mutation.available_permits(), 1);
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
