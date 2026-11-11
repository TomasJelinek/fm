//! One automatic Jobs session retains outcomes independently of bounded display history.
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy)]
struct Session {
    owner: OperationId,
    owner_completed: bool,
    needs_attention: bool,
}

#[derive(Default)]
pub(super) struct AutomaticJobs {
    // Refresh synchronizes presentation ownership while borrowing the presenter immutably.
    session: Cell<Option<Session>>,
}

impl AutomaticJobs {
    pub(super) fn observe_copy(&self, progress: &CopyProgress) {
        let Some(mut session) = self.session.get() else {
            return;
        };
        if progress.state.is_terminal() {
            let successful = progress.state == CopyOperationState::Completed
                && progress.failed == 0
                && progress.errors.is_empty()
                && progress.source_residual_count == 0;
            if progress.id == session.owner {
                session.owner_completed = successful;
            }
            session.needs_attention |= !successful;
            self.session.set(Some(session));
        }
    }

    pub(super) fn observe_mutation(&self, progress: &OperationProgress) {
        if progress.state.is_terminal()
            && (progress.state != OperationState::Completed
                || progress.failed > 0
                || !progress.errors.is_empty())
        {
            self.retain_attention();
        }
    }

    pub(super) fn retain_attention(&self) {
        if let Some(mut session) = self.session.get() {
            session.needs_attention = true;
            self.session.set(Some(session));
        }
    }
}

impl Presenter {
    /// The window's automatic owner is authoritative; manual opening/closing clears it.
    pub(crate) fn sync_automatic_copy_jobs(&self, owner: Option<OperationId>) {
        let tracker = &self.copies.automatic_jobs;
        if tracker.session.get().map(|session| session.owner) == owner {
            return;
        }
        tracker.session.set(owner.map(|owner| Session {
            owner,
            owner_completed: false,
            needs_attention: false,
        }));
        if let Some(owner) = owner {
            // A refresh can open after several events in one UI tick. Include only this
            // session's already-observed outcomes, never unrelated old history.
            for progress in self.copies.progress.iter().filter(|p| p.id >= owner) {
                tracker.observe_copy(progress);
            }
            for progress in self
                .operations
                .progress
                .iter()
                .filter(|p| p.id >= owner && !self.operations.unconfirmed.contains(&p.id))
            {
                tracker.observe_mutation(progress);
            }
        }
    }

    pub(crate) fn may_close_automatic_copy_jobs(&self, id: OperationId) -> bool {
        self.copies
            .automatic_jobs
            .session
            .get()
            .is_some_and(|session| {
                session.owner == id && session.owner_completed && !session.needs_attention
            })
            && !self.has_live_operations()
            && self.copies.conflicts.is_empty()
            && self.copies.failures.is_empty()
            && self.operations.metadata_conflicts.is_empty()
            && self.operations.preview.is_none()
            && self.desktop.notice.is_none()
    }
}
