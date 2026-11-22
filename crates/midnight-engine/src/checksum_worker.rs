//! Serial, pinned native reads. Called only from a bounded backend blocking worker.
use md5::Md5;
use midnight_core::*;
use midnight_work_budget::Admission;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::{
    borrow::Cow,
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// Source storage classification for the process-wide disk gate. As for folder
/// sizes, thumbnails and the image viewer, only a positively identified local SSD
/// reads beside other disk I/O (`Shared`); unknown, rotational and network storage
/// is read alone (`Exclusive`).
pub(super) trait Storage: Sync {
    fn confirmed_local_ssd(&self, path: &Path) -> bool;
}
impl Storage for Arc<dyn FileSystem> {
    fn confirmed_local_ssd(&self, path: &Path) -> bool {
        self.storage_kind(path)
            .is_ok_and(|kind| kind == StorageKind::LocalSsd)
    }
}
/// Storage already classified for every path of a run; `Known(false)` when no
/// classifier exists.
pub(super) struct Known(pub(super) bool);
impl Storage for Known {
    fn confirmed_local_ssd(&self, _: &Path) -> bool {
        self.0
    }
}
pub(super) const UNCLASSIFIED: Known = Known(false);

/// Test storage: `rule` classifies each path; every query is recorded.
#[cfg(test)]
pub(super) struct Recorded {
    pub(super) rule: Box<dyn Fn(&Path) -> bool + Send + Sync>,
    pub(super) queries: std::sync::Mutex<Vec<PathBuf>>,
}
#[cfg(test)]
impl Recorded {
    pub(super) fn new(rule: impl Fn(&Path) -> bool + Send + Sync + 'static) -> Self {
        Self {
            rule: Box::new(rule),
            queries: std::sync::Mutex::default(),
        }
    }
    pub(super) fn queries(&self) -> Vec<PathBuf> {
        self.queries.lock().unwrap().clone()
    }
}
#[cfg(test)]
impl Storage for Recorded {
    fn confirmed_local_ssd(&self, path: &Path) -> bool {
        self.queries.lock().unwrap().push(path.to_owned());
        (self.rule)(path)
    }
}
/// In a native call: whether the caller's own disk permit is exclusive. Shared
/// and exclusive admission exclude each other, so other tests cannot blur this.
#[cfg(test)]
pub(super) fn holds_exclusive_io() -> bool {
    midnight_work_budget::shared_io().snapshot().exclusive
}

/// Native revalidation reopens a source's parent and name (and, for manifest
/// entries, every ancestor), so it is not repeated after every 256 KiB chunk.
/// Bytes come from the pinned handle; the unconditional checks before the first
/// and after the last read establish the live-file guarantee. This cadence only
/// bounds how long a replaced or rewritten source can keep streaming.
pub(super) const REVALIDATION_INTERVAL: Duration = Duration::from_millis(250);

/// Due-time tracker for intermediate revalidation between the mandatory checks.
pub(super) struct Cadence {
    interval: Duration,
    last: Instant,
}
impl Cadence {
    pub(super) fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: Instant::now(),
        }
    }
    /// Whether an intermediate check is due; a due check restarts the interval.
    pub(super) fn due(&mut self) -> bool {
        let now = Instant::now();
        let due = now.saturating_duration_since(self.last) >= self.interval;
        if due {
            self.last = now;
        }
        due
    }
}

#[derive(Clone, Copy)]
struct Settings {
    revalidation: Duration,
    /// How often progress, listed files and finished rows reach the window.
    update: Duration,
    /// List every file before reading any, so the window shows them all with the
    /// run's total. Verification already knows its files from the manifest.
    listing: bool,
    /// A selected folder contributes its own files even without recursion.
    /// Verification names files only: a folder there stays a skipped entry.
    folders: bool,
}
const CALCULATION: Settings = Settings {
    revalidation: REVALIDATION_INTERVAL,
    update: UPDATE_INTERVAL,
    listing: true,
    folders: true,
};
/// Progress, listed files and finished rows reach the window this often.
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);
/// Listed files go to the window in batches of at most this many.
const PLANNED_BATCH: usize = 512;

pub(super) fn calculate_captured(
    request: ChecksumRequest,
    source: Option<&dyn CapturedChecksumFiles>,
    service: &dyn TransferService,
    storage: &dyn Storage,
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
            rows: ChecksumRows::default(),
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
                diagnostics: vec![error],
            },
            retention,
        });
    }
    calculate(request, service, storage, cancel, retention, emit)
}

