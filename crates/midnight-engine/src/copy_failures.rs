//! Versioned failure pauses. These transitions run only on the transfer worker.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Context {
    Entry,
    /// The top folder itself cannot be read further; Skip leaves out the rest of it.
    Enumeration,
    /// One child of the top folder cannot be opened, or in a Move prepared for Move
    /// (`Job::failed_child`); Skip leaves out only that child and the folder continues
    /// with the next one.
    Child,
    DirectoryRemoval,
    Move,
    /// An uncertain commit or cleanup of one entry (`Job::committed`). Its error and
    /// recovery facts are already in the job's results and its data stays where it
    /// is; Skip goes on with the next entry, and there is no Retry, which could
    /// replay the commit.
    Committed,
}

/// An entry whose change committed, or whose cleanup failed, without certainty: a
/// publication that needs recovery or whose durability is unconfirmed, a link stage
/// that cannot be cleaned up, a native Move rename that needs recovery, or the
/// cleanup or durability of a verified source removal. `Job::pause_committed`
/// records the error at once; the job then asks only whether to go on.
pub(super) struct Committed {
    pub(super) error: FsError,
    pub(super) stage: CopyErrorStage,
    pub(super) source: Option<std::path::PathBuf>,
    pub(super) destination: std::path::PathBuf,
    /// A Move entry whose source authority Skip releases.
    pub(super) entry: Option<(EntryRemoval, TransferIdentity)>,
    /// Whether the entry leaves something in a Move's source tree: its source, a private
    /// stage or recovery data. Skip then keeps the source folders above it. A removal whose
    /// durability alone is unconfirmed leaves nothing there.
    pub(super) leaves_source: bool,
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

    /// Records `committed`'s error with the job's results, and pauses on it before the
    /// next step (`Context::Committed`). A second one before that pause is only
    /// recorded: one question covers the entry.
    pub(super) fn pause_committed(&mut self, committed: Committed) {
        retain_error(&mut self.progress, committed.error.clone(), true);
        if self.committed.is_none() {
            self.committed = Some(committed);
        }
    }

    /// Asks about the pending uncertain commit or cleanup.
    pub(super) fn wait_for_committed(&mut self) -> Step {
        let error = self
            .committed
            .as_ref()
            .expect("pending committed failure")
            .error
            .clone();
        self.wait_for_failure(error, Context::Committed)
    }

    pub(super) fn wait_for_failure(&mut self, error: FsError, context: Context) -> Step {
        // A committed failure is already in the job's results.
        let recorded = matches!(context, Context::Committed);
        if self.cancel.is_cancelled() {
            if recorded {
                self.committed = None;
            } else if error.kind != io::ErrorKind::Interrupted {
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
                // Retry opens (or prepares for Move) the child again by its name. A Move
                // could not take the child from its folder before copying anything.
                (
                    if frame.removal == EntryRemoval::Ordinary {
                        CopyErrorStage::MovingSource
                    } else {
                        CopyErrorStage::Reading
                    },
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
            Context::Committed => {
                let committed = self.committed.as_ref().expect("committed failure");
                (committed.stage, committed.source.clone(), false, true)
            }
        };
        let can_retry = can_retry && retryable_error(&error);
        // A Move invariant failure has neither: the job ends.
        if !can_retry && !can_skip {
            self.record(error);
            return Step::Done;
        }
        let Some(version) = self.version.checked_add(1) else {
            if recorded {
                self.committed = None;
            } else {
                self.record(error);
            }
            return Step::Done;
        };
        self.version = version;
        let destination = match (context, &self.failed_child, self.stack.last()) {
            (Context::Child, Some(child), Some(frame)) => frame.destination.path().join(child),
            (Context::Committed, _, _) => self
                .committed
                .as_ref()
                .expect("committed failure")
                .destination
                .clone(),
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
        // An archive member whose password was not entered: the password was asked
        // once before the copy (or the archive has none), so the copy never asks per
        // member and skips it with that reason in its results.
        let password = midnight_core::is_archive_password_required(&failure.error);
        if can_skip && (password || self.skipped_errors.contains(&Scope::from(&failure))) {
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
        let recorded = matches!(self.failure_context, Context::Committed);
        match choice {
            CopyChoice::Cancel => {
                if recorded {
                    self.committed = None;
                } else {
                    self.record_decided_error(failure.error);
                }
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
                    Context::Committed => unreachable!("a committed failure cannot be retried"),
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
                if !recorded {
                    self.record_decided_error(failure.error);
                }
                self.progress.message.clear();
                match self.failure_context {
                    Context::Committed => self.go_on_after_committed(),
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
                        match result {
                            Ok(None) => {}
                            // The source is back; a cleanup after it failed.
                            Ok(Some(error)) => self.pause_committed(Committed {
                                error,
                                stage: CopyErrorStage::Cleanup,
                                source: failure.source,
                                destination: failure.destination,
                                entry: None,
                                leaves_source: true,
                            }),
                            Err(error) => self.record(error),
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

    /// Skip of an uncertain commit or cleanup: the entry stays as it is, and the job goes
    /// on with the next one. A Move releases the entry's source authority, and keeps the
    /// folders above it when the entry left something in them (its source, a private stage
    /// or recovery data), not after a removal whose durability alone is unconfirmed; a
    /// cleanup that then fails is asked about the same way.
    fn go_on_after_committed(&mut self) {
        let Some(committed) = self.committed.take() else {
            return;
        };
        let Some(move_work) = self.move_work.as_mut() else {
            return;
        };
        if committed.leaves_source {
            for frame in &mut self.stack {
                frame.retain_source = true;
            }
        }
        let Some((removal, identity)) = committed.entry else {
            return;
        };
        if let Err(error) = move_work.discard_entry(removal, identity) {
            self.pause_committed(Committed {
                error,
                stage: CopyErrorStage::Cleanup,
                entry: None,
                ..committed
            });
        }
    }

    /// Retry of a child that could not be opened, or prepared for Move: open or prepare
    /// it again by name. While it still fails, it stays the pending child.
    fn reopen_failed_child(&mut self) -> Result<(), FsError> {
        let child = self.failed_child.as_ref().expect("failed child");
        let frame = self.stack.last().expect("failed child's folder");
        if frame.removal == EntryRemoval::Ordinary {
            self.reprepared_child = Some(frame.source.prepare_move_child(
                child,
                self.links,
                &self.cancel,
            )?);
        } else {
            self.reopened_child = Some(frame.source.open_child(child, &self.cancel)?);
        }
        self.failed_child = None;
        Ok(())
    }
}
