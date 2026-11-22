//! Independently submitted copies must reach native I/O without completing one another.
use super::*;
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
    sync::{Condvar, OnceLock},
    time::SystemTime,
};

static CASE_SERIAL: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoldAfter {
    SourceRead,
    StagedWrite,
}

#[derive(Default)]
struct HeldCalls {
    entered: [bool; 2],
    released: [bool; 2],
}

struct Control {
    point: HoldAfter,
    sources: [PathBuf; 2],
    markers: [u8; 2],
    calls: Mutex<HeldCalls>,
    changed: Condvar,
    entered: mpsc::UnboundedSender<usize>,
}

impl Control {
    fn source_index(&self, path: &Path) -> usize {
        let matches: Vec<_> = self
            .sources
            .iter()
            .enumerate()
            .filter(|(_, source)| path == source.as_path() || path.starts_with(source))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "unidentified or ambiguous source {path:?}"
        );
        matches[0]
    }

    fn write_index(&self, written: &[u8]) -> usize {
        assert!(!written.is_empty());
        let index = self
            .markers
            .iter()
            .position(|marker| *marker == written[0])
            .expect("staged write has an unknown operation marker");
        assert!(written.iter().all(|byte| *byte == self.markers[index]));
        index
    }

    fn hold(&self, index: usize, point: HoldAfter) {
        if point != self.point {
            return;
        }
        let mut calls = self.calls.lock().unwrap();
        if calls.entered[index] {
            return;
        }
        calls.entered[index] = true;
        // The signal is emitted only after a successful delegated native call.
        let _ = self.entered.send(index);
        while !calls.released[index] {
            // Cancellation cannot pretend that this actual provider call returned.
            calls = self.changed.wait(calls).unwrap();
        }
    }

    fn release_all(&self) {
        self.calls.lock().unwrap().released = [true; 2];
        self.changed.notify_all();
    }
}

struct ReleaseAll(Arc<Control>);
impl Drop for ReleaseAll {
    fn drop(&mut self) {
        self.0.release_all();
    }
}

struct Source {
    inner: Box<dyn TransferSource>,
    control: Arc<Control>,
}

impl Source {
    fn wrap(inner: Box<dyn TransferSource>, control: &Arc<Control>) -> Box<dyn TransferSource> {
        Box::new(Self {
            inner,
            control: control.clone(),
        })
    }
}

impl TransferSource for Source {
    fn native_namespace_scope(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.inner.native_namespace_scope(cancel)
    }
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.inner.native_identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.inner.discover(cancel, visit)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        let count = self.inner.read_chunk(buffer, cancel)?;
        if count != 0 {
            let index = self.control.source_index(&self.inner.entry().item.path);
            assert!(
                buffer[..count]
                    .iter()
                    .all(|byte| *byte == self.control.markers[index])
            );
            self.control.hold(index, HoldAfter::SourceRead);
        }
        Ok(count)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner
            .next_child(cancel)
            .map(|child| child.map(|child| Self::wrap(child, &self.control)))
    }
    fn open_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner
            .open_child(name, cancel)
            .map(|child| Self::wrap(child, &self.control))
    }
    fn supports_read_retry(&self) -> bool {
        self.inner.supports_read_retry()
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.retry_read_from(offset, cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.inner.seek_for_validation(offset, cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.inner.prepare_modified_time(cancel)
    }
}

struct Service {
    inner: midnight_platform::LocalFileSystem,
    control: Arc<Control>,
}

// Native delegates and fixture state are destroyed synchronously; no retirement queues are hidden.
impl midnight_core::AuditedSynchronousTransferService for Service {}
impl TransferService for Service {
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .prepare_checksum_directory(path, cancel)
            .map(|directory| Directory::wrap(directory, &self.control))
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner
            .prepare_transfer_source(path, cancel)
            .map(|source| Source::wrap(source, &self.control))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .prepare_transfer_destination(path, cancel)
            .map(|directory| Directory::wrap(directory, &self.control))
    }
}

struct Directory {
    inner: Arc<dyn TransferDirectory>,
    control: Arc<Control>,
}

impl Directory {
    fn wrap(
        inner: Arc<dyn TransferDirectory>,
        control: &Arc<Control>,
    ) -> Arc<dyn TransferDirectory> {
        Arc::new(Self {
            inner,
            control: control.clone(),
        })
    }
    fn stage(&self, inner: Box<dyn StagedTransferFile>) -> Box<dyn StagedTransferFile> {
        Box::new(Stage {
            inner,
            control: self.control.clone(),
        })
    }
}

