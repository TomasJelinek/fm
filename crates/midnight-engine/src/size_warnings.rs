//! Streaming diagnostic records with retained, immutable text snapshots.
//! Construction, recording and reads belong on backend workers; retained owners retire native
//! storage independently of the final caller or its executor lifetime.
//!
//! Every generated-content spool (these reports, tree listing errors and File List
//! output) chooses its private per-user root with `private_spool_roots` and opens
//! it with `create_private_spool`.
use crate::report_arena::{ReportArena, Retained, Retention};
use midnight_core::{
    Cancellation, FsError, GeneratedTextSource, SizeWarningReport, SizeWarningSource,
    SizeWarningSummary,
};
use midnight_worker_platform::spool::{AppendSpool, MAX_IO, PrivateSpoolDirectory, SpoolPrefix};
use std::{
    fmt,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, PoisonError, Weak},
};

#[cfg(target_os = "linux")]
#[path = "size_warnings_roots_linux.rs"]
mod session;
#[cfg(windows)]
#[path = "size_warnings_roots_windows.rs"]
mod session;

const BUFFER_BYTES: usize = 16 * 1024;
const DEFAULT_TITLE: &str = "Directory size warnings";
const TOOLTIP_BYTES: usize = 768;
/// The common path component limit (ext4 bytes, NTFS UTF-16 units).
const MAX_COMPONENT: usize = 255;
/// The spool name shared by all diagnostic report logs.
const REPORTS: &str = "reports";

trait Storage: Send {
    fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<u64>;
    fn length(&self) -> u64;
    fn prefix(&self, start: u64, end: u64) -> io::Result<SpoolPrefix>;
}
impl Storage for AppendSpool {
    fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<u64> {
        self.append(offset, bytes)
    }
    fn length(&self) -> u64 {
        self.length()
    }
    fn prefix(&self, start: u64, end: u64) -> io::Result<SpoolPrefix> {
        self.prefix(start, end)
    }
}

#[derive(Clone, Copy, Default)]
struct Boundary {
    bytes: u64,
    observed: u64,
    stored: u64,
}

struct State {
    /// Candidate private roots, tried in order until one opens and stores. `None`
    /// stands for this user's report roots, resolved only when the first record
    /// needs storage: most logs never record anything.
    directories: Option<Vec<SpoolRoot>>,
    title: &'static str,
    storage: Option<Retained<Mutex<Box<dyn Storage>>>>,
    arena: ReportArena,
    failure: Option<io::Error>,
    boundary: Boundary,
    first_issues: Vec<Weak<OnceLock<Arc<str>>>>,
}

#[derive(Clone)]
pub(crate) struct ReportLog(Arc<Mutex<State>>);

/// A subtree's starting boundary. It owns no per-error list or path copies.
#[derive(Clone)]
pub(crate) struct ReportMark {
    state: Arc<Mutex<State>>,
    start: Boundary,
    first_issue: Arc<OnceLock<Arc<str>>>,
}

#[derive(Clone, Copy)]
struct End {
    boundary: Boundary,
    cancelled: bool,
}

pub(crate) struct ReportRange {
    mark: ReportMark,
    end: Mutex<Option<End>>,
}

impl fmt::Debug for ReportRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SizeWarningReport").finish_non_exhaustive()
    }
}

/// One candidate private spool root (see `private_spool_roots`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpoolRoot {
    pub(crate) path: PathBuf,
    /// Only this process uses the root, so it is removed after its last spool
    /// closes instead of accumulating one directory per run.
    pub(crate) per_process: bool,
}

/// What a spool holds. Only whether a disk-backed root comes first depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpoolContent {
    /// Diagnostic reports stay small, so the session's runtime directory comes first.
    Reports,
    /// Generated output that can be large, such as a file list: the user's
    /// disk-backed cache precedes the runtime directory, a tmpfs that is sized
    /// for small runtime files.
    Large,
}

/// This session's per-user bases, gathered by the platform module. Bases that
/// are not absolute paths are ignored.
struct Session {
    /// The application's directory name and name prefix on this platform.
    application: &'static str,
    /// The user's disk-backed cache directory (Linux `$XDG_CACHE_HOME`).
    cache: Option<PathBuf>,
    /// The session's owner-only runtime directory (Linux `$XDG_RUNTIME_DIR`).
    runtime: Option<PathBuf>,
    /// A file-name-safe, lossless label of this account for names in the
    /// temporary directory, which other accounts may share.
    account: Option<String>,
    temporary: PathBuf,
}

