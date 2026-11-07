use super::*;

#[path = "destination.rs"]
mod destination;

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
        if row.state != ChecksumRowState::Calculated || row.digests.is_empty() {
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
        if s.running || s.save.is_some() {
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
            s.error = "Acknowledge the displayed excluded files and incomplete discovery before saving a partial manifest.".into();
            return false;
        }
        let destination = match destination::resolve(&s.suggested, &destination) {
            Ok(path) => path,
            Err(error) => {
                s.error = error.into();
                return false;
            }
        };
        let command = Command::SaveChecksumManifest {
            save_id,
            job: id,
            generation: s.generation,
            request: Box::new(ChecksumManifestRequest {
                algorithm,
                destination,
                acknowledge_partial: acknowledge.then_some(counts),
            }),
        };
        if !send(command) {
            s.error = "Manifest queue is busy. Try saving again.".into();
            return false;
        }
        s.save = Some(ChecksumSave {
            id: save_id,
            accepted: false,
            stopping: false,
        });
        s.error.clear();
        s.save_message = "Waiting to save manifest…".into();
        true
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
