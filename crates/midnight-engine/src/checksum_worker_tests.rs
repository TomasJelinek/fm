use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

struct ReadBarrier {
    entered: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
    dropped: AtomicBool,
}
impl ReadBarrier {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            released: std::sync::Mutex::new(false),
            wake: std::sync::Condvar::new(),
            dropped: AtomicBool::new(false),
        })
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
struct ReleaseOnDrop(Arc<ReadBarrier>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct Reads {
    bytes: AtomicU64,
    calls: AtomicUsize,
    /// Reads made while holding exclusive disk admission.
    exclusive: AtomicUsize,
    revalidations: AtomicUsize,
    buffer: AtomicUsize,
    interrupted: AtomicBool,
    changed: AtomicBool,
}
impl Default for Reads {
    fn default() -> Self {
        Self {
            bytes: AtomicU64::new(0),
            calls: AtomicUsize::new(0),
            exclusive: AtomicUsize::new(0),
            revalidations: AtomicUsize::new(0),
            buffer: AtomicUsize::new(0),
            interrupted: AtomicBool::new(false),
            changed: AtomicBool::new(false),
        }
    }
}
#[derive(Clone)]
struct Provider {
    data: Arc<[u8]>,
    reads: Arc<Reads>,
    chunk: usize,
    change: bool,
    cancellation: Option<Cancellation>,
    fail: bool,
    declared: Option<u64>,
    barrier: Option<Arc<ReadBarrier>>,
}
impl Provider {
    fn new(data: &[u8]) -> Self {
        Self {
            data: data.to_vec().into(),
            reads: Arc::default(),
            chunk: 3,
            change: false,
            cancellation: None,
            fail: false,
            declared: None,
            barrier: None,
        }
    }
}
impl TransferService for Provider {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(Source {
            provider: self.clone(),
            offset: 0,
            reading: false,
            entry: TransferEntry {
                item: MutationItem {
                    path: path.to_owned(),
                    kind: EntryKind::File,
                    metadata: EntryMetadata {
                        size: Some(self.declared.unwrap_or(self.data.len() as u64)),
                        ..EntryMetadata::default()
                    },
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [1; 16],
                },
            },
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(failure(
            path,
            io::ErrorKind::Unsupported,
            "read-only test provider",
        ))
    }
}
struct Source {
    provider: Provider,
    offset: usize,
    entry: TransferEntry,
    /// A read began: only that handle's drop counts for the read barrier (the
    /// listing pass opens and drops a handle without reading).
    reading: bool,
}
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.provider
            .reads
            .revalidations
            .fetch_add(1, Ordering::SeqCst);
        if cancel.is_cancelled() {
            return Err(failure(
                &self.entry.item.path,
                io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        if self.provider.reads.changed.load(Ordering::SeqCst) {
            return Err(failure(
                &self.entry.item.path,
                io::ErrorKind::InvalidData,
                "changed",
            ));
        }
        Ok(())
    }
    fn read_chunk(&mut self, buffer: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        self.reading = true;
        if let Some(barrier) = &self.provider.barrier {
            barrier.entered.notify_one();
            let mut released = barrier.released.lock().unwrap();
            while !*released {
                released = barrier.wake.wait(released).unwrap();
            }
        }
        self.provider.reads.calls.fetch_add(1, Ordering::SeqCst);
        if holds_exclusive_io() {
            self.provider.reads.exclusive.fetch_add(1, Ordering::SeqCst);
        }
        self.provider
            .reads
            .buffer
            .fetch_max(buffer.len(), Ordering::SeqCst);
        if self
            .provider
            .reads
            .interrupted
            .swap(false, Ordering::SeqCst)
        {
            return Err(failure(
                &self.entry.item.path,
                io::ErrorKind::Interrupted,
                "transient interruption",
            ));
        }
        if self.provider.fail && self.offset > 0 {
            return Err(failure(
                &self.entry.item.path,
                io::ErrorKind::PermissionDenied,
                "read failure",
            ));
        }
        let count = self
            .provider
            .chunk
            .min(buffer.len())
            .min(self.provider.data.len() - self.offset);
        buffer[..count].copy_from_slice(&self.provider.data[self.offset..self.offset + count]);
        self.offset += count;
        self.provider
            .reads
            .bytes
            .fetch_add(count as u64, Ordering::SeqCst);
        if count > 0 {
            if self.provider.change {
                self.provider.reads.changed.store(true, Ordering::SeqCst);
            }
            if let Some(cancel) = &self.provider.cancellation {
                cancel.cancel();
            }
        }
        Ok(count)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Err(failure(
            &self.entry.item.path,
            io::ErrorKind::Unsupported,
            "not a directory",
        ))
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        if let Some(barrier) = self.provider.barrier.as_ref().filter(|_| self.reading) {
            barrier.dropped.store(true, Ordering::SeqCst);
        }
    }
}
fn request() -> ChecksumRequest {
    ChecksumRequest::new(
        ChecksumJobId(700),
        ChecksumGeneration(1),
        vec![
            std::env::current_dir()
                .unwrap()
                .join("_build/virtual-checksum-file"),
        ],
    )
}
fn run(request: ChecksumRequest, provider: &Provider, cancel: &Cancellation) -> Arc<ChecksumRun> {
    calculate(
        request,
        provider,
        &UNCLASSIFIED,
        cancel,
        ChecksumRetention::new(()),
        |_| true,
    )
}
/// The run's row at `index`.
fn row(run: &ChecksumRun, index: usize) -> ChecksumRowRef<'_> {
    run.rows.get(index).expect("a row")
}
/// A calculation of `request` with other settings, reporting to `emit`.
fn run_with(
    request: &ChecksumRequest,
    service: &dyn TransferService,
    settings: Settings,
    emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let targets = Targets::Request(&request.targets);
    let pass = Pass {
        request,
        targets: &targets,
        base: common_base(&request.targets),
        settings,
    };
    super::run(
        pass,
        Arc::from([]),
        service,
        &UNCLASSIFIED,
        &Cancellation::default(),
        ChecksumRetention::new(()),
        emit,
    )
}
/// A verification of the files the request names, below their folder.
fn verify_run(
    request: &ChecksumRequest,
    service: &dyn TransferService,
    emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let folder: Arc<Path> = common_base(&request.targets);
    let names: Vec<Box<Path>> = request
        .targets
        .iter()
        .map(|target| target.path.strip_prefix(&folder).unwrap().into())
        .collect();
    calculate_for_verification(
        request,
        folder.clone(),
        &names,
        folder.join("checksums.sha256"),
        service,
        &UNCLASSIFIED,
        &Cancellation::default(),
        ChecksumRetention::new(()),
        emit,
    )
}

#[test]
fn short_reads_and_interruption_feed_all_hashes_with_one_ordered_read_stream() {
    let provider = Provider::new(b"abcdefghijklm");
    provider.reads.interrupted.store(true, Ordering::SeqCst);
    let mut request = request();
    request.algorithms = ChecksumAlgorithms::all();
    let result = run(request, &provider, &Cancellation::default());
    assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(row(&result, 0).digests.iter().count(), 5);
    assert_eq!(provider.reads.bytes.load(Ordering::SeqCst), 13);
    assert_eq!(provider.reads.calls.load(Ordering::SeqCst), 7); // EINTR, five short reads, EOF.
    assert_eq!(
        provider.reads.buffer.load(Ordering::SeqCst),
        CHECKSUM_BUFFER_BYTES
    );
    assert_eq!(
        row(&result, 0)
            .digest(ChecksumAlgorithm::Sha256)
            .unwrap()
            .bytes,
        Sha256::digest(b"abcdefghijklm").as_slice()
    );
}

#[test]
fn crc32_uses_sfv_byte_order_and_preserves_incremental_reads() {
    for (input, expected) in [
        (b"".as_slice(), "00000000"),
        (b"abc".as_slice(), "352441c2"),
        (b"123456789".as_slice(), "cbf43926"),
    ] {
        let provider = Provider::new(input);
        let mut request = request();
        request.algorithms = ChecksumAlgorithms::new(&[ChecksumAlgorithm::Crc32]).unwrap();
        let result = run(request, &provider, &Cancellation::default());
        assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
        assert_eq!(row(&result, 0).digests.iter().count(), 1);
        assert_eq!(
            row(&result, 0)
                .digest(ChecksumAlgorithm::Crc32)
                .unwrap()
                .hex(),
            expected
        );
        assert_eq!(
            provider.reads.bytes.load(Ordering::SeqCst),
            input.len() as u64
        );
    }
}

#[test]
fn changed_unreadable_cancelled_and_wrong_length_files_never_publish_partial_digests() {
    for scenario in 0..4 {
        let cancel = Cancellation::default();
        let mut provider = Provider::new(b"source bytes");
        match scenario {
            0 => provider.change = true,
            1 => provider.fail = true,
            2 => provider.cancellation = Some(cancel.clone()),
            _ => provider.declared = Some(999),
        }
        let result = run(request(), &provider, &cancel);
        assert_eq!(result.rows.len(), 1);
        assert_ne!(row(&result, 0).state, ChecksumRowState::Calculated);
        assert!(row(&result, 0).digests.is_empty());
        assert!(row(&result, 0).bytes_read > 0);
        assert_ne!(result.summary.completion, ChecksumCompletion::Completed);
    }
}

#[test]
fn source_revalidation_brackets_reads_and_repeats_only_at_a_bounded_cadence() {
    let mut provider = Provider::new(&[7; 200]);
    provider.chunk = 1;
    let started = Instant::now();
    let result = run(request(), &provider, &Cancellation::default());
    let elapsed = started.elapsed();
    assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(provider.reads.calls.load(Ordering::SeqCst), 201);
    // Each check reopens the native parent and name: once before the first and
    // once after the last read, plus at most one per elapsed interval, never per chunk.
    let revalidations = provider.reads.revalidations.load(Ordering::SeqCst);
    let periodic = (elapsed.as_nanos() / REVALIDATION_INTERVAL.as_nanos()) as usize;
    assert!(
        (2..=2 + periodic).contains(&revalidations),
        "{revalidations} revalidations for 200 chunks in {elapsed:?}"
    );
}

#[test]
fn due_revalidation_stops_a_changed_source_before_its_end() {
    let mut provider = Provider::new(&[1; 30]);
    provider.change = true;
    let settings = Settings {
        revalidation: Duration::ZERO,
        ..CALCULATION
    };
    let result = run_with(&request(), &provider, settings, |_| true);
    assert_eq!(row(&result, 0).state, ChecksumRowState::SourceChanged);
    assert!(row(&result, 0).digests.is_empty());
    assert_eq!(
        row(&result, 0).bytes_read,
        3,
        "stopped after the first chunk"
    );
}

fn export(run: &ChecksumRun, directory: &Path) -> Result<Vec<u8>, FsError> {
    let included = run
        .rows
        .iter()
        .filter(|row| row.state == ChecksumRowState::Calculated)
        .count() as u64;
    let partial = ChecksumPartialAcknowledgement {
        included,
        excluded: run.rows.len() as u64 - included,
        incomplete_discovery: run.summary.completion != ChecksumCompletion::Completed,
    };
    let request = ChecksumManifestRequest {
        algorithm: ChecksumAlgorithm::Sha256,
        destination: directory.join("out.sha256"),
        acknowledge_partial: (partial.excluded > 0 || partial.incomplete_discovery)
            .then_some(partial),
        overwrite: false,
    };
    super::super::manifest::encode(run, &request, directory, &Cancellation::default())
        .map(|(bytes, _)| bytes)
}

#[test]
fn duplicate_roots_do_not_skip_a_later_distinct_source() {
    let provider = Provider::new(b"abc");
    let mut request = request();
    request.targets.push(request.targets[0].clone());
    let mut second = request.targets[0].clone();
    second.path.set_file_name("second");
    request.targets.push(second);
    let result = run(request, &provider, &Cancellation::default());
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.summary.progress.bytes_read, 6);
    assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
}

#[test]
fn large_declared_files_allocate_only_one_fixed_buffer_and_cancel_responsively() {
    let cancel = Cancellation::default();
    let mut provider = Provider::new(b"first bounded block");
    provider.declared = Some(4 * 1024 * 1024 * 1024 + 100);
    provider.cancellation = Some(cancel.clone());
    let result = run(request(), &provider, &cancel);
    assert_eq!(result.summary.completion, ChecksumCompletion::Cancelled);
    assert_eq!(provider.reads.buffer.load(Ordering::SeqCst), 256 * 1024);
    assert!(row(&result, 0).digests.is_empty());
}

#[test]
fn empty_stream_has_all_four_standard_digests() {
    let mut request = request();
    request.algorithms = ChecksumAlgorithms::all();
    let result = run(request, &Provider::new(b""), &Cancellation::default());
    for (algorithm, hex) in [
        (ChecksumAlgorithm::Md5, "d41d8cd98f00b204e9800998ecf8427e"),
        (
            ChecksumAlgorithm::Sha1,
            "da39a3ee5e6b4b0d3255bfef95601890afd80709",
        ),
        (
            ChecksumAlgorithm::Sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            ChecksumAlgorithm::Sha512,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
        ),
    ] {
        assert_eq!(row(&result, 0).digest(algorithm).unwrap().hex(), hex);
    }
}

#[tokio::test]
async fn controller_cancellation_and_retained_job_slots_are_bounded() {
    let _quota_fixture = super::super::test_quota_fixture().await;
    let provider = Arc::new(Provider::new(b"abc"));
    let mut controller =
        super::super::Controller::new(super::super::Retirement::testing(), provider);
    assert!(controller.try_next().is_none());
    let mut runs = Vec::new();
    for job in 1..=32 {
        let mut request = request();
        request.job = ChecksumJobId(job);
        controller.start(request).unwrap();
    }
    let mut rejected = request();
    rejected.job = ChecksumJobId(33);
    assert_eq!(
        controller.start(rejected.clone()).unwrap_err().kind,
        io::ErrorKind::WouldBlock
    );
    for _ in 0..32 {
        loop {
            if let ChecksumUpdate::Finished(run) =
                tokio::time::timeout(Duration::from_secs(5), controller.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .update
            {
                runs.push(run);
                break;
            }
        }
    }
    assert!(controller.start(rejected.clone()).is_err()); // Results own the job slot.
    let released = runs.pop().unwrap();
    let retained_chunk = released.rows.chunks()[0].clone();
    drop(released);
    // A chunk of its rows alone (as a window keeps them) still holds the slot.
    assert!(controller.start(rejected.clone()).is_err());
    drop(retained_chunk);
    controller.start(rejected).unwrap();
    controller.cancel(ChecksumJobId(33), ChecksumGeneration(1));
    let cancelled = loop {
        if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
            break run;
        }
    };
    assert_eq!(cancelled.summary.completion, ChecksumCompletion::Cancelled);
    controller.shutdown().await;
}

#[tokio::test]
async fn cancellation_and_shutdown_retain_an_inflight_read_and_its_io_permit() {
    let _quota_fixture = super::super::test_quota_fixture().await;
    let barrier = ReadBarrier::new();
    let _release = ReleaseOnDrop(barrier.clone());
    let mut provider = Provider::new(b"abc");
    provider.barrier = Some(barrier.clone());
    let controller =
        super::super::Controller::new(super::super::Retirement::testing(), Arc::new(provider));
    controller.start(request()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), barrier.entered.notified())
        .await
        .unwrap();
    controller.cancel(ChecksumJobId(700), ChecksumGeneration(1));
    let kept_handle = !barrier.dropped.load(Ordering::SeqCst);
    let kept_permit = midnight_work_budget::shared_io().snapshot().available == 0;
    let closing = tokio::spawn(controller.shutdown());
    tokio::task::yield_now().await;
    let still_waiting = !closing.is_finished();
    barrier.release();
    tokio::time::timeout(Duration::from_secs(5), closing)
        .await
        .unwrap()
        .unwrap();
    assert!(kept_handle && kept_permit && still_waiting);
    assert!(barrier.dropped.load(Ordering::SeqCst));
    // Other tests may immediately acquire the global gate after this read exits.
    // Prove all permits can be reacquired, rather than sampling a transient idle state.
    let permit = tokio::task::spawn_blocking(|| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        midnight_work_budget::shared_io()
            .acquire(midnight_work_budget::Admission::Exclusive, || {
                std::time::Instant::now() >= deadline
            })
    })
    .await
    .unwrap();
    assert!(permit.is_some(), "shutdown must release its I/O permit");
}

