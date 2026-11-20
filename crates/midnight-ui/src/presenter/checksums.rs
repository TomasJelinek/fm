//! Captured checksum sessions. Enumeration, hashing, export and clipboard I/O stay in the engine.
use super::*;

mod events;
mod output;
mod picker;
mod report;
mod verification;

pub(crate) const MAX_CHECKSUM_WINDOWS: usize = 4;
pub(crate) const OUTPUT_ALGORITHMS: [ChecksumAlgorithm; 5] = [
    ChecksumAlgorithm::Sha256,
    ChecksumAlgorithm::Sha512,
    ChecksumAlgorithm::Md5,
    ChecksumAlgorithm::Sha1,
    ChecksumAlgorithm::Crc32,
];

fn preferred_algorithm(algorithms: ChecksumAlgorithms) -> ChecksumAlgorithm {
    OUTPUT_ALGORITHMS
        .into_iter()
        .find(|&a| algorithms.contains(a))
        .unwrap_or(ChecksumAlgorithm::Sha256)
}

#[derive(Default)]
pub(crate) struct ChecksumState {
    pub sessions: HashMap<ChecksumJobId, ChecksumSession>,
    pub(super) tree: Option<tree_checksums::Pending>,
    next_job: u64,
    next_save: u64,
}

pub(crate) struct ChecksumSession {
    pub revision: u64,
    /// The captured panel's folder; only tests read it now that the window shows no details.
    #[allow(dead_code)]
    pub directory: PathBuf,
    pub files: SelectedFiles,
    pub tree_sources: Option<Arc<dyn CapturedChecksumFiles>>,
    pub generation: ChecksumGeneration,
    pub algorithms: ChecksumAlgorithms,
    pub recursive: bool,
    pub running: bool,
    pub stopping: bool,
    pub progress: ChecksumProgress,
    pub run: Option<Arc<ChecksumRun>>,
    pub error: String,
    pub save: Option<ChecksumSave>,
    /// What the running save is doing.
    pub save_message: String,
    /// The last completed save of this run: where it went and what it replaced.
    pub saved: Option<ChecksumManifestReceipt>,
    pub suggested: PathBuf,
    pub manifest_suggested: PathBuf,
    pub verification: Option<PathBuf>,
    pub verify_initial: bool,
    pub destination_revision: u64,
    pub manifest_revision: u64,
    pub picker: Option<picker::Pending>,
    /// A destination was just chosen: the window saves to it on its next refresh.
    pub save_pending: Option<ChecksumAlgorithm>,
    /// A question the user must answer before a save goes ahead.
    pub save_prompt: Option<SavePrompt>,
    streamed: Vec<Arc<ChecksumRow>>,
    /// The running calculation's listed files, in reading order; those without a
    /// row yet show as waiting. Cleared when the run finishes.
    planned: Planned,
}

/// Listed files and which of them already have a row. Rows normally arrive in
/// listing order, so the waiting files are the tail after `first`.
#[derive(Default)]
pub(crate) struct Planned {
    files: Vec<ChecksumPlannedFile>,
    done: Vec<bool>,
    by_path: std::collections::HashMap<PathBuf, usize>,
    finished: usize,
    /// No listed file before this one waits.
    first: usize,
    /// Files at or after `first` that already have a row.
    later_done: usize,
}

impl Planned {
    fn add(&mut self, files: &[ChecksumPlannedFile]) {
        use std::collections::hash_map::Entry;
        let room = ChecksumLimits::default()
            .max_rows
            .saturating_sub(self.files.len());
        for file in files.iter().take(room) {
            if let Entry::Vacant(entry) = self.by_path.entry(file.path.clone()) {
                entry.insert(self.files.len());
                self.files.push(file.clone());
                self.done.push(false);
            }
        }
    }
    fn finished(&mut self, path: &Path) {
        let Some(&index) = self.by_path.get(path) else {
            return;
        };
        if std::mem::replace(&mut self.done[index], true) {
            return;
        }
        self.finished += 1;
        if index >= self.first {
            self.later_done += 1;
        }
        while self.done.get(self.first).copied().unwrap_or(false) {
            self.first += 1;
            self.later_done -= 1;
        }
    }
    /// The `k`th waiting file, without a scan when rows came in listing order.
    fn waiting(&self, k: usize) -> Option<&ChecksumPlannedFile> {
        if self.later_done == 0 {
            return self.files.get(self.first + k);
        }
        self.files[self.first..]
            .iter()
            .zip(&self.done[self.first..])
            .filter(|(_, done)| !**done)
            .map(|(file, _)| file)
            .nth(k)
    }
}

