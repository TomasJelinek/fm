//! Bounded, backend-measured activity retained with an operation's progress.
use std::sync::Arc;

/// Peak measured activity within one retained interval, never estimated remaining work.
/// Adjacent intervals are combined as a long operation's time scale grows; each
/// series preserves its own observed peak, which need not occur simultaneously.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ThroughputSample {
    /// Monotonic start of this interval since the job's activity tracker started.
    pub start_millis: u64,
    /// End of the latest actual observation included in this interval.
    pub elapsed_millis: u64,
    pub transfer_per_second: f64,
    /// Destination readback and additional Move proof reads, separate from writes.
    pub verification_per_second: f64,
    /// Reported removed entries, including folders; Trash may report roots only.
    pub items_per_second: f64,
}

/// At most 240 intervals spanning the entire operation, shared between progress snapshots.
/// History retains measured peaks as intervals are combined. Current rates remain
/// the latest measured activity, and are zero while inactive or terminal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OperationThroughput {
    pub samples: Arc<[ThroughputSample]>,
    pub transfer_per_second: f64,
    pub verification_per_second: f64,
    pub items_per_second: f64,
}