/// This user's private roots for the spool `name`, in preference order. All
/// generated-content spools share this policy. Linux uses the XDG directories
/// first: for large content the disk-backed cache, then the session's owner-only
/// `$XDG_RUNTIME_DIR`. The temporary directory (per user on Windows, usually the
/// shared `/tmp` on Linux) always comes last, with a per-user name and then a
/// per-process name. Every root must open as an owner-only directory of this user
/// that is reached without links (`PrivateSpoolDirectory::open`), so a directory
/// that another account created first, by accident or to deny storage, is
/// skipped rather than used. Open spools with `create_private_spool`, which
/// removes a per-process root after its last spool.
pub(crate) fn private_spool_roots(name: &str, content: SpoolContent) -> Vec<SpoolRoot> {
    spool_roots(&session::current(), std::process::id(), name, content)
}

fn spool_roots(
    session: &Session,
    process: u32,
    name: &str,
    content: SpoolContent,
) -> Vec<SpoolRoot> {
    let application = session.application;
    let kept = |path| SpoolRoot {
        path,
        per_process: false,
    };
    let cache = session
        .cache
        .as_ref()
        .filter(|_| content == SpoolContent::Large);
    let mut roots: Vec<_> = cache
        .into_iter()
        .chain(&session.runtime)
        .filter(|base| base.is_absolute())
        .map(|base| kept(base.join(application).join(name)))
        .collect();
    let temporary = &session.temporary;
    if temporary.is_absolute() {
        let per_user = session
            .account
            .as_ref()
            .map(|account| format!("{application}-{account}-{name}"))
            // A label too long for one path component keeps the per-process root.
            .filter(|component| component.len() <= MAX_COMPONENT);
        roots.extend(per_user.map(|component| kept(temporary.join(component))));
        roots.push(SpoolRoot {
            path: temporary.join(format!("{application}-{name}-{process}")),
            per_process: true,
        });
    }
    roots
}

/// A spool in a private root. Field order closes the spool, and the root
/// handles it pins, before the guard that removes an empty per-process root.
/// Read-only prefixes of the spool must not outlive it.
pub(crate) struct PrivateSpool<T> {
    spool: T,
    _root: Option<Arc<ProcessRoot>>,
}
impl<T> std::ops::Deref for PrivateSpool<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.spool
    }
}
impl<T> std::ops::DerefMut for PrivateSpool<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.spool
    }
}
impl<T: Storage> Storage for PrivateSpool<T> {
    fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<u64> {
        self.spool.append(offset, bytes)
    }
    fn length(&self) -> u64 {
        self.spool.length()
    }
    fn prefix(&self, start: u64, end: u64) -> io::Result<SpoolPrefix> {
        self.spool.prefix(start, end)
    }
}

/// Create one spool with `create` in the first candidate root that opens as an
/// owner-only directory of this user and accepts the spool. A refused root, or
/// one whose filesystem lacks anonymous files, yields to the next candidate; the
/// last candidate's error is returned otherwise. Backend workers only (native I/O).
pub(crate) fn create_private_spool<T>(
    roots: &[SpoolRoot],
    create: impl Fn(&PrivateSpoolDirectory) -> io::Result<T>,
) -> io::Result<PrivateSpool<T>> {
    let mut failure = io::Error::new(
        io::ErrorKind::NotFound,
        "No private per-user spool directory is available",
    );
    for root in roots {
        // The guard exists before the open, so the root cannot be removed
        // between its open and the spool creation in it.
        let guard = root.per_process.then(|| ProcessRoot::acquire(&root.path));
        match PrivateSpoolDirectory::open(&root.path).and_then(|directory| create(&directory)) {
            Ok(spool) => {
                return Ok(PrivateSpool {
                    spool,
                    _root: guard,
                });
            }
            Err(error) => {
                failure = io::Error::new(error.kind(), format!("{}: {error}", root.path.display()));
            }
        }
    }
    Err(failure)
}

/// Guards of live per-process roots. A guard is created, and an unused root is
/// removed, only under this lock, so no opener can see its root disappear.
static PROCESS_ROOTS: Mutex<Vec<(PathBuf, Weak<ProcessRoot>)>> = Mutex::new(Vec::new());