pub(super) fn calculate(
    request: ChecksumRequest,
    service: &dyn TransferService,
    storage: &dyn Storage,
    cancel: &Cancellation,
    retention: Arc<ChecksumRetention>,
    emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let base = common_base(&request.targets);
    let roots: Arc<[_]> = request
        .targets
        .iter()
        .map(|target| target.path.clone())
        .collect();
    let targets = Targets::Request(&request.targets);
    let pass = Pass {
        request: &request,
        targets: &targets,
        base,
        settings: CALCULATION,
    };
    run(pass, roots, service, storage, cancel, retention, emit)
}

/// Read a manifest's entries, named below its `folder`, one target at a time. They are
/// compared, never exported or listed first.
#[allow(clippy::too_many_arguments)] // A pass plus the worker's own inputs.
pub(super) fn calculate_for_verification(
    request: &ChecksumRequest,
    folder: Arc<Path>,
    names: &[Box<Path>],
    manifest: PathBuf,
    service: &dyn TransferService,
    storage: &dyn Storage,
    cancel: &Cancellation,
    retention: Arc<ChecksumRetention>,
    emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let targets = Targets::Manifest {
        folder: &folder,
        names,
    };
    let pass = Pass {
        request,
        targets: &targets,
        base: folder.clone(),
        settings: Settings {
            listing: false,
            folders: false,
            ..CALCULATION
        },
    };
    run(
        pass,
        vec![manifest].into(),
        service,
        storage,
        cancel,
        retention,
        emit,
    )
}

/// What a run reads: the request's targets, or a manifest's entries made one at a time.
pub(super) enum Targets<'a> {
    Request(&'a [ChecksumTarget]),
    Manifest {
        folder: &'a Path,
        names: &'a [Box<Path>],
    },
}
impl Targets<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Request(targets) => targets.len(),
            Self::Manifest { names, .. } => names.len(),
        }
    }
    fn get(&self, index: usize) -> Option<Cow<'_, ChecksumTarget>> {
        match self {
            Self::Request(targets) => targets.get(index).map(Cow::Borrowed),
            Self::Manifest { folder, names } => names
                .get(index)
                .map(|name| Cow::Owned(ChecksumTarget::from(folder.join(name)))),
        }
    }
}

/// What both passes of a run share.
struct Pass<'a> {
    request: &'a ChecksumRequest,
    targets: &'a Targets<'a>,
    /// Rows and listed files are named below this folder.
    base: Arc<Path>,
    settings: Settings,
}

/// The deepest folder that holds every target: rows are named below it, so most rows keep
/// a short name. Targets with no common folder (other volumes) keep whole paths.
pub(super) fn common_base(targets: &[ChecksumTarget]) -> Arc<Path> {
    let mut base: Option<PathBuf> = None;
    for target in targets {
        let parent = target.path.parent().unwrap_or(&target.path);
        base = Some(match base {
            Some(base) if parent.starts_with(&base) => base,
            Some(base) => base
                .components()
                .zip(parent.components())
                .take_while(|(known, next)| known == next)
                .map(|(known, _)| known)
                .collect(),
            None => parent.to_owned(),
        });
    }
    base.unwrap_or_default().into()
}

