//! A metadata snapshot improves progress without becoming execution authority.
use super::*;
use midnight_core::{DiscoveryScope, DiscoveryState, OperationDiscovery};
use std::time::{Duration, Instant};

impl Job {
    pub(in crate::copy_operations) fn prepare_discovery(&mut self) {
        // Native rename must stay constant-time. Mixed batches use selected roots too
        // rather than disguise unscanned descendants or native moves as copied work.
        let native_rename_candidate = self.move_work.is_some()
            && self.roots.iter().any(|source| {
                source
                    .native_identity()
                    .is_some_and(|identity| identity.device == self.destination.identity().device)
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
        // The fixture injects CrossDevice eligibility while preserving real native
        // identities on one owned volume. Production scope selection stays above.
        self.discover_scope(DiscoveryScope::Recursive);
    }
    fn discover_scope(&mut self, scope: DiscoveryScope) {
        let mut discovery = OperationDiscovery::scanning(scope);
        self.progress.discovery = discovery.clone();
        self.store_progress();
        let mut budget = work_budget::Budget::discovery();
        let mut published = Instant::now();
        for source in &self.roots {
            if !self.cancel.wait_if_paused() {
                // Keep the owned Job on its normal finish path: preparation may already
                // retain metadata stages or Move cleanup authority.
                discovery.unavailable("Transfer discovery cancelled.");
                break;
            }
            let mut visit = |item: &MutationItem| {
                discovery.observe(item);
                budget.observe(item, self.verification);
                if published.elapsed() >= Duration::from_millis(50) {
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
                    .revalidate(&self.cancel)
                    .map(|()| visit(&source.entry().item))
            } else {
                source.discover(&self.cancel, &mut visit)
            };
            if let Err(failure) = result {
                discovery.unavailable(&format!(
                    "Metadata discovery unavailable: {}",
                    failure.message
                ));
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
