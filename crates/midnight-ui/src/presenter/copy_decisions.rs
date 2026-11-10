//! Versioned copy decisions share scheduling, but error and name-conflict policies stay separate.
use super::*;

impl Presenter {
    pub(super) fn accept_copy_decision(&mut self, id: OperationId, version: u64) -> bool {
        if self.operations.closing
            || !self.copies.progress.iter().any(|p| {
                p.id == id
                    && !p.state.is_terminal()
                    && p.state != CopyOperationState::CancelRequested
            })
            || self
                .copies
                .decision_versions
                .get(&id)
                .is_some_and(|old| *old >= version)
        {
            return false;
        }
        let active = self.active_copy_decision();
        self.copies.decision_versions.insert(id, version);
        self.copies.conflicts.retain(|c| c.id != id);
        self.copies.failures.retain(|f| f.id != id);
        self.copies.deferred_decisions.retain(|(job, _)| *job != id);
        self.copies.selected_decision = Some(
            active
                .filter(|(job, _)| *job != id)
                .unwrap_or((id, version)),
        );
        true
    }

    pub(crate) fn active_copy_decision(&self) -> Option<(OperationId, u64)> {
        let pending = || {
            self.copies
                .conflicts
                .iter()
                .map(|c| (c.id, c.version))
                .chain(self.copies.failures.iter().map(|f| (f.id, f.version)))
                .filter(|key| !self.copies.deferred_decisions.contains(key))
        };
        self.copies
            .selected_decision
            .filter(|key| pending().any(|item| item == *key))
            .or_else(|| pending().next())
    }

    pub(crate) fn defer_copy_decision(&mut self, token: &str) {
        let key = self
            .copies
            .conflicts
            .iter()
            .find(|c| token == format!("copy:{}:{}", c.id.0, c.version))
            .map(|c| (c.id, c.version))
            .or_else(|| {
                self.copies
                    .failures
                    .iter()
                    .find(|f| token == format!("copy-error:{}:{}", f.id.0, f.version))
                    .map(|f| (f.id, f.version))
            });
        if let Some(key) = key {
            self.copies.deferred_decisions.insert(key);
            self.copies.selected_decision = None;
        }
    }

    pub(crate) fn resolve_copy_decision(&mut self, token: &str) {
        let key = self
            .copies
            .conflicts
            .iter()
            .map(|c| (c.id, c.version))
            .chain(self.copies.failures.iter().map(|f| (f.id, f.version)))
            .find(|(id, _)| token == format!("copy-job:{}", id.0));
        if let Some(key) = key {
            self.copies.deferred_decisions.remove(&key);
            self.copies.selected_decision = Some(key);
        }
    }

    pub(super) fn decide_copy_failure(
        &mut self,
        token: &str,
        choice: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(index) = self
            .copies
            .failures
            .iter()
            .position(|f| token == format!("copy-error:{}:{}", f.id.0, f.version))
        else {
            return;
        };
        let failure = &self.copies.failures[index];
        let choice = match choice {
            8 if failure.can_retry => CopyChoice::Retry,
            0 if failure.can_skip => CopyChoice::Skip,
            9 if failure.can_skip => CopyChoice::SkipAllErrors,
            3 => CopyChoice::Cancel,
            _ => return,
        };
        let id = failure.id;
        let cancelling = matches!(choice, CopyChoice::Cancel);
        if send(Command::DecideCopy(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice,
        })) {
            self.copies.failures.remove(index);
            self.copies.selected_decision = None;
            if cancelling {
                self.mark_copy_cancel_requested(id);
            }
        }
    }
}