fn run(
    pass: Pass<'_>,
    roots: Arc<[PathBuf]>,
    service: &dyn TransferService,
    storage: &dyn Storage,
    cancel: &Cancellation,
    retention: Arc<ChecksumRetention>,
    mut emit: impl FnMut(ChecksumUpdate) -> bool,
) -> Arc<ChecksumRun> {
    let request = pass.request;
    // First list every file the reading pass will read, in its order, so the window
    // shows them all and the run's total. Entries that cannot be read are reported by
    // the reading pass, which walks the same way.
    let mut planned = (0, 0);
    let mut admissions = std::collections::HashMap::new();
    if pass.settings.listing {
        let mut listing = State::new(&pass, cancel, storage, retention.clone(), &mut emit, true);
        listing.announce();
        listing.visit(service);
        listing.flush_planned();
        planned = (listing.progress.planned, listing.progress.planned_bytes);
        admissions = std::mem::take(&mut listing.admissions);
    }
    let mut state = State::new(&pass, cancel, storage, retention.clone(), &mut emit, false);
    (state.progress.planned, state.progress.planned_bytes) = planned;
    state.admissions = admissions;
    if pass.settings.listing {
        state.announce();
    }
    state.visit(service);
    state.flush_batch();
    state.progress.current = None;
    state.progress.current_read = 0;
    state.progress.current_size = None;
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
        rows: std::mem::take(&mut state.rows),
        retention,
        summary: ChecksumSummary {
            completion,
            progress: state.progress.clone(),
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
    targets: &'a Targets<'a>,
    /// Rows and listed files are named below this folder.
    base: Arc<Path>,
    settings: Settings,
    cancel: &'a Cancellation,
    storage: &'a dyn Storage,
    /// Disk admission for the current root's volume.
    admission: Admission,
    /// Classified directories, queried once per run: the reading pass reuses what
    /// the listing pass learned.
    admissions: std::collections::HashMap<PathBuf, Admission>,
    retention: Arc<ChecksumRetention>,
    /// The rows sent so far, in chunks shared with the window.
    rows: ChecksumRows,
    batch: Vec<ChecksumRow>,
    progress: ChecksumProgress,
    diagnostics: Vec<FsError>,
    /// Folders entered, by identity: a folder reached again (through a link or another
    /// target) is not walked twice, so a walk ends whatever the links.
    directories: HashSet<TransferIdentity>,
    /// Targets that another target can also reach (a selected file in a selected
    /// folder), and those of them already counted. Empty when every target is in one
    /// folder, which no target can then hold.
    nested: HashSet<PathBuf>,
    counted: HashSet<PathBuf>,
    incomplete: bool,
    last_progress: Instant,
    /// When listed files or rows last went to the window.
    last_flush: Instant,
    buffer: Vec<u8>,
    emit: &'a mut F,
    /// The listing pass: files are listed, not read, and failures wait for the
    /// reading pass.
    listing: bool,
    planned: Vec<ChecksumPlannedFile>,
}
impl<'a, F: FnMut(ChecksumUpdate) -> bool> State<'a, F> {
    fn new(
        pass: &Pass<'a>,
        cancel: &'a Cancellation,
        storage: &'a dyn Storage,
        retention: Arc<ChecksumRetention>,
        emit: &'a mut F,
        listing: bool,
    ) -> Self {
        Self {
            request: pass.request,
            targets: pass.targets,
            base: pass.base.clone(),
            settings: pass.settings,
            cancel,
            storage,
            admission: Admission::Exclusive,
            admissions: std::collections::HashMap::new(),
            retention,
            rows: ChecksumRows::default(),
            batch: Vec::new(),
            progress: ChecksumProgress {
                listing,
                ..ChecksumProgress::default()
            },
            diagnostics: Vec::new(),
            directories: HashSet::new(),
            nested: match pass.targets {
                Targets::Request(targets) => nested_targets(targets),
                // A manifest names each file once, below one folder.
                Targets::Manifest { .. } => HashSet::new(),
            },
            counted: HashSet::new(),
            incomplete: false,
            last_progress: Instant::now(),
            last_flush: Instant::now(),
            // Listing reads no contents.
            buffer: if listing {
                Vec::new()
            } else {
                vec![0; CHECKSUM_BUFFER_BYTES]
            },
            emit,
            listing,
            planned: Vec::new(),
        }
    }
}

/// The targets another target may also reach: none when every target is a distinct
/// entry of one folder (a flat panel selection), otherwise all of them.
fn nested_targets(targets: &[ChecksumTarget]) -> HashSet<PathBuf> {
    let first = targets.first().and_then(|target| target.path.parent());
    let mut distinct = HashSet::with_capacity(targets.len());
    let flat = targets
        .iter()
        .all(|target| target.path.parent() == first && distinct.insert(target.path.as_path()));
    if flat {
        return HashSet::new();
    }
    targets.iter().map(|target| target.path.clone()).collect()
}

impl<F: FnMut(ChecksumUpdate) -> bool> State<'_, F> {
    /// Walk the run's targets: list their files, or read them.
    fn visit(&mut self, service: &dyn TransferService) {
        let request = self.request;
        let cancel = self.cancel;
        let targets = self.targets;
        for index in 0..targets.len() {
            let Some(target) = targets.get(index) else {
                break;
            };
            let target = target.as_ref();
            if self.stopped() {
                break;
            }
            // A target that another target already reached, or the same one twice, is
            // counted once.
            if !self.first_time(&target.path) {
                continue;
            }
            self.discover();
            // An entry lives on its directory's volume.
            self.classify_parent(target.path.parent().unwrap_or(&target.path));
            let opened = with_io(self.admission, cancel, &target.path, || {
                service.prepare_transfer_source(&target.path, cancel)
            });
            match opened {
                Ok(source) => {
                    // A walk never crosses a native volume boundary, but a selected
                    // directory can itself be a mount point of another volume.
                    if request.recursive && source.entry().item.kind == EntryKind::Directory {
                        self.admission = self.admission_of(&target.path);
                    }
                    if !matches_capture(source.entry(), target) {
                        if request.recursive
                            && (target.kind == Some(EntryKind::Directory)
                                || source.entry().item.kind == EntryKind::Directory)
                        {
                            self.incomplete = true;
                        }
                        self.failed(
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
                        self.walk(source);
                    }
                }
                Err(error) => self.open_failed(error, target.kind),
            }
        }
    }
    fn stopped(&self) -> bool {
        self.cancel.is_cancelled()
    }
    fn admission_of(&mut self, path: &Path) -> Admission {
        if let Some(admission) = self.admissions.get(path) {
            return *admission;
        }
        let admission =
            midnight_work_budget::storage_admission(self.storage.confirmed_local_ssd(path));
        self.admissions.insert(path.to_owned(), admission);
        admission
    }
    /// Admission for an entry of `directory` (selected entries usually share one).
    fn classify_parent(&mut self, directory: &Path) {
        self.admission = self.admission_of(directory);
    }
    /// Whether a target, or a path another target may also reach, is met for the first
    /// time. Any other path is: a walk lists each name of a folder once.
    fn first_time(&mut self, path: &Path) -> bool {
        !self.nested.contains(path) || self.counted.insert(path.to_owned())
    }
    /// One more entry found; there is no limit to how many.
    fn discover(&mut self) {
        self.progress.discovered = self.progress.discovered.saturating_add(1);
    }
    /// A path's name below the run's base folder; the whole path when it is not below it.
    fn name(&self, path: &Path) -> Box<Path> {
        path.strip_prefix(&self.base).unwrap_or(path).into()
    }
    /// Walk one target without recursion: a stack of the open folders on the way down.
    /// Every level is walked; a folder reached again (a link loop, another target) is
    /// recognized by its identity and walked once.
    fn walk(&mut self, source: Box<dyn TransferSource>) {
        let root_device = source.entry().identity.device;
        let mut stack = Vec::<Box<dyn TransferSource>>::new();
        let mut current = Some(ListedChild::Source(source));
        // The children a pass opens: the folders it enters (only with Include
        // subfolders) and, when reading, the files it reads, and links and special
        // entries, whose rows then carry the provider's own refusal. The listing pass
        // opens no file; neither pass opens a subfolder it leaves out.
        let (recursive, reading) = (self.request.recursive, !self.listing);
        let open = move |kind: EntryKind| {
            if kind == EntryKind::Directory {
                recursive
            } else {
                reading
            }
        };
        loop {
            if self.stopped() {
                break;
            }
            if let Some(child) = current.take() {
                let entry = child.entry().clone();
                // A selected folder contributes its own files; Include subfolders adds
                // the levels below, whose folders are otherwise left out without rows,
                // also one that is a mount point of another volume.
                let left_out = entry.item.kind == EntryKind::Directory
                    && !self.request.recursive
                    && self.settings.folders
                    && !stack.is_empty();
                if left_out {
                    continue;
                }
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
                    EntryKind::File => self.file(child),
                    EntryKind::Directory if self.request.recursive
                        || (self.settings.folders && stack.is_empty()) => {
                        let ListedChild::Source(source) = child else {
                            // A folder to enter is always opened (`open`); a source
                            // that only described it leaves its contents unread.
                            self.open_failed(failure(&entry.item.path, io::ErrorKind::InvalidData,
                                "The folder was listed but not opened."), Some(EntryKind::Directory));
                            continue;
                        };
                        if !self.directories.insert(entry.identity) { continue; }
                        if let Err(error) = with_io(self.admission, self.cancel,&entry.item.path, || source.revalidate(self.cancel)) {
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
            let next = with_io(self.admission, self.cancel, &parent_path, || {
                parent.next_listed_child(&open, self.cancel)
            });
            match next {
                Ok(Some(child)) => {
                    if self.first_time(&child.entry().item.path) {
                        self.discover();
                        current = Some(child);
                    }
                }
                Ok(None) => {
                    stack.pop();
                }
                Err(error) => {
                    let directory_failed = error.path == parent_path;
                    if directory_failed {
                        // An unreadable folder fails alone; the run goes on.
                        self.open_failed(error, Some(EntryKind::Directory));
                        self.incomplete = true;
                        stack.pop();
                    } else if self.first_time(&error.path) {
                        self.discover();
                        self.open_failed(error, None);
                    }
                }
            }
        }
    }
    fn file(&mut self, child: ListedChild) {
        let entry = child.entry().clone();
        if self.listing {
            self.plan(ChecksumPlannedFile {
                name: self.name(&entry.item.path),
                size: entry.item.metadata.size,
            });
            return;
        }
        let mut source = match child {
            ListedChild::Source(source) => source,
            // The reading pass opens every file it is given (`walk`); a source that
            // only described one leaves it unread.
            ListedChild::Entry(_) => {
                let error = failure(
                    &entry.item.path,
                    io::ErrorKind::InvalidData,
                    "The file was listed but not opened.",
                );
                self.failed(
                    &entry.item.path,
                    Some(&entry),
                    ChecksumRowState::ReadError,
                    error,
                );
                return;
            }
        };
        self.progress.current = Some(entry.item.path.clone());
        self.progress.current_read = 0;
        self.progress.current_size = entry.item.metadata.size;
        let mut hashers = Hashers::new(self.request.algorithms);
        let mut read = 0u64;
        let result = (|| {
            with_io(self.admission, self.cancel, &entry.item.path, || {
                source.revalidate(self.cancel)
            })?;
            let mut cadence = Cadence::new(self.settings.revalidation);
            loop {
                if self.cancel.is_cancelled() {
                    return Err(failure(
                        &entry.item.path,
                        io::ErrorKind::Interrupted,
                        "Checksum calculation cancelled.",
                    ));
                }
                let count = match with_io(self.admission, self.cancel, &entry.item.path, || {
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
                if entry.item.metadata.size.is_none_or(|length| read > length) {
                    return Err(failure(
                        &entry.item.path,
                        io::ErrorKind::InvalidData,
                        "Source grew or has no regular-file length.",
                    ));
                }
                hashers.update(&self.buffer[..count]);
                if cadence.due() {
                    with_io(self.admission, self.cancel, &entry.item.path, || {
                        source.revalidate(self.cancel)
                    })?;
                }
                self.progress.current_read = read;
                self.progress();
            }
            with_io(self.admission, self.cancel, &entry.item.path, || {
                source.revalidate(self.cancel)
            })?;
            if entry.item.metadata.size != Some(read) {
                return Err(failure(
                    &entry.item.path,
                    io::ErrorKind::InvalidData,
                    "Source length changed during calculation.",
                ));
            }
            Ok(())
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
                (state, ChecksumDigests::default(), Some(Box::new(error)))
            }
        };
        let name = self.name(&entry.item.path);
        self.row(ChecksumRow {
            id: self.next_id(),
            name,
            identity: Some(entry.identity),
            kind: Some(entry.item.kind),
            state,
            bytes_read: read,
            digests,
            error,
        });
    }
    fn open_failed(&mut self, error: FsError, known_kind: Option<EntryKind>) {
        if self.listing {
            return;
        }
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
        if self.listing {
            return;
        }
        self.diagnostic(&error);
        if state == ChecksumRowState::Skipped {
            self.progress.skipped += 1;
        } else if state != ChecksumRowState::Cancelled {
            self.progress.errors += 1;
        }
        let name = self.name(path);
        self.row(ChecksumRow {
            id: self.next_id(),
            name,
            identity: entry.map(|entry| entry.identity),
            kind: entry.map(|entry| entry.item.kind),
            state,
            bytes_read: 0,
            digests: ChecksumDigests::default(),
            error: Some(Box::new(error)),
        });
    }
    fn diagnostic(&mut self, error: &FsError) {
        if self.diagnostics.len() < MAX_CHECKSUM_DIAGNOSTICS {
            self.diagnostics.push(error.clone());
        }
    }
    /// Rows are numbered from one in the order found.
    fn next_id(&self) -> ChecksumRowId {
        ChecksumRowId((self.rows.len() + self.batch.len()) as u64 + 1)
    }
    fn row(&mut self, row: ChecksumRow) {
        self.batch.push(row);
        // A finished file reaches the window at once, unless many finish quickly:
        // those wait at most the update interval (see `progress`).
        if self.batch.len() >= CHECKSUM_BATCH_SIZE
            || self.last_flush.elapsed() >= self.settings.update
        {
            self.flush_batch();
        }
        self.progress();
    }
    /// Send the waiting rows as one chunk, which the run keeps too.
    fn flush_batch(&mut self) {
        self.last_flush = Instant::now();
        if self.batch.is_empty() {
            return;
        }
        let chunk = Arc::new(ChecksumChunk {
            base: self.base.clone(),
            rows: std::mem::take(&mut self.batch).into_boxed_slice(),
            retention: self.retention.clone(),
        });
        self.rows.push(chunk.clone());
        if !(self.emit)(ChecksumUpdate::Batch(chunk)) {
            self.cancel.cancel();
        }
    }
    fn plan(&mut self, file: ChecksumPlannedFile) {
        self.progress.planned += 1;
        self.progress.planned_bytes = self
            .progress
            .planned_bytes
            .saturating_add(file.size.unwrap_or(0));
        self.planned.push(file);
        if self.planned.len() >= PLANNED_BATCH || self.last_flush.elapsed() >= self.settings.update
        {
            self.flush_planned();
        }
        self.progress();
    }
    fn flush_planned(&mut self) {
        self.last_flush = Instant::now();
        if !self.planned.is_empty()
            && !(self.emit)(ChecksumUpdate::Planned(Arc::new(ChecksumPlanned {
                base: self.base.clone(),
                files: std::mem::take(&mut self.planned).into_boxed_slice(),
            })))
        {
            self.cancel.cancel();
        }
    }
    fn progress(&mut self) {
        if self.last_progress.elapsed() >= self.settings.update {
            // Rows or listed files held back by quick successions go first.
            if !self.batch.is_empty() {
                self.flush_batch();
            }
            if !self.planned.is_empty() {
                self.flush_planned();
            }
            self.announce();
        }
    }
    /// Report progress now, for example as the reading pass starts.
    fn announce(&mut self) {
        self.last_progress = Instant::now();
        if !(self.emit)(ChecksumUpdate::Progress(self.progress.clone())) {
            self.cancel.cancel();
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
    /// Every chosen algorithm's digest, kept at its fixed length in one allocation.
    fn finish(self) -> ChecksumDigests {
        let md5 = self.md5.map(Md5::finalize);
        let sha1 = self.sha1.map(Sha1::finalize);
        let sha256 = self.sha256.map(Sha256::finalize);
        let sha512 = self.sha512.map(Sha512::finalize);
        let crc32 = self.crc32.map(|hash| hash.finalize().to_be_bytes());
        let values: Vec<(ChecksumAlgorithm, &[u8])> = [
            (
                ChecksumAlgorithm::Md5,
                md5.as_ref().map(|value| value.as_slice()),
            ),
            (
                ChecksumAlgorithm::Sha1,
                sha1.as_ref().map(|value| value.as_slice()),
            ),
            (
                ChecksumAlgorithm::Sha256,
                sha256.as_ref().map(|value| value.as_slice()),
            ),
            (
                ChecksumAlgorithm::Sha512,
                sha512.as_ref().map(|value| value.as_slice()),
            ),
            (
                ChecksumAlgorithm::Crc32,
                crc32.as_ref().map(|value| value.as_slice()),
            ),
        ]
        .into_iter()
        .filter_map(|(algorithm, value)| Some((algorithm, value?)))
        .collect();
        ChecksumDigests::new(&values).expect("each hash yields its algorithm's digest length")
    }
}

/// One call-scoped admission on the process-wide disk gate per native call, so
/// other disk users interleave between a run's chunks (see `Storage`).
pub(super) fn with_io<T>(
    admission: Admission,
    cancel: &Cancellation,
    path: &Path,
    action: impl FnOnce() -> Result<T, FsError>,
) -> Result<T, FsError> {
    let _permit = midnight_work_budget::shared_io()
        .acquire(admission, || cancel.is_cancelled())
        .ok_or_else(|| failure(path, io::ErrorKind::Interrupted, "Checksum I/O cancelled."))?;
    action()
}
pub(super) fn failure(path: &Path, kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io("checksums", path, io::Error::new(kind, message))
}

#[cfg(test)]
#[path = "checksum_worker_tests.rs"]
mod tests;