impl TransferDirectory for Directory {
    fn native_child_namespace_scope(
        &self,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.inner.native_child_namespace_scope(name, kind, cancel)
    }
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.inner.capabilities()
    }
    fn supports_native_move_replacement(&self, source: TransferIdentity) -> bool {
        self.inner.supports_native_move_replacement(source)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.inner.prepare_target(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .create_directory(name, cancel)
            .map(|directory| Self::wrap(directory, &self.control))
    }
    fn open_merge_directory(
        &self,
        name: &OsStr,
        expected: &TransferEntry,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner
            .open_merge_directory(name, expected, cancel)
            .map(|directory| Self::wrap(directory, &self.control))
    }
    fn open_read_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner.open_read_child(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner.stage_file(cancel).map(|stage| self.stage(stage))
    }
    fn stage_file_with_metadata(
        &self,
        policy: TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner
            .stage_file_with_metadata(policy, cancel)
            .map(|stage| self.stage(stage))
    }
    fn stage_file_for_target(
        &self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner
            .stage_file_for_target(name, target, policy, cancel)
            .map(|stage| self.stage(stage))
    }
    fn stage_file_preserving_target(
        &self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner
            .stage_file_preserving_target(name, target, policy, cancel)
            .map(|stage| self.stage(stage))
    }
    fn admit_link(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.admit_link(representation, cancel)
    }
    fn stage_link(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferLink>, TransferLinkStageError> {
        self.inner.stage_link(representation, cancel)
    }
}

struct Stage {
    inner: Box<dyn StagedTransferFile>,
    control: Arc<Control>,
}

impl Read for Stage {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}
impl Write for Stage {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(data)?;
        if count != 0 {
            let index = self.control.write_index(&data[..count]);
            self.control.hold(index, HoldAfter::StagedWrite);
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
impl StagedTransferFile for Stage {
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.sync(cancel)
    }
    fn len(&self) -> Result<u64, FsError> {
        self.inner.len()
    }
    fn set_modified_time(
        &mut self,
        expected: SystemTime,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.set_modified_time(expected, cancel)
    }
    fn modified_time(&self, cancel: &Cancellation) -> Result<SystemTime, FsError> {
        self.inner.modified_time(cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        self.inner.revalidate_publication(cancel)
    }
    fn prepare_replacement(
        &mut self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.prepare_replacement(name, target, policy, cancel)
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_no_replace(name, cancel)
    }
    fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_replace(name, cancel)
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.inner.cleanup()
    }
}

async fn both_default_copies_enter_native_io(directory_roots: bool, point: HoldAfter) {
    // Each case retains two real permits from its own configured Gate(2).
    // Keep these filesystem fixtures serialized without changing global settings.
    let _case = CASE_SERIAL
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let temporary = fixture();
    let base = temporary.path();
    let markers = [0x35, 0x79];
    let contents = [
        vec![markers[0]; 96 * 1024 + 7],
        vec![markers[1]; 128 * 1024 + 19],
    ];
    let sources;
    let destinations;
    let source_files;
    let outputs;
    if directory_roots {
        sources = [base.join("first tree"), base.join("second tree")];
        destinations = [
            base.join("first destination"),
            base.join("second destination"),
        ];
        source_files = [
            sources[0].join("nested/payload.bin"),
            sources[1].join("nested/payload.bin"),
        ];
        outputs = [
            destinations[0].join("first tree/nested/payload.bin"),
            destinations[1].join("second tree/nested/payload.bin"),
        ];
        for index in 0..2 {
            fs::create_dir_all(source_files[index].parent().unwrap()).unwrap();
            fs::create_dir(&destinations[index]).unwrap();
        }
    } else {
        let shared_source = base.join("shared source");
        let shared_destination = base.join("shared destination");
        fs::create_dir(&shared_source).unwrap();
        fs::create_dir(&shared_destination).unwrap();
        sources = [
            shared_source.join("first.bin"),
            shared_source.join("second.bin"),
        ];
        destinations = [shared_destination.clone(), shared_destination];
        source_files = sources.clone();
        outputs = [
            destinations[0].join("first.bin"),
            destinations[1].join("second.bin"),
        ];
    }
    for index in 0..2 {
        fs::write(&source_files[index], &contents[index]).unwrap();
    }
    let (entered, mut calls) = mpsc::unbounded_channel();
    let control = Arc::new(Control {
        point,
        sources: sources.clone(),
        markers,
        calls: Mutex::default(),
        changed: Condvar::new(),
        entered,
    });
    // Exactly two actual configured slots; no extra copy semaphore participates.
    let mut h = Harness::with_slots(
        Some(Arc::new(Service {
            inner: midnight_platform::LocalFileSystem::new(None),
            control: control.clone(),
        })),
        2,
    );
    let _release = ReleaseAll(control.clone());
    for index in 0..2 {
        h.controller
            .enqueue(request(
                index as u64 + 1,
                std::slice::from_ref(&sources[index]),
                &destinations[index],
            ))
            .await;
    }
    let overlap = tokio::time::timeout(Duration::from_secs(6), async {
        let mut seen = [false; 2];
        while seen != [true; 2] {
            tokio::select! {
                entered = calls.recv() => {
                    let index = entered.expect("provider entry stream closed");
                    assert!(index < 2 && !seen[index], "duplicate or foreign provider entry");
                    seen[index] = true;
                }
                event = h.events.recv() => {
                    match event.expect("copy event stream closed") {
                        Event::Copy(CopyEvent::Progress(progress)) => {
                            assert!(!progress.state.is_terminal(),
                                "operation completed before both native calls entered: {progress:?}");
                        }
                        Event::Copy(event) => panic!("unexpected copy event before entry: {event:?}"),
                        _ => panic!("unexpected non-copy event before both calls entered"),
                    }
                }
            }
        }
    }).await;
    let held_before_release = {
        let held = control.calls.lock().unwrap();
        (held.entered, held.released)
    };
    let unpublished_before_release = outputs.iter().all(|path| !path.exists());
    control.release_all();
    if overlap.is_err() {
        h.controller.shutdown();
    }
    // Collect both terminals together: a per-id helper can discard the other job's terminal.
    let completed = tokio::time::timeout(Duration::from_secs(8), async {
        let mut completed: [Option<Arc<CopyProgress>>; 2] = [None, None];
        while completed.iter().any(Option::is_none) {
            match h.events.recv().await.expect("copy event stream closed") {
                Event::Copy(CopyEvent::Progress(progress)) => {
                    let index = match progress.id {
                        OperationId(1) => 0,
                        OperationId(2) => 1,
                        id => panic!("foreign operation {id:?}"),
                    };
                    if progress.state.is_terminal() {
                        assert!(completed[index].is_none(), "duplicate terminal");
                        completed[index] = Some(progress);
                    }
                }
                Event::Copy(CopyEvent::RootsCompleted { id, .. }) => {
                    assert!(matches!(id, OperationId(1) | OperationId(2)));
                }
                Event::Copy(event) => {
                    panic!("unexpected decision or failure during copy: {event:?}")
                }
                _ => panic!("unexpected non-copy event during copy"),
            }
        }
        completed
    })
    .await
    .expect("both released copies must publish terminal progress");
    h.close().await;
    assert!(
        overlap.is_ok(),
        "both default copies must enter native I/O before either is released"
    );
    assert_eq!(held_before_release, ([true; 2], [false; 2]));
    assert!(
        unpublished_before_release,
        "held calls cannot publish either output"
    );
    for (index, progress) in completed.into_iter().enumerate() {
        let progress = progress.unwrap();
        let length = contents[index].len() as u64;
        assert_eq!(
            progress.state,
            CopyOperationState::Completed,
            "{progress:?}"
        );
        assert_eq!(progress.verification, TransferVerification::FullSha256);
        assert_eq!(progress.copied_bytes, length);
        assert_eq!(progress.verified_bytes, length);
        assert_eq!(progress.completed_bytes, length);
        assert_eq!(progress.completed_roots, 1);
        assert_eq!(progress.files_completed, 1);
        assert_eq!(
            (progress.failed, progress.skipped, progress.recovery_count),
            (0, 0, 0)
        );
        assert!(progress.errors.is_empty(), "{progress:?}");
        assert_eq!(fs::read(&outputs[index]).unwrap(), contents[index]);
        assert_eq!(fs::read(&source_files[index]).unwrap(), contents[index]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_same_folder_files_overlap_after_native_source_reads() {
    both_default_copies_enter_native_io(false, HoldAfter::SourceRead).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_same_folder_files_overlap_after_native_staged_writes() {
    both_default_copies_enter_native_io(false, HoldAfter::StagedWrite).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_disjoint_directory_roots_overlap_after_native_source_reads() {
    both_default_copies_enter_native_io(true, HoldAfter::SourceRead).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_disjoint_directory_roots_overlap_after_native_staged_writes() {
    both_default_copies_enter_native_io(true, HoldAfter::StagedWrite).await;
}
