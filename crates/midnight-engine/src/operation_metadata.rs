//! Typed metadata previews and iterative postorder execution on the mutation worker.
use super::*;
use midnight_platform_common::metadata::{case_name, validate_patch};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    io,
    path::Path,
    time::{Instant, SystemTime},
};

pub(super) struct Edit {
    pub(super) preview: MetadataPreview,
    snapshots: Vec<MetadataSnapshot>,
    names: Vec<Option<OsString>>,
    issues: Vec<Option<FsError>>,
    pub(super) conflicts: MetadataConflictPolicy,
    pub(super) conflict_state: Arc<ConflictState>,
    pub(super) result: Arc<Mutex<Option<Arc<MetadataResult>>>>,
}

#[derive(Default)]
pub(super) struct ConflictState {
    next: AtomicU64,
    pending: Mutex<Option<(u64, std::sync::mpsc::SyncSender<MetadataConflictPolicy>)>>,
}

impl ConflictState {
    /// Whether the worker currently awaits a decision (it registers before sending).
    #[cfg(test)]
    pub(super) fn is_pending(&self) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
    pub(super) fn resolve(&self, token: u64, choice: MetadataConflictPolicy) {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((expected, sender)) = pending.as_ref()
            && *expected == token
        {
            let _ = sender.try_send(choice);
        }
    }
    fn ask(
        &self,
        id: OperationId,
        source: std::path::PathBuf,
        destination: std::path::PathBuf,
        outbox: &Outbox,
        cancel: &Cancellation,
    ) -> MetadataConflictPolicy {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((token, sender));
        let sent = outbox.blocking_send(
            Event::Operation(OperationEvent::MetadataConflict(Arc::new(
                MetadataConflict {
                    id,
                    token,
                    source,
                    destination,
                },
            ))),
            cancel,
        );
        let decision = if sent {
            loop {
                if cancel.is_cancelled() {
                    break MetadataConflictPolicy::Cancel;
                }
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(choice) => break choice,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break MetadataConflictPolicy::Cancel,
                }
            }
        } else {
            MetadataConflictPolicy::Cancel
        };
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
        decision
    }
}

fn error(path: &Path, kind: io::ErrorKind, text: &str) -> FsError {
    FsError::from_io("metadata operation", path, io::Error::new(kind, text))
}

impl Edit {
    pub(super) fn prepare(
        id: OperationId,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::with_snapshots(id, 0, MetadataPatch::default(), targets, cancel)
    }

    /// Recursive execution: a freshly pinned child is read once and the confirmed patch
    /// is applied directly, without a preview refresh.
    pub(super) fn prepare_confirmed(
        id: OperationId,
        revision: u64,
        patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        Self::with_snapshots(id, revision, patch, targets, cancel)
    }

    fn with_snapshots(
        id: OperationId,
        revision: u64,
        patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        let mut snapshots = Vec::with_capacity(targets.len());
        for target in targets {
            snapshots.push(target.metadata_snapshot(cancel)?);
        }
        let mut edit = Self {
            preview: MetadataPreview {
                id,
                revision: 0,
                patch: MetadataPatch::default(),
                localized_times: Default::default(),
                time_input_format: None,
                capabilities: MetadataCapabilities::default(),
                total: targets.len() as u64,
                conflicts: 0,
                unsupported: 0,
                hard_linked: 0,
                sample: vec![],
                errors: vec![],
            },
            snapshots,
            names: vec![],
            issues: vec![],
            conflicts: MetadataConflictPolicy::Skip,
            conflict_state: Arc::default(),
            result: Arc::default(),
        };
        edit.apply(revision, patch, targets, cancel, false);
        Ok(edit)
    }

