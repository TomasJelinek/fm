use super::*;

#[path = "destination.rs"]
pub(super) mod destination;

/// A save about to be sent, exactly as the user chose it.
struct PlannedSave {
    generation: ChecksumGeneration,
    algorithm: ChecksumAlgorithm,
    destination: PathBuf,
    acknowledged: Option<ChecksumPartialAcknowledgement>,
    overwrite: bool,
}

/// A copy that the clipboard worker builds from the rows and writes: a newer copy of the
/// same window, or closing it, cancels it.
pub(crate) struct PendingCopy {
    pub(crate) request: DesktopRequestId,
    pub(crate) content: Arc<ChecksumCopyText>,
    pub(crate) cancel: Cancellation,
}

impl Presenter {
    /// Ctrl+C (`whole_row` false) or Ctrl+Shift+C: the selected rows' checksums of
    /// `algorithm`, one per line, or their full rows, in the order shown.
    pub(crate) fn copy_checksums(
        &mut self,
        id: ChecksumJobId,
        algorithm: ChecksumAlgorithm,
        whole_row: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let kind = if whole_row {
            ChecksumCopyKind::Rows
        } else {
            ChecksumCopyKind::Hashes(algorithm)
        };
        self.copy_rows(id, None, kind, send);
    }

    /// A row's context menu: the whole selection when `selection` is set and the row is
    /// in it (file-manager convention), otherwise that row alone. The token is the row's,
    /// captured when the menu opened.
    pub(crate) fn copy_checksum(
        &mut self,
        id: ChecksumJobId,
        token: &str,
        algorithm: ChecksumAlgorithm,
        whole_row: bool,
        selection: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.checksums.sessions.get(&id) else {
            return;
        };
        let Some(slot) = s.token_slot(token) else {
            return;
        };
        let only = (!(selection && s.selection.is_selected(slot))).then_some(slot);
        let kind = if whole_row {
            ChecksumCopyKind::Rows
        } else {
            ChecksumCopyKind::Hashes(algorithm)
        };
        self.copy_rows(id, only, kind, send);
    }

    /// Copy the selected rows as a checksum file, in the order shown: the lines Save
    /// manifest… writes to the suggested folder, names relative to that folder. A row
    /// with no calculated checksum, or one it cannot name there, is left out, and a note
    /// (not an error) says how many were, for each reason.
    pub(crate) fn copy_checksum_file(
        &mut self,
        id: ChecksumJobId,
        algorithm: ChecksumAlgorithm,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.copy_rows(id, None, ChecksumCopyKind::Lines(algorithm), send);
    }

    /// Copy the selected rows, or only the row in `only` (a slot). The UI thread only
    /// snapshots the selection's bits and shares the row chunks; the clipboard worker
    /// builds the text in one pass and writes it, and its result brings the note back
    /// (`checksum_copy_finished`).
    fn copy_rows(
        &mut self,
        id: ChecksumJobId,
        only: Option<usize>,
        kind: ChecksumCopyKind,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.checksums.sessions.get_mut(&id) else {
            return;
        };
        // Nothing selected, a listed file alone or a row that adds nothing: decided here,
        // and the clipboard keeps what it holds.
        let nothing = match only {
            None if s.selection.is_empty() => Some(None),
            Some(slot) if slot % 2 == 1 => Some(Some(report::WAITING)),
            Some(slot) => s
                .rows()
                .by_id(slot as u64 / 2 + 1)
                .and_then(|row| checksum_copy_left_out(&row, kind))
                .map(Some),
            None => None,
        };
        if let Some(reason) = nothing {
            s.copy_note = reason.map(|reason| CopyNote {
                copied: 0,
                left_out: vec![(reason, 1)],
                rows: kind == ChecksumCopyKind::Rows,
            });
            s.revision = s.revision.wrapping_add(1);
            return;
        }
        let selection = match only {
            None => ChecksumCopyRows::Selected(s.selection.words().into()),
            Some(slot) => ChecksumCopyRows::Row(slot as u64 / 2 + 1),
        };
        let folder = s.suggested.parent().unwrap_or(&s.directory).to_path_buf();
        let content = Arc::new(ChecksumCopyText::new(
            s.rows().clone(),
            selection,
            kind,
            folder,
        ));
        if let Some(superseded) = s.copy.take() {
            superseded.cancel.cancel();
        }
        let request = self.desktop.request();
        let cancel = Cancellation::default();
        if send(Command::CopyGeneratedText {
            request,
            content: content.clone(),
            cancel: cancel.clone(),
        }) {
            s.copy = Some(PendingCopy {
                request,
                content,
                cancel,
            });
        } else {
            self.desktop
                .report("Clipboard queue busy", "Try copying again.".into());
        }
    }

