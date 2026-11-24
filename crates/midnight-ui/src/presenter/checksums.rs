//! Captured checksum sessions. Enumeration, hashing, export and clipboard I/O stay in the engine.
use super::*;
use std::hash::BuildHasher as _;

mod events;
mod output;
mod picker;
mod report;
pub(crate) mod selection;
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
    /// The captured panel's folder; Copy as a checksum file names files relative to it
    /// when the suggested checksum file has no folder.
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
    /// What the last Copy of the checksums as a checksum file left out.
    pub copy_note: Option<CopyNote>,
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
    /// The running run's rows, as the chunks the engine sent; the finished run holds the
    /// same chunks.
    streamed: ChecksumRows,
    /// The running calculation's listed files, in reading order; those without a
    /// row yet show as waiting. Cleared when the run finishes.
    planned: Planned,
    /// The result list's selection, cursor and anchor (see `selection`).
    pub selection: crate::list_selection::ListSelection,
    /// The previous run's selected files, while a run of the same files lists and reads
    /// them again (see `selection`).
    carried: Option<selection::Carried>,
    /// The copy the clipboard worker builds and writes now (see output).
    pub(crate) copy: Option<output::PendingCopy>,
    /// Which rows a copy can take, by slot (see selection).
    copyable: selection::Copyable,
}

/// How far from the reading position a row's file is looked for among the listed ones
/// before the listing is searched by name. Rows come in listing order, apart from rows
/// of entries the listing leaves out (an unreadable folder, a link): those match no
/// listed file. A file listed but gone by the time it is read never gets a row, nor do
/// the files of a folder that is unreadable or gone by then. A listed file this far
/// behind the reading position no longer waits: the reading pass does not go back.
const LISTED_LOOKAHEAD: usize = 64;

/// Listed files, as the engine sent them, and which of them already have a row (or no
/// longer wait for one). Rows normally arrive in listing order, so the waiting files are
/// a few near the reading position, then every file after it.
#[derive(Default)]
pub(crate) struct Planned {
    chunks: Vec<Arc<ChecksumPlanned>>,
    /// The place of each chunk's first file.
    starts: Vec<usize>,
    len: usize,
    done: Vec<bool>,
    finished: usize,
    /// No listed file before this one waits.
    first: usize,
    /// One past the furthest listed file with a row: every file from here on waits.
    /// `first <= next` and `next - first <= LISTED_LOOKAHEAD`, unless all are done.
    next: usize,
    /// Files in `first..next` that no longer wait.
    later_done: usize,
    /// The waiting files' places by the hash of their names, built when a row is not
    /// found near the reading position (a gap in the listing, or a row for an entry the
    /// listing left out); then every later row costs one lookup, never a scan.
    by_name: Option<HashMap<u64, usize>>,
    hasher: std::hash::RandomState,
}