/// Asked in the window instead of failing: partial results, or an existing file to replace.
/// It belongs to the run it asks about; a new run discards it.
pub(crate) struct SavePrompt {
    pub generation: ChecksumGeneration,
    pub algorithm: ChecksumAlgorithm,
    pub destination: PathBuf,
    /// The exact partial counts the question showed; the save sends these, never a recount.
    pub acknowledged: Option<ChecksumPartialAcknowledgement>,
    pub overwrite: bool,
    pub message: String,
}

pub(crate) struct ChecksumSave {
    pub id: ChecksumManifestId,
    pub algorithm: ChecksumAlgorithm,
    pub destination: PathBuf,
    pub acknowledged: Option<ChecksumPartialAcknowledgement>,
    pub accepted: bool,
    pub stopping: bool,
}

impl ChecksumSession {
    pub fn rows(&self) -> &[Arc<ChecksumRow>] {
        self.run.as_ref().map_or(&self.streamed, |run| &run.rows)
    }

    /// Listed files of the running calculation that have no row yet, in reading order.
    pub fn pending(&self) -> impl Iterator<Item = &ChecksumPlannedFile> {
        let planned = &self.planned;
        planned
            .files
            .iter()
            .zip(&planned.done)
            .filter(|(_, done)| !**done)
            .map(|(file, _)| file)
    }

    pub fn pending_count(&self) -> usize {
        self.planned.files.len() - self.planned.finished
    }

    /// The `k`th listed file still to come; see [`Self::pending`].
    pub fn pending_at(&self, k: usize) -> Option<&ChecksumPlannedFile> {
        self.planned.waiting(k)
    }

    /// Accept listed files and rows of the running calculation.
    pub(super) fn plan(&mut self, files: &[ChecksumPlannedFile]) {
        if self.run.is_none() {
            self.planned.add(files);
        }
    }

    pub(super) fn stream(&mut self, rows: &[Arc<ChecksumRow>]) {
        let remaining = ChecksumLimits::default()
            .max_rows
            .saturating_sub(self.streamed.len());
        for row in rows.iter().take(remaining) {
            self.planned.finished(&row.path);
            self.streamed.push(row.clone());
        }
    }

    /// The run ended or a new one starts: the old rows and listed files go.
    pub(super) fn take_streamed(&mut self) -> (Vec<Arc<ChecksumRow>>, Planned) {
        (mem::take(&mut self.streamed), mem::take(&mut self.planned))
    }

    pub fn row(&self, token: &str) -> Option<&ChecksumRow> {
        let (generation, row) = token.split_once(':')?;
        if generation.parse::<u64>().ok()? != self.generation.0 {
            return None;
        }
        let id = row.parse::<u64>().ok()?;
        // The worker emits immutable sequential IDs starting at one. Check the
        // identity too: no callback can substitute an arbitrary displayed index.
        self.rows()
            .get(usize::try_from(id.checked_sub(1)?).ok()?)
            .filter(|row| row.id.0 == id)
            .map(AsRef::as_ref)
    }

    /// A new run makes every answer about the previous one obsolete: its save
    /// question, a destination chosen for it and the report of its last save.
    fn forget_saves(&mut self) {
        self.save_prompt = None;
        self.save_pending = None;
        self.save_message.clear();
        self.saved = None;
    }

    pub fn export_counts(
        &self,
        algorithm: ChecksumAlgorithm,
    ) -> Option<ChecksumPartialAcknowledgement> {
        if self.verification.is_some() {
            return None;
        }
        let run = self.run.as_ref()?;
        if !run.algorithms.contains(algorithm) {
            return None;
        }
        // Every Calculated backend row contains every requested algorithm. The
        // summary count avoids a full result scan in a UI callback; export checks
        // its own retained rows and exact acknowledgement independently.
        let included = run.summary.progress.calculated;
        let excluded = (run.rows.len() as u64).checked_sub(included)?;
        Some(ChecksumPartialAcknowledgement {
            included,
            excluded,
            incomplete_discovery: matches!(
                run.summary.completion,
                ChecksumCompletion::Incomplete
                    | ChecksumCompletion::Cancelled
                    | ChecksumCompletion::Failed
            ),
        })
    }
}

pub(super) fn window_limit_message() -> String {
    format!("Close one of the {MAX_CHECKSUM_WINDOWS} open checksum windows first.")
}

impl ChecksumState {
    fn session_mut(&mut self, id: ChecksumJobId) -> Option<&mut ChecksumSession> {
        self.session_mut_if(id, |_| true)
    }

