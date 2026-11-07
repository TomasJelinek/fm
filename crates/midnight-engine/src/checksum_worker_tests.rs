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
    buffer: AtomicUsize,
    interrupted: AtomicBool,
    changed: AtomicBool,
}
impl Default for Reads {
    fn default() -> Self {
        Self {
            bytes: AtomicU64::new(0),
            calls: AtomicUsize::new(0),
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
        cancel,
        ChecksumRetention::new(()),
        |_| true,
    )
}

#[test]
fn short_reads_and_interruption_feed_four_hashes_with_one_ordered_read_stream() {
    let provider = Provider::new(b"abcdefghijklm");
    provider.reads.interrupted.store(true, Ordering::SeqCst);
    let mut request = request();
    request.algorithms = ChecksumAlgorithms::all();
    let result = run(request, &provider, &Cancellation::default());
    assert_eq!(result.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(result.rows[0].digests.len(), 4);
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
    let provider = Arc::new(Provider::new(b"abc"));
    let mut controller = super::super::Controller::new(provider);
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
    runs.pop();
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
    let barrier = ReadBarrier::new();
    let _release = ReleaseOnDrop(barrier.clone());
    let mut provider = Provider::new(b"abc");
    provider.barrier = Some(barrier.clone());
    let controller = super::super::Controller::new(Arc::new(provider));
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
    assert_eq!(
        midnight_work_budget::shared_io().snapshot().available,
        midnight_work_budget::IO_SLOTS
    );
}