impl Planned {
    /// Accept listed files; returns the places they take.
    fn add(&mut self, planned: &Arc<ChecksumPlanned>) -> std::ops::Range<usize> {
        let start = self.len;
        if !planned.files.is_empty() {
            self.starts.push(start);
            self.len += planned.files.len();
            self.chunks.push(planned.clone());
            self.done.resize(self.len, false);
            if let Some(by_name) = self.by_name.as_mut() {
                for (offset, file) in planned.files.iter().enumerate() {
                    by_name
                        .entry(self.hasher.hash_one(&*file.name))
                        .or_insert(start + offset);
                }
            }
        }
        start..self.len
    }
    pub(crate) fn file(&self, index: usize) -> Option<&ChecksumPlannedFile> {
        if index >= self.len {
            return None;
        }
        let chunk = self.starts.partition_point(|start| *start <= index) - 1;
        self.chunks[chunk].files.get(index - self.starts[chunk])
    }
    /// The listing's base folder (every name is below it).
    pub(crate) fn base(&self) -> Option<&Path> {
        self.chunks.first().map(|chunk| chunk.base.as_ref())
    }
    fn waits_as(&self, index: usize, name: &Path) -> bool {
        !self.done[index] && self.file(index).is_some_and(|file| *file.name == *name)
    }
    /// The waiting file named `name`: near the reading position, else by name.
    fn find(&mut self, name: &Path) -> Option<usize> {
        let near = self.first.max(self.next.saturating_sub(LISTED_LOOKAHEAD))
            ..self.len.min(self.next.saturating_add(LISTED_LOOKAHEAD));
        if let Some(index) = near.into_iter().find(|index| self.waits_as(*index, name)) {
            return Some(index);
        }
        let hash = self.hasher.hash_one(name);
        if self.by_name.is_none() {
            let mut by_name = HashMap::with_capacity(self.len - self.first);
            for index in (self.first..self.len).filter(|index| !self.done[*index]) {
                if let Some(file) = self.file(index) {
                    by_name
                        .entry(self.hasher.hash_one(&*file.name))
                        .or_insert(index);
                }
            }
            self.by_name = Some(by_name);
        }
        // A hash shared by two names finds only the first; the other is then
        // matched near the reading position, or waits until the run ends.
        let index = *self.by_name.as_ref()?.get(&hash)?;
        self.waits_as(index, name).then_some(index)
    }
    /// A row for `name` arrived: the place of the listed file it finishes, if any.
    /// `abandoned` is told the place of each listed file that now waits no longer
    /// because reading went on too far past it.
    fn finished(&mut self, name: &Path, abandoned: &mut dyn FnMut(usize)) -> Option<usize> {
        let index = self.find(name)?;
        self.done[index] = true;
        self.finished += 1;
        // Files skipped on the way to this one still wait, before the new `next`.
        self.later_done += 1;
        self.next = self.next.max(index + 1);
        while self.first < self.next
            && (self.done[self.first] || self.first + LISTED_LOOKAHEAD < self.next)
        {
            if self.done[self.first] {
                self.later_done -= 1;
            } else {
                self.done[self.first] = true;
                self.finished += 1;
                abandoned(self.first);
            }
            self.first += 1;
        }
        Some(index)
    }
    pub(crate) fn waiting_count(&self) -> usize {
        self.len - self.finished
    }
    /// Waiting files before the reading position.
    fn waiting_before_next(&self) -> usize {
        self.next - self.first - self.later_done
    }
    /// The place of the `k`th waiting file, scanning at most the few files before the
    /// reading position.
    pub(crate) fn waiting_index(&self, k: usize) -> Option<usize> {
        let before = self.waiting_before_next();
        if k < before {
            return (self.first..self.next)
                .filter(|index| !self.done[*index])
                .nth(k);
        }
        let index = self.next.checked_add(k - before)?;
        (index < self.len).then_some(index)
    }
    /// Which waiting file the listed file at `index` is; none once it has a row.
    pub(crate) fn waiting_position(&self, index: usize) -> Option<usize> {
        if index >= self.len || self.done[index] || index < self.first {
            return None;
        }
        if index >= self.next {
            return Some(self.waiting_before_next() + (index - self.next));
        }
        Some(
            self.done[self.first..index]
                .iter()
                .filter(|done| !**done)
                .count(),
        )
    }
    /// The places of the waiting files from the `k`th on, in order.
    pub(crate) fn waiting_from(&self, k: usize) -> impl Iterator<Item = usize> + '_ {
        let start = self.waiting_index(k).unwrap_or(self.len);
        (start..self.len).filter(|index| !self.done[*index])
    }
}

