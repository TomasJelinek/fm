//! A metadata snapshot improves progress without becoming execution authority.
use super::*;
use midnight_core::{DiscoveryScope, DiscoveryState, OperationDiscovery};
use std::time::{Duration, Instant};

/// Discovery callbacks coalesce status updates; tests observe every callback instead
/// of sleeping across the interval.
const DISCOVERY_PUBLISH_INTERVAL: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(50)
};

impl Job {
    pub(in crate::copy_operations) fn prepare_discovery(&mut self) {
        // A streamed selection totals its items as the job reaches them.
        if self.streamed.is_some() {
            self.start_streamed_discovery();
            return;
        }
        // Native rename must stay constant-time. Mixed batches use selected roots too
        // rather than disguise unscanned descendants or native moves as copied work.
        let native_rename_candidate = self.move_work.as_ref().is_some_and(|work| {
            self.roots.iter().any(|source| {
                work.entry_removal(&**source)
                    .native_move_identity(source.native_identity())
                    .is_some_and(|identity| identity.device == self.destination.identity().device)
            })
        });
        let scope = if native_rename_candidate {
            DiscoveryScope::SelectedRoots
        } else {
            DiscoveryScope::Recursive
        };
        self.discover_scope(scope);
    }
    #[cfg(test)]
    pub(in crate::copy_operations) fn discover_cross_device_fixture(&mut self) {
        if self.streamed.is_some() {
            self.progress.discovery = OperationDiscovery::scanning(DiscoveryScope::Recursive);
            self.store_progress();
            return;
        }
        // The fixture injects CrossDevice eligibility while preserving real native
        // identities on one owned volume. Production scope selection stays above.
        self.discover_scope(DiscoveryScope::Recursive);
    }
    fn discover_scope(&mut self, scope: DiscoveryScope) {
        let mut discovery = OperationDiscovery::scanning(scope);
        self.progress.discovery = discovery.clone();
        self.store_progress();
        let mut budget = work_budget::Budget::discovery();
        let uses_directory_projection =
            scope == DiscoveryScope::Recursive && self.links.materializes_directory_links();
        let mut published = Instant::now();
        // Optional discovery runs inside an admitted blocking step. A pause
        // interrupts it so the outer worker can release the slot and acknowledge.
        let traversal = self.cancel.child_interrupted_by_pause();
        for source in &self.roots {
            if traversal.is_cancelled() {
                discovery.unavailable(if self.cancel.is_cancelled() {
                    "Transfer discovery cancelled."
                } else {
                    "Transfer discovery interrupted by pause."
                });
                break;
            }
            let mut unprojected_link = false;
            // A link that the policy recreates as a link is an expected leaf.
            let recreated_links = self.links.recreates_links();
            let mut visit = |item: &MutationItem| {
                unprojected_link |= uses_directory_projection
                    && !recreated_links
                    && link_job::is_link_kind(item.kind);
                discovery.observe(item);
                budget.observe(item, self.verification);
                if published.elapsed() >= DISCOVERY_PUBLISH_INTERVAL {
                    let mut status = self
                        .status
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    status.discovery = discovery.clone();
                    status.current_path = Some(item.path.clone());
                    published = Instant::now();
                }
            };
            let result = if scope == DiscoveryScope::SelectedRoots {
                source
                    .revalidate(&traversal)
                    .map(|()| visit(&source.entry().item))
            } else {
                source.discover(&traversal, &mut visit)
            };
            if let Err(failure) = result {
                if failure.kind == std::io::ErrorKind::Interrupted && self.cancel.pause_requested()
                {
                    discovery.unavailable("Transfer discovery interrupted by pause.");
                } else {
                    discovery.unavailable(&format!(
                        "Metadata discovery unavailable: {}",
                        failure.message
                    ));
                }
                break;
            }
            if unprojected_link {
                discovery.unavailable(
                    "Recursive totals are unavailable for an unprojected source link.",
                );
                break;
            }
        }
        discovery.finish();
        self.progress.discovery = discovery;
        self.work_budget = budget;
        self.progress.current_path = None;
        self.store_progress();
    }
    pub(super) fn check_discovery_progress(&mut self) {
        let discovery = &mut self.progress.discovery;
        if discovery.state != DiscoveryState::Complete
            || discovery.scope != DiscoveryScope::Recursive
        {
            return;
        }
        if self.progress.files_completed > discovery.files
            || self.progress.completed_directories > discovery.directories
            || discovery.bytes.is_some_and(|bytes| {
                self.progress
                    .completed_bytes
                    .checked_add(self.progress.current_file_copied)
                    .is_none_or(|done| done > bytes)
            })
        {
            discovery
                .unavailable("The source changed after discovery; total progress is unavailable.");
        }
    }
    pub(super) fn finish_discovery(&mut self) {
        let discovery = &mut self.progress.discovery;
        if discovery.state == DiscoveryState::Complete
            && discovery.scope == DiscoveryScope::Recursive
            && self.progress.failed == 0
            && self.progress.skipped == 0
            && !self.cancel.is_cancelled()
            && (self.progress.files_completed != discovery.files
                || self.progress.completed_directories != discovery.directories
                || discovery
                    .bytes
                    .is_some_and(|bytes| bytes != self.progress.completed_bytes))
        {
            discovery.unavailable(
                "Completed work differs from the metadata discovery snapshot; the source changed.",
            );
        }
    }
}