    /// Native locale discovery and formatting stay on this preparation worker.
    pub(super) fn localize(&mut self, locale: Option<&dyn LocaleService>) {
        self.preview.localized_times.clear();
        self.preview.time_input_format = locale.and_then(LocaleService::datetime_input_format);
        let Some(locale) = locale else {
            return;
        };
        let times = self
            .preview
            .sample
            .iter()
            .flat_map(|row| {
                [&row.current, &row.proposed]
                    .into_iter()
                    .flat_map(|snapshot| {
                        [snapshot.created, snapshot.modified, snapshot.accessed]
                            .into_iter()
                            .flatten()
                    })
            })
            .chain(
                [
                    self.preview.patch.created,
                    self.preview.patch.modified,
                    self.preview.patch.accessed,
                ]
                .into_iter()
                .filter_map(|edit| match edit {
                    TimeEdit::Set(value) => Some(value),
                    _ => None,
                }),
            );
        for value in times {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.preview.localized_times.entry(value)
                && let Some(text) = locale.format_datetime(value)
            {
                entry.insert(text);
            }
        }
    }

    pub(super) fn update(
        &mut self,
        revision: u64,
        patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
    ) {
        self.apply(revision, patch, targets, cancel, revision != 0);
    }

    fn apply(
        &mut self,
        revision: u64,
        mut patch: MetadataPatch,
        targets: &[Arc<dyn PreparedMutation>],
        cancel: &Cancellation,
        refresh: bool,
    ) {
        patch.resolve_now(SystemTime::now());
        self.preview.revision = revision;
        self.preview.patch = patch.clone();
        self.preview.conflicts = 0;
        self.preview.unsupported = 0;
        self.preview.hard_linked = 0;
        self.preview.errors.clear();
        self.preview.sample.clear();
        self.names.clear();
        self.issues.clear();
        // Re-read the pinned targets for every explicit refresh. Native adapters
        // revalidate identity; failed reads must not publish cached values as current.
        let snapshot_errors = targets
            .iter()
            .zip(&mut self.snapshots)
            .map(|(target, snapshot)| {
                if !refresh {
                    return None;
                }
                match target.metadata_snapshot(cancel) {
                    Ok(current) => {
                        *snapshot = current;
                        None
                    }
                    Err(error) => {
                        *snapshot = MetadataSnapshot::default();
                        Some(error)
                    }
                }
            })
            .collect::<Vec<_>>();
        let mut caps = self
            .snapshots
            .first()
            .map(|s| s.capabilities)
            .unwrap_or_default();
        let mut destinations = HashMap::<std::path::PathBuf, usize>::new();
        for (index, (target, snapshot)) in targets.iter().zip(&self.snapshots).enumerate() {
            let path = &target.item().path;
            let c = snapshot.capabilities;
            caps.windows_attributes &= c.windows_attributes;
            caps.unix_permissions &= c.unix_permissions;
            caps.compressed &= c.compressed;
            caps.created &= c.created;
            caps.modified &= c.modified;
            caps.accessed &= c.accessed;
            caps.rename &= c.rename;
            self.preview.hard_linked += u64::from(snapshot.hard_links > 1);
            let mut issue = snapshot_errors[index]
                .clone()
                .or_else(|| validate_patch(&patch, c, path).err());
            let mut name = None;
            if cancel.is_cancelled() {
                issue = Some(error(path, io::ErrorKind::Interrupted, "preview cancelled"));
            }
            if issue.is_none()
                && let Some(change) = patch.case
            {
                match case_name(path.file_name().unwrap_or_default(), change) {
                    Ok(proposed) => {
                        issue = target.metadata_name_available(&proposed, cancel).err();
                        let destination = path.with_file_name(&proposed);
                        if let Some(previous) = destinations.insert(destination, index) {
                            let conflict = error(
                                path,
                                io::ErrorKind::AlreadyExists,
                                "multiple selected names produce the same destination",
                            );
                            self.issues[previous] = Some(conflict.clone());
                            issue = Some(conflict);
                        }
                        name = Some(proposed);
                    }
                    Err(error) => issue = Some(error),
                }
            }
            self.names.push(name);
            self.issues.push(issue);
        }
        self.preview.capabilities = caps;
        for (index, target) in targets.iter().enumerate() {
            let issue = self.issues[index].clone();
            if let Some(error) = &issue {
                if error.kind == io::ErrorKind::AlreadyExists {
                    self.preview.conflicts += 1;
                } else {
                    self.preview.unsupported += 1;
                }
                if self.preview.errors.len() < 16 {
                    self.preview.errors.push(error.clone());
                }
            }
            if self.preview.sample.len() < MAX_METADATA_TARGETS {
                self.preview.sample.push(MetadataPreviewRow {
                    path: target.item().path.clone(),
                    proposed_name: self.names[index].clone(),
                    current: self.snapshots[index].clone(),
                    proposed: proposed_metadata(&self.snapshots[index], &patch),
                    issue,
                });
            }
        }
    }
}

