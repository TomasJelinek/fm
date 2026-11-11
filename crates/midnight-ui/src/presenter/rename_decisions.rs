//! Rename shares the overwrite window while retaining its own job/version authority.
use super::*;

impl Presenter {
    pub(super) fn apply_rename_conflict(&mut self, conflict: Arc<CopyConflict>) {
        if self.operations.closing
            || !self.operations.progress.iter().any(|p| {
                p.id == conflict.id
                    && p.kind == MutationKind::Rename
                    && !p.state.is_terminal()
                    && p.state != OperationState::CancelRequested
            })
            || self
                .operations
                .rename_versions
                .get(&conflict.id)
                .is_some_and(|v| *v >= conflict.version)
        {
            return;
        }
        self.operations
            .rename_versions
            .insert(conflict.id, conflict.version);
        self.operations
            .rename_conflicts
            .retain(|c| c.id != conflict.id);
        self.operations
            .rename_deferred
            .retain(|(id, _)| *id != conflict.id);
        if self
            .operations
            .rename_selected
            .is_some_and(|(id, _)| id == conflict.id)
        {
            self.operations.rename_selected = Some((conflict.id, conflict.version));
        }
        self.operations.rename_conflicts.push(conflict);
    }
    pub(crate) fn active_rename_decision(&self) -> Option<(OperationId, u64)> {
        let pending = || {
            self.operations
                .rename_conflicts
                .iter()
                .map(|c| (c.id, c.version))
                .filter(|key| !self.operations.rename_deferred.contains(key))
        };
        self.operations
            .rename_selected
            .filter(|key| pending().any(|item| item == *key))
            .or_else(|| {
                self.active_copy_decision()
                    .is_none()
                    .then(|| pending().next())
                    .flatten()
            })
    }
    pub(super) fn retire_rename_decision(&mut self, id: OperationId) {
        self.retire_rename_decisions(|job| job == id);
    }
    pub(super) fn retire_rename_decisions(&mut self, removed: impl Fn(OperationId) -> bool) {
        self.operations.rename_conflicts.retain(|c| !removed(c.id));
        self.operations
            .rename_versions
            .retain(|id, _| !removed(*id));
        self.operations
            .rename_deferred
            .retain(|(id, _)| !removed(*id));
        if self
            .operations
            .rename_selected
            .is_some_and(|(id, _)| removed(id))
        {
            self.operations.rename_selected = None;
        }
    }
    pub(super) fn defer_rename_decision(&mut self, token: &str) -> bool {
        let Some(key) = self
            .operations
            .rename_conflicts
            .iter()
            .find(|c| token == format!("rename:{}:{}", c.id.0, c.version))
            .map(|c| (c.id, c.version))
        else {
            return false;
        };
        self.operations.rename_deferred.insert(key);
        self.operations.rename_selected = None;
        true
    }
    pub(super) fn resolve_rename_decision(&mut self, token: &str) -> bool {
        let Some(key) = self
            .operations
            .rename_conflicts
            .iter()
            .find(|c| token == format!("rename-job:{}", c.id.0))
            .map(|c| (c.id, c.version))
        else {
            return false;
        };
        self.operations.rename_deferred.remove(&key);
        self.operations.rename_selected = Some(key);
        true
    }
    pub(super) fn decide_rename(
        &mut self,
        token: &str,
        choice: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(index) = self
            .operations
            .rename_conflicts
            .iter()
            .position(|c| token == format!("rename:{}:{}", c.id.0, c.version))
        else {
            return;
        };
        let conflict = &self.operations.rename_conflicts[index];
        let choice = match choice {
            0 | 1 => CopyChoice::Skip,
            3 => CopyChoice::Cancel,
            4 | 5 if conflict.can_replace => CopyChoice::ReplaceRecoverable { all: false },
            _ => return,
        };
        let id = conflict.id;
        let cancelling = matches!(choice, CopyChoice::Cancel | CopyChoice::Skip);
        if send(Command::DecideRename(CopyDecision {
            id,
            version: conflict.version,
            choice,
        })) {
            self.operations.rename_conflicts.remove(index);
            self.operations.rename_selected = None;
            if cancelling
                && let Some(progress) = self.operations.progress.iter_mut().find(|p| p.id == id)
            {
                Arc::make_mut(progress).state = OperationState::CancelRequested;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    fn presenter() -> Presenter {
        let mut p = Presenter::new("left".into(), "right".into());
        p.operations.progress.push(Arc::new(OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(3),
            kind: MutationKind::Rename,
            state: OperationState::Running,
            total: 1,
            completed: 0,
            failed: 0,
            processed_entries: 0,
            throughput: Arc::default(),
            current_path: None,
            message: String::new(),
            errors: vec![],
            source_directory: "left".into(),
            renamed_to: None,
        }));
        p
    }
    fn conflict(version: u64) -> Arc<CopyConflict> {
        Arc::new(CopyConflict {
            id: OperationId(3),
            version,
            source: MutationItem {
                path: "left/source".into(),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            },
            target: MutationItem {
                path: "left/existing".into(),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            },
            destination: "left/existing".into(),
            can_replace: true,
            can_merge: false,
        })
    }
    #[test]
    fn rename_decision_rejects_stale_and_replayed_consent_and_preserves_failed_send() {
        let mut p = presenter();
        p.apply_rename_conflict(conflict(2));
        let sent = RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        p.decide_copy("rename:3:1", 4, String::new(), &send);
        p.decide_copy("copy:3:2", 4, String::new(), &send);
        p.decide_copy("rename:3:2", 6, String::new(), &send);
        assert!(sent.borrow().is_empty());
        p.decide_copy("rename:3:2", 4, String::new(), &|_| false);
        assert_eq!(p.operations.rename_conflicts.len(), 1);
        p.decide_copy("rename:3:2", 5, String::new(), &send);
        p.decide_copy("rename:3:2", 4, String::new(), &send);
        assert_eq!(sent.borrow().len(), 1);
        assert!(matches!(
            &sent.borrow()[0],
            Command::DecideRename(CopyDecision {
                id: OperationId(3),
                version: 2,
                choice: CopyChoice::ReplaceRecoverable { all: false }
            })
        ));
        p.apply_rename_conflict(conflict(2));
        assert!(p.operations.rename_conflicts.is_empty());
        p.apply_rename_conflict(conflict(3));
        assert_eq!(p.active_rename_decision(), Some((OperationId(3), 3)));
    }
    #[test]
    fn rename_decision_can_be_deferred_resolved_cancelled_and_retired() {
        let mut p = presenter();
        p.apply_rename_conflict(conflict(1));
        p.defer_copy_decision("rename:3:1");
        assert_eq!(p.active_rename_decision(), None);
        p.resolve_copy_decision("rename-job:3");
        assert_eq!(p.active_rename_decision(), Some((OperationId(3), 1)));
        p.decide_copy("rename:3:1", 3, String::new(), &|command| {
            assert!(matches!(
                command,
                Command::DecideRename(CopyDecision {
                    choice: CopyChoice::Cancel,
                    ..
                })
            ));
            true
        });
        assert_eq!(
            p.operations.progress[0].state,
            OperationState::CancelRequested
        );
        p.apply_rename_conflict(conflict(2));
        assert!(p.operations.rename_conflicts.is_empty());
        let mut done = (*p.operations.progress[0]).clone();
        done.state = OperationState::Cancelled;
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(done))));
        p.apply_rename_conflict(conflict(3));
        assert!(p.operations.rename_conflicts.is_empty());
        assert!(p.operations.rename_versions.is_empty());
    }
    #[test]
    fn rename_job_cancel_and_shutdown_use_mutation_authority() {
        let mut p = presenter();
        p.apply_rename_conflict(conflict(1));
        p.cancel_operation("rename-job:3", &|command| {
            assert!(matches!(command, Command::CancelOperation(OperationId(3))));
            true
        });
        p.request_operation_shutdown(&|_| true);
        assert!(p.operations.rename_conflicts.is_empty());
        p.apply_rename_conflict(conflict(2));
        assert_eq!(p.active_rename_decision(), None);
    }
}
