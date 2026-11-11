//! Monotonic activity sampling shared by transfer and mutation workers.
use midnight_core::{
    CopyOperationState, CopyProgress, MutationKind, OperationProgress, OperationState,
    OperationThroughput, ThroughputSample,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) const CADENCE: Duration = Duration::from_millis(250);
const MAX_SAMPLES: usize = 240;

#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct Counters {
    pub transfer: u64,
    pub verification: u64,
    pub items: u64,
}

impl Counters {
    fn difference(self, previous: Self) -> Option<Self> {
        Some(Self {
            transfer: self.transfer.checked_sub(previous.transfer)?,
            verification: self.verification.checked_sub(previous.verification)?,
            items: self.items.checked_sub(previous.items)?,
        })
    }

    fn add(self, other: Self) -> Self {
        Self {
            transfer: self.transfer.saturating_add(other.transfer),
            verification: self.verification.saturating_add(other.verification),
            items: self.items.saturating_add(other.items),
        }
    }
}

#[derive(Clone, Copy)]
struct Bucket {
    sample: ThroughputSample,
    observations: u64,
}

impl Bucket {
    fn include(&mut self, sample: ThroughputSample) {
        self.sample.start_millis = self.sample.start_millis.min(sample.start_millis);
        self.sample.elapsed_millis = sample.elapsed_millis;
        self.sample.transfer_per_second = self
            .sample
            .transfer_per_second
            .max(sample.transfer_per_second);
        self.sample.verification_per_second = self
            .sample
            .verification_per_second
            .max(sample.verification_per_second);
        self.sample.items_per_second = self.sample.items_per_second.max(sample.items_per_second);
    }
}

struct PeakHistory {
    buckets: Vec<Bucket>,
    observations_per_bucket: u64,
}

impl Default for PeakHistory {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            observations_per_bucket: 1,
        }
    }
}

impl PeakHistory {
    fn record(&mut self, sample: ThroughputSample, revises_last: bool) {
        if revises_last && let Some(last) = self.buckets.last_mut() {
            // Final partial work may extend the last observation, but cannot
            // erase a peak that was already measured and shown to the user.
            last.include(sample);
            return;
        }
        if self.buckets.len() == MAX_SAMPLES
            && self
                .buckets
                .last()
                .is_some_and(|last| last.observations == self.observations_per_bucket)
        {
            self.compact();
        }
        if let Some(last) = self.buckets.last_mut()
            && last.observations < self.observations_per_bucket
        {
            last.include(sample);
            last.observations += 1;
        } else {
            self.buckets.push(Bucket {
                sample,
                observations: 1,
            });
        }
    }

    fn compact(&mut self) {
        // The same resolution applies to old and future observations. Simply
        // appending fresh single samples after each compaction would eventually
        // collapse nearly the entire early operation into one oversized bucket.
        let mut written = 0;
        for read in (0..self.buckets.len()).step_by(2) {
            let mut merged = self.buckets[read];
            if let Some(next) = self.buckets.get(read + 1) {
                merged.include(next.sample);
                merged.observations = merged.observations.saturating_add(next.observations);
            }
            self.buckets[written] = merged;
            written += 1;
        }
        self.buckets.truncate(written);
        self.observations_per_bucket = self.observations_per_bucket.saturating_mul(2);
    }

    fn samples(&self) -> Arc<[ThroughputSample]> {
        self.buckets.iter().map(|bucket| bucket.sample).collect()
    }
}

#[derive(Default)]
pub(crate) struct Sampler {
    baseline: Counters,
    baseline_at: Duration,
    observed_at: Duration,
    active: bool,
    started: bool,
    history: PeakHistory,
    last_counts: Counters,
    last_duration: Duration,
    last_started: Duration,
    last_recorded: Duration,
    snapshot: Arc<OperationThroughput>,
}