fn proposed_metadata(current: &MetadataSnapshot, patch: &MetadataPatch) -> MetadataSnapshot {
    let mut proposed = current.clone();
    if let Some(value) = proposed.windows_attributes {
        proposed.windows_attributes =
            Some((value & !patch.windows_mask) | (patch.windows_value & patch.windows_mask));
    }
    if let Some(value) = proposed.unix_mode {
        proposed.unix_mode =
            Some((value & !patch.unix_mask) | (patch.unix_value & patch.unix_mask));
    }
    if let Some(value) = patch.compressed {
        proposed.compressed = Some(value);
    }
    for (value, edit) in [
        (&mut proposed.created, patch.created),
        (&mut proposed.modified, patch.modified),
        (&mut proposed.accessed, patch.accessed),
    ] {
        if let TimeEdit::Set(time) = edit {
            *value = Some(time);
        }
    }
    proposed
}

/// One entry of the current traversal path. A listed folder keeps its name snapshot until
/// its last child is done; its children are pinned one at a time from that snapshot.
struct Frame {
    target: Arc<dyn PreparedMutation>,
    depth: usize,
    issue: Option<FsError>,
    name: Option<OsString>,
    visited: bool,
    children: Option<Listing>,
}

/// A listed folder's captured child names and how far its traversal got.
struct Listing {
    children: Box<dyn MetadataChildren>,
    /// Captured siblings whose case change is a conflict; empty without a case change.
    collides: Vec<bool>,
    /// The captured index pinned next.
    next: usize,
    /// What this snapshot still holds of the traversal's name budget.
    names: usize,
    bytes: usize,
}

/// What the name snapshots of the folders on one root's traversal path may still hold.
/// A folder's names are charged when it is listed. Each name's bytes return as soon as
/// its entry is pinned, which releases the name, and the rest returns once the folder is
/// done, when its snapshot and the slots of its released names are dropped. So the bytes
/// bound the names not processed yet, while the count covers every name the snapshots on
/// the current path were captured with.
///
/// Retained memory along one path is therefore at most 100,000 name slots of 24 bytes and
/// conflict flags of one byte, 4 MiB of unprocessed names held without spare capacity, and
/// one allocation per unprocessed name (allocator minimum and header, up to about 32 bytes
/// each): about 10 MiB. Listing a folder for a case change also builds, only until its
/// first entry is pinned, a set of its names and a map of its changing names' new
/// spellings (case mapping at most triples a name's bytes): at most about 25 MiB for
/// 100,000 names of 4 MiB, typically a few MiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NameBudget {
    names: usize,
    bytes: usize,
}

impl NameBudget {
    const FULL: Self = Self {
        names: MAX_METADATA_NAMES,
        bytes: MAX_METADATA_NAMES_BYTES,
    };

    /// Charges a folder's snapshot. One larger than the adapter was allowed to return
    /// breaks the `metadata_children` contract; it is refused rather than held.
    fn take(
        &mut self,
        path: &Path,
        children: Box<dyn MetadataChildren>,
        case: Option<CaseChange>,
    ) -> Result<Listing, FsError> {
        let names = children.names().len();
        let bytes = children
            .names()
            .iter()
            .fold(0usize, |sum, name| sum.saturating_add(name.len()));
        if names > self.names || bytes > self.bytes {
            return Err(error(
                path,
                io::ErrorKind::Other,
                "metadata traversal exceeds its name budget",
            ));
        }
        self.names -= names;
        self.bytes -= bytes;
        let collides = sibling_collisions(children.names(), case);
        Ok(Listing {
            children,
            collides,
            next: 0,
            names,
            bytes,
        })
    }