    /// Whether `request` is the copy a checksum window waits for.
    pub(in crate::presenter) fn owns_checksum_copy(&self, request: DesktopRequestId) -> bool {
        self.checksums
            .sessions
            .values()
            .any(|s| s.copy.as_ref().is_some_and(|copy| copy.request == request))
    }

    /// The clipboard worker wrote a copy, or could not: the note says what it took and
    /// left out. A copy that took nothing leaves the clipboard alone and says why; a
    /// cancelled one (superseded or closed) says nothing.
    pub(in crate::presenter) fn checksum_copy_finished(
        &mut self,
        request: DesktopRequestId,
        result: Result<DesktopActionResult, FsError>,
    ) {
        let Some(s) = self
            .checksums
            .sessions
            .values_mut()
            .find(|s| s.copy.as_ref().is_some_and(|copy| copy.request == request))
        else {
            return;
        };
        let Some(copy) = s.copy.take() else {
            return;
        };
        let note = copy.content.report().map(|report| CopyNote {
            rows: copy.content.kind() == ChecksumCopyKind::Rows,
            copied: report.copied,
            left_out: report.left_out,
        });
        let note = note.filter(|note| !note.left_out.is_empty());
        match result {
            Ok(_) => {
                // The copy succeeded: an earlier error stays, and the note replaces only
                // the previous copy's note.
                s.copy_note = note;
                s.revision = s.revision.wrapping_add(1);
                self.desktop.status = "Copied to clipboard".into();
            }
            Err(error) if error.message == CHECKSUM_COPY_NOTHING => {
                s.copy_note = note;
                s.revision = s.revision.wrapping_add(1);
            }
            Err(error) if error.kind == std::io::ErrorKind::Interrupted => {}
            Err(error) => self.desktop.report(
                "Desktop action failed",
                crate::error_presentation::describe(&error),
            ),
        }
        self.retired.push(Box::new(copy.content));
    }

    /// Send one save of the session's current run. The raw destination path is kept
    /// as chosen; a partial run needs the exact counts the user accepted.
    fn submit_checksum_save(
        &mut self,
        id: ChecksumJobId,
        save: PlannedSave,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        // IDs are never reused, including refused or stale attempts.
        let Some(save_id) = self
            .checksums
            .next_save
            .checked_add(1)
            .map(ChecksumManifestId)
        else {
            return false;
        };
        self.checksums.next_save = save_id.0;
        let Some(s) = self
            .checksums
            .session_mut(id)
            .filter(|s| s.generation == save.generation)
        else {
            return false;
        };
        if s.running || s.save.is_some() || s.picker.is_some() {
            return false;
        }
        let Some(counts) = s
            .export_counts(save.algorithm)
            .filter(|counts| counts.included > 0)
        else {
            s.error = "No calculated checksums for this algorithm can be saved.".into();
            return false;
        };
        let partial = counts.excluded > 0 || counts.incomplete_discovery;
        if partial && save.acknowledged != Some(counts) {
            s.error = "Confirm saving a partial manifest first.".into();
            return false;
        }
        let mut destination = save.destination;
        if destination.extension().is_none() {
            destination.set_extension(save.algorithm.extension());
        }
        let command = Command::SaveChecksumManifest {
            save_id,
            job: id,
            generation: s.generation,
            request: Box::new(ChecksumManifestRequest {
                algorithm: save.algorithm,
                destination: destination.clone(),
                acknowledge_partial: partial.then_some(counts),
                overwrite: save.overwrite,
            }),
        };
        if !send(command) {
            s.error = "Manifest queue is busy. Try saving again.".into();
            return false;
        }
        s.save = Some(ChecksumSave {
            algorithm: save.algorithm,
            destination,
            acknowledged: partial.then_some(counts),
            id: save_id,
            accepted: false,
            stopping: false,
        });
        s.error.clear();
        s.saved = None;
        s.copy_note = None;
        s.save_message = "Waiting to save manifest…".into();
        true
    }

