//! Versioned failure pauses. These transitions run only on the transfer worker.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Context {
    Entry,
    Enumeration,
    DirectoryRemoval,
    Move,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Scope(CopyErrorStage, io::ErrorKind, &'static str);
impl From<&CopyFailure> for Scope {
    fn from(failure: &CopyFailure) -> Self {
        Self(failure.stage, failure.error.kind, failure.error.operation)
    }
}

impl Job {
    pub(crate) fn set_decision_version(&mut self, version: u64) {
        self.version = version;
    }

    pub(super) fn wait_for_failure(&mut self, error: FsError, context: Context) -> Step {
        if self.cancel.is_cancelled() {
            if error.kind != io::ErrorKind::Interrupted {
                self.record_cleanup_error(error);
            }
            return Step::Done;
        }
        let (stage, source, can_retry, can_skip) = match context {
            Context::Entry => {
                let work = self.current.as_ref().expect("failed entry remains owned");
                let stage = work.copy.as_ref().map_or_else(
                    || {
                        if let Some(link) = &work.link {
                            link.failure_stage()
                        } else if work.entry().item.kind == EntryKind::Directory {
                            CopyErrorStage::CreatingDirectory
                        } else {
                            CopyErrorStage::Preparing
                        }
                    },
                    VerifiedFileCopy::failure_stage,
                );
                (
                    stage,
                    Some(work.entry().item.path.clone()),
                    work.link.as_ref().map_or_else(
                        || {
                            work.copy.as_ref().map_or(work.source.is_some(), |copy| {
                                copy.can_retry()
                                    || matches!(
                                        copy.phase(),
                                        CopyPhase::Copying
                                            | CopyPhase::Syncing
                                            | CopyPhase::Verifying
                                            | CopyPhase::ReadyToPublish
                                    )
                            })
                        },
                        link_job::LinkCopy::can_retry,
                    ),
                    true,
                )
            }
            Context::Enumeration => (
                CopyErrorStage::Enumerating,
                self.stack
                    .last()
                    .map(|frame| frame.source.entry().item.path.clone()),
                false,
                true,
            ),
            Context::DirectoryRemoval => (
                CopyErrorStage::RemovingSource,
                self.stack
                    .last()
                    .map(|frame| frame.source.entry().item.path.clone()),
                true,
                true,
            ),
            Context::Move => (
                self.move_work.as_ref().map_or(
                    CopyErrorStage::RemovingSource,
                    move_job::MoveWork::failure_stage,
                ),
                self.progress.current_path.clone(),
                self.move_work
                    .as_ref()
                    .is_some_and(move_job::MoveWork::can_retry),
                self.move_work
                    .as_ref()
                    .is_some_and(move_job::MoveWork::can_skip_failure),
            ),
        };
        let can_retry = can_retry && retryable_error(&error);
        // There is no useful retry/skip after an uncertain native commit or cleanup.
        if !can_retry && !can_skip {
            self.record(error);
            return Step::Done;
        }
        let Some(version) = self.version.checked_add(1) else {
            self.record(error);
            return Step::Done;
        };
        self.version = version;
        let failure = CopyFailure {
            id: self.progress.id,
            version,
            error,
            source,
            destination: self.current.as_ref().map_or_else(
                || self.progress.destination_directory.clone(),
                |work| work.destination.path().join(&work.name),
            ),
            stage,
            can_retry,
            can_skip,
        };
        self.failure_context = context;
        self.progress.message = failure.error.to_string();
        if can_skip && self.skipped_errors.contains(&Scope::from(&failure)) {
            self.failure = Some(failure);
            self.decision = Some(CopyChoice::Skip);
            return Step::Continue;
        }
        self.failure = Some(failure.clone());
        Step::Failure(Arc::new(failure))
    }

    pub(super) fn advance_failure(&mut self) -> Step {
        let Some(choice) = self.decision.take() else {
            return Step::Failure(Arc::new(self.failure.as_ref().unwrap().clone()));
        };
        let failure = self.failure.take().expect("pending error decision");
        match choice {
            CopyChoice::Cancel => {
                self.record_decided_error(failure.error);
                self.cancel.cancel();
                Step::Done
            }
            CopyChoice::Retry if failure.can_retry => {
                let result = match self.failure_context {
                    Context::Entry => self
                        .current
                        .as_mut()
                        .and_then(|work| work.copy.as_mut())
                        .filter(|copy| copy.phase() == CopyPhase::Failed)
                        .map_or(Ok(()), |copy| copy.retry(&self.cancel)),
                    Context::Move => {
                        let mut work = self.move_work.take().expect("failed move remains owned");
                        let result = work.retry(self);
                        self.move_work = Some(work);
                        result
                    }
                    Context::Enumeration => unreachable!("enumeration cannot advertise retry"),
                    Context::DirectoryRemoval => Ok(()),
                };
                self.progress.message.clear();
                match result {
                    Ok(()) => Step::Continue,
                    Err(error) => self.wait_for_failure(error, self.failure_context),
                }
            }
            CopyChoice::Skip | CopyChoice::SkipAllErrors if failure.can_skip => {
                if matches!(choice, CopyChoice::SkipAllErrors) {
                    self.skipped_errors.insert(Scope::from(&failure));
                }
                self.record_decided_error(failure.error);
                self.progress.message.clear();
                match self.failure_context {
                    Context::Entry => self.skip_current(),
                    Context::Enumeration | Context::DirectoryRemoval => {
                        let frame = self.stack.pop().expect("failed enumeration frame");
                        for parent in &mut self.stack {
                            parent.retain_source = true;
                        }
                        if let Some(work) = self.move_work.as_mut()
                            && let Err(error) = work.discard(frame.source.entry().identity)
                        {
                            self.record(error);
                        }
                        self.progress.skipped += 1;
                    }
                    Context::Move => {
                        for frame in &mut self.stack {
                            frame.retain_source = true;
                        }
                        let mut work = self.move_work.take().expect("failed move remains owned");
                        let result = work.skip_failed(self);
                        self.move_work = Some(work);
                        if let Err(error) = result {
                            self.record(error);
                        }
                        self.progress.skipped += 1;
                    }
                }
                Step::Continue
            }
            _ => {
                self.failure = Some(failure.clone());
                Step::Failure(Arc::new(failure))
            }
        }
    }
}