    /// Returns the bytes of a name the snapshot released when its entry was pinned.
    fn release(&mut self, listing: &mut Listing, bytes: usize) {
        let bytes = bytes.min(listing.bytes);
        listing.bytes -= bytes;
        self.bytes += bytes;
    }

    /// Returns the rest of a done or stopped folder's charge. Dropping the listing closes
    /// its directory handle.
    fn give_back(&mut self, listing: Listing) {
        self.names += listing.names;
        self.bytes += listing.bytes;
    }
}

/// Captured siblings whose case change cannot complete: two changing names that map to the
/// same new name, or a changing name whose new name another captured sibling has now. A
/// sibling whose own name stays is never marked and keeps its other changes. Found before
/// any sibling is renamed, so these conflicts do not depend on the processing order; a
/// sibling that later fails validation still counts. Each child's own probe, made when it
/// is processed, sees entries that other programs create and the renames of earlier
/// siblings, so it can depend on that order: Mixed titlecasing is not idempotent. Only the
/// no-replace rename is authoritative, which keeps every order safe.
fn sibling_collisions(names: &[OsString], case: Option<CaseChange>) -> Vec<bool> {
    let Some(change) = case else {
        return Vec::new();
    };
    let current: HashSet<&OsStr> = names.iter().map(OsString::as_os_str).collect();
    let mut first = HashMap::<OsString, usize>::new();
    let mut collides = vec![false; names.len()];
    for (index, name) in names.iter().enumerate() {
        let Ok(proposed) = case_name(name, change) else {
            continue;
        };
        if proposed == *name {
            continue;
        }
        if current.contains(proposed.as_os_str()) {
            collides[index] = true;
        }
        match first.entry(proposed) {
            std::collections::hash_map::Entry::Occupied(earlier) => {
                collides[*earlier.get()] = true;
                collides[index] = true;
            }
            std::collections::hash_map::Entry::Vacant(free) => {
                free.insert(index);
            }
        }
    }
    collides
}

/// A count with thousands separators, as in "100,000".
fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// A byte count in binary units, as in "4 MiB" or "1.5 KiB".
fn binary_size(bytes: usize) -> String {
    const KIB: usize = 1024;
    let (unit, scale) = match bytes {
        bytes if bytes >= KIB * KIB => ("MiB", KIB * KIB),
        bytes if bytes >= KIB => ("KiB", KIB),
        bytes => return format!("{bytes} bytes"),
    };
    if bytes.is_multiple_of(scale) {
        format!("{} {unit}", bytes / scale)
    } else {
        format!("{:.1} {unit}", bytes as f64 / scale as f64)
    }
}

/// Names why a folder's names could not be held, then keeps the adapter's own text: an
/// adapter can report another failure with the same `ErrorKind::Other` as its refusal.
fn name_shortfall(path: &Path, free: NameBudget, refusal: &FsError) -> FsError {
    let limit = format!(
        "at most {} names or {} of names",
        grouped(MAX_METADATA_NAMES),
        binary_size(MAX_METADATA_NAMES_BYTES)
    );
    let cause = if free == NameBudget::FULL {
        format!("this folder's names exceed what one recursive edit can hold ({limit})")
    } else {
        format!(
            "this folder's names exceed the {} names or {} of names still free; one \
             recursive edit holds {limit} at once, and the folders above it that are still \
             being processed hold the rest",
            grouped(free.names),
            binary_size(free.bytes)
        )
    };
    error(
        path,
        io::ErrorKind::Other,
        &format!(
            "{cause}; this folder and its contents were not changed. The file system \
             reported: {}",
            refusal.message
        ),
    )
}

/// Lists a folder once for recursive traversal, within what is left of the name budget.
fn list(
    frame: &Frame,
    cancel: &Cancellation,
    budget: &mut NameBudget,
    case: Option<CaseChange>,
) -> Result<Listing, FsError> {
    let path = &frame.target.item().path;
    if frame.depth >= MAX_METADATA_DEPTH {
        // Checked first: a folder beyond the depth budget is never listed.
        return Err(error(
            path,
            io::ErrorKind::Other,
            "metadata traversal exceeds its depth budget",
        ));
    }
    match frame
        .target
        .metadata_children(cancel, budget.names, budget.bytes)
    {
        Ok(children) => budget.take(path, children, case),
        Err(refusal) if refusal.kind == io::ErrorKind::Other => {
            Err(name_shortfall(path, *budget, &refusal))
        }
        Err(error) => Err(error),
    }
}

