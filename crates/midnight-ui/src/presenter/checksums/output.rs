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

impl Presenter {
    pub(crate) fn copy_checksum(
        &mut self,
        id: ChecksumJobId,
        token: &str,
        algorithm: ChecksumAlgorithm,
        whole_row: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(row) = self.checksums.sessions.get(&id).and_then(|s| s.row(token)) else {
            return;
        };
        if !matches!(
            row.state,
            ChecksumRowState::Calculated | ChecksumRowState::Verified | ChecksumRowState::Mismatch
        ) || row.digests.is_empty()
        {
            return;
        }
        let text = if whole_row {
            let hashes = row
                .digests
                .iter()
                .map(|d| format!("{}: {}", d.algorithm.label(), d.hex()))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{}\n{}", row.path.display(), hashes)
        } else {
            let Some(digest) = row.digest(algorithm) else {
                return;
            };
            digest.hex()
        };
        self.copy_viewer_text(text, send);
    }

    /// Copy the calculated checksums as the checksum file Save would write to the
    /// suggested folder: the same lines, names relative to that folder. A file it
    /// cannot name there is left out, and a note (not an error) says how many were,
    /// for each reason.
    pub(crate) fn copy_checksum_file(
        &mut self,
        id: ChecksumJobId,
        algorithm: ChecksumAlgorithm,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.checksums.sessions.get_mut(&id) else {
            return;
        };
        if s.running {
            return;
        }
        let directory = s.suggested.parent().unwrap_or(&s.directory).to_owned();
        let mut text = String::new();
        let mut note = CopyNote::default();
        for row in s.rows() {
            let Some(digest) = (row.state == ChecksumRowState::Calculated)
                .then(|| row.digest(algorithm))
                .flatten()
            else {
                continue;
            };
            match checksum_file_line(algorithm, &digest.hex(), &row.path, &directory) {
                Ok(line) => {
                    text.push_str(&line);
                    note.copied += 1;
                }
                Err((_, reason)) => note.left_out(reason),
            }
        }
        // The copy succeeded: an earlier error stays, and the note replaces only the
        // previous copy's note.
        s.copy_note = (!note.left_out.is_empty()).then_some(note);
        s.revision = s.revision.wrapping_add(1);
        if !text.is_empty() {
            self.copy_viewer_text(text, send);
        }
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
                    "{} files were excluded{}. Save the {} completed checksums anyway?",
                    counts.excluded,
                    if counts.incomplete_discovery {
                        " and discovery is incomplete"
                    } else {
                        ""
                    },
                    counts.included
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
