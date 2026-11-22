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

/// What a copy of rows writes for each of them.
#[derive(Clone, Copy)]
enum Copied {
    /// The checksum of the algorithm, one per line.
    Hashes(ChecksumAlgorithm),
    /// The path and every checksum, with a blank line between rows.
    Rows,
    /// The checksum-file line, exactly as Save manifest… writes it.
    Lines(ChecksumAlgorithm),
}

/// Why a row adds nothing to a copy of this kind; none when it does add something.
fn left_out(row: &ChecksumRow, copied: Copied) -> Option<&'static str> {
    use report::{CANCELLED, FAILED, NO_DIGEST, SKIPPED, VERIFICATION};
    let digest = |algorithm| row.digest(algorithm).is_some();
    match row.state {
        ChecksumRowState::ReadError
        | ChecksumRowState::SourceChanged
        | ChecksumRowState::Missing => Some(FAILED),
        ChecksumRowState::Skipped => Some(SKIPPED),
        ChecksumRowState::Cancelled => Some(CANCELLED),
        ChecksumRowState::Verified | ChecksumRowState::Mismatch
            if matches!(copied, Copied::Lines(_)) =>
        {
            Some(VERIFICATION)
        }
        _ => match copied {
            Copied::Hashes(algorithm) | Copied::Lines(algorithm) if !digest(algorithm) => {
                Some(NO_DIGEST)
            }
            Copied::Rows if row.digests.is_empty() => Some(NO_DIGEST),
            _ => None,
        },
    }
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
        let copied = if whole_row {
            Copied::Rows
        } else {
            Copied::Hashes(algorithm)
        };
        self.copy_rows(id, None, copied, send);
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
        let copied = if whole_row {
            Copied::Rows
        } else {
            Copied::Hashes(algorithm)
        };
        self.copy_rows(id, only, copied, send);
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
        self.copy_rows(id, None, Copied::Lines(algorithm), send);
    }

    /// Copy the selected rows, or only the row in `only` (a slot), in one pass into a text
    /// sized for them.
    fn copy_rows(
        &mut self,
        id: ChecksumJobId,
        only: Option<usize>,
        copied: Copied,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.checksums.sessions.get(&id) else {
            return;
        };
        let (rows, waiting) = match only {
            None => s.selected_rows(),
            Some(slot) if slot % 2 == 1 => (Vec::new(), 1),
            Some(slot) => (
                s.rows()
                    .by_id(slot as u64 / 2 + 1)
                    .into_iter()
                    .collect::<Vec<_>>(),
                0,
            ),
        };
        let folder = s.suggested.parent().unwrap_or(&s.directory);
        let mut note = CopyNote {
            rows: matches!(copied, Copied::Rows),
            ..CopyNote::default()
        };
        let capacity = rows.iter().fold(0usize, |total, row| {
            total.saturating_add(match copied {
                Copied::Hashes(algorithm) => algorithm.bytes() * 2 + 1,
                Copied::Lines(algorithm) => algorithm.bytes() * 2 + 4 + row.name.as_os_str().len(),
                Copied::Rows => {
                    row.base.as_os_str().len()
                        + row.name.as_os_str().len()
                        + row
                            .digests
                            .iter()
                            .map(|d| d.bytes.len() * 2 + 10)
                            .sum::<usize>()
                        + 3
                }
            })
        });
        let mut text = String::with_capacity(capacity);
        for row in &rows {
            if let Some(reason) = left_out(row, copied) {
                note.left_out(reason);
                continue;
            }
            match copied {
                Copied::Hashes(algorithm) => {
                    let Some(digest) = row.digest(algorithm) else {
                        continue;
                    };
                    if note.copied > 0 {
                        text.push('\n');
                    }
                    digest.push_hex(&mut text);
                }
                Copied::Rows => {
                    if note.copied > 0 {
                        text.push_str("\n\n");
                    }
                    text.push_str(&row.path().display().to_string());
                    for digest in row.digests.iter() {
                        text.push('\n');
                        text.push_str(digest.algorithm.label());
                        text.push_str(": ");
                        digest.push_hex(&mut text);
                    }
                }
                Copied::Lines(algorithm) => {
                    let Some(digest) = row.digest(algorithm) else {
                        continue;
                    };
                    let Some(name) = checksum_file_name(row.base, &row.name, folder) else {
                        note.left_out(CHECKSUM_NAME_NOT_BELOW);
                        continue;
                    };
                    if let Err((_, reason)) = push_checksum_file_line(&mut text, digest, &name) {
                        note.left_out(reason);
                        continue;
                    }
                }
            }
            note.copied += 1;
        }
        // Selected files still waiting for their rows come after the rows shown.
        for _ in 0..waiting {
            note.left_out(report::WAITING);
        }
        let Some(s) = self.checksums.sessions.get_mut(&id) else {
            return;
        };
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
