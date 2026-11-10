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
        let Some(generation) = s.generation.0.checked_add(1).map(ChecksumGeneration) else {
            return false;
        };
        if !send(Command::VerifyChecksumManifest(Box::new(
            ChecksumVerifyRequest {
                job: id,
                generation,
                manifest: manifest.clone(),
                limits: ChecksumLimits::default(),
            },
        ))) {
            s.error = "Checksum queue is busy. Try Verify again.".into();
            return false;
        }
        s.generation = generation;
        s.verification = Some(manifest.clone());
        s.manifest_suggested = manifest;
        s.running = true;
        s.stopping = false;
        s.error.clear();
        s.save_message.clear();
        s.progress = ChecksumProgress::default();
        self.retired.push(Box::new(mem::take(&mut s.streamed)));
        self.retired.push(Box::new(s.run.take()));
        true
    }
}
