//! One bounded native maintenance sweep; provider work still uses the pane lane.
use super::*;
use crate::providers::tree::ListingState;
use midnight_core::{TreeDiagnostic, TreeEvent, TreeRefreshRequest};
use std::time::Duration;

/// Cadence for retrying exclusive transfer-namespace admission. Matches the
/// transfers' own non-blocking admission retries.
pub(crate) const NAMESPACE_POLL: Duration = Duration::from_millis(25);

pub(super) struct Maintenance {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    pub(super) remaining: VecDeque<BranchId>,
    active: Option<BranchId>,
    // Display-relevant state of `active` before its listing was requested.
    before: Option<ListingState>,
    first_error: Option<TreeDiagnostic>,
    details: Vec<Arc<dyn midnight_core::GeneratedTextSource>>,
    failures: usize,
}

/// What the model worker waits for when queued work could not be admitted.
pub(crate) enum Wait {
    /// A pane-local permit; waiting for one cannot starve another consumer.
    Permit(Arc<Semaphore>),
    /// Exclusive transfer-namespace admission. Never queue on that fair
    /// semaphore: a waiter would reserve every released transfer share and
    /// block new transfers. Retry admission every [`NAMESPACE_POLL`] instead.
    Namespace,
}

impl Session {
    pub(crate) fn refresh_loaded(
        &mut self,
        request: &TreeRefreshRequest,
        cancel: &Cancellation,
    ) -> Result<(), ProviderError> {
        self.check_view(&request.view)?;
        check_cancel(cancel)?;
        if request.request.0 == 0 || request.request.0 <= self.last_request.0 {
            return Err(ProviderError::InvalidResponse(
                "refresh requires a fresh request identity",
            ));
        }
        // One reconciliation pass at a time; the UI keeps one request in flight.
        if self.maintenance.is_some() || self.refresh_pending.is_some() {
            return Err(ProviderError::ResourceLimited);
        }
        if !matches!(
            self.tree.root_target(cancel)?.value()?,
            crate::providers::ProviderTarget::Native(_)
        ) {
            return Err(ProviderError::Unsupported);
        }
        // A running branch job is not a refusal (the UI would retry in a busy loop):
        // the pass starts once that listing has been installed, so it reconciles
        // that branch too. Queued explicit work interleaves with the pass.
        self.refresh_pending = Some((request.view.stamp, request.request));
        self.last_request = request.request;
        self.prefer_maintenance = true;
        if self.budget.branch.available_permits() > 0 {
            self.start_pending_refresh();
        }
        Ok(())
    }

    /// Called only while no branch job is in flight.
    pub(super) fn start_pending_refresh(&mut self) {
        if self.maintenance.is_some() {
            return;
        }
        let Some((stamp, request)) = self.refresh_pending.take() else {
            return;
        };
        self.maintenance = Some(Maintenance {
            stamp,
            request,
            remaining: self.tree.loaded_native_branches().into(),
            active: None,
            before: None,
            first_error: None,
            details: Vec::new(),
            failures: 0,
        });
    }

    pub(super) fn skip_removed_refresh_branches(&mut self) {
        if let Some(maintenance) = &mut self.maintenance {
            while maintenance
                .remaining
                .front()
                .is_some_and(|id| !self.tree.loaded_native_branch(*id))
            {
                maintenance.remaining.pop_front();
            }
        }
    }

    pub(super) fn next_is_maintenance(&self) -> bool {
        self.maintenance.as_ref().is_some_and(|maintenance| {
            maintenance.active.is_none()
                && !maintenance.remaining.is_empty()
                && !self.queue.iter().any(|pending| {
                    Some(&pending.branch) == maintenance.remaining.front()
                        && pending.kind != TreeRequest::Probe
                })
                && (self.prefer_maintenance || !self.explicit_work_ready())
        })
    }

    /// Select what actually blocked admission; hold no other permit while waiting.
    pub(crate) fn waiting_on(&self) -> Wait {
        if self.runnable_maintenance() && self.namespace_busy() {
            Wait::Namespace
        } else if self.budget.branch.available_permits() == 0 {
            Wait::Permit(self.budget.branch.clone())
        } else {
            Wait::Permit(self.budget.listing.clone())
        }
    }

    /// A transfer holds at least one namespace share.
    fn namespace_busy(&self) -> bool {
        self.budget.mutation.available_permits() < crate::transfer_leases::NAMESPACE_WEIGHT as usize
    }

    pub(super) fn runnable_maintenance(&self) -> bool {
        // Only maintenance needs the exclusive transfer namespace. While a transfer
        // holds it, other queued work (explicit, recursive and probes) proceeds.
        !self.recursive_batch_active()
            && self.next_is_maintenance()
            && !(self.namespace_busy() && (self.recursive_ready() || !self.queue.is_empty()))
    }

    pub(super) fn take_refresh_branch(&mut self) -> Result<PendingBranch, ProviderError> {
        let maintenance = self
            .maintenance
            .as_mut()
            .ok_or(ProviderError::StaleReference)?;
        let branch = maintenance
            .remaining
            .pop_front()
            .ok_or(ProviderError::StaleReference)?;
        // The full maintenance listing subsumes speculative disclosure probes.
        // Explicit same-branch work is selected first by next_is_maintenance.
        self.queue.retain(|pending| pending.branch != branch);
        let version = self.next_work;
        self.next_work = version
            .checked_add(1)
            .ok_or(ProviderError::ResourceLimited)?;
        self.versions.insert(branch, version);
        // Captured before the request clears a displayed failure of this branch.
        maintenance.before = self.tree.listing_state(branch);
        maintenance.active = Some(branch);
        self.prefer_maintenance = false;
        Ok(PendingBranch {
            branch,
            origin: WorkOrigin::Maintenance,
            kind: TreeRequest::Refresh,
            version,
            request: maintenance.request,
        })
    }

