//! The current copied-file attempt: its verification chart, which reuses the bounded
//! throughput sampler, and how long it has been running.
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
    /// This attempt's running time so far, and since when it is running now.
    running: Duration,
    running_since: Option<Duration>,
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
            self.running = Duration::ZERO;
            self.running_since = None;
        }
        self.time_attempt(elapsed, progress);
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

    /// The current attempt's time counts only while the job runs: pauses, waits and
    /// decisions hold it, so a window can tell a slow file from a stopped job.
    fn time_attempt(&mut self, elapsed: Duration, progress: &mut CopyProgress) {
        if let Some(since) = self.running_since.take() {
            self.running = self.running.saturating_add(elapsed.saturating_sub(since));
        }
        let running = self.id != 0
            && progress.state == CopyOperationState::Running
            && !progress.paused
            && progress.wait_reason.is_none();
        if running {
            self.running_since = Some(elapsed);
        }
        progress.current_file_active_millis = if self.id == 0 {
            0
        } else {
            self.running.as_millis().min(u128::from(u64::MAX)) as u64
        };
    }
}

#[cfg(test)]
#[path = "verification_pass_tests.rs"]
mod tests;
