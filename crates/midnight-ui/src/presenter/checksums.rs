//! Captured checksum sessions. Enumeration, hashing, export and clipboard I/O stay in the engine.
use super::*;

mod events;
mod output;
mod picker;
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
    pub save_message: String,
    pub suggested: PathBuf,
    pub manifest_suggested: PathBuf,
    pub verification: Option<PathBuf>,
    pub verify_initial: bool,
    pub destination_revision: u64,
    pub manifest_revision: u64,
    pub picker: Option<picker::Pending>,
    streamed: Vec<Arc<ChecksumRow>>,
}

pub(crate) struct ChecksumSave {
    pub id: ChecksumManifestId,
    pub accepted: bool,
    pub stopping: bool,
}

pub(crate) struct ChecksumSaveForm {
    pub generation: u64,
    pub algorithm: ChecksumAlgorithm,
    pub destination: String,
    pub acknowledge: bool,
}

impl ChecksumSession {
    pub fn rows(&self) -> &[Arc<ChecksumRow>] {
        self.run.as_ref().map_or(&self.streamed, |run| &run.rows)
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

impl ChecksumState {
    fn session_mut(&mut self, id: ChecksumJobId) -> Option<&mut ChecksumSession> {
        let session = self.sessions.get_mut(&id)?;
        session.revision = session.revision.wrapping_add(1);
        Some(session)
    }
}

impl Presenter {
    pub(crate) fn request_checksums(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
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
        if p.selected.is_empty()
            && p.selection_revision == p.selection_resolved
            && (p.parent_cursor
                || p.cursor
                    .and_then(|id| p.entry_by_id(id))
                    .is_none_or(|e| e.kind != EntryKind::File))
        {
            self.open_captured_checksums(
                pane,
                p.display_path().to_owned(),
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
        _pane: PaneId,
        directory: PathBuf,
        files: SelectedFiles,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<ChecksumJobId> {
        self.open_checksum_sources(directory, files, None, send)
    }

    pub(super) fn open_checksum_sources(
        &mut self,
        directory: PathBuf,
        files: SelectedFiles,
        tree_sources: Option<Arc<dyn CapturedChecksumFiles>>,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<ChecksumJobId> {
        if self.checksums.sessions.len() >= MAX_CHECKSUM_WINDOWS || self.operations.closing {
            self.desktop.report(
                "Checksums",
                "Close one of the four checksum windows first.".into(),
            );
            self.retired.push(Box::new((files, tree_sources)));
            return None;
        }
        let Some(id) = self.checksums.next_job.checked_add(1).map(ChecksumJobId) else {
            self.retired.push(Box::new((files, tree_sources)));
            return None;
        };
        self.checksums.next_job = id.0;
        let algorithms = self.preferences.values.checksum_algorithms;
        let manifest = files
            .fallback
            .as_ref()
            .filter(|path| {
                if !files.selection.is_empty() {
                    return false;
                }
                path.extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| {
                        ChecksumAlgorithm::ALL
                            .iter()
                            .any(|a| extension.eq_ignore_ascii_case(a.extension()))
                    })
            })
            .cloned();
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
                streamed: Vec::new(),
            },
        );
        if !verify_initial {
            self.start_checksums(id, algorithms, false, send);
        }
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
        s.save_message.clear();
        s.progress = ChecksumProgress::default();
        self.retired.push(Box::new(mem::take(&mut s.streamed)));
        self.retired.push(Box::new(s.run.take()));
        if self.preferences.values.checksum_algorithms != algorithms {
            self.preferences.edited.checksum_algorithms = true;
            let mut values = self.preferences.values.clone();
            values.checksum_algorithms = algorithms;
            self.update_preferences(values, send);
        }
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