    pub(super) fn apply_refresh(
        &mut self,
        completion: BranchCompletion,
        cancel: &Cancellation,
    ) -> Result<bool, ProviderError> {
        let Some(maintenance) = self.maintenance.as_mut() else {
            return Ok(false);
        };
        if completion.stamp.session != maintenance.stamp.session
            || completion.stamp.navigation != maintenance.stamp.navigation
            || completion.stamp.pane != maintenance.stamp.pane
            || completion.pending.request != maintenance.request
        {
            return Ok(false);
        }
        maintenance.active = None;
        let before = maintenance.before.take();
        let branch = completion.pending.branch;
        if self.versions.get(&branch) != Some(&completion.pending.version) {
            if self.tree.loaded_native_branch(branch) {
                maintenance.remaining.push_front(branch);
            }
            return Ok(false);
        }
        self.versions.remove(&branch);
        let checkpoint = self.tree.checkpoint(branch);
        if !self.tree.apply(completion.result) {
            if self.tree.loaded_native_branch(branch) {
                maintenance.remaining.push_front(branch);
            }
            return Ok(false);
        }
        if Arc::ptr_eq(&self.built_sizes, &self.options.sizes)
            && let (Some(before), Some(after)) = (&before, self.tree.listing_state(branch))
            && before.unchanged(&after)
        {
            // Nothing a projection displays changed: skip the full rebuild and the
            // publication round trip. The pass end still settles watch state.
            return Ok(false);
        }
        let provider_error = self.tree.state(branch).and_then(|row| row.error);
        let result: Result<(), ProviderError> = (|| {
            let mut state = self.projection_state.clone();
            if branch == self.tree.root() && provider_error.is_none() {
                state.refresh_root_actions(&self.tree, cancel)?;
            }
            let projection = state.build(
                &self.tree,
                &self.projection.root,
                self.projection.stamp.session,
                Some(&self.projection),
                &self.options,
                cancel,
            )?;
            let selection = self.selection.rebased(projection.clone())?;
            check_cancel(cancel)?;
            self.projection_state = state;
            self.projection = projection;
            self.selection = selection;
            self.built_sizes = self.options.sizes.clone();
            Ok(())
        })();
        let error = match result {
            Ok(()) => provider_error,
            Err(error) => {
                self.tree.restore(checkpoint, branch, error.clone());
                // Keep the complete accepted publication if even a diagnostic projection is too large.
                let _ = self.update_projection(cancel);
                Some(error)
            }
        };
        if let Some(error) = error {
            self.refresh_failed(branch, error);
        }
        // The completion owns the mutation/listing permits through installation,
        // then drops them before the controller can wait on its event outbox.
        Ok(true)
    }

    fn refresh_failed(&mut self, branch: BranchId, error: ProviderError) {
        let diagnostic = self
            .tree
            .state(branch)
            .and_then(|row| row.error_details)
            .map_or_else(
                || crate::tree_diagnostics::diagnostic(&self.reports, &error),
                |source| TreeDiagnostic::with_details(&error.to_string(), source),
            );
        if let Some(maintenance) = &mut self.maintenance {
            maintenance.failures = maintenance.failures.saturating_add(1);
            if let Some(source) = diagnostic.details() {
                maintenance.details.push(source.clone());
            }
            maintenance.first_error.get_or_insert(diagnostic);
            maintenance
                .remaining
                .retain(|id| !in_subtree(&self.tree, *id, branch));
        }
    }

    pub(crate) fn finish_refresh(&mut self) -> Option<TreeEvent> {
        self.skip_removed_refresh_branches();
        if self.maintenance.as_ref().is_none_or(|maintenance| {
            maintenance.active.is_some() || !maintenance.remaining.is_empty()
        }) {
            return None;
        }
        let maintenance = self.maintenance.take()?;
        let result = maintenance.first_error.map_or(Ok(()), |error| {
            Err(TreeDiagnostic::with_details(
                &format!(
                    "{} ({} refresh failures)",
                    error.as_str(),
                    maintenance.failures
                ),
                crate::tree_diagnostics::composed(maintenance.details),
            ))
        });
        Some(TreeEvent::Refreshed {
            stamp: maintenance.stamp,
            request: maintenance.request,
            result,
        })
    }

    pub(crate) fn abort_refresh(&mut self, error: &ProviderError) -> Option<TreeEvent> {
        let mut maintenance = self.maintenance.take()?;
        let error = crate::tree_diagnostics::diagnostic(&self.reports, &error);
        if let Some(source) = error.details() {
            maintenance.details.push(source.clone());
        }
        Some(TreeEvent::Refreshed {
            stamp: maintenance.stamp,
            request: maintenance.request,
            result: Err(TreeDiagnostic::with_details(
                error.as_str(),
                crate::tree_diagnostics::composed(maintenance.details),
            )),
        })
    }
}