/// A child failure that may mean its folder changed: conflicts, cancellation and fields an
/// entry does not support concern only that entry.
fn may_mean_folder_changed(error: &FsError) -> bool {
    !matches!(
        error.kind,
        io::ErrorKind::AlreadyExists | io::ErrorKind::Interrupted | io::ErrorKind::Unsupported
    )
}

/// After one of a listed folder's entries failed, revalidates the folder once. A folder
/// that another program moved, replaced or removed would fail each remaining entry the same
/// way, so it stops with one error that counts them; the outcomes of processed entries stay.
fn stop_if_changed(folder: &mut Frame, cancel: &Cancellation, budget: &mut NameBudget) {
    let Some(listing) = &folder.children else {
        return;
    };
    let total = listing.children.names().len();
    let remaining = total - listing.next;
    // The folder's own edit revalidates it after its last entry.
    if remaining == 0 {
        return;
    }
    let Err(failure) = folder.target.metadata_snapshot(cancel) else {
        return;
    };
    if failure.kind == io::ErrorKind::Interrupted {
        return;
    }
    if let Some(listing) = folder.children.take() {
        budget.give_back(listing);
    }
    let kind = match failure.kind {
        io::ErrorKind::AlreadyExists => io::ErrorKind::Other,
        kind => kind,
    };
    folder.issue = Some(error(
        &folder.target.item().path,
        kind,
        &format!(
            "this folder changed while its entries were processed, so {} of its {} entries \
             and the folder itself were not changed. The file system reported: {}",
            grouped(remaining),
            grouped(total),
            failure.message
        ),
    ));
}

/// What pinning a listed folder's next entry gave: a frame to process, or the outcome of an
/// entry that could not be pinned and fails alone.
enum Pinned {
    Child(Frame),
    Failed(MetadataOutcome),
}

/// Pins a listed folder's next captured entry and plans its edit: the issue and new name
/// that `Edit::prepare_confirmed` finds for that entry alone, then the sibling conflicts
/// found on the captured names.
fn pin_next(
    folder: &mut Frame,
    budget: &mut NameBudget,
    plan: (OperationId, &MetadataPreview),
    cancel: &Cancellation,
) -> Option<Pinned> {
    let listing = folder.children.as_mut()?;
    let index = listing.next;
    let bytes = listing.children.names().get(index)?.len();
    listing.next += 1;
    let target = match listing.children.pin(index, cancel) {
        Ok(target) => target,
        Err(error) => {
            // A failed pin keeps its captured name.
            let name = listing.children.names().get(index).cloned();
            let path = &folder.target.item().path;
            let mut outcome = MetadataOutcome::unchanged(path.join(name.unwrap_or_default()));
            outcome.error = Some(error);
            return Some(Pinned::Failed(outcome));
        }
    };
    budget.release(listing, bytes);
    let (id, preview) = plan;
    let (mut issue, name) = match Edit::prepare_confirmed(
        id,
        preview.revision,
        preview.patch.clone(),
        std::slice::from_ref(&target),
        cancel,
    ) {
        Ok(planned) => (
            planned.issues.into_iter().next().flatten(),
            planned.names.into_iter().next().flatten(),
        ),
        Err(error) => (Some(error), None),
    };
    if listing.collides.get(index) == Some(&true)
        && issue
            .as_ref()
            .is_none_or(|issue| issue.kind == io::ErrorKind::AlreadyExists)
    {
        issue = Some(error(
            &target.item().path,
            io::ErrorKind::AlreadyExists,
            "another entry of this folder has or would get the same name",
        ));
    }
    Some(Pinned::Child(Frame {
        target,
        depth: folder.depth + 1,
        issue,
        name,
        visited: false,
        children: None,
    }))
}