/// Keeps a per-process root while a spool of this process may use it.
struct ProcessRoot(PathBuf);
impl ProcessRoot {
    fn acquire(path: &Path) -> Arc<Self> {
        let mut roots = PROCESS_ROOTS.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(live) = roots
            .iter()
            .filter(|(root, _)| root == path)
            .find_map(|(_, guard)| guard.upgrade())
        {
            return live;
        }
        roots.retain(|(root, _)| root != path);
        let guard = Arc::new(Self(path.to_owned()));
        roots.push((path.to_owned(), Arc::downgrade(&guard)));
        guard
    }
}
impl Drop for ProcessRoot {
    fn drop(&mut self) {
        let mut roots = PROCESS_ROOTS.lock().unwrap_or_else(PoisonError::into_inner);
        if roots
            .iter()
            .any(|(root, guard)| *root == self.0 && guard.strong_count() != 0)
        {
            // A newer guard keeps the root for its own spool.
            return;
        }
        roots.retain(|(root, _)| *root != self.0);
        // Spools are anonymous (Linux) or deleted on close (Windows), so the root
        // is empty after its last spool closed. `remove_dir` removes only an empty
        // directory, never a link target; a root it cannot remove is kept.
        let _ = std::fs::remove_dir(&self.0);
    }
}

impl ReportLog {
    pub(crate) fn for_scan(arena: &ReportArena) -> Self {
        Self::private(arena, DEFAULT_TITLE)
    }
    /// A log in this user's private report root (see `private_spool_roots`),
    /// shared by size warnings and listing errors.
    pub(crate) fn private(arena: &ReportArena, title: &'static str) -> Self {
        Self::with_roots(arena, None, title)
    }
    #[cfg(test)]
    pub(crate) fn new(arena: &ReportArena, directory: PathBuf) -> Self {
        Self::with_title(arena, directory, DEFAULT_TITLE)
    }
    #[cfg(test)]
    pub(crate) fn with_title(arena: &ReportArena, directory: PathBuf, title: &'static str) -> Self {
        let root = SpoolRoot {
            path: directory,
            per_process: false,
        };
        Self::with_candidates(arena, vec![root], title)
    }
    #[cfg(test)]
    fn with_candidates(
        arena: &ReportArena,
        directories: Vec<SpoolRoot>,
        title: &'static str,
    ) -> Self {
        Self::with_roots(arena, Some(directories), title)
    }
    fn with_roots(
        arena: &ReportArena,
        directories: Option<Vec<SpoolRoot>>,
        title: &'static str,
    ) -> Self {
        Self(Arc::new(Mutex::new(State {
            directories,
            title,
            storage: None,
            arena: arena.clone(),
            failure: None,
            boundary: Boundary::default(),
            first_issues: Vec::new(),
        })))
    }
    pub(crate) fn mark(&self) -> ReportMark {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let first_issue = Arc::new(OnceLock::new());
        state.first_issues.retain(|weak| weak.strong_count() != 0);
        state.first_issues.push(Arc::downgrade(&first_issue));
        ReportMark {
            state: self.0.clone(),
            start: state.boundary,
            first_issue,
        }
    }
    /// Recording failures never stop filesystem counting. Snapshot text reports
    /// exactly how many issue records were observed and how many were preserved.
    pub(crate) fn record(
        &self,
        location: impl fmt::Display,
        operation: &str,
        kind: io::ErrorKind,
        reason: &str,
    ) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.boundary.observed = state.boundary.observed.saturating_add(1);
        if !state.first_issues.is_empty() {
            let mut summary = ShortText(String::new());
            let _ = fmt::write(
                &mut summary,
                format_args!(
                    "{}: {} ({kind:?}) — {}",
                    Escaped(operation),
                    Escaped(reason),
                    Escaped(&location)
                ),
            );
            let summary: Arc<str> = summary.0.into();
            for pending in state.first_issues.drain(..) {
                if let Some(pending) = pending.upgrade() {
                    let _ = pending.set(summary.clone());
                }
            }
        }
        if state.failure.is_some() {
            return;
        }
        let result = (|| {
            if state.storage.is_none() {
                let reservation = state.arena.reserve()?;
                // Storage is created once: a failure is final for this log.
                let directories = state
                    .directories
                    .take()
                    .unwrap_or_else(|| private_spool_roots(REPORTS, SpoolContent::Reports));
                // Every read prefix holds a retention of this storage, so the
                // per-process root guard inside it outlives all of them.
                let storage: Box<dyn Storage> = Box::new(create_private_spool(
                    &directories,
                    PrivateSpoolDirectory::create_append_only,
                )?);
                state.storage = Some(reservation.retain(Mutex::new(storage)));
            }
            let storage = state.storage.as_ref().expect("created private report");
            let mut storage = storage.lock().unwrap_or_else(|error| error.into_inner());
            let mut writer = RecordWriter::new(storage.as_mut());
            write!(
                writer,
                "Path: {}\nOperation: {}\nKind: {kind:?}\nReason: {}\n\n",
                Escaped(&location),
                Escaped(operation),
                Escaped(reason),
            )?;
            writer.flush()?;
            Ok(writer.storage.length())
        })();
        match result {
            Ok(bytes) => {
                state.boundary.bytes = bytes;
                state.boundary.stored = state.boundary.stored.saturating_add(1);
            }
            Err(error) => state.failure = Some(error),
        }
    }
}

