//! Serial, pinned native reads. Called only from a bounded backend blocking worker.
use md5::Md5;
use midnight_core::*;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::HashSet,
    io,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) fn calculate_captured(
    request: ChecksumRequest,
    source: Option<&dyn CapturedChecksumFiles>,
    service: &dyn TransferService,
    cancel: &Cancellation,
    retention: Arc<ChecksumRetention>,
    emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    // Check retained provider contexts again after waiting for the serial worker.
    // A changed source never substitutes new paths into an already queued request.
    let validation = source.map_or(Ok(()), |source| {
        let targets = source.resolve(cancel)?;
        if targets.len() == request.targets.len()
            && targets
                .iter()
                .zip(&request.targets)
                .all(|(current, original)| {
                    current.path == original.path && current.kind == original.kind
                })
        {
            Ok(())
        } else {
            Err(failure(
                Path::new(""),
                io::ErrorKind::InvalidData,
                "Captured checksum targets changed.",
            ))
        }
    });
    if let Err(error) = validation {
        let cancelled = cancel.is_cancelled() || error.kind == io::ErrorKind::Interrupted;
        return Arc::new(ChecksumRun {
            job: request.job,
            generation: request.generation,
            algorithms: request.algorithms,
            captured_roots: request
                .targets
                .into_iter()
                .map(|target| target.path)
                .collect(),
            rows: Arc::from([]),
            summary: ChecksumSummary {
                completion: if cancelled {
                    ChecksumCompletion::Cancelled
                } else {
                    ChecksumCompletion::Failed
                },
                progress: ChecksumProgress {
                    errors: u64::from(!cancelled),
                    ..Default::default()
                },
                reached_budgets: vec![],
                diagnostics: vec![error],
            },
            retention,
        });
    }
    calculate(request, service, cancel, retention, emit)
}

pub(super) fn calculate(
    request: ChecksumRequest,
    service: &dyn TransferService,
    cancel: &Cancellation,
    retention: Arc<ChecksumRetention>,
    mut emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let roots: Arc<[_]> = request
        .targets
        .iter()
        .map(|target| target.path.clone())
        .collect();
    let mut state = State {
        request: &request,
        cancel,
        retention: retention.clone(),
        rows: Vec::new(),
        batch: Vec::new(),
        progress: ChecksumProgress::default(),
        budgets: Vec::new(),
        diagnostics: Vec::new(),
        seen: HashSet::new(),
        directories: HashSet::new(),
        retained: 0,
        incomplete: false,
        last_progress: Instant::now(),
        buffer: vec![0; CHECKSUM_BUFFER_BYTES],
        emit: &mut emit,
    };
    for target in &request.targets {
        if state.stopped() {
            break;
        }
        if !state.charge(&target.path) {
            if state.stopped() {
                break;
            }
            continue;
        }
        let opened = with_io(cancel, &target.path, || {
            service.prepare_transfer_source(&target.path, cancel)
        });
        match opened {
            Ok(source) => {
                if !matches_capture(source.entry(), target) {
                    if request.recursive
                        && (target.kind == Some(EntryKind::Directory)
                            || source.entry().item.kind == EntryKind::Directory)
                    {
                        state.incomplete = true;
                    }
                    state.failed(
                        &target.path,
                        Some(source.entry()),
                        ChecksumRowState::SourceChanged,
                        failure(
                            &target.path,
                            io::ErrorKind::InvalidData,
                            "Captured source metadata or identity changed.",
                        ),
                    );
                } else {
                    state.walk(source);
                }
            }
            Err(error) => state.open_failed(error, target.kind),
        }
    }
    state.flush_batch();
    state.progress.current = None;
    let completion = if cancel.is_cancelled() {
        ChecksumCompletion::Cancelled
    } else if state.incomplete {
        ChecksumCompletion::Incomplete
    } else if state.progress.errors > 0 || state.progress.skipped > 0 {
        ChecksumCompletion::CompletedWithErrors
    } else {
        ChecksumCompletion::Completed
    };
    Arc::new(ChecksumRun {
        job: request.job,
        generation: request.generation,
        algorithms: request.algorithms,
        captured_roots: roots,
        rows: std::mem::take(&mut state.rows).into(),
        retention,
        summary: ChecksumSummary {
            completion,
            progress: state.progress.clone(),
            reached_budgets: std::mem::take(&mut state.budgets),
            diagnostics: std::mem::take(&mut state.diagnostics),
        },
    })
}

