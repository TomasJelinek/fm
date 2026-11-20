//! Bounded backend activity; the owning progress field defines the measurement scope.
use std::sync::Arc;

/// Peak measured activity within one retained interval, never estimated remaining work.
/// Adjacent intervals are combined as a long operation's time scale grows; each
/// series preserves its own observed peak, which need not occur simultaneously.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ThroughputSample {
    /// Monotonic interval start relative to the owning field's activity tracker.
    /// Whole-job and current copied-file attempt trackers have separate origins.
    pub start_millis: u64,
    /// End of the latest actual observation included in this interval.
    pub elapsed_millis: u64,
    pub transfer_per_second: f64,
    /// CopyProgress.throughput includes destination readback and Move proof reads.
    /// CopyProgress.verification_throughput includes only the current copied-file
    /// attempt's destination readback; it excludes Move proofs and source writes.
    pub verification_per_second: f64,
    /// Reported removed entries, including folders; Trash may report roots only.
    pub items_per_second: f64,
}

/// At most 240 intervals for the scope selected by the owning progress field,
/// shared between snapshots: the whole operation for CopyProgress.throughput,
/// or the current copied-file attempt for CopyProgress.verification_throughput.
/// History retains measured peaks as intervals are combined. Current rates remain
/// the latest measured activity, and are zero while inactive or terminal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OperationThroughput {
    pub samples: Arc<[ThroughputSample]>,
    pub transfer_per_second: f64,
    pub verification_per_second: f64,
    pub items_per_second: f64,
    /// The basis of a remaining-time estimate: the same writes and reads averaged
    /// over roughly the last eight seconds of activity, each interval weighted by its
    /// duration (an exponential average over time, not over samples), so a burst
    /// between two observations barely moves it. Zero until a second of activity
    /// was measured. Unlike the current rates these stay while the job is paused or
    /// waiting, so an estimate holds still and resumes where it stopped.
    pub smoothed_transfer_per_second: f64,
    pub smoothed_verification_per_second: f64,
    /// The same average of entries the job completed, counted as its progress
    /// counts them: the files, links and folders a Copy finished (a Move's files
    /// once their source is removed), its selected items when only those are
    /// counted, or the entries a Delete removed.
    pub smoothed_entries_per_second: f64,
}