/// What a copy of selected rows did when it had to leave some out: how many lines (or,
/// for full rows, rows) it copied, and each reason a row added nothing with how many
/// did not, in the order first met. The window shows it as a note, not an error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CopyNote {
    pub copied: u64,
    pub left_out: Vec<(&'static str, u64)>,
    /// Full rows were copied, not checksums.
    pub rows: bool,
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
    /// The rows so far: the running run's streamed chunks, or the finished run's.
    pub fn rows(&self) -> &ChecksumRows {
        self.run.as_ref().map_or(&self.streamed, |run| &run.rows)
    }

    /// Listed files of the running calculation that have no row yet, in reading order.
    #[cfg(test)]
    pub fn pending(&self) -> impl Iterator<Item = &ChecksumPlannedFile> {
        self.planned
            .waiting_from(0)
            .filter_map(|index| self.planned.file(index))
    }

    pub fn pending_count(&self) -> usize {
        self.planned.waiting_count()
    }

    /// The `k`th listed file still to come, with its place in the listing (its identity
    /// while it waits); see [`Self::pending`].
    pub fn pending_at(&self, k: usize) -> Option<(usize, &ChecksumPlannedFile)> {
        let index = self.planned.waiting_index(k)?;
        Some((index, self.planned.file(index)?))
    }

    /// The folder the listed files are named below.
    pub fn pending_base(&self) -> Option<&Path> {
        self.planned.base()
    }

    /// Accept listed files of the running calculation.
    pub(super) fn plan(&mut self, planned: &Arc<ChecksumPlanned>) {
        if self.run.is_none() {
            let added = self.planned.add(planned);
            self.select_carried_listed(added);
        }
    }

    /// Accept rows of the running run; a listed file's row takes its place.
    pub(super) fn stream(&mut self, chunk: &Arc<ChecksumChunk>) {
        for row in chunk.rows.iter() {
            let marks = &mut self.selection;
            let listed = self.planned.finished(&row.name, &mut |place| {
                // As when the run ends: a file that gets no row leaves the selection.
                if let Some(slot) = selection::listed_slot(place) {
                    marks.set(slot, false);
                }
            });
            self.row_arrived(&chunk.base, row, listed);
        }
        self.streamed.push(chunk.clone());
    }

    /// The run ended or a new one starts: the old rows and listed files go.
    pub(super) fn take_streamed(&mut self) -> (ChecksumRows, Planned) {
        (mem::take(&mut self.streamed), mem::take(&mut self.planned))
    }

    /// A new run makes every answer about the previous one obsolete: its save
    /// question, a destination chosen for it, the report of its last save and the
    /// note of its last copy.
    fn forget_saves(&mut self) {
        self.save_prompt = None;
        self.save_pending = None;
        self.save_message.clear();
        self.saved = None;
        self.copy_note = None;
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

/// A checksum file is a regular file named with a checksum format's extension. A
/// folder (or a link) is never one, whatever its name, e.g. `photos.sfv`.
fn is_checksum_file(path: &Path, kind: EntryKind) -> bool {
    kind == EntryKind::File
        && path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                ChecksumAlgorithm::ALL
                    .iter()
                    .any(|a| extension.eq_ignore_ascii_case(a.extension()))
            })
}

/// The focused file when it is a checksum file and is selected, or nothing is. `kind`
/// is the focused entry's kind, when known.
fn focused_checksum_file(files: &SelectedFiles, kind: Option<EntryKind>) -> Option<PathBuf> {
    files
        .fallback
        .as_ref()
        .filter(|path| {
            kind.is_some_and(|kind| is_checksum_file(path, kind))
                && (files.selection.is_empty() || files.selection.contains_path(path))
        })
        .cloned()
}

/// The kind of the panel row the captured files name at `path`, found through the
/// captured view without a scan; nothing when the row is gone or is another one now.
fn captured_kind(pane: &Pane, files: &SelectedFiles, path: &Path) -> Option<EntryKind> {
    let id = *files.view.by_path.get(path)?;
    pane.entry_by_id(id)
        .filter(|entry| entry.path == path)
        .map(|entry| entry.kind)
}

/// The checksum file among the captured files, found from the selection itself and never
/// by scanning the folder's index on the UI thread: the focused one, else the first
/// selected one in display order. The bitset skips unselected rows 64 at a time.
fn selected_checksum_file(pane: &Pane, files: &SelectedFiles) -> Option<PathBuf> {
    let focused = files
        .fallback
        .as_deref()
        .and_then(|path| captured_kind(pane, files, path));
    if let Some(focused) = focused_checksum_file(files, focused) {
        return Some(focused);
    }
    let selection = &files.selection;
    if selection.is_empty() || !Arc::ptr_eq(selection.basis(), pane.display_index()) {
        return None;
    }
    first_selected_checksum_file(files, |position| pane.display_entry(position))
}