fn matches_capture(entry: &TransferEntry, target: &ChecksumTarget) -> bool {
    entry.item.path == target.path
        && target.kind.is_none_or(|kind| kind == entry.item.kind)
        && target
            .size
            .is_none_or(|size| entry.item.metadata.size == Some(size))
        && target
            .modified
            .is_none_or(|time| entry.item.metadata.modified == Some(time))
        && target
            .identity
            .is_none_or(|identity| entry.identity == identity)
}

struct State<'a, F> {
    request: &'a ChecksumRequest,
    cancel: &'a Cancellation,
    retention: Arc<ChecksumRetention>,
    rows: Vec<Arc<ChecksumRow>>,
    batch: Vec<Arc<ChecksumRow>>,
    progress: ChecksumProgress,
    budgets: Vec<ChecksumBudget>,
    diagnostics: Vec<FsError>,
    seen: HashSet<std::path::PathBuf>,
    directories: HashSet<TransferIdentity>,
    retained: usize,
    incomplete: bool,
    last_progress: Instant,
    buffer: Vec<u8>,
    emit: &'a mut F,
}
impl<F: FnMut(ChecksumUpdate) -> bool> State<'_, F> {
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled() || !self.budgets.is_empty()
    }
    fn budget(&mut self, budget: ChecksumBudget) {
        self.incomplete = true;
        if !self.budgets.contains(&budget) {
            self.budgets.push(budget);
        }
    }
    fn charge(&mut self, path: &Path) -> bool {
        if self.seen.contains(path) {
            return false;
        }
        if self.progress.discovered >= self.request.limits.max_nodes {
            self.budget(ChecksumBudget::Nodes);
            return false;
        }
        // Charge retained path/index/diagnostic copies and bounded digest/style data, not file length.
        let cost = path
            .as_os_str()
            .as_encoded_bytes()
            .len()
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(1024));
        let Some(total) = cost.and_then(|cost| self.retained.checked_add(cost)) else {
            self.budget(ChecksumBudget::RetainedBytes);
            return false;
        };
        if total > self.request.limits.max_retained_bytes {
            self.budget(ChecksumBudget::RetainedBytes);
            return false;
        }
        if self.rows.len() >= self.request.limits.max_rows {
            self.budget(ChecksumBudget::Rows);
            return false;
        }
        self.retained = total;
        self.progress.discovered += 1;
        self.seen.insert(path.to_owned());
        true
    }
    fn walk(&mut self, source: Box<dyn TransferSource>) {
        let root_device = source.entry().identity.device;
        let mut stack = Vec::<Box<dyn TransferSource>>::new();
        let mut current = Some(source);
        loop {
            if self.stopped() {
                break;
            }
            if let Some(source) = current.take() {
                let entry = source.entry().clone();
                if entry.identity.device != root_device {
                    self.incomplete |= entry.item.kind == EntryKind::Directory;
                    self.failed(
                        &entry.item.path,
                        Some(&entry),
                        ChecksumRowState::Skipped,
                        failure(
                            &entry.item.path,
                            io::ErrorKind::Unsupported,
                            "Recursive checksums do not cross native mount/volume boundaries.",
                        ),
                    );
                    continue;
                }
                match entry.item.kind {
                    EntryKind::File => self.file(source),
                    EntryKind::Directory if self.request.recursive => {
                        if stack.len() >= self.request.limits.max_depth { self.budget(ChecksumBudget::Depth); break; }
                        if !self.directories.insert(entry.identity) { continue; }
                        if let Err(error) = with_io(self.cancel, &entry.item.path, || source.revalidate(self.cancel)) {
                            self.failed(&entry.item.path, Some(&entry), ChecksumRowState::ReadError, error);
                            self.incomplete = true;
                        } else { stack.push(source); }
                    }
                    _ => self.failed(&entry.item.path, Some(&entry), ChecksumRowState::Skipped,
                        failure(&entry.item.path, io::ErrorKind::Unsupported,
                            "Directories require explicit recursion; links and special entries are skipped.")),
                }
            }
            let Some(parent) = stack.last_mut() else {
                break;
            };
            let parent_path = parent.entry().item.path.clone();
            let next = with_io(self.cancel, &parent_path, || parent.next_child(self.cancel));
            match next {
                Ok(Some(child)) => {
                    if self.charge(&child.entry().item.path) {
                        current = Some(child);
                    }
                }
                Ok(None) => {
                    stack.pop();
                }
                Err(error) => {
                    let directory_failed = error.path == parent_path;
                    if directory_failed {
                        self.open_failed(error, Some(EntryKind::Directory));
                        self.incomplete = true;
                        stack.pop();
                    } else if self.charge(&error.path) {
                        self.open_failed(error, None);
                    }
                }
            }
        }
    }
    fn file(&mut self, mut source: Box<dyn TransferSource>) {
        let entry = source.entry().clone();
        self.progress.current = Some(entry.item.path.clone());
        let mut hashers = Hashers::new(self.request.algorithms);
        let mut read = 0u64;
        let result = (|| {
            with_io(self.cancel, &entry.item.path, || {
                source.revalidate(self.cancel)
            })?;
            loop {
                if self.cancel.is_cancelled() {
                    return Err(failure(
                        &entry.item.path,
                        io::ErrorKind::Interrupted,
                        "Checksum calculation cancelled.",
                    ));
                }
                let count = match with_io(self.cancel, &entry.item.path, || {
                    source.read_chunk(&mut self.buffer, self.cancel)
                }) {
                    Err(error)
                        if error.kind == io::ErrorKind::Interrupted
                            && !self.cancel.is_cancelled() =>
                    {
                        continue;
                    }
                    result => result?,
                };
                if count > self.buffer.len() {
                    return Err(failure(
                        &entry.item.path,
                        io::ErrorKind::InvalidData,
                        "Provider returned an invalid read length.",
                    ));
                }
                if count == 0 {
                    with_io(self.cancel, &entry.item.path, || {
                        source.revalidate(self.cancel)
                    })?;
                    if entry.item.metadata.size != Some(read) {
                        return Err(failure(
                            &entry.item.path,
                            io::ErrorKind::InvalidData,
                            "Source length changed during calculation.",
                        ));
                    }
                    break;
                }
                read = read.checked_add(count as u64).ok_or_else(|| {
                    failure(
                        &entry.item.path,
                        io::ErrorKind::InvalidData,
                        "Checksum byte count overflow.",
                    )
                })?;
                self.progress.bytes_read = self
                    .progress
                    .bytes_read
                    .checked_add(count as u64)
                    .ok_or_else(|| {
                        failure(
                            &entry.item.path,
                            io::ErrorKind::InvalidData,
                            "Aggregate checksum byte count overflow.",
                        )
                    })?;
                with_io(self.cancel, &entry.item.path, || {
                    source.revalidate(self.cancel)
                })?;
                if entry.item.metadata.size.is_none_or(|length| read > length) {
                    return Err(failure(
                        &entry.item.path,
                        io::ErrorKind::InvalidData,
                        "Source grew or has no regular-file length.",
                    ));
                }
                hashers.update(&self.buffer[..count]);
                self.progress();
            }
            with_io(self.cancel, &entry.item.path, || {
                source.revalidate(self.cancel)
            })
        })();
        let (state, digests, error) = match result {
            Ok(()) => {
                self.progress.calculated += 1;
                (ChecksumRowState::Calculated, hashers.finish(), None)
            }
            Err(error) => {
                let state = if self.cancel.is_cancelled() {
                    ChecksumRowState::Cancelled
                } else if matches!(
                    error.kind,
                    io::ErrorKind::InvalidData
                        | io::ErrorKind::InvalidInput
                        | io::ErrorKind::NotFound
                ) {
                    ChecksumRowState::SourceChanged
                } else {
                    ChecksumRowState::ReadError
                };
                self.diagnostic(&error);
                if state != ChecksumRowState::Cancelled {
                    self.progress.errors += 1;
                }
                (state, Vec::new(), Some(error))
            }
        };
        self.row(ChecksumRow {
            id: ChecksumRowId(self.rows.len() as u64 + 1),
            path: entry.item.path,
            identity: Some(entry.identity),
            kind: Some(entry.item.kind),
            state,
            bytes_read: read,
            digests,
            error,
            retention: self.retention.clone(),
        });
    }
    fn open_failed(&mut self, error: FsError, known_kind: Option<EntryKind>) {
        let path = error.path.clone();
        let state = if error.kind == io::ErrorKind::Unsupported {
            ChecksumRowState::Skipped
        } else if self.cancel.is_cancelled() {
            ChecksumRowState::Cancelled
        } else {
            ChecksumRowState::ReadError
        };
        // An inaccessible/unknown recursive child can hide an uncounted subtree.
        // A single error row must not imply that every excluded file was enumerated.
        if self.request.recursive
            && (known_kind == Some(EntryKind::Directory)
                || (state == ChecksumRowState::ReadError && known_kind != Some(EntryKind::File)))
        {
            self.incomplete = true;
        }
        self.failed(&path, None, state, error);
    }
    fn failed(
        &mut self,
        path: &Path,
        entry: Option<&TransferEntry>,
        state: ChecksumRowState,
        error: FsError,
    ) {
        self.diagnostic(&error);
        if state == ChecksumRowState::Skipped {
            self.progress.skipped += 1;
        } else if state != ChecksumRowState::Cancelled {
            self.progress.errors += 1;
        }
        self.row(ChecksumRow {
            id: ChecksumRowId(self.rows.len() as u64 + 1),
            path: path.to_owned(),
            identity: entry.map(|entry| entry.identity),
            kind: entry.map(|entry| entry.item.kind),
            state,
            bytes_read: 0,
            digests: Vec::new(),
            error: Some(error),
            retention: self.retention.clone(),
        });
    }
    fn diagnostic(&mut self, error: &FsError) {
        if self.diagnostics.len() < MAX_CHECKSUM_DIAGNOSTICS {
            self.diagnostics.push(error.clone());
        }
    }
    fn row(&mut self, row: ChecksumRow) {
        let row = Arc::new(row);
        self.rows.push(row.clone());
        self.batch.push(row);
        if self.batch.len() >= CHECKSUM_BATCH_SIZE {
            self.flush_batch();
        }
        self.progress();
    }
    fn flush_batch(&mut self) {
        if !self.batch.is_empty()
            && !(self.emit)(ChecksumUpdate::Batch(
                std::mem::take(&mut self.batch).into(),
            ))
        {
            self.cancel.cancel();
        }
    }
    fn progress(&mut self) {
        if self.last_progress.elapsed() >= Duration::from_millis(250) {
            self.last_progress = Instant::now();
            if !(self.emit)(ChecksumUpdate::Progress(self.progress.clone())) {
                self.cancel.cancel();
            }
        }
    }
}