/// A directory root holding one file of the provider's data on the same volume.
struct Mounted(Provider);
struct Folder {
    entry: TransferEntry,
    child: Option<Box<dyn TransferSource>>,
}
impl TransferService for Mounted {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(Folder {
            entry: TransferEntry {
                item: MutationItem {
                    path: path.to_owned(),
                    kind: EntryKind::Directory,
                    metadata: EntryMetadata::default(),
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [2; 16],
                },
            },
            child: Some(self.0.prepare_transfer_source(&path.join("file"), cancel)?),
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_transfer_destination(path, cancel)
    }
}
impl TransferSource for Folder {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Err(failure(
            &self.entry.item.path,
            io::ErrorKind::Unsupported,
            "a directory",
        ))
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Ok(self.child.take())
    }
}

#[test]
fn only_confirmed_local_ssd_reads_share_the_disk_gate_and_a_directory_is_classified_once() {
    for ssd in [true, false] {
        let provider = Provider::new(b"abcdefgh");
        let mut request = request();
        let directory = request.targets[0].path.parent().unwrap().to_owned();
        for name in ["second", "third"] {
            let mut target = request.targets[0].clone();
            target.path.set_file_name(name);
            request.targets.push(target);
        }
        let volumes = Recorded::new(move |_| ssd);
        let result = calculate(
            request,
            &provider,
            &volumes,
            &Cancellation::default(),
            ChecksumRetention::new(()),
            |_| true,
        );
        assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
        assert_eq!(result.rows.len(), 3);
        assert_eq!(volumes.queries(), [directory], "one query per directory");
        let reads = provider.reads.calls.load(Ordering::SeqCst);
        let exclusive = provider.reads.exclusive.load(Ordering::SeqCst);
        assert!(reads > 0);
        // Unknown, rotational and network storage is never read beside other I/O.
        assert_eq!(exclusive, if ssd { 0 } else { reads }, "ssd: {ssd}");
    }
}