fn first_selected_checksum_file<'a>(
    files: &SelectedFiles,
    entry_at: impl Fn(usize) -> Option<&'a Entry>,
) -> Option<PathBuf> {
    if !Arc::ptr_eq(files.selection.basis(), &files.view) {
        return None;
    }
    files
        .selection
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
        .filter_map(entry_at)
        .find(|entry| is_checksum_file(&entry.path, entry.kind))
        .map(|entry| entry.path.clone())
}

impl Presenter {
    pub(crate) fn request_checksums(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if let Some(feed) = self.feed(pane) {
            let Some(files) = feed.files() else {
                return;
            };
            let Some(focused) = feed.focused() else {
                return;
            };
            let directory = focused.path.parent().unwrap_or(&focused.path).to_owned();
            let manifest = Some(focused)
                .filter(|entry| {
                    is_checksum_file(&entry.path, entry.kind)
                        && (files.selection.is_empty()
                            || files.selection.contains_path(&entry.path))
                })
                .map(|entry| entry.path.clone())
                .or_else(|| {
                    first_selected_checksum_file(&files, |position| feed.entries.get(position))
                });
            self.open_checksum_session(directory, files, None, manifest, send);
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
        // folder): Calculate covers the shown folder (see `open_captured_checksums`).
        // A focused file or folder is the selection action's fallback below, so Ctrl+F8
        // always opens with something to calculate.
        if p.selected.is_empty()
            && p.selection_revision == p.selection_resolved
            && (p.parent_cursor || p.cursor.and_then(|id| p.entry_by_id(id)).is_none())
        {
            let directory = p.display_path().to_owned();
            self.open_captured_checksums(
                pane,
                directory,
                SelectedFiles {
                    selection: Arc::new(SelectionSnapshot::default()),
                    view: Arc::new(DirectoryIndex::default()),
                    fallback: None,
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
        mut files: SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<ChecksumJobId> {
        // Nothing selected and no row under the cursor, also when that is only known
        // once a pending selection (a deselect just before Ctrl+F8) is published:
        // Calculate covers the shown folder's files, and its subfolders when Include
        // subfolders is on. The shown folder is never a checksum file to verify.
        if files.selection.is_empty() && files.fallback.is_none() {
            files.fallback = Some(directory.clone());
            return self.open_checksum_session(directory, files, None, None, send);
        }
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
        // A captured tree names a checksum file only through its focused row, which
        // the tree capture accepts only as a regular file (and only without marks).
        let manifest = focused_checksum_file(&files, Some(EntryKind::File));
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
        // verify it, so the window opens on Verify with that file chosen. Otherwise it
        // opens on Calculate, which always has a target.
        let verify_initial = manifest.is_some();
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
                copy_note: None,
                streamed: ChecksumRows::default(),
                planned: Planned::default(),
                selection: Default::default(),
                carried: None,
                copy: None,
                copyable: selection::Copyable::default(),
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
            }))
        } else {
            Command::StartSelectedChecksums(Box::new(SelectedChecksumRequest {
                job: id,
                generation,
                files: s.files.clone(),
                algorithms,
                recursive,
            }))
        };
        if !send(command) {
            s.error = "Checksum queue is busy. Try Calculate again.".into();
            return false;
        }
        // The window's captured files never change: a calculation after one with the same
        // Include subfolders reads the same files, and selects the same ones again.
        let same_files = s.generation.0 > 0 && s.verification.is_none() && s.recursive == recursive;
        s.restart_selection(same_files);
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
        // A copy the clipboard worker has not written yet is not written.
        if let Some(copy) = s.copy.take() {
            copy.cancel.cancel();
        }
        self.retired
            .push(Box::new(self.checksums.sessions.remove(&id)));
        true
    }
}
