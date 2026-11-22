//! Explicit verification of a captured manifest; parsing and hashing are backend work.
use super::*;

impl Presenter {
    pub(crate) fn start_checksum_verification(
        &mut self,
        id: ChecksumJobId,
        input: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(s) = self.checksums.session_mut(id) else {
            return false;
        };
        if s.running || s.save.is_some() || s.picker.is_some() {
            return false;
        }
        let manifest = match super::output::destination::resolve(&s.manifest_suggested, input) {
            Ok(path) => path,
            Err(message) => {
                s.error = message.into();
                return false;
            }
        };
        self.verify_checksum_file(id, manifest, send)
    }

    /// Verify again the checksum file whose results the window shows, by its exact
    /// path, whatever the Verify page's field holds now.
    pub(crate) fn repeat_checksum_verification(
        &mut self,
        id: ChecksumJobId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(manifest) = self
            .checksums
            .sessions
            .get(&id)
            .and_then(|s| s.verification.clone())
        else {
            return false;
        };
        self.verify_checksum_file(id, manifest, send)
    }

    fn verify_checksum_file(
        &mut self,
        id: ChecksumJobId,
        manifest: PathBuf,
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
        if !send(Command::VerifyChecksumManifest(Box::new(
            ChecksumVerifyRequest {
                job: id,
                generation,
                manifest: manifest.clone(),
            },
        ))) {
            s.error = "Checksum queue is busy. Try Verify again.".into();
            return false;
        }
        // Verifying the same checksum file again reads the same files, and selects the
        // same ones again; another file's results start with no selection.
        let same_files = s.verification.as_deref() == Some(manifest.as_path());
        s.restart_selection(same_files);
        s.generation = generation;
        s.verification = Some(manifest.clone());
        s.manifest_suggested = manifest;
        s.running = true;
        s.stopping = false;
        s.error.clear();
        s.forget_saves();
        s.progress = ChecksumProgress::default();
        // As for a calculation: the old rows and any listed files go.
        self.retired.push(Box::new(s.take_streamed()));
        self.retired.push(Box::new(s.run.take()));
        true
    }
}