impl ReportMark {
    pub(crate) fn report(&self) -> Arc<ReportRange> {
        Arc::new(ReportRange {
            mark: self.clone(),
            end: Mutex::new(None),
        })
    }
}
impl ReportRange {
    pub(crate) fn finish(&self, cancelled: bool) {
        let mut state = self
            .mark
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut end = self.end.lock().unwrap_or_else(|error| error.into_inner());
        if end.is_none() {
            *end = Some(End {
                boundary: state.boundary,
                cancelled,
            });
            let first = Arc::downgrade(&self.mark.first_issue);
            state
                .first_issues
                .retain(|weak| !weak.ptr_eq(&first) && weak.strong_count() != 0);
        }
    }
    /// Freeze this publication's complete records, counts, and first reason.
    /// A later sibling or subsequent error cannot change an already visible total.
    pub(crate) fn handle(&self) -> Option<SizeWarningReport> {
        let state = self
            .mark
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let end = self.end.lock().unwrap_or_else(|error| error.into_inner());
        let boundary = end.as_ref().map_or(state.boundary, |end| end.boundary);
        let observed = boundary.observed - self.mark.start.observed;
        if observed == 0 {
            return None;
        }
        let stored = boundary.stored - self.mark.start.stored;
        let first = self
            .mark
            .first_issue
            .get()
            .map_or("Details unavailable", AsRef::as_ref);
        let mut tooltip = format!("{first}\n{observed} issue(s). Click for full details.");
        if stored != observed {
            tooltip.push_str(&format!(
                "\nReport storage failed: only {stored} details stored."
            ));
        }
        Some(SizeWarningReport::new(
            Arc::new(FrozenReport {
                mark: self.mark.clone(),
                boundary,
                end: *end,
            }),
            SizeWarningSummary {
                observed,
                stored,
                tooltip: tooltip.into(),
            },
        ))
    }
    pub(crate) fn snapshot(
        &self,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn GeneratedTextSource>, FsError> {
        check_cancel(cancel)?;
        // Both methods lock in this order. A snapshot cannot observe a partial
        // record or borrow a live writer cursor, even during concurrent appends.
        let state = self
            .mark
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let end = self.end.lock().unwrap_or_else(|error| error.into_inner());
        let boundary = end.as_ref().map_or(state.boundary, |end| end.boundary);
        snapshot_at(&self.mark, &state, boundary, *end, cancel)
    }
}

struct FrozenReport {
    mark: ReportMark,
    boundary: Boundary,
    end: Option<End>,
}
impl fmt::Debug for FrozenReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrozenSizeWarningReport")
            .finish_non_exhaustive()
    }
}
impl SizeWarningSource for FrozenReport {
    fn snapshot(&self, cancel: &Cancellation) -> Result<Arc<dyn GeneratedTextSource>, FsError> {
        check_cancel(cancel)?;
        let state = self
            .mark
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        snapshot_at(&self.mark, &state, self.boundary, self.end, cancel)
    }
}
fn snapshot_at(
    mark: &ReportMark,
    state: &State,
    boundary: Boundary,
    end: Option<End>,
    cancel: &Cancellation,
) -> Result<Arc<dyn GeneratedTextSource>, FsError> {
    let observed = boundary.observed - mark.start.observed;
    let stored = boundary.stored - mark.start.stored;
    let status = match end {
        None => "Scan running",
        Some(end) if end.cancelled => "Scan cancelled",
        Some(_) => "Scan finished",
    };
    let status = if state.title == DEFAULT_TITLE {
        status.to_owned()
    } else {
        status.replace("Scan", "Report")
    };
    let header = format!(
        "{}\n{status}\nObserved issues: {observed}\nStored details: {stored}\n\n",
        state.title
    );
    let body = if boundary.bytes > mark.start.bytes {
        let storage = state.storage.as_ref().expect("committed report storage");
        let retention = storage.retention();
        let prefix = storage
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .prefix(mark.start.bytes, boundary.bytes)
            .map_err(failure)?;
        Some(Body {
            prefix,
            _retention: retention,
        })
    } else {
        None
    };
    // A finished child must not inherit a later sibling's storage failure.
    let footer = if observed != stored {
        format!(
            "\nReport storage failed: {}\n{} continued; {} issue details could not be stored.\n",
            Escaped(state.failure.as_ref().map_or_else(
                || "Unavailable private report storage".to_owned(),
                ToString::to_string,
            )),
            if state.title == DEFAULT_TITLE {
                "Counting"
            } else {
                "Reporting"
            },
            observed - stored,
        )
    } else {
        String::new()
    };
    check_cancel(cancel)?;
    Ok(Arc::new(Text {
        header,
        body,
        footer,
    }))
}