    /// Only an accepted event marks its session changed; a stale backend tail
    /// must not force a full checksum-window refresh.
    fn session_mut_if(
        &mut self,
        id: ChecksumJobId,
        accept: impl FnOnce(&ChecksumSession) -> bool,
    ) -> Option<&mut ChecksumSession> {
        let session = self
            .sessions
            .get_mut(&id)
            .filter(|session| accept(session))?;
        session.revision = session.revision.wrapping_add(1);
        Some(session)
    }
}

fn is_checksum_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            ChecksumAlgorithm::ALL
                .iter()
                .any(|a| extension.eq_ignore_ascii_case(a.extension()))
        })
}

/// The focused file when it is a checksum file and is selected, or nothing is.
fn focused_checksum_file(files: &SelectedFiles) -> Option<PathBuf> {
    files
        .fallback
        .as_ref()
        .filter(|path| {
            is_checksum_file(path)
                && (files.selection.is_empty() || files.selection.contains_path(path))
        })
        .cloned()
}

/// The checksum file among the captured files, found from the selection itself and never
/// by scanning the folder's index on the UI thread: the focused one, else the first
/// selected one in display order. Only as many selected rows are looked at as one
/// calculation could hold; the bitset skips unselected rows 64 at a time.
fn selected_checksum_file(pane: &Pane, files: &SelectedFiles) -> Option<PathBuf> {
    if let Some(focused) = focused_checksum_file(files) {
        return Some(focused);
    }
    let selection = &files.selection;
    if selection.is_empty() || !Arc::ptr_eq(selection.basis(), pane.display_index()) {
        return None;
    }
    selection
        .bits()
        .iter()
        .enumerate()
        .flat_map(|(word, bits)| {
            let mut remaining = *bits;
            std::iter::from_fn(move || {
                let bit = (remaining != 0).then(|| remaining.trailing_zeros() as usize)?;
                remaining &= remaining - 1;
                Some(word * 64 + bit)
            })
        })
        .take(ChecksumLimits::default().max_rows)
        .filter_map(|position| pane.display_entry(position))
        .map(|entry| &entry.path)
        .find(|path| is_checksum_file(path))
        .cloned()
}

impl Presenter {
    pub(crate) fn request_checksums(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.feed_refuses(pane, "Checksums") {
            return;
        }
        if self.panes[pane.index()].tree_view().is_some() {
            self.request_tree_checksums(pane, send);
            return;
        }
        if !self.tree_action_allowed(pane) {
            return;
        }
        if !self.panes[pane.index()].native_capabilities() {
            self.desktop.report("Checksums", "Checksums are available for native files.\nArchive member checksums are not available yet.".into());
            return;
        }
        let p = &self.panes[pane.index()];
        // Nothing selected and no row under the cursor (the parent row, an empty
        // folder): Calculate covers the shown folder's files, and its subfolders when
        // Include subfolders is on. A focused file or folder is the selection action's
        // fallback below, so Ctrl+F8 always opens with something to calculate.
        if p.selected.is_empty()
            && p.selection_revision == p.selection_resolved
            && (p.parent_cursor || p.cursor.and_then(|id| p.entry_by_id(id)).is_none())
        {
            let directory = p.display_path().to_owned();
            self.open_captured_checksums(
                pane,
                directory.clone(),
                SelectedFiles {
                    selection: Arc::new(SelectionSnapshot::default()),
                    view: Arc::new(DirectoryIndex::default()),
                    fallback: Some(directory),
                },
                send,
            );
            return;
        }
        self.request_selection_action(
            pane,
            selection_actions::SelectionIntent::Checksums {
                pane,
                directory: p.display_path().to_owned(),
            },
            send,
        );
    }