    /// Save to the destination the user just chose. Partial results need a yes first;
    /// an existing file is asked about when the engine reports it.
    pub(crate) fn begin_checksum_save(
        &mut self,
        id: ChecksumJobId,
        algorithm: ChecksumAlgorithm,
        send: &dyn Fn(Command) -> bool,
    ) {
        let locale = &self.number_locale;
        let Some(s) = self.checksums.session_mut(id) else {
            return;
        };
        if s.running || s.save.is_some() || s.picker.is_some() {
            return;
        }
        let Some(counts) = s
            .export_counts(algorithm)
            .filter(|counts| counts.included > 0)
        else {
            s.error = "No calculated checksums for this algorithm can be saved.".into();
            return;
        };
        let save = PlannedSave {
            generation: s.generation,
            algorithm,
            destination: s.suggested.clone(),
            acknowledged: None,
            overwrite: false,
        };
        if counts.excluded > 0 || counts.incomplete_discovery {
            s.save_prompt = Some(SavePrompt {
                generation: save.generation,
                algorithm,
                destination: save.destination,
                acknowledged: Some(counts),
                overwrite: false,
                message: format!(
                    "{} excluded{}. Save the {} anyway?",
                    crate::job_presentation::counted(
                        counts.excluded,
                        "file was",
                        "files were",
                        locale
                    ),
                    if counts.incomplete_discovery {
                        " and discovery is incomplete"
                    } else {
                        ""
                    },
                    crate::job_presentation::counted(
                        counts.included,
                        "completed checksum",
                        "completed checksums",
                        locale
                    )
                ),
            });
            return;
        }
        self.submit_checksum_save(id, save, send);
    }

    /// The user answered Save or Replace on the prompt. It saves exactly what the
    /// question was about; a refused answer is reported and the question kept.
    pub(crate) fn confirm_checksum_save(
        &mut self,
        id: ChecksumJobId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.checksums.session_mut(id) else {
            return;
        };
        let Some(prompt) = s.save_prompt.as_ref() else {
            return;
        };
        if prompt.generation != s.generation {
            // Unreachable while a new run discards the question; never save other results.
            s.save_prompt = None;
            s.error = "The results changed after this question. Save the manifest again.".into();
            return;
        }
        if s.running || s.save.is_some() || s.picker.is_some() {
            s.error = "Wait for the current checksum work to finish, then answer again.".into();
            return;
        }
        let save = PlannedSave {
            generation: prompt.generation,
            algorithm: prompt.algorithm,
            destination: prompt.destination.clone(),
            acknowledged: prompt.acknowledged,
            overwrite: prompt.overwrite,
        };
        if self.submit_checksum_save(id, save, send)
            && let Some(s) = self.checksums.sessions.get_mut(&id)
        {
            s.save_prompt = None;
        }
    }

    pub(crate) fn dismiss_checksum_save_prompt(&mut self, id: ChecksumJobId) {
        if let Some(s) = self.checksums.session_mut(id)
            && s.save_prompt.take().is_some()
        {
            s.revision = s.revision.wrapping_add(1);
        }
    }

    /// Run saves whose destination was just chosen; called by the window's refresh.
    pub(crate) fn begin_pending_checksum_saves(&mut self, send: &dyn Fn(Command) -> bool) {
        let pending: Vec<_> = self
            .checksums
            .sessions
            .iter_mut()
            .filter_map(|(id, s)| s.save_pending.take().map(|algorithm| (*id, algorithm)))
            .collect();
        for (id, algorithm) in pending {
            self.begin_checksum_save(id, algorithm, send);
        }
    }

    pub(crate) fn cancel_checksum_save(
        &mut self,
        id: ChecksumJobId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.checksums.session_mut(id) else {
            return;
        };
        let Some(save) = s.save.as_mut().filter(|save| !save.stopping) else {
            return;
        };
        if send(Command::CancelChecksumManifest { save_id: save.id }) {
            save.stopping = true;
            s.save_message = "Stopping manifest save…".into();
        } else {
            s.error = "Cancellation queue is busy. Try Stop save again.".into();
        }
    }
}

#[cfg(test)]
impl Presenter {
    /// What the clipboard worker does with the checksum copies in `sent` that a window
    /// still waits for: build each, then report the result back. Returns the texts written.
    pub(crate) fn run_checksum_copies(&mut self, sent: &[Command]) -> Vec<String> {
        let mut texts = Vec::new();
        for command in sent {
            let Command::CopyGeneratedText {
                request,
                content,
                cancel,
            } = command
            else {
                continue;
            };
            if !self.owns_checksum_copy(*request) {
                continue;
            }
            let result = match content.prepare(cancel) {
                Ok(()) => {
                    texts.push(content.take_text().unwrap_or_default());
                    Ok(DesktopActionResult::ClipboardCopied)
                }
                Err(error) => Err(FsError::from_io("clipboard", Path::new(""), error)),
            };
            self.apply(Event::DesktopActionFinished {
                request: *request,
                result,
            });
        }
        texts
    }
}