struct Hashers {
    md5: Option<Md5>,
    sha1: Option<Sha1>,
    sha256: Option<Sha256>,
    sha512: Option<Sha512>,
    crc32: Option<crc32fast::Hasher>,
}
impl Hashers {
    fn new(algorithms: ChecksumAlgorithms) -> Self {
        Self {
            md5: algorithms.contains(ChecksumAlgorithm::Md5).then(Md5::new),
            sha1: algorithms.contains(ChecksumAlgorithm::Sha1).then(Sha1::new),
            sha256: algorithms
                .contains(ChecksumAlgorithm::Sha256)
                .then(Sha256::new),
            sha512: algorithms
                .contains(ChecksumAlgorithm::Sha512)
                .then(Sha512::new),
            crc32: algorithms
                .contains(ChecksumAlgorithm::Crc32)
                .then(crc32fast::Hasher::new),
        }
    }
    fn update(&mut self, bytes: &[u8]) {
        if let Some(hash) = &mut self.md5 {
            hash.update(bytes);
        }
        if let Some(hash) = &mut self.sha1 {
            hash.update(bytes);
        }
        if let Some(hash) = &mut self.sha256 {
            hash.update(bytes);
        }
        if let Some(hash) = &mut self.sha512 {
            hash.update(bytes);
        }
        if let Some(hash) = &mut self.crc32 {
            hash.update(bytes);
        }
    }
    fn finish(self) -> Vec<ChecksumDigest> {
        let mut result = Vec::new();
        if let Some(hash) = self.md5 {
            result.push(ChecksumDigest {
                algorithm: ChecksumAlgorithm::Md5,
                bytes: hash.finalize().to_vec(),
            });
        }
        if let Some(hash) = self.sha1 {
            result.push(ChecksumDigest {
                algorithm: ChecksumAlgorithm::Sha1,
                bytes: hash.finalize().to_vec(),
            });
        }
        if let Some(hash) = self.sha256 {
            result.push(ChecksumDigest {
                algorithm: ChecksumAlgorithm::Sha256,
                bytes: hash.finalize().to_vec(),
            });
        }
        if let Some(hash) = self.sha512 {
            result.push(ChecksumDigest {
                algorithm: ChecksumAlgorithm::Sha512,
                bytes: hash.finalize().to_vec(),
            });
        }
        if let Some(hash) = self.crc32 {
            result.push(ChecksumDigest {
                algorithm: ChecksumAlgorithm::Crc32,
                bytes: hash.finalize().to_be_bytes().to_vec(),
            });
        }
        result
    }
}

pub(super) fn with_io<T>(
    cancel: &Cancellation,
    path: &Path,
    action: impl FnOnce() -> Result<T, FsError>,
) -> Result<T, FsError> {
    let _permit = midnight_work_budget::shared_io()
        .acquire(midnight_work_budget::Admission::Exclusive, || {
            cancel.is_cancelled()
        })
        .ok_or_else(|| failure(path, io::ErrorKind::Interrupted, "Checksum I/O cancelled."))?;
    action()
}
pub(super) fn failure(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("checksums", path, io::Error::new(kind, message))
}

#[cfg(test)]
#[path = "checksum_worker_tests.rs"]
mod tests;