    pub(super) fn open_captured_checksums(
        &mut self,
        pane: PaneId,
        directory: PathBuf,
        files: SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<ChecksumJobId> {
        let manifest = selected_checksum_file(&self.panes[pane.index()], &files);
        self.open_checksum_session(directory, files, None, manifest, send)
    }

    pub(super) fn open_checksum_sources(
        &mut self,
        directory: PathBuf,
        files: SelectedFiles,
        tree_sources: Option<Arc<dyn CapturedChecksumFiles>>,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<ChecksumJobId> {
        // A captured tree names a checksum file only through its focused row.
        let manifest = focused_checksum_file(&files);
        self.open_checksum_session(directory, files, tree_sources, manifest, send)
    }

    fn open_checksum_session(
        &mut self,
        directory: PathBuf,
        files: SelectedFiles,
        tree_sources: Option<Arc<dyn CapturedChecksumFiles>>,
        manifest: Option<PathBuf>,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<ChecksumJobId> {
        if self.checksums.sessions.len() >= MAX_CHECKSUM_WINDOWS || self.operations.closing {
            // Shutdown refuses silently; there may be no window to close.
            if !self.operations.closing {
                self.desktop.report("Checksums", window_limit_message());
            }
            self.retired.push(Box::new((files, tree_sources)));
            return None;
        }
        let Some(id) = self.checksums.next_job.checked_add(1).map(ChecksumJobId) else {
            self.retired.push(Box::new((files, tree_sources)));
            return None;
        };
        self.checksums.next_job = id.0;
        // Every window starts with SHA-256 alone; other algorithms are a per-window choice.
        let algorithms = ChecksumAlgorithms::default();
        // A checksum file in the selection (or under the cursor) means the user wants to
        // verify it, so the window opens on Verify with that file chosen.
        let verify_initial = manifest.is_some()
            || (tree_sources.is_none() && files.selection.is_empty() && files.fallback.is_none());
        let algorithm = preferred_algorithm(algorithms);
        self.checksums.sessions.insert(
            id,
            ChecksumSession {
                revision: 0,
                suggested: directory.join(format!("checksums.{}", algorithm.extension())),
                manifest_suggested: manifest.unwrap_or_else(|| directory.join("checksums.sha256")),
                verification: None,
                verify_initial,
                destination_revision: 0,
                manifest_revision: 0,
                picker: None,
                save_pending: None,
                save_prompt: None,
                directory,
                files,
                tree_sources,
                generation: ChecksumGeneration(0),
                algorithms,
                recursive: false,
                running: false,
                stopping: false,
                progress: ChecksumProgress::default(),
                run: None,
                error: String::new(),
                save: None,
                save_message: String::new(),
                saved: None,
                streamed: Vec::new(),
                planned: Planned::default(),
            },
        );
        // Nothing is calculated until the user presses Calculate.
        let _ = send;
        Some(id)
    }

    pub(crate) fn start_checksums(
        &mut self,
        id: ChecksumJobId,
        algorithms: ChecksumAlgorithms,
        recursive: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(s) = self.checksums.session_mut(id) else {
            return false;
        };
        if s.running || s.save.is_some() || s.picker.is_some() {
            return false;
        }
        let Some(generation) = s.generation.0.checked_add(1).map(ChecksumGeneration) else {
            return false;
        };
        let command = if let Some(sources) = &s.tree_sources {
            Command::StartCapturedChecksums(Box::new(CapturedChecksumRequest {
                job: id,
                generation,
                sources: sources.clone(),
                algorithms,
                recursive,
                limits: ChecksumLimits::default(),
            }))
        } else {
            Command::StartSelectedChecksums(Box::new(SelectedChecksumRequest {
                job: id,
                generation,
                files: s.files.clone(),
                algorithms,
                recursive,
                limits: ChecksumLimits::default(),
            }))
        };
        if !send(command) {
            s.error = "Checksum queue is busy. Try Calculate again.".into();
            return false;
        }
        s.generation = generation;
        s.verification = None;
        s.algorithms = algorithms;
        if s.suggested
            .file_stem()
            .is_some_and(|name| name == "checksums")
        {
            s.suggested
                .set_extension(preferred_algorithm(algorithms).extension());
        }
        s.recursive = recursive;
        s.running = true;
        s.stopping = false;
        s.error.clear();
        s.forget_saves();
        s.progress = ChecksumProgress::default();
        self.retired.push(Box::new(s.take_streamed()));
        self.retired.push(Box::new(s.run.take()));
        true
    }

    pub(crate) fn stop_checksums(&mut self, id: ChecksumJobId, send: &dyn Fn(Command) -> bool) {
        let Some(s) = self.checksums.session_mut(id) else {
            return;
        };
        if !s.running || s.stopping {
            return;
        }
        if send(Command::CancelChecksums {
            job: id,
            generation: s.generation,
        }) {
            s.stopping = true;
        } else {
            s.error = "Cancellation queue is busy. Try Stop again.".into();
        }
    }

    pub(crate) fn close_checksums(
        &mut self,
        id: ChecksumJobId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if !self.cancel_checksum_picker(id, send) {
            return false;
        }
        let Some(s) = self.checksums.session_mut(id) else {
            return true;
        };
        if s.generation.0 > 0
            && !send(Command::ReleaseChecksums {
                job: id,
                generation: s.generation,
            })
        {
            s.error = "Request queue is busy. Try closing again.".into();
            return false;
        }
        self.retired
            .push(Box::new(self.checksums.sessions.remove(&id)));
        true
    }
}
