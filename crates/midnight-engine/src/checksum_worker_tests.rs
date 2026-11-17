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
        if let Some(barrier) = &self.provider.barrier {
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

#[test]
fn short_reads_and_interruption_feed_all_hashes_with_one_ordered_read_stream() {
    let provider = Provider::new(b"abcdefghijklm");
    provider.reads.interrupted.store(true, Ordering::SeqCst);
    let mut request = request();
    request.algorithms = ChecksumAlgorithms::all();
    let result = run(request, &provider, &Cancellation::default());
    assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(result.rows[0].digests.len(), 5);
    assert_eq!(provider.reads.bytes.load(Ordering::SeqCst), 13);
    assert_eq!(provider.reads.calls.load(Ordering::SeqCst), 7); // EINTR, five short reads, EOF.
    assert_eq!(
        provider.reads.buffer.load(Ordering::SeqCst),
        CHECKSUM_BUFFER_BYTES
    );
    assert_eq!(
        result.rows[0]
            .digest(ChecksumAlgorithm::Sha256)
            .unwrap()
            .bytes,
        Sha256::digest(b"abcdefghijklm").to_vec()
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
        assert_eq!(result.rows[0].digests.len(), 1);
        assert_eq!(
            result.rows[0]
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
        assert_ne!(result.rows[0].state, ChecksumRowState::Calculated);
        assert!(result.rows[0].digests.is_empty());
        assert!(result.rows[0].bytes_read > 0);
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
    let result = super::run(
        request(),
        &provider,
        &UNCLASSIFIED,
        &Cancellation::default(),
        ChecksumRetention::new(()),
        settings,
        |_| true,
    );
    assert_eq!(result.rows[0].state, ChecksumRowState::SourceChanged);
    assert!(result.rows[0].digests.is_empty());
    assert_eq!(
        result.rows[0].bytes_read, 3,
        "stopped after the first chunk"
    );
}

/// Name of exactly `length` bytes; the index keeps paths distinct.
fn padded(index: usize, length: usize) -> String {
    let prefix = format!("{index:05}-");
    format!("{prefix}{}", "n".repeat(length - prefix.len()))
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
    };
    super::super::manifest::encode(run, &request, directory, &Cancellation::default())
        .map(|(bytes, _)| bytes)
}

#[test]
fn completed_and_budget_limited_calculations_always_fit_the_manifest_verifier() {
    // Very long virtual names fill the budget with a dozen rows, keeping the
    // shared I/O gate traffic of this unit test (also path-included) small.
    let directory = std::env::current_dir()
        .unwrap()
        .join("_build/virtual-export-budget");
    let limit = ChecksumLimits::default().max_retained_bytes;
    let charge = |path: &Path| path.as_os_str().as_encoded_bytes().len() * 4 + 1024;
    let name = 160_000;
    let per_row = charge(&directory.join(padded(0, name)));
    let count = limit / per_row - 1;
    let uniform = (0..count).map(|index| directory.join(padded(index, name)));

    // A completed run that leaves less than one row of its budget unused.
    let provider = Provider::new(b"x");
    let mut whole = request();
    whole.targets = uniform.clone().map(Into::into).collect();
    let completed = run(whole, &provider, &Cancellation::default());
    assert_eq!(completed.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(completed.rows.len(), count);
    let bytes = export(&completed, &directory).expect("a completed run must be exportable");
    assert!(bytes.len() > 1024 * 1024);

    // A final row whose plain charge lands within 4 bytes of the limit would make
    // the verifier's per-line name check overflow; it must stop the calculation
    // with an honest budget instead, and the acknowledged partial run must save.
    let remaining = limit - count * per_row;
    let base = directory.as_os_str().as_encoded_bytes().len() + 1;
    let last = directory.join(padded(count, (remaining - 1024) / 4 - base));
    assert!(remaining - charge(&last) < 4);
    let mut limited = request();
    limited.targets = uniform.chain([last]).map(Into::into).collect();
    let request = limited;
    let limited = run(request.clone(), &provider, &Cancellation::default());
    assert_eq!(limited.summary.completion, ChecksumCompletion::Incomplete);
    assert_eq!(
        limited.summary.reached_budgets,
        [ChecksumBudget::RetainedBytes]
    );
    assert_eq!(limited.rows.len(), count);
    export(&limited, &directory).expect("an acknowledged partial run must be exportable");
    // Verification rows are never exported and keep the plain per-entry charge.
    let verified = calculate_for_verification(
        request,
        &provider,
        &UNCLASSIFIED,
        &Cancellation::default(),
        ChecksumRetention::new(()),
        |_| true,
    );
    assert_eq!(verified.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(verified.rows.len(), count + 1);
}

#[test]
fn duplicate_roots_do_not_skip_a_later_distinct_source_and_row_limit_is_honest() {
    let provider = Provider::new(b"abc");
    let mut request = request();
    request.targets.push(request.targets[0].clone());
    let mut second = request.targets[0].clone();
    second.path.set_file_name("second");
    request.targets.push(second);
    let result = run(request.clone(), &provider, &Cancellation::default());
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.summary.progress.bytes_read, 6);
    request.limits.max_rows = 1;
    let limited = run(request, &provider, &Cancellation::default());
    assert_eq!(limited.rows.len(), 1);
    assert_eq!(limited.summary.completion, ChecksumCompletion::Incomplete);
    assert!(
        limited
            .summary
            .reached_budgets
            .contains(&ChecksumBudget::Rows)
    );
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
    assert!(result.rows[0].digests.is_empty());
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
        assert_eq!(result.rows[0].digest(algorithm).unwrap().hex(), hex);
    }
}

#[tokio::test]
async fn controller_cancellation_and_retained_job_quota_are_bounded() {
    let _quota_fixture = super::super::test_quota_fixture().await;
    let provider = Arc::new(Provider::new(b"abc"));
    let mut controller =
        super::super::Controller::new(super::super::Retirement::testing(), provider);
    assert!(controller.try_next().is_none());
    let mut runs = Vec::new();
    for job in 1..=32 {
        let mut request = request();
        request.job = ChecksumJobId(job);
        request.limits.max_retained_bytes = 16 * 1024;
        controller.start(request).unwrap();
    }
    let mut rejected = request();
    rejected.job = ChecksumJobId(33);
    rejected.limits.max_retained_bytes = 16 * 1024;
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
    assert!(controller.start(rejected.clone()).is_err()); // Results own the job permit.
    let released = runs.pop().unwrap();
    let retained_row = released.rows[0].clone();
    drop(released);
    assert!(controller.start(rejected.clone()).is_err()); // A row alone retains quota.
    drop(retained_row);
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
