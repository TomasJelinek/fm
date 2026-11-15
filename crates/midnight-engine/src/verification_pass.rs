//! A current-file verification chart reuses the bounded throughput sampler.
use super::{Counters, Sampler};
use midnight_core::{CopyOperationPhase, CopyOperationState, CopyProgress};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct PassIds {
    last: u64,
}

impl PassIds {
    pub(crate) fn next(&mut self) -> Result<u64, &'static str> {
        let next = self
            .last
            .checked_add(1)
            .ok_or("Verification pass identifiers exhausted.")?;
        self.last = next;
        Ok(next)
    }
}

#[derive(Default)]
pub(super) struct PassSampler {
    id: u64,
    started: Duration,
    sampler: Sampler,
    active: bool,
    verified: u64,
}

impl PassSampler {
    pub(super) fn observe(&mut self, elapsed: Duration, progress: &mut CopyProgress) {
        let id = if progress.current_file_size.is_some() {
            progress.verification_pass_id
        } else {
            0
        };
        if id != self.id {
            self.id = id;
            self.started = elapsed;
            self.sampler = Sampler::default();
            self.active = false;
            self.verified = progress.current_file_verified;
        }
        if id == 0 {
            progress.verification_pass_id = 0;
            progress.verification_throughput = self.sampler.snapshot.clone();
            return;
        }
        let active = progress.state == CopyOperationState::Running
            && progress.phase == CopyOperationPhase::Verifying
            && !progress.paused
            && progress.wait_reason.is_none();
        let counters = Counters {
            verification: progress.current_file_verified,
            ..Counters::default()
        };
        // One inactive boundary flushes a short pass. Further idle observations
        // retain its history without inventing readback or sampling wait time.
        if active || self.active || self.verified != progress.current_file_verified {
            progress.verification_throughput =
                self.sampler
                    .observe(elapsed.saturating_sub(self.started), counters, active);
        } else {
            progress.verification_throughput = self.sampler.snapshot.clone();
        }
        self.active = active;
        self.verified = progress.current_file_verified;
    }
}

#[cfg(test)]
#[path = "verification_pass_tests.rs"]
mod tests;