#[test]
fn a_recursive_directory_root_is_read_as_its_own_volume_allows() {
    // A selected directory can be a mount point of a volume unlike its parent's.
    for root_ssd in [true, false] {
        let provider = Provider::new(b"abc");
        let mut request = request();
        request.recursive = true;
        let root = request.targets[0].path.clone();
        let parent = root.parent().unwrap().to_owned();
        let volumes = Recorded::new({
            let root = root.clone();
            move |path| (path == root) == root_ssd
        });
        let result = calculate(
            request,
            &Mounted(provider.clone()),
            &volumes,
            &Cancellation::default(),
            ChecksumRetention::new(()),
            |_| true,
        );
        assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(volumes.queries(), [parent, root]);
        let reads = provider.reads.calls.load(Ordering::SeqCst);
        let exclusive = provider.reads.exclusive.load(Ordering::SeqCst);
        assert!(reads > 0);
        assert_eq!(
            exclusive,
            if root_ssd { 0 } else { reads },
            "root ssd: {root_ssd}"
        );
    }
}

/// Three files of eight bytes, calculated with `update` as the update interval: the
/// run, everything it sent, the files' paths and their provider.
fn listed_run(
    update: Duration,
) -> (
    Arc<ChecksumRun>,
    Vec<ChecksumUpdate>,
    Vec<PathBuf>,
    Provider,
) {
    let provider = Provider::new(b"abcdefgh");
    let mut request = request();
    for name in ["second", "third"] {
        let mut target = request.targets[0].clone();
        target.path.set_file_name(name);
        request.targets.push(target);
    }
    let paths: Vec<_> = request
        .targets
        .iter()
        .map(|target| target.path.clone())
        .collect();
    let mut updates = Vec::new();
    let result = run_with(
        &request,
        &provider,
        Settings {
            update,
            ..CALCULATION
        },
        |update| {
            updates.push(update);
            true
        },
    );
    (result, updates, paths, provider)
}

