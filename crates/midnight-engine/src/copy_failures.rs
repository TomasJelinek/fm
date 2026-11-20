//! Versioned failure pauses. These transitions run only on the transfer worker.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Context {
    Entry,
    /// The top folder itself cannot be read further; Skip leaves out the rest of it.
    Enumeration,
    /// One child of the top folder cannot be opened (`Job::failed_child`); Skip
    /// leaves out only that child and the folder continues with the next one.
    Child,
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
                        } else if work.source_move_failed {
                            CopyErrorStage::MovingSource
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
            Context::Child => {
                let frame = self.stack.last().expect("failed child's folder");
                let child = self.failed_child.as_ref().expect("failed child");
                // Retry opens the child again by its name.
                (
                    CopyErrorStage::Reading,
                    Some(frame.source.entry().item.path.join(child)),
                    true,
                    true,
                )
            }
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
        let destination = match (context, &self.failed_child, self.stack.last()) {
            (Context::Child, Some(child), Some(frame)) => frame.destination.path().join(child),
            _ => self.current.as_ref().map_or_else(
                || self.progress.destination_directory.clone(),
                |work| work.destination.path().join(&work.name),
            ),
        };
        let failure = CopyFailure {
            id: self.progress.id,
            version,
            error,
            source,
            destination,
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
                if matches!(self.failure_context, Context::Entry)
                    && self
                        .current
                        .as_ref()
                        .is_some_and(|work| work.copy.is_some())
                    && let Err(error) = self.start_verification_pass()
                {
                    self.record(error);
                    return Step::Done;
                }
                let result = match self.failure_context {
                    Context::Entry => self
                        .current
                        .as_mut()
                        .and_then(|work| work.copy.as_mut())
                        .filter(|copy| copy.phase() == CopyPhase::Failed)
                        .map_or(Ok(()), |copy| copy.retry(&self.cancel)),
                    Context::Move => {
                        let mut work = ownership::Armed::new(
                            self.move_work.take().expect("failed move remains owned"),
                        );
                        let result = work.retry(self);
                        self.move_work = Some(work.into_inner());
                        result
                    }
                    Context::Enumeration => unreachable!("enumeration cannot advertise retry"),
                    Context::Child => self.reopen_failed_child(),
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
                    Context::Child => {
                        self.failed_child = None;
                        // The skipped child stays in its folder, so a Move keeps that
                        // folder and every folder above it.
                        for frame in &mut self.stack {
                            frame.retain_source = true;
                        }
                        self.progress.skipped += 1;
                    }
                    Context::Enumeration | Context::DirectoryRemoval => {
                        let frame = ownership::Armed::new(
                            self.stack.pop().expect("failed enumeration frame"),
                        );
                        for parent in &mut self.stack {
                            parent.retain_source = true;
                        }
                        if let Some(work) = self.move_work.as_mut()
                            && let Err(error) =
                                work.discard_entry(frame.removal, frame.source.entry().identity)
                        {
                            self.record(error);
                        }
                        drop(frame.into_inner());
                        self.progress.skipped += 1;
                    }
                    Context::Move => {
                        for frame in &mut self.stack {
                            frame.retain_source = true;
                        }
                        let mut work = ownership::Armed::new(
                            self.move_work.take().expect("failed move remains owned"),
                        );
                        let result = work.skip_failed(self);
                        self.move_work = Some(work.into_inner());
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

    /// Retry of a child that could not be opened: open it again by name. While it
    /// still fails, it stays the pending child.
    fn reopen_failed_child(&mut self) -> Result<(), FsError> {
        let child = self.failed_child.as_ref().expect("failed child");
        let frame = self.stack.last().expect("failed child's folder");
        self.reopened_child = Some(frame.source.open_child(child, &self.cancel)?);
        self.failed_child = None;
        Ok(())
    }
}
