//! Session-only display history. Clearing receipts never dispatches work or cancellation.
use super::*;
use std::collections::HashSet;

impl Presenter {
    pub(super) fn forgotten_operation(&self, id: OperationId) -> bool {
        id.0 <= self.operations.forgotten_through
            && !self.operations.progress.iter().any(|p| p.id == id)
    }
    pub(super) fn forget_unconfirmed_operation(&mut self, id: OperationId) {
        self.forget_operations(&HashSet::from([id]));
    }
    fn forget_operations(&mut self, removed: &HashSet<OperationId>) {
        let Some(highest) = removed.iter().map(|id| id.0).max() else {
            return;
        };
        self.operations.forgotten_through = self.operations.forgotten_through.max(highest);
        // Scan each session collection once, even when thousands of receipts retire.
        self.operations
            .progress
            .retain(|p| !removed.contains(&p.id));
        self.operations
            .unconfirmed
            .retain(|id| !removed.contains(id));
        let results: Vec<_> = self
            .operations
            .metadata_results
            .extract_if(|id, _| removed.contains(id))
            .map(|(_, result)| result)
            .collect();
        if !results.is_empty() {
            self.retired.push(Box::new(results));
        }
        self.operations
            .metadata_conflicts
            .retain(|p| !removed.contains(&p.id));
        self.retire_rename_decisions(|id| removed.contains(&id));
        if self
            .operations
            .pending
            .is_some_and(|id| removed.contains(&id))
        {
            self.operations.pending = None;
            self.operations.preview = None;
        }
        if self.metadata.id.is_some_and(|id| removed.contains(&id)) {
            self.retire_metadata();
        }
    }
    /// `without_errors` retains terminal failures and outstanding recovery problems.
    pub(crate) fn clear_finished_jobs(&mut self, without_errors: bool) {
        self.copies.progress.retain(|p| {
            !p.state.is_terminal()
                || (without_errors
                    && (p.failed > 0
                        || !p.errors.is_empty()
                        || p.source_residual_count > 0
                        || p.source_removal_durability_unconfirmed > 0
                        || matches!(
                            p.state,
                            CopyOperationState::Failed | CopyOperationState::CompletedWithErrors
                        )))
        });
        self.trim_copy_history();
        let removed: HashSet<_> = self
            .operations
            .progress
            .iter()
            .filter(|p| {
                p.state.is_terminal()
                    && (!without_errors
                        || (p.failed == 0
                            && p.errors.is_empty()
                            && !matches!(
                                p.state,
                                OperationState::Failed | OperationState::CompletedWithErrors
                            )))
            })
            .map(|p| p.id)
            .collect();
        self.forget_operations(&removed);
        self.clear_finished_command_jobs(without_errors);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    fn mutation(id: u64, state: OperationState) -> OperationProgress {
        OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(id),
            kind: MutationKind::Delete(DeleteMode::Permanent),
            state,
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
        }
    }
    #[test]
    fn metadata_job_history_keeps_details_until_explicit_clear() {
        let mut p = Presenter::new("left".into(), "right".into());
        for id in 1..=20 {
            let mut progress = mutation(id, OperationState::Completed);
            progress.kind = MutationKind::EditMetadata;
            if id == 2 {
                progress.state = OperationState::Failed;
                progress.failed = 1;
            }
            if id == 3 {
                progress.state = OperationState::Running;
            }
            p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
                progress,
            ))));
            p.apply(Event::Operation(OperationEvent::MetadataResult(Arc::new(
                MetadataResult {
                    id: OperationId(id),
                    processed: id,
                    changed: id,
                    skipped: 0,
                    failed: u64::from(id == 2),
                    sample: vec![],
                },
            ))));
        }
        let rows = crate::job_presentation::rows(&p);
        assert_eq!(rows.len(), 20);
        for id in 1..=20 {
            let row = rows
                .iter()
                .find(|row| row.identity.as_str() == format!("metadata-job:{id}"))
                .unwrap();
            assert!(
                row.detail
                    .contains(&format!("{id} examined · {id} changed")),
                "details of metadata operation {id} must survive later operations"
            );
        }
        let replacement = Arc::new(MetadataResult {
            id: OperationId(1),
            processed: 2,
            changed: 2,
            skipped: 0,
            failed: 0,
            sample: vec![],
        });
        p.apply(Event::Operation(OperationEvent::MetadataResult(
            replacement.clone(),
        )));
        assert_eq!(p.operations.metadata_results.len(), 20);
        assert!(
            crate::job_presentation::rows(&p)
                .iter()
                .find(|row| row.identity.as_str() == "metadata-job:1")
                .unwrap()
                .detail
                .contains("2 examined · 2 changed")
        );
        p.clear_finished_jobs(true);
        assert_eq!(p.operations.metadata_results.len(), 2);
        let rows = crate::job_presentation::rows(&p);
        assert!(
            rows.iter()
                .any(|row| row.identity.as_str() == "metadata-job:2")
        );
        assert!(
            rows.iter()
                .any(|row| row.identity.as_str() == "metadata-job:3")
        );
        p.apply(Event::Operation(OperationEvent::MetadataResult(
            replacement,
        )));
        assert_eq!(
            p.operations.metadata_results.len(),
            2,
            "late cleared result stays retired"
        );
        p.clear_finished_jobs(false);
        assert_eq!(p.operations.metadata_results.len(), 1);
        assert_eq!(
            crate::job_presentation::rows(&p)[0].identity.as_str(),
            "metadata-job:3"
        );
    }

    #[test]
    fn metadata_job_payloads_are_reclaimed_off_ui_after_replace_clear_shutdown_or_late_delivery() {
        for cause in 0..4 {
            let mut p = Presenter::new("left".into(), "right".into());
            let mut progress = mutation(1, OperationState::Completed);
            progress.kind = MutationKind::EditMetadata;
            p.operations.progress.push(Arc::new(progress));
            if cause == 3 {
                p.clear_finished_jobs(false);
            }
            let result = Arc::new(MetadataResult {
                id: OperationId(1),
                processed: 1,
                changed: 1,
                skipped: 0,
                failed: 0,
                sample: vec![MetadataOutcome::unchanged("retained-details".into())],
            });
            let weak = Arc::downgrade(&result);
            p.apply(Event::Operation(OperationEvent::MetadataResult(result)));
            match cause {
                0 => p.apply(Event::Operation(OperationEvent::MetadataResult(Arc::new(
                    MetadataResult {
                        id: OperationId(1),
                        processed: 2,
                        changed: 2,
                        skipped: 0,
                        failed: 0,
                        sample: vec![],
                    },
                )))),
                1 => p.clear_finished_jobs(false),
                2 => p.retire_all(),
                _ => {}
            }
            assert!(
                weak.upgrade().is_some(),
                "metadata payload dropped on UI for case {cause}"
            );
            let retired = p.take_retired();
            std::thread::spawn(move || drop(retired)).join().unwrap();
            assert!(
                weak.upgrade().is_none(),
                "retired metadata is no longer cached"
            );
        }
    }

    #[test]
    fn jobs_clear_finished_without_errors_keeps_failures_and_every_unfinished_state() {
        let mut p = Presenter::new("left".into(), "right".into());
        for (index, state) in [
            OperationState::Preparing,
            OperationState::AwaitingConfirmation,
            OperationState::Queued,
            OperationState::Running,
            OperationState::CancelRequested,
            OperationState::Cancelled,
            OperationState::Completed,
            OperationState::CompletedWithErrors,
            OperationState::Failed,
        ]
        .into_iter()
        .enumerate()
        {
            p.operations
                .progress
                .push(Arc::new(mutation(index as u64 + 1, state)));
        }
        for (index, state) in [
            CopyOperationState::Queued,
            CopyOperationState::Preparing,
            CopyOperationState::Running,
            CopyOperationState::WaitingForDecision,
            CopyOperationState::CancelRequested,
            CopyOperationState::Cancelled,
            CopyOperationState::Completed,
            CopyOperationState::CompletedWithErrors,
            CopyOperationState::Failed,
        ]
        .into_iter()
        .enumerate()
        {
            let mut progress = CopyProgress::queued(
                OperationId(index as u64 + 20),
                "target".into(),
                1,
                if index % 2 == 0 {
                    TransferIntent::Copy
                } else {
                    TransferIntent::Move
                },
            );
            progress.state = state;
            p.copies.progress.push(Arc::new(progress));
        }
        let mut cancelled_error = mutation(10, OperationState::Cancelled);
        cancelled_error.errors.push(FsError::from_io(
            "cleanup",
            std::path::Path::new("retained"),
            std::io::Error::other("cleanup failed"),
        ));
        p.operations.progress.push(Arc::new(cancelled_error));
        let mut residual =
            CopyProgress::queued(OperationId(30), "target".into(), 1, TransferIntent::Move);
        residual.state = CopyOperationState::Cancelled;
        residual.source_residual_count = 1;
        p.copies.progress.push(Arc::new(residual));
        p.clear_finished_jobs(true);
        assert_eq!(
            p.operations
                .progress
                .iter()
                .map(|p| p.id.0)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 8, 9, 10]
        );
        assert_eq!(
            p.copies.progress.iter().map(|p| p.id.0).collect::<Vec<_>>(),
            [20, 21, 22, 23, 24, 27, 28, 30]
        );
        p.clear_finished_jobs(false);
        assert_eq!(p.operations.progress.len(), 5);
        assert_eq!(p.copies.progress.len(), 5);
        assert!(p.operations.progress.iter().all(|p| !p.state.is_terminal()));
        assert!(p.copies.progress.iter().all(|p| !p.state.is_terminal()));
        // Late terminal receipts cannot resurrect a row the user explicitly cleared.
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
            mutation(10, OperationState::Cancelled),
        ))));
        assert_eq!(p.operations.progress.len(), 5);
        let mut finished = mutation(1, OperationState::Completed);
        finished.completed = 1;
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
            finished,
        ))));
        assert_eq!(p.operations.progress[0].state, OperationState::Completed);
    }
    #[test]
    fn jobs_clear_large_mixed_history_retires_related_receipts_once_and_preserves_live_authority() {
        let mut p = Presenter::new("left".into(), "right".into());
        p.operations.progress = vec![
            Arc::new(mutation(1, OperationState::AwaitingConfirmation)),
            Arc::new(mutation(2, OperationState::Failed)),
            Arc::new(mutation(3, OperationState::Running)),
        ];
        p.operations
            .progress
            .extend((100..2148).map(|id| Arc::new(mutation(id, OperationState::Completed))));
        let preview = Arc::new(OperationPreview {
            id: OperationId(1),
            kind: MutationKind::EditMetadata,
            files: 1,
            folders: 0,
            file_bytes: Some(1),
            sample: vec![],
        });
        p.operations.pending = Some(OperationId(1));
        p.operations.preview = Some(preview.clone());
        p.operations
            .unconfirmed
            .extend([OperationId(1), OperationId(100)]);
        p.metadata.id = Some(OperationId(1));
        p.metadata.error = "retained editor".into();
        for id in [OperationId(1), OperationId(100)] {
            p.operations.metadata_results.insert(
                id,
                Arc::new(MetadataResult {
                    id,
                    processed: 1,
                    changed: 1,
                    skipped: 0,
                    failed: 0,
                    sample: vec![],
                }),
            );
            p.operations
                .metadata_conflicts
                .push(Arc::new(MetadataConflict {
                    id,
                    token: 1,
                    source: "source".into(),
                    destination: "target".into(),
                }));
        }
        for id in [OperationId(3), OperationId(101)] {
            let item = MutationItem {
                path: "source".into(),
                kind: EntryKind::File,
                metadata: Default::default(),
            };
            p.operations.rename_conflicts.push(Arc::new(CopyConflict {
                id,
                version: 7,
                source: item.clone(),
                target: item,
                destination: "target".into(),
                can_replace: true,
                can_merge: false,
            }));
            p.operations.rename_versions.insert(id, 7);
            p.operations.rename_deferred.insert((id, 7));
        }
        p.operations.rename_selected = Some((OperationId(3), 7));
        p.clear_finished_jobs(true);
        assert_eq!(
            p.operations
                .progress
                .iter()
                .map(|p| p.id.0)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(p.operations.forgotten_through, 2147);
        assert_eq!(p.operations.unconfirmed, HashSet::from([OperationId(1)]));
        assert_eq!(p.operations.pending, Some(OperationId(1)));
        assert!(Arc::ptr_eq(
            p.operations.preview.as_ref().unwrap(),
            &preview
        ));
        assert_eq!(p.metadata.error, "retained editor");
        assert_eq!(
            p.operations
                .metadata_results
                .values()
                .map(|r| r.id.0)
                .collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(
            p.operations
                .metadata_conflicts
                .iter()
                .map(|r| r.id.0)
                .collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(
            p.operations
                .rename_conflicts
                .iter()
                .map(|r| r.id.0)
                .collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(p.operations.rename_versions.len(), 1);
        assert_eq!(
            p.operations.rename_deferred,
            HashSet::from([(OperationId(3), 7)])
        );
        assert_eq!(p.operations.rename_selected, Some((OperationId(3), 7)));
        // Low live IDs survive the high-water mark and can finish after the batch.
        for id in [1, 3] {
            p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
                mutation(id, OperationState::Completed),
            ))));
        }
        p.clear_finished_jobs(false);
        assert!(p.operations.progress.is_empty());
        assert!(p.operations.pending.is_none() && p.operations.preview.is_none());
        assert!(p.metadata.id.is_none());
        assert!(p.operations.unconfirmed.is_empty());
        assert!(
            p.operations.metadata_results.is_empty() && p.operations.metadata_conflicts.is_empty()
        );
        assert!(
            p.operations.rename_conflicts.is_empty() && p.operations.rename_versions.is_empty()
        );
        assert!(p.operations.rename_deferred.is_empty() && p.operations.rename_selected.is_none());
        p.apply(Event::Operation(OperationEvent::Progress(Arc::new(
            mutation(2147, OperationState::Completed),
        ))));
        assert!(
            p.operations.progress.is_empty(),
            "a late cleared receipt cannot resurrect history"
        );
    }
}