impl Sampler {
    pub(crate) fn observe(
        &mut self,
        elapsed: Duration,
        counters: Counters,
        active: bool,
    ) -> Arc<OperationThroughput> {
        if elapsed < self.observed_at {
            return self.snapshot.clone();
        }
        self.observed_at = elapsed;
        if active && !self.active {
            self.started = true;
            self.active = true;
            self.baseline = counters;
            self.baseline_at = elapsed;
            return self.snapshot.clone();
        }
        if !self.started {
            return self.snapshot.clone();
        }
        let duration = elapsed.saturating_sub(self.baseline_at);
        // A native call can finish real work after cancellation was requested.
        // Account for those counters even while current activity labels are zero.
        let delta = counters.difference(self.baseline);
        if let Some(delta) = delta {
            // Flush genuine work at a pause/terminal boundary, even for short jobs.
            if !duration.is_zero()
                && (duration >= CADENCE || (!active && delta != Counters::default()))
            {
                self.record(elapsed, duration, delta, active);
                self.baseline = counters;
                self.baseline_at = elapsed;
            }
        } else {
            // A retried/reset counter is a new baseline, never a negative delta.
            self.baseline = counters;
            self.baseline_at = elapsed;
            stop(&mut self.snapshot);
        }
        self.active = active;
        if !active {
            stop(&mut self.snapshot);
        }
        self.snapshot.clone()
    }

    fn record(
        &mut self,
        elapsed: Duration,
        mut duration: Duration,
        mut delta: Counters,
        active: bool,
    ) {
        let revises_last = !self.history.buckets.is_empty()
            && elapsed.saturating_sub(self.last_recorded) < CADENCE;
        let started = if revises_last {
            delta = delta.add(self.last_counts);
            duration = duration.saturating_add(self.last_duration);
            self.last_started
        } else {
            self.baseline_at
        };
        self.last_counts = delta;
        self.last_duration = duration;
        self.last_started = started;
        self.last_recorded = elapsed;
        let seconds = duration.as_secs_f64();
        let sample = ThroughputSample {
            start_millis: started.as_millis().min(u128::from(u64::MAX)) as u64,
            elapsed_millis: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
            transfer_per_second: delta.transfer as f64 / seconds,
            verification_per_second: delta.verification as f64 / seconds,
            items_per_second: delta.items as f64 / seconds,
        };
        self.history.record(sample, revises_last);
        self.snapshot = Arc::new(OperationThroughput {
            samples: self.history.samples(),
            transfer_per_second: if active {
                sample.transfer_per_second
            } else {
                0.0
            },
            verification_per_second: if active {
                sample.verification_per_second
            } else {
                0.0
            },
            items_per_second: if active { sample.items_per_second } else { 0.0 },
        });
    }
}

/// Zero current labels without copying history or changing an already idle Arc.
pub(crate) fn stop(snapshot: &mut Arc<OperationThroughput>) {
    if snapshot.transfer_per_second != 0.0
        || snapshot.verification_per_second != 0.0
        || snapshot.items_per_second != 0.0
    {
        *snapshot = Arc::new(OperationThroughput {
            samples: snapshot.samples.clone(),
            ..OperationThroughput::default()
        });
    }
}

pub(crate) struct Tracker {
    started: Instant,
    sampler: Sampler,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            sampler: Sampler::default(),
        }
    }
}

impl Tracker {
    pub(crate) fn copy(&mut self, progress: &mut CopyProgress) {
        progress.throughput = self.sampler.observe(
            self.started.elapsed(),
            copy_counters(progress),
            progress.state == CopyOperationState::Running && !progress.paused,
        );
    }

    pub(crate) fn deletion(&mut self, progress: &mut OperationProgress) {
        if matches!(progress.kind, MutationKind::Delete(_)) {
            progress.throughput = self.sampler.observe(
                self.started.elapsed(),
                Counters {
                    items: progress.processed_entries,
                    ..Counters::default()
                },
                progress.state == OperationState::Running && !progress.paused,
            );
        }
    }
}

fn copy_counters(progress: &CopyProgress) -> Counters {
    Counters {
        transfer: progress.copied_bytes,
        verification: progress
            .verified_bytes
            .saturating_add(progress.validation_bytes),
        items: progress.native_entries_moved,
    }
}

#[cfg(test)]
#[path = "operation_throughput_tests.rs"]
mod tests;