/// Only the UI summary is bounded; full paths and reasons stream to the report.
struct ShortText(String);
impl fmt::Write for ShortText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let remaining = TOOLTIP_BYTES.saturating_sub(self.0.len());
        if text.len() <= remaining {
            self.0.push_str(text);
            return Ok(());
        }
        let mut end = remaining;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push_str(&text[..end]);
        self.0.push_str("...");
        Err(fmt::Error)
    }
}

/// One bounded buffer; a very long path/reason streams through as many chunks as
/// necessary. No byte/record quota is added to the original filesystem operation.
struct RecordWriter<'a> {
    storage: &'a mut dyn Storage,
    bytes: [u8; BUFFER_BYTES],
    used: usize,
}
impl<'a> RecordWriter<'a> {
    fn new(storage: &'a mut dyn Storage) -> Self {
        Self {
            storage,
            bytes: [0; BUFFER_BYTES],
            used: 0,
        }
    }
}
impl Write for RecordWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let original = bytes.len();
        let mut remaining = bytes;
        while !remaining.is_empty() {
            let count = remaining.len().min(self.bytes.len() - self.used);
            self.bytes[self.used..self.used + count].copy_from_slice(&remaining[..count]);
            self.used += count;
            remaining = &remaining[count..];
            if self.used == self.bytes.len() {
                self.flush()?;
            }
        }
        Ok(original)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.used != 0 {
            self.storage
                .append(self.storage.length(), &self.bytes[..self.used])?;
            self.used = 0;
        }
        Ok(())
    }
}

struct Escaped<T>(T);
impl<T: fmt::Display> fmt::Display for Escaped<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(&mut Escaping(formatter), format_args!("{}", self.0))
    }
}
struct Escaping<'a, 'b>(&'a mut fmt::Formatter<'b>);
impl fmt::Write for Escaping<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut start = 0;
        for (offset, character) in text.char_indices() {
            if character.is_control() {
                self.0.write_str(&text[start..offset])?;
                write!(self.0, "{}", character.escape_debug())?;
                start = offset + character.len_utf8();
            }
        }
        self.0.write_str(&text[start..])
    }
}

// The raw prefix must be released before the token can queue the actual file owner.
struct Body {
    prefix: SpoolPrefix,
    _retention: Retention,
}
struct Text {
    header: String,
    body: Option<Body>,
    footer: String,
}
impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SizeWarningText")
            .field("length", &self.length())
            .finish()
    }
}
impl GeneratedTextSource for Text {
    fn length(&self) -> u64 {
        self.header.len() as u64
            + self.body.as_ref().map_or(0, |body| body.prefix.length())
            + self.footer.len() as u64
    }
    fn read_at(&self, mut offset: u64, output: &mut [u8]) -> io::Result<usize> {
        if output.len() > MAX_IO || offset.checked_add(output.len() as u64).is_none() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if offset < self.header.len() as u64 {
            return Ok(copy_text(&self.header, offset, output));
        }
        offset -= self.header.len() as u64;
        if let Some(body) = &self.body {
            if offset < body.prefix.length() {
                return body.prefix.read_at(offset, output);
            }
            offset -= body.prefix.length();
        }
        Ok(copy_text(&self.footer, offset, output))
    }
}
fn copy_text(text: &str, offset: u64, output: &mut [u8]) -> usize {
    let count = (text.len() as u64)
        .saturating_sub(offset)
        .min(output.len() as u64) as usize;
    if count != 0 {
        output[..count].copy_from_slice(&text.as_bytes()[offset as usize..offset as usize + count]);
    }
    count
}
fn check_cancel(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(failure(io::ErrorKind::Interrupted.into()))
    } else {
        Ok(())
    }
}
fn failure(error: io::Error) -> FsError {
    FsError::from_io("size warning report", Path::new(""), error)
}

#[cfg(test)]
#[path = "size_warnings_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "size_warning_retirement_tests.rs"]
mod report_ownership_tests;