/// One metadata job's report: item outcomes, conflict decisions and throttled progress.
struct Tally<'a> {
    edit: &'a Edit,
    outbox: &'a Outbox,
    cancel: &'a Cancellation,
    status: &'a Status,
    root_count: usize,
    progress: OperationProgress,
    result: MetadataResult,
    policy: MetadataConflictPolicy,
    last: Instant,
}

impl Tally<'_> {
    /// Records one entry's outcome; a name conflict applies, or asks for, a decision.
    fn record(&mut self, outcome: MetadataOutcome, depth: usize, name: Option<&OsStr>) {
        self.result.processed += 1;
        self.result.changed += u64::from(outcome.changes.any());
        if depth == 0 && self.root_count == 1 && outcome.changes.renamed {
            self.progress.renamed_to = Some(outcome.current_path.clone());
        }
        self.progress.current_path = Some(outcome.current_path.clone());
        if outcome.changes.any() {
            self.progress.processed_entries += 1;
        }
        if let Some(error) = &outcome.error {
            if error.kind == io::ErrorKind::AlreadyExists {
                let choice = if self.policy == MetadataConflictPolicy::Skip {
                    let destination = name.map_or_else(
                        || outcome.current_path.clone(),
                        |name| outcome.original_path.with_file_name(name),
                    );
                    self.edit.conflict_state.ask(
                        self.progress.id,
                        outcome.original_path.clone(),
                        destination,
                        self.outbox,
                        self.cancel,
                    )
                } else {
                    self.policy
                };
                match choice {
                    MetadataConflictPolicy::Cancel => self.cancel.cancel(),
                    MetadataConflictPolicy::Skip => self.result.skipped += 1,
                    MetadataConflictPolicy::SkipAll => {
                        self.result.skipped += 1;
                        self.policy = MetadataConflictPolicy::SkipAll;
                    }
                }
            } else if error.kind != io::ErrorKind::Interrupted {
                self.result.failed += 1;
            }
            if self.progress.errors.len() < 16 {
                self.progress.errors.push(error.clone());
            }
        }
        if self.result.sample.len() < 32 {
            self.result.sample.push(outcome);
        }
        if self.last.elapsed() >= Duration::from_millis(250) {
            *self.status.lock().unwrap_or_else(|e| e.into_inner()) = self.progress.clone();
            if self
                .outbox
                .sender
                .try_send(Event::Operation(OperationEvent::Progress(Arc::new(
                    self.progress.clone(),
                ))))
                .is_ok()
            {
                (self.outbox.wakeup)();
            }
            self.last = Instant::now();
        }
    }
}