/// How many rows each batch sent carried, in order.
fn batches(updates: &[ChecksumUpdate]) -> Vec<usize> {
    updates
        .iter()
        .filter_map(|update| match update {
            ChecksumUpdate::Batch(chunk) => Some(chunk.rows.len()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_calculation_lists_its_files_first_and_reports_each_one_as_it_finishes() {
    // Every file takes longer than a zero update interval: no test waits in real time.
    let (result, updates, paths, provider) = listed_run(Duration::ZERO);
    assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
    // Every file is listed, with its length, before any is read.
    assert!(matches!(&updates[0], ChecksumUpdate::Progress(progress) if progress.listing));
    let first_row = updates
        .iter()
        .position(|update| matches!(update, ChecksumUpdate::Batch(_)))
        .unwrap();
    let listed: Vec<_> = updates[..first_row]
        .iter()
        .filter_map(|update| match update {
            ChecksumUpdate::Planned(planned) => Some(
                planned
                    .files
                    .iter()
                    .map(|file| (planned.base.join(&file.name), file.size))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        listed
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>(),
        paths
    );
    assert!(listed.iter().all(|(_, size)| *size == Some(8)));
    assert_eq!(
        provider.reads.bytes.load(Ordering::SeqCst),
        24,
        "listing reads nothing; each file is read once"
    );
    // Reading reports the listed totals and the file being read.
    let reading = updates
        .iter()
        .find_map(|update| match update {
            ChecksumUpdate::Progress(progress) if !progress.listing => Some(progress),
            _ => None,
        })
        .unwrap();
    assert_eq!((reading.planned, reading.planned_bytes), (3, 24));
    assert!(updates.iter().any(|update| matches!(
        update,
        ChecksumUpdate::Progress(progress)
            if progress.current.is_some() && progress.current_size == Some(8)
    )));
    assert_eq!(result.summary.progress.planned, 3);
    // Files that take longer than the update interval arrive one by one, not as
    // one batch at the end.
    assert_eq!(batches(&updates), [1, 1, 1]);
    // Files that all finish within the interval wait for it and go out together.
    let (result, updates, ..) = listed_run(Duration::MAX);
    assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(batches(&updates), [3]);
}

#[test]
fn verification_reads_without_listing() {
    let provider = Provider::new(b"abc");
    let mut updates = Vec::new();
    verify_run(&request(), &provider, |update| {
        updates.push(update);
        true
    });
    assert!(
        !updates
            .iter()
            .any(|update| matches!(update, ChecksumUpdate::Planned(_))
                || matches!(update, ChecksumUpdate::Progress(progress) if progress.listing))
    );
}

/// A folder holding `top.txt` and a subfolder `sub` holding `deep.txt`; `sub` is on
/// the volume with the given device number (the folder's is 1).
struct Nested(Provider, u64);
struct Dir {
    entry: TransferEntry,
    children: Vec<Box<dyn TransferSource>>,
}
impl TransferSource for Dir {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Err(failure(
            &self.entry.item.path,
            io::ErrorKind::Unsupported,
            "a directory",
        ))
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Ok(self.children.pop())
    }
}
fn dir(
    path: &Path,
    (device, object): (u64, u8),
    children: Vec<Box<dyn TransferSource>>,
) -> Box<dyn TransferSource> {
    Box::new(Dir {
        entry: TransferEntry {
            item: MutationItem {
                path: path.to_owned(),
                kind: EntryKind::Directory,
                metadata: EntryMetadata::default(),
            },
            identity: TransferIdentity {
                device,
                object: [object; 16],
            },
        },
        children,
    })
}
impl TransferService for Nested {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let top = self
            .0
            .prepare_transfer_source(&path.join("top.txt"), cancel)?;
        let sub = path.join("sub");
        let deep = self
            .0
            .prepare_transfer_source(&sub.join("deep.txt"), cancel)?;
        // Children are taken from the end: top.txt first, then sub.
        Ok(dir(
            path,
            (1, 3),
            vec![dir(&sub, (self.1, 4), vec![deep]), top],
        ))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_transfer_destination(path, cancel)
    }
}

#[test]
fn a_selected_folder_contributes_its_own_files_and_include_subfolders_the_rest() {
    for recursive in [false, true] {
        let mut request = request();
        request.recursive = recursive;
        let root = request.targets[0].path.clone();
        let run = calculate(
            request,
            &Nested(Provider::new(b"abc"), 1),
            &UNCLASSIFIED,
            &Cancellation::default(),
            ChecksumRetention::new(()),
            |_| true,
        );
        let rows: Vec<_> = run.rows.iter().map(|row| (row.path(), row.state)).collect();
        let mut wanted = vec![(root.join("top.txt"), ChecksumRowState::Calculated)];
        if recursive {
            wanted.push((
                root.join("sub").join("deep.txt"),
                ChecksumRowState::Calculated,
            ));
        }
        // Without Include subfolders the subfolder is left out, not reported.
        assert_eq!(rows, wanted, "recursive: {recursive}");
        assert_eq!(run.summary.completion, ChecksumCompletion::Completed);
    }
    // Verification names files: a folder in its place is still skipped.
    let run = verify_run(&request(), &Nested(Provider::new(b"abc"), 1), |_| true);
    assert_eq!(run.rows.len(), 1);
    assert_eq!(row(&run, 0).state, ChecksumRowState::Skipped);
}

#[test]
fn a_subfolder_mounted_from_another_volume_is_left_out_unless_subfolders_are_included() {
    for recursive in [false, true] {
        let mut request = request();
        request.recursive = recursive;
        let root = request.targets[0].path.clone();
        // `sub` is a mount point, as ~/nas or /media/<user>/stick is.
        let run = calculate(
            request,
            &Nested(Provider::new(b"abc"), 2),
            &UNCLASSIFIED,
            &Cancellation::default(),
            ChecksumRetention::new(()),
            |_| true,
        );
        let rows: Vec<_> = run.rows.iter().map(|row| (row.path(), row.state)).collect();
        let mut wanted = vec![(root.join("top.txt"), ChecksumRowState::Calculated)];
        // Without Include subfolders it is left out like any subfolder: no row, and
        // the run is complete. With them, a walk does not cross into another volume:
        // the mount point is skipped and the run says it is incomplete.
        let completion = if recursive {
            wanted.push((root.join("sub"), ChecksumRowState::Skipped));
            ChecksumCompletion::Incomplete
        } else {
            ChecksumCompletion::Completed
        };
        assert_eq!(rows, wanted, "recursive: {recursive}");
        assert_eq!(run.summary.completion, completion, "recursive: {recursive}");
        assert_eq!(
            run.summary.progress.skipped,
            u64::from(recursive),
            "recursive: {recursive}"
        );
    }
}

/// A child of [`Tree`]: a three-byte file, a folder of `children` (one whose listing fails
/// when `unreadable`), a link or a special entry, on `device`, with an object identity.
struct Node {
    name: String,
    kind: EntryKind,
    device: u64,
    object: u8,
    unreadable: bool,
    children: Vec<Node>,
}
fn node(name: &str, kind: EntryKind, object: u8) -> Node {
    Node {
        name: name.to_owned(),
        kind,
        device: 1,
        object,
        unreadable: false,
        children: Vec::new(),
    }
}

/// What a [`Tree`] opened, by kind: files, folders, and links or special entries.
#[derive(Default)]
struct Opens {
    files: AtomicUsize,
    folders: AtomicUsize,
    others: AtomicUsize,
}
impl Opens {
    fn snapshot(&self) -> (usize, usize, usize) {
        (
            self.files.load(Ordering::SeqCst),
            self.folders.load(Ordering::SeqCst),
            self.others.load(Ordering::SeqCst),
        )
    }
}

/// A selected folder (device 1) whose folders either describe the children they are
/// not asked to open, as native sources do, or open every child as `next_child` does,
/// which is the default listing and the one before it. Links and special entries are
/// refused when opened, as native sources refuse them.
struct Tree {
    root: fn() -> Vec<Node>,
    describes: bool,
    opens: Arc<Opens>,
}
struct TreeFolder {
    entry: TransferEntry,
    children: std::collections::VecDeque<Node>,
    describes: bool,
    unreadable: bool,
    opens: Arc<Opens>,
}
impl TreeFolder {
    fn entry_of(&self, node: &Node) -> TransferEntry {
        TransferEntry {
            item: MutationItem {
                path: self.entry.item.path.join(&node.name),
                kind: node.kind,
                metadata: EntryMetadata {
                    size: (node.kind == EntryKind::File).then_some(3),
                    ..EntryMetadata::default()
                },
            },
            identity: TransferIdentity {
                device: node.device,
                object: [node.object; 16],
            },
        }
    }
    fn open(&self, node: Node) -> Result<Box<dyn TransferSource>, FsError> {
        let entry = self.entry_of(&node);
        let path = &entry.item.path;
        match node.kind {
            EntryKind::File => {
                self.opens.files.fetch_add(1, Ordering::SeqCst);
                Provider::new(b"abc").prepare_transfer_source(path, &Cancellation::default())
            }
            EntryKind::Directory => {
                self.opens.folders.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(TreeFolder {
                    entry,
                    children: node.children.into(),
                    describes: self.describes,
                    unreadable: node.unreadable,
                    opens: self.opens.clone(),
                }))
            }
            _ => {
                self.opens.others.fetch_add(1, Ordering::SeqCst);
                Err(failure(
                    path,
                    io::ErrorKind::Unsupported,
                    "copying links and special entries is not supported",
                ))
            }
        }
    }
}
impl TransferService for Tree {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(TreeFolder {
            entry: TransferEntry {
                item: MutationItem {
                    path: path.to_owned(),
                    kind: EntryKind::Directory,
                    metadata: EntryMetadata::default(),
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [100; 16],
                },
            },
            children: (self.root)().into(),
            describes: self.describes,
            unreadable: false,
            opens: self.opens.clone(),
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(failure(path, io::ErrorKind::Unsupported, "read-only tree"))
    }
}
impl TransferSource for TreeFolder {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Err(failure(
            &self.entry.item.path,
            io::ErrorKind::Unsupported,
            "a directory",
        ))
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        if self.unreadable {
            return Err(failure(
                &self.entry.item.path,
                io::ErrorKind::PermissionDenied,
                "Access is denied.",
            ));
        }
        self.children
            .pop_front()
            .map(|node| self.open(node))
            .transpose()
    }
    fn next_listed_child(
        &mut self,
        open: &dyn Fn(EntryKind) -> bool,
        cancel: &Cancellation,
    ) -> Result<Option<ListedChild>, FsError> {
        if self.unreadable {
            return Err(failure(
                &self.entry.item.path,
                io::ErrorKind::PermissionDenied,
                "Access is denied.",
            ));
        }
        if !self.describes {
            return self
                .next_child(cancel)
                .map(|child| child.map(ListedChild::Source));
        }
        let Some(node) = self.children.pop_front() else {
            return Ok(None);
        };
        let mount = node.kind == EntryKind::Directory && node.device != self.entry.identity.device;
        if open(node.kind) && !mount {
            return self
                .open(node)
                .map(|source| Some(ListedChild::Source(source)));
        }
        Ok(Some(ListedChild::Entry(self.entry_of(&node))))
    }
}

/// Files, a link, a folder with a file and a mount point of another volume with a file.
fn tree() -> Vec<Node> {
    let mut sub = node("sub", EntryKind::Directory, 2);
    sub.children.push(node("deep.txt", EntryKind::File, 3));
    let mut mount = node("mnt", EntryKind::Directory, 4);
    mount.device = 2;
    mount.children.push(node("far.txt", EntryKind::File, 5));
    vec![
        node("a.txt", EntryKind::File, 6),
        node("link", EntryKind::Symlink, 7),
        sub,
        mount,
        node("b.txt", EntryKind::File, 8),
    ]
}

/// What one calculation of [`tree`] planned and reported, and what it opened while
/// listing (counted when the reading pass starts) and in all.
struct Listed {
    planned: Vec<PathBuf>,
    rows: Vec<(PathBuf, ChecksumRowState, String)>,
    completion: ChecksumCompletion,
    listing_opens: (usize, usize, usize),
    opens: (usize, usize, usize),
}
fn list_tree(describes: bool, recursive: bool) -> Listed {
    let opens = Arc::new(Opens::default());
    let service = Tree {
        root: tree,
        describes,
        opens: opens.clone(),
    };
    let mut request = request();
    request.recursive = recursive;
    let (mut planned, mut listing_opens) = (Vec::new(), None);
    let run = calculate(
        request,
        &service,
        &UNCLASSIFIED,
        &Cancellation::default(),
        ChecksumRetention::new(()),
        |update| {
            match &update {
                ChecksumUpdate::Planned(files) => {
                    planned.extend(files.files.iter().map(|file| files.base.join(&file.name)))
                }
                ChecksumUpdate::Progress(progress) if !progress.listing => {
                    listing_opens.get_or_insert_with(|| opens.snapshot());
                }
                _ => {}
            }
            true
        },
    );
    Listed {
        planned,
        rows: run
            .rows
            .iter()
            .map(|row| {
                let message = row.error.as_ref().map(|error| error.message.clone());
                (row.path(), row.state, message.unwrap_or_default())
            })
            .collect(),
        completion: run.summary.completion,
        listing_opens: listing_opens.expect("the reading pass started"),
        opens: opens.snapshot(),
    }
}

#[test]
fn listing_opens_no_file_and_plans_the_rows_it_planned_before() {
    for recursive in [false, true] {
        let before = list_tree(false, recursive);
        let after = list_tree(true, recursive);
        let root = request().targets[0].path.clone();
        let mut planned = vec![root.join("a.txt")];
        if recursive {
            planned.push(root.join("sub").join("deep.txt"));
        }
        planned.push(root.join("b.txt"));
        // The same files are listed, in reading order, and the same rows follow: the
        // link refused as it was, the mount point skipped (with Include subfolders) or
        // left out with the other subfolder (without).
        assert_eq!(before.planned, planned, "recursive: {recursive}");
        assert_eq!(after.planned, planned, "recursive: {recursive}");
        assert_eq!(after.rows, before.rows, "recursive: {recursive}");
        assert_eq!(
            after.completion, before.completion,
            "recursive: {recursive}"
        );
        let mut states: Vec<_> = after
            .rows
            .iter()
            .map(|(path, state, _)| (path.clone(), *state))
            .collect();
        states.sort_by(|a, b| a.0.cmp(&b.0));
        let mut wanted: Vec<_> = planned
            .iter()
            .map(|path| (path.clone(), ChecksumRowState::Calculated))
            .chain([(root.join("link"), ChecksumRowState::Skipped)])
            .chain(recursive.then(|| (root.join("mnt"), ChecksumRowState::Skipped)))
            .collect();
        wanted.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(states, wanted, "recursive: {recursive}");
        // Listing opens no file and no link; only a folder it enters. Reading opens each
        // file once, and the link to report its refusal. A subfolder left out, and
        // another volume's folder, are never opened.
        let folders = usize::from(recursive);
        assert_eq!(
            after.listing_opens,
            (0, folders, 0),
            "recursive: {recursive}"
        );
        assert_eq!(
            after.opens,
            (planned.len(), 2 * folders, 1),
            "recursive: {recursive}"
        );
        // Before, listing opened every file, the link and both subfolders.
        assert_eq!(
            before.listing_opens,
            (planned.len(), 2, 1),
            "recursive: {recursive}"
        );
    }
}

/// The native service with the listing of before: every child opened as `next_child`
/// opens it (the default `next_listed_child`).
struct OpensEveryChild(midnight_platform::LocalFileSystem);
struct Opened(Box<dyn TransferSource>);
impl TransferService for OpensEveryChild {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0
            .prepare_transfer_source(path, cancel)
            .map(|source| Box::new(Opened(source)) as Box<dyn TransferSource>)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_transfer_destination(path, cancel)
    }
}
impl TransferSource for Opened {
    fn entry(&self) -> &TransferEntry {
        self.0.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.0.read_chunk(buffer, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Ok(self
            .0
            .next_child(cancel)?
            .map(|child| Box::new(Opened(child)) as Box<dyn TransferSource>))
    }
}

/// A timing, not a check (run it explicitly): the listing pass of a folder of small files
/// on the native filesystem, as before (every file opened) and now, alternately. The count
/// is `MIDNIGHT_CHECKSUM_LISTING_FILES` (20,000 by default).
#[test]
#[ignore = "times a native folder of many files; run explicitly"]
fn native_listing_pass_timing() {
    let count = std::env::var("MIDNIGHT_CHECKSUM_LISTING_FILES")
        .ok()
        .and_then(|count| count.parse().ok())
        .unwrap_or(20_000usize);
    let folder = tempfile::tempdir().unwrap();
    for index in 0..count {
        std::fs::write(
            folder.path().join(format!("{index:06}.txt")),
            b"0123456789abcdef",
        )
        .unwrap();
    }
    let native = midnight_platform::LocalFileSystem::new(None);
    let before = OpensEveryChild(midnight_platform::LocalFileSystem::new(None));
    let time = |service: &dyn TransferService| {
        let mut request = request();
        request.targets = vec![folder.path().to_owned().into()];
        let started = Instant::now();
        let mut listed = None;
        let run = calculate(
            request,
            service,
            &UNCLASSIFIED,
            &Cancellation::default(),
            ChecksumRetention::new(()),
            |update| {
                if matches!(&update, ChecksumUpdate::Progress(progress) if !progress.listing) {
                    listed.get_or_insert_with(|| started.elapsed());
                }
                true
            },
        );
        assert_eq!(run.summary.progress.calculated, count as u64);
        (listed.unwrap(), started.elapsed())
    };
    for round in 1..=3 {
        let (old_listing, old_total) = time(&before);
        let (new_listing, new_total) = time(&native);
        eprintln!(
            "round {round}, {count} files: listing {old_listing:?} before, {new_listing:?} now; \
             whole run {old_total:?} before, {new_total:?} now"
        );
    }
}

/// A calculation of the tree `root` below the selected folder (`request()`'s target),
/// with Include subfolders as given, cancelled by `cancel`, reporting to `emit`.
fn walk_tree(
    root: fn() -> Vec<Node>,
    recursive: bool,
    cancel: &Cancellation,
    emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let mut request = request();
    request.recursive = recursive;
    calculate(
        request,
        &Tree {
            root,
            describes: true,
            opens: Arc::default(),
        },
        &UNCLASSIFIED,
        cancel,
        ChecksumRetention::new(()),
        emit,
    )
}
/// The rows' paths below the selected folder, with their states.
fn below_root(run: &ChecksumRun) -> Vec<(PathBuf, ChecksumRowState)> {
    let root = request().targets[0].path.clone();
    run.rows
        .iter()
        .map(|row| {
            let path = row.path();
            (
                path.strip_prefix(&root).unwrap_or(&path).to_owned(),
                row.state,
            )
        })
        .collect()
}

const WIDE_FILES: usize = 16_500;
fn wide() -> Vec<Node> {
    (0..WIDE_FILES)
        .map(|index| node(&format!("{index:05}.bin"), EntryKind::File, 1))
        .collect()
}

/// A run has no row limit: more files than the former 16,384 rows are all listed, read
/// and kept, found by ID; and a selection names any number of targets (4,096 before).
#[test]
fn a_run_takes_any_number_of_files_and_targets() {
    let mut listed = 0;
    let walked = walk_tree(wide, false, &Cancellation::default(), |update| {
        if let ChecksumUpdate::Planned(planned) = &update {
            listed += planned.files.len();
        }
        true
    });
    assert_eq!(walked.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(listed, WIDE_FILES);
    assert_eq!(walked.rows.len(), WIDE_FILES);
    assert_eq!(walked.summary.progress.calculated, WIDE_FILES as u64);
    let last = walked.rows.by_id(WIDE_FILES as u64).expect("the last row");
    assert_eq!(
        last.path(),
        request().targets[0]
            .path
            .join(format!("{:05}.bin", WIDE_FILES - 1))
    );
    let folder = request().targets[0].path.parent().unwrap().to_owned();
    let mut many = request();
    many.targets = (0..5_000)
        .map(|index| folder.join(format!("{index}.bin")).into())
        .collect();
    assert!(many.validate().is_ok());
    let selected = run(many, &Provider::new(b"abc"), &Cancellation::default());
    assert_eq!(selected.rows.len(), 5_000);
    assert_eq!(selected.summary.completion, ChecksumCompletion::Completed);
}

const LEVELS: usize = 80;
/// Folders `d0/d1/…/d79` below the selected folder, a file at every level and one at
/// the bottom.
fn deep() -> Vec<Node> {
    let mut children = vec![node("bottom.txt", EntryKind::File, 1)];
    for depth in (0..LEVELS).rev() {
        let mut folder = node(&format!("d{depth}"), EntryKind::Directory, 10 + depth as u8);
        folder.children = std::mem::take(&mut children);
        children = vec![node(&format!("f{depth}.txt"), EntryKind::File, 1), folder];
    }
    children
}

/// Include subfolders walks every level: deeper than the former 64-level limit, without
/// recursion in the walk itself.
#[test]
fn include_subfolders_reaches_the_bottom_of_any_depth() {
    let run = walk_tree(deep, true, &Cancellation::default(), |_| true);
    assert_eq!(run.summary.completion, ChecksumCompletion::Completed);
    let rows = below_root(&run);
    assert_eq!(rows.len(), LEVELS + 1);
    assert!(
        rows.iter()
            .all(|(_, state)| *state == ChecksumRowState::Calculated)
    );
    let bottom = (0..LEVELS)
        .map(|depth| format!("d{depth}"))
        .collect::<PathBuf>()
        .join("bottom.txt");
    assert!(rows.iter().any(|(path, _)| *path == bottom), "{bottom:?}");
}

/// `a.txt`, `l1/b.txt`, `l1/l2/c.txt` and `l1/l2/l3/d.txt`.
fn levels() -> Vec<Node> {
    let mut l3 = node("l3", EntryKind::Directory, 13);
    l3.children.push(node("d.txt", EntryKind::File, 1));
    let mut l2 = node("l2", EntryKind::Directory, 12);
    l2.children = vec![node("c.txt", EntryKind::File, 1), l3];
    let mut l1 = node("l1", EntryKind::Directory, 11);
    l1.children = vec![node("b.txt", EntryKind::File, 1), l2];
    vec![node("a.txt", EntryKind::File, 1), l1]
}

/// Include subfolders (regression: the walk was already recursive): every nesting level
/// with it, only the selected folder's own files without; a saved manifest names each
/// file relative to the folder it is saved in, with `/` separators.
#[test]
fn include_subfolders_walks_three_levels_and_saves_relative_names() {
    let flat = walk_tree(levels, false, &Cancellation::default(), |_| true);
    assert_eq!(
        below_root(&flat),
        [(PathBuf::from("a.txt"), ChecksumRowState::Calculated)]
    );
    let run = walk_tree(levels, true, &Cancellation::default(), |_| true);
    assert_eq!(run.summary.completion, ChecksumCompletion::Completed);
    let names: Vec<_> = below_root(&run).into_iter().map(|(path, _)| path).collect();
    assert_eq!(
        names,
        [
            PathBuf::from("a.txt"),
            ["l1", "b.txt"].iter().collect(),
            ["l1", "l2", "c.txt"].iter().collect(),
            ["l1", "l2", "l3", "d.txt"].iter().collect(),
        ]
    );
    let folder = request().targets[0].path.clone();
    let text = String::from_utf8(export(&run, &folder).unwrap()).unwrap();
    let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(
        text,
        format!("{hex} *a.txt\n{hex} *l1/b.txt\n{hex} *l1/l2/c.txt\n{hex} *l1/l2/l3/d.txt\n")
    );
}

/// The selected folder reached again inside itself, and a folder reached again below
/// itself (a bind mount, a hard-linked folder).
fn looped() -> Vec<Node> {
    let mut again = node("again", EntryKind::Directory, 100);
    again.children.push(node("x.txt", EntryKind::File, 1));
    let mut back = node("back", EntryKind::Directory, 2);
    back.children.push(node("y.txt", EntryKind::File, 1));
    let mut sub = node("sub", EntryKind::Directory, 2);
    sub.children = vec![node("s.txt", EntryKind::File, 1), back];
    vec![node("a.txt", EntryKind::File, 1), again, sub]
}

/// A folder reached again is recognized by its identity and walked once: the walk ends.
#[test]
fn a_folder_reached_again_is_walked_once() {
    let run = walk_tree(looped, true, &Cancellation::default(), |_| true);
    assert_eq!(run.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(
        below_root(&run),
        [
            (PathBuf::from("a.txt"), ChecksumRowState::Calculated),
            (
                ["sub", "s.txt"].iter().collect(),
                ChecksumRowState::Calculated
            ),
        ]
    );
}

/// `a.txt`, a folder whose listing is refused, and `open/b.txt`.
fn locked() -> Vec<Node> {
    let mut locked = node("locked", EntryKind::Directory, 2);
    locked.unreadable = true;
    locked.children.push(node("hidden.txt", EntryKind::File, 1));
    let mut open = node("open", EntryKind::Directory, 3);
    open.children.push(node("b.txt", EntryKind::File, 1));
    vec![node("a.txt", EntryKind::File, 1), locked, open]
}

/// A subfolder that cannot be read is its own failed row; the run reads everything else
/// and says it may have missed files.
#[test]
fn an_unreadable_subfolder_fails_alone_and_the_run_goes_on() {
    let mut planned = Vec::new();
    let run = walk_tree(locked, true, &Cancellation::default(), |update| {
        if let ChecksumUpdate::Planned(files) = &update {
            planned.extend(files.files.iter().map(|file| file.name.to_path_buf()));
        }
        true
    });
    assert_eq!(
        below_root(&run),
        [
            (PathBuf::from("a.txt"), ChecksumRowState::Calculated),
            (PathBuf::from("locked"), ChecksumRowState::ReadError),
            (
                ["open", "b.txt"].iter().collect(),
                ChecksumRowState::Calculated
            ),
        ]
    );
    let failed = row(&run, 1);
    assert_eq!(
        failed.error.as_ref().map(|error| error.kind),
        Some(io::ErrorKind::PermissionDenied)
    );
    assert_eq!(run.summary.completion, ChecksumCompletion::Incomplete);
    assert_eq!(run.summary.progress.errors, 1);
    assert_eq!(
        planned.len(),
        2,
        "the listing leaves the unreadable folder out"
    );
}

/// Stop as the first rows of the walk of `root` (`files` files) arrive.
fn stop_at_first_rows(root: fn() -> Vec<Node>, recursive: bool, files: usize) {
    let cancel = Cancellation::default();
    let run = walk_tree(root, recursive, &cancel, |update| {
        if matches!(update, ChecksumUpdate::Batch(_)) {
            cancel.cancel();
        }
        true
    });
    assert_eq!(run.summary.completion, ChecksumCompletion::Cancelled);
    assert!(
        (1..files).contains(&run.rows.len()),
        "{} of {files} rows",
        run.rows.len()
    );
}

/// Stop ends the walk between files, in a folder or deep inside Include subfolders: no
/// row after it, and the run says so.
#[test]
fn cancelling_stops_the_walk_between_files() {
    stop_at_first_rows(wide, false, WIDE_FILES);
    stop_at_first_rows(deep, true, LEVELS + 1);
}

/// A timing, not a check (run it explicitly in a release build): checksums of one large
/// file on the native filesystem with each algorithm alone and with all five, beside a
/// plain read of the same file in 1 MiB chunks (the disk's or the file cache's rate).
/// The file (`MIDNIGHT_CHECKSUM_LARGE_MIB`, default 1,024 MiB) is created once in
/// `MIDNIGHT_HOT_PATH_ROOT` (else the temporary folder) and left there for later runs.
#[test]
#[ignore = "times checksums of one large native file; run explicitly"]
fn native_large_file_throughput() {
    use std::io::Read;
    let mib: u64 = std::env::var("MIDNIGHT_CHECKSUM_LARGE_MIB")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1024);
    let root = std::env::var_os("MIDNIGHT_HOT_PATH_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("midnight-hot-path-measure"));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("large-{mib}.bin"));
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(mib << 20) {
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut chunk = vec![0u8; 1 << 20];
        for _ in 0..mib {
            for byte in &mut chunk {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            std::io::Write::write_all(&mut file, &chunk).unwrap();
        }
    }
    let native = midnight_platform::LocalFileSystem::new(None);
    let rate = |seconds: f64| (mib as f64) / seconds;
    let runs = 5;
    let mut read = Vec::new();
    for _ in 0..runs {
        let started = Instant::now();
        let mut file = std::fs::File::open(&path).unwrap();
        let mut buffer = vec![0u8; 1 << 20];
        while file.read(&mut buffer).unwrap() != 0 {}
        read.push(started.elapsed().as_secs_f64());
    }
    read.sort_by(f64::total_cmp);
    eprintln!("plain read: median {:.0} MiB/s", rate(read[runs / 2]));
    let sets: [(&str, &[ChecksumAlgorithm]); 6] = [
        ("CRC32", &[ChecksumAlgorithm::Crc32]),
        ("MD5", &[ChecksumAlgorithm::Md5]),
        ("SHA-1", &[ChecksumAlgorithm::Sha1]),
        ("SHA-256", &[ChecksumAlgorithm::Sha256]),
        ("SHA-512", &[ChecksumAlgorithm::Sha512]),
        ("all five", &ChecksumAlgorithm::ALL),
    ];
    for (label, algorithms) in sets {
        let mut times = Vec::new();
        for _ in 0..runs {
            let mut request = request();
            request.targets = vec![path.clone().into()];
            request.algorithms = ChecksumAlgorithms::new(algorithms).unwrap();
            let started = Instant::now();
            let run = calculate(
                request,
                &native,
                &UNCLASSIFIED,
                &Cancellation::default(),
                ChecksumRetention::new(()),
                |_| true,
            );
            times.push(started.elapsed().as_secs_f64());
            assert_eq!(run.summary.progress.calculated, 1);
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "{label}: median {:.0} MiB/s ({:.3} s for {mib} MiB), range {:.0}-{:.0} MiB/s",
            rate(times[runs / 2]),
            times[runs / 2],
            rate(times[runs - 1]),
            rate(times[0]),
        );
    }
}
