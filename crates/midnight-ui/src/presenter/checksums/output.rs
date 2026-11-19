use super::*;

#[path = "destination.rs"]
pub(super) mod destination;

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

    pub(crate) fn save_checksum_manifest(
        &mut self,
        id: ChecksumJobId,
        form: ChecksumSaveForm,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let ChecksumSaveForm {
            generation,
            algorithm,
            destination,
            acknowledge,
            overwrite,
        } = form;
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
            .filter(|s| s.generation.0 == generation)
        else {
            return false;
        };
        if s.running || s.save.is_some() || s.picker.is_some() {
            return false;
        }
        let Some(counts) = s
            .export_counts(algorithm)
            .filter(|counts| counts.included > 0)
        else {
            s.error = "No calculated checksums for this algorithm can be saved.".into();
            return false;
        };
        if (counts.excluded > 0 || counts.incomplete_discovery) && !acknowledge {
            s.error = "Confirm saving a partial manifest first.".into();
            return false;
        }
        let mut destination = match destination::resolve(&s.suggested, &destination) {
            Ok(path) => path,
            Err(error) => {
                s.error = error.into();
                return false;
            }
        };
        if destination.extension().is_none() {
            destination.set_extension(algorithm.extension());
        }
        let command = Command::SaveChecksumManifest {
            save_id,
            job: id,
            generation: s.generation,
            request: Box::new(ChecksumManifestRequest {
                algorithm,
                destination,
                acknowledge_partial: acknowledge.then_some(counts),
                overwrite,
            }),
        };
        let saved_to = match &command {
            Command::SaveChecksumManifest { request, .. } => request.destination.clone(),
            _ => unreachable!(),
        };
        if !send(command) {
            s.error = "Manifest queue is busy. Try saving again.".into();
            return false;
        }
        s.save = Some(ChecksumSave {
            algorithm,
            destination: saved_to,
            acknowledged: acknowledge,
            id: save_id,
            accepted: false,
            stopping: false,
        });
        s.error.clear();
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
        let generation = s.generation.0;
        let destination = s.suggested.to_string_lossy().into_owned();
        if counts.excluded > 0 || counts.incomplete_discovery {
            s.save_prompt = Some(SavePrompt {
                algorithm,
                destination: s.suggested.clone(),
                acknowledged: true,
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
            s.revision = s.revision.wrapping_add(1);
            return;
        }
        self.save_checksum_manifest(
            id,
            ChecksumSaveForm {
                generation,
                algorithm,
                destination,
                acknowledge: false,
                overwrite: false,
            },
            send,
        );
    }

    /// The user answered Save or Replace on the prompt.
    pub(crate) fn confirm_checksum_save(
        &mut self,
        id: ChecksumJobId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.checksums.session_mut(id) else {
            return;
        };
        let Some(prompt) = s.save_prompt.take() else {
            return;
        };
        s.revision = s.revision.wrapping_add(1);
        let generation = s.generation.0;
        self.save_checksum_manifest(
            id,
            ChecksumSaveForm {
                generation,
                algorithm: prompt.algorithm,
                destination: prompt.destination.to_string_lossy().into_owned(),
                acknowledge: prompt.acknowledged,
                overwrite: prompt.overwrite,
            },
            send,
        );
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