/// Edits one root and, for a recursive patch, every entry below it in postorder. A folder
/// is listed once and its entries are pinned one at a time, only when they are processed,
/// so at most one pinned child per listed level is alive: at most `MAX_METADATA_DEPTH`
/// traversal targets, besides the selected roots of the shared budget. The name snapshots
/// on the current path share one `NameBudget`.
fn traverse(
    tally: &mut Tally,
    root: Arc<dyn PreparedMutation>,
    issue: Option<FsError>,
    name: Option<OsString>,
) {
    let edit = tally.edit;
    let cancel = tally.cancel;
    let patch = &edit.preview.patch;
    let plan = (tally.progress.id, &edit.preview);
    let mut budget = NameBudget::FULL;
    let mut stack = vec![Frame {
        target: root,
        depth: 0,
        issue,
        name,
        visited: false,
        children: None,
    }];
    while let Some(mut frame) = stack.pop() {
        if cancel.is_cancelled() {
            break;
        }
        if !frame.visited {
            frame.visited = true;
            if frame.issue.is_none()
                && patch.recursive
                && frame.target.item().kind == EntryKind::Directory
            {
                match list(&frame, cancel, &mut budget, patch.case) {
                    Ok(listing) => frame.children = Some(listing),
                    Err(issue) => frame.issue = Some(issue),
                }
            }
        }
        match pin_next(&mut frame, &mut budget, plan, cancel) {
            Some(Pinned::Child(child)) => {
                stack.push(frame);
                stack.push(child);
                // One pinned child per listed level, and a folder at the depth budget is
                // never listed: at most MAX_METADATA_DEPTH traversal targets. Only the job
                // that holds every mutation-gate share executes.
                debug_assert!(stack.len() <= MAX_METADATA_DEPTH + 1);
                continue;
            }
            Some(Pinned::Failed(outcome)) => {
                let changed = outcome.error.as_ref().is_some_and(may_mean_folder_changed);
                tally.record(outcome, frame.depth + 1, None);
                if changed {
                    stop_if_changed(&mut frame, cancel, &mut budget);
                }
                stack.push(frame);
                continue;
            }
            None => {}
        }
        // The folder is done: its snapshot and directory handle close, like every pinned
        // child before, ahead of its own edit (Windows refuses to rename a folder while a
        // handle below it is open).
        if let Some(listing) = frame.children.take() {
            budget.give_back(listing);
        }
        let outcome = match frame.issue.take() {
            Some(issue) => {
                let mut outcome = MetadataOutcome::unchanged(frame.target.item().path.clone());
                outcome.error = Some(issue);
                outcome
            }
            None => frame
                .target
                .edit_metadata(patch, frame.name.as_deref(), cancel),
        };
        let changed = outcome.error.as_ref().is_some_and(may_mean_folder_changed);
        tally.record(outcome, frame.depth, frame.name.as_deref());
        drop(frame);
        if changed && let Some(folder) = stack.last_mut() {
            stop_if_changed(folder, cancel, &mut budget);
        }
    }
}

pub(super) fn execute(mut job: worker::Job, outbox: Outbox) -> OperationProgress {
    let edit = job.metadata.as_ref().expect("metadata job");
    let patch = &edit.preview.patch;
    let mut progress = job.progress();
    if patch.recursive {
        progress.total = progress.total.saturating_sub(
            job.metadata_covered
                .iter()
                .filter(|covered| **covered)
                .count() as u64,
        );
    }
    // Completed descendant pins must close before a selected ancestor is renamed.
    // Windows can refuse parent rename while a completed child still has open handles.
    // The retained capture stays in job until every root/descendant pin has closed.
    let root_count = job.targets.len();
    let roots = std::mem::take(&mut job.targets);
    let mut tally = Tally {
        edit,
        outbox: &outbox,
        cancel: &job.cancel,
        status: &job.status,
        root_count,
        result: MetadataResult {
            id: progress.id,
            processed: 0,
            changed: 0,
            skipped: 0,
            failed: 0,
            sample: vec![],
        },
        progress,
        policy: edit.conflicts,
        last: Instant::now(),
    };
    for (root_index, root) in roots.into_iter().enumerate() {
        // Typed occurrence ancestry, captured before preview, handles selected overlaps.
        // The covering root walks this subtree once; nonrecursive edits keep every root.
        if patch.recursive && job.metadata_covered.get(root_index) == Some(&true) {
            continue;
        }
        if job.cancel.is_cancelled() {
            break;
        }
        let before_failures = tally.result.failed;
        traverse(
            &mut tally,
            root,
            edit.issues[root_index].clone(),
            edit.names[root_index].clone(),
        );
        if tally.result.failed > before_failures {
            tally.progress.failed += 1;
        } else if !job.cancel.is_cancelled() {
            tally.progress.completed += 1;
        }
    }
    let Tally {
        mut progress,
        result,
        ..
    } = tally;
    progress.state = if job.cancel.is_cancelled() {
        OperationState::Cancelled
    } else if result.failed > 0 || result.skipped > 0 {
        OperationState::CompletedWithErrors
    } else {
        OperationState::Completed
    };
    progress.message = format!(
        "{} examined; {} changed; {} skipped; {} failed. Completed changes are retained.",
        result.processed, result.changed, result.skipped, result.failed
    );
    // Terminal delivery belongs to the async controller, after this worker releases native
    // handles and its mutation permit. Cancellation must retain all completed item outcomes.
    *edit.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(result));
    if job.cancel.is_cancelled() {
        progress.state = OperationState::Cancelled;
    }
    progress
}
