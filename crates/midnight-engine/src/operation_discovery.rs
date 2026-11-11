//! Read-only discovery immediately before deletion; execution retains its existing pins.
use super::*;
use std::time::{Duration, Instant};

impl Job {
    pub(super) fn discover_deletion(&self, progress: &mut OperationProgress) {
        let MutationKind::Delete(mode) = progress.kind else {
            return;
        };
        // Trash publishes a selected namespace root as one operation. Descendant callbacks
        // and physical reclaimed-byte counts are not available from the OS trash API.
        let scope = if mode == DeleteMode::Trash {
            DiscoveryScope::SelectedRoots
        } else {
            DiscoveryScope::Recursive
        };
        progress.discovery = OperationDiscovery::scanning(scope);
        self.store_progress(progress);
        let mut published = Instant::now();
        for target in &self.targets {
            if !self.cancel.wait_if_paused() {
                break;
            }
            let mut visit = |item: &MutationItem| {
                progress.discovery.observe(item);
                progress.current_path = Some(item.path.clone());
                if published.elapsed() >= Duration::from_millis(50) {
                    self.store_progress(progress);
                    published = Instant::now();
                }
            };
            let result = if scope == DiscoveryScope::SelectedRoots {
                visit(target.item());
                Ok(())
            } else {
                target.discover(&self.cancel, &mut visit)
            };
            if let Err(error) = result {
                progress.discovery.unavailable(&format!(
                    "Metadata discovery unavailable: {}",
                    error.message
                ));
                break;
            }
        }
        if self.cancel.is_cancelled() {
            progress.discovery.unavailable("Discovery cancelled.");
        } else {
            progress.discovery.finish();
        }
        progress.current_path = None;
        self.store_progress(progress);
    }
}
