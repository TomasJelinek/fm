//! Resumable traversal and one verified file at a time. Every method runs on a worker.
use super::*;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use std::{
    collections::{HashSet, VecDeque},
    ffi::OsString,
    path::Component,
};

pub(super) enum Step {
    Continue,
    Conflict(Arc<CopyConflict>),
    Done,
}
struct Frame {
    source: Box<dyn TransferSource>,
    destination: Arc<dyn TransferDirectory>,
}
struct EntryWork {
    source: Option<Box<dyn TransferSource>>,
    copy: Option<VerifiedFileCopy>,
    destination: Arc<dyn TransferDirectory>,
    name: OsString,
}
impl EntryWork {
    fn entry(&self) -> &TransferEntry {
        if let Some(source) = &self.source {
            source.entry()
        } else {
            self.copy
                .as_ref()
                .expect("entry owns a source or verified copy")
                .source_entry()
        }
    }
}

pub(super) struct Job {
    status: Status,
    progress: CopyProgress,
    cancel: Cancellation,
    destination: Arc<dyn TransferDirectory>,
    roots: VecDeque<Box<dyn TransferSource>>,
    stack: Vec<Frame>,
    current: Option<EntryWork>,
    conflict: Option<CopyConflict>,
    decision: Option<CopyChoice>,
    version: u64,
    skip_all: bool,
}
impl Job {
    pub(super) fn prepare(
        request: CopyRequest,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        let destination = service.prepare_transfer_destination(&request.destination, &cancel)?;
        if destination.path() != request.destination {
            return Err(error(
                &request.destination,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        let mut seen = HashSet::new();
        let mut roots = VecDeque::new();
        for path in &request.clipboard.paths {
            if cancel.is_cancelled() {
                return Err(error(path, io::ErrorKind::Interrupted, "Copy cancelled."));
            }
            if !seen.insert(path) {
                continue;
            }
            let source = service.prepare_transfer_source(path, &cancel)?;
            if source.entry().item.path != *path {
                return Err(error(
                    path,
                    io::ErrorKind::InvalidInput,
                    "Provider returned a different source.",
                ));
            }
            if source.entry().item.kind == EntryKind::Directory {
                destination.reject_source_ancestor(source.entry().identity, &cancel)?;
            }
            roots.push_back(source);
        }
        Ok(Self::from_sources(destination, roots, cancel, status))
    }

    pub(super) fn prepare_archive(
        path: std::path::PathBuf,
        roots: VecDeque<Box<dyn TransferSource>>,
        service: &dyn TransferService,
        cancel: Cancellation,
        status: Status,
    ) -> Result<Self, FsError> {
        let destination = service.prepare_transfer_destination(&path, &cancel)?;
        if destination.path() != path {
            return Err(error(
                &path,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        Ok(Self::from_sources(destination, roots, cancel, status))
    }

    fn from_sources(
        destination: Arc<dyn TransferDirectory>,
        roots: VecDeque<Box<dyn TransferSource>>,
        cancel: Cancellation,
        status: Status,
    ) -> Self {
        Self {
            progress: snapshot(&status),
            status,
            cancel,
            destination,
            roots,
            stack: vec![],
            current: None,
            conflict: None,
            decision: None,
            version: 0,
            skip_all: false,
        }
    }

    pub(super) fn accept_decision(&mut self, decision: CopyDecision) -> bool {
        if self.conflict.as_ref().is_some_and(|conflict| {
            conflict.id == decision.id && conflict.version == decision.version
        }) {
            self.decision = Some(decision.choice);
            true
        } else {
            false
        }
    }

    pub(super) fn step(&mut self) -> Step {
        let step = self.advance();
        self.progress.state = if self.cancel.is_cancelled() {
            CopyOperationState::CancelRequested
        } else {
            CopyOperationState::Running
        };
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = self.progress.clone();
        step
    }
    fn advance(&mut self) -> Step {
        if self.cancel.is_cancelled() {
            return Step::Done;
        }
        if let Some(choice) = self.decision.take() {
            self.conflict = None;
            match choice {
                CopyChoice::Cancel => {
                    self.cancel.cancel();
                    return Step::Done;
                }
                CopyChoice::Skip | CopyChoice::SkipAll => {
                    self.skip_all |= matches!(choice, CopyChoice::SkipAll);
                    self.skip_current();
                    return Step::Continue;
                }
                CopyChoice::Rename(name) => {
                    let valid = !name.is_empty()
                        && !name.as_encoded_bytes().contains(&0)
                        && Path::new(&name).file_name() == Some(name.as_os_str())
                        && matches!(
                            (
                                Path::new(&name).components().next(),
                                Path::new(&name).components().count()
                            ),
                            (Some(Component::Normal(_)), 1)
                        );
                    if !valid {
                        self.record(error(
                            &self.progress.destination_directory,
                            io::ErrorKind::InvalidInput,
                            "Enter one native basename without dot or path components.",
                        ));
                        self.drop_current();
                        return Step::Continue;
                    }
                    if let Some(work) = self.current.as_mut() {
                        work.name = name;
                    }
                }
            }
        }
        if let Some(conflict) = &self.conflict {
            return Step::Conflict(Arc::new(conflict.clone()));
        }
        if let Some(work) = self.current.take() {
            return self.process(work);
        }
        self.progress.phase = CopyOperationPhase::Planning;
        self.progress.current_path = None;
        let next = if let Some(frame) = self.stack.last_mut() {
            match frame.source.next_child(&self.cancel) {
                Ok(Some(source)) => Some((source, frame.destination.clone())),
                Ok(None) => {
                    self.stack.pop();
                    return Step::Continue;
                }
                Err(error) => {
                    self.record(error);
                    self.stack.pop();
                    return Step::Continue;
                }
            }
        } else {
            self.roots
                .pop_front()
                .map(|source| (source, self.destination.clone()))
        };
        let Some((source, destination)) = next else {
            return Step::Done;
        };
        let path = &source.entry().item.path;
        let Some(name) = path.file_name() else {
            self.record(error(
                path,
                io::ErrorKind::InvalidInput,
                "A source basename is required.",
            ));
            return Step::Continue;
        };
        let name = name.to_owned();
        self.current = Some(EntryWork {
            source: Some(source),
            copy: None,
            destination,
            name,
        });
        Step::Continue
    }

    fn process(&mut self, mut work: EntryWork) -> Step {
        self.progress.current_path = Some(work.entry().item.path.clone());
        let result = if work.copy.is_some() {
            self.copy_step(&mut work)
        } else {
            self.prepare_entry(&mut work)
        };
        match result {
            Ok(Action::Keep) => {
                self.current = Some(work);
                Step::Continue
            }
            Ok(Action::Done) => Step::Continue,
            Ok(Action::Directory(destination)) => {
                self.stack.push(Frame {
                    source: work.source.take().expect("directory source"),
                    destination,
                });
                Step::Continue
            }
            Ok(Action::Conflict(target)) => self.wait_for_conflict(work, target),
            Err(failure) if failure.kind == io::ErrorKind::AlreadyExists => {
                // The destination appeared after the last inspect. Never replace it.
                match work.destination.inspect(&work.name, &self.cancel) {
                    Ok(Some(target)) => self.wait_for_conflict(work, target),
                    Ok(None) => {
                        self.current = Some(work);
                        Step::Continue
                    }
                    Err(error) => {
                        self.cleanup_work(&mut work);
                        self.record(error);
                        Step::Continue
                    }
                }
            }
            Err(error) => {
                self.cleanup_work(&mut work);
                self.record(error);
                Step::Continue
            }
        }
    }

    fn prepare_entry(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        let source = work.source.as_ref().expect("unstarted entry source");
        source.revalidate(&self.cancel)?;
        if let Some(target) = work.destination.inspect(&work.name, &self.cancel)? {
            return Ok(Action::Conflict(target));
        }
        match source.entry().item.kind {
            EntryKind::Directory => {
                if let Some(identity) = source.native_identity() {
                    work.destination
                        .reject_source_ancestor(identity, &self.cancel)?;
                }
                let destination = work
                    .destination
                    .create_directory(&work.name, &self.cancel)?;
                self.progress.directories_created += 1;
                Ok(Action::Directory(destination))
            }
            EntryKind::File => {
                let stage = work.destination.stage_file(&self.cancel)?;
                let source = work.source.take().expect("regular source");
                work.copy = Some(VerifiedFileCopy::new(source, stage)?);
                self.progress.phase = CopyOperationPhase::Copying;
                Ok(Action::Keep)
            }
            _ => Err(error(
                &source.entry().item.path,
                io::ErrorKind::Unsupported,
                "Copying links and special entries is unsupported.",
            )),
        }
    }

    fn copy_step(&mut self, work: &mut EntryWork) -> Result<Action, FsError> {
        let copy = work.copy.as_mut().expect("started copy");
        if copy.phase() == CopyPhase::ReadyToPublish {
            self.progress.phase = CopyOperationPhase::Publishing;
            match copy.publish(&work.name, &self.cancel)? {
                TransferPublication::Published(_) => self.progress.files_completed += 1,
                TransferPublication::PublishedDurabilityUnconfirmed { path, mut error } => {
                    error.message = format!(
                        "Target published at {}; durability is unconfirmed: {}",
                        path.display(),
                        error.message
                    );
                    self.record(error);
                }
            }
            return Ok(Action::Done);
        }
        let before_copy = copy.copied_bytes();
        let before_verify = copy.verified_bytes();
        let result = copy.step(&self.cancel);
        self.progress.copied_bytes = self
            .progress
            .copied_bytes
            .checked_add(copy.copied_bytes() - before_copy)
            .ok_or_else(|| {
                error(
                    &self.progress.destination_directory,
                    io::ErrorKind::InvalidData,
                    "Copied byte total overflow.",
                )
            })?;
        self.progress.verified_bytes = self
            .progress
            .verified_bytes
            .checked_add(copy.verified_bytes() - before_verify)
            .ok_or_else(|| {
                error(
                    &self.progress.destination_directory,
                    io::ErrorKind::InvalidData,
                    "Verified byte total overflow.",
                )
            })?;
        self.progress.phase = match result? {
            CopyPhase::Copying => CopyOperationPhase::Copying,
            CopyPhase::Syncing => CopyOperationPhase::Syncing,
            CopyPhase::Verifying => CopyOperationPhase::Verifying,
            CopyPhase::ReadyToPublish | CopyPhase::Published => CopyOperationPhase::Publishing,
            CopyPhase::Failed => {
                return Err(error(
                    &self.progress.destination_directory,
                    io::ErrorKind::InvalidData,
                    "The copy failed verification.",
                ));
            }
        };
        Ok(Action::Keep)
    }

    fn wait_for_conflict(&mut self, work: EntryWork, target: TransferEntry) -> Step {
        self.current = Some(work);
        if self.skip_all {
            self.skip_current();
            return Step::Continue;
        }
        let Some(version) = self.version.checked_add(1) else {
            self.record(error(
                &self.progress.destination_directory,
                io::ErrorKind::InvalidData,
                "Conflict version exhausted.",
            ));
            self.drop_current();
            return Step::Continue;
        };
        self.version = version;
        let work = self.current.as_ref().expect("conflicting entry");
        let conflict = CopyConflict {
            id: self.progress.id,
            version,
            source: work.entry().item.clone(),
            target: target.item,
            destination: work.destination.path().join(&work.name),
        };
        self.conflict = Some(conflict.clone());
        self.progress.message = "The destination exists. Safe replacement is unsupported; choose another name, skip, or cancel.".into();
        Step::Conflict(Arc::new(conflict))
    }
    fn record(&mut self, error: FsError) {
        self.progress.failed += 1;
        if self.progress.errors.len() < 16 {
            self.progress.errors.push(error);
        }
    }
    fn cleanup_work(&mut self, work: &mut EntryWork) {
        if let Some(copy) = &mut work.copy
            && let Err(error) = copy.cleanup()
        {
            self.record(error);
        }
    }
    fn drop_current(&mut self) {
        if let Some(mut work) = self.current.take() {
            self.cleanup_work(&mut work);
        }
        self.progress.current_path = None;
    }
    fn skip_current(&mut self) {
        self.drop_current();
        self.progress.skipped += 1;
        self.progress.phase = CopyOperationPhase::Planning;
    }
    pub(super) fn finish(mut self) -> CopyProgress {
        self.drop_current();
        self.progress.state = if self.cancel.is_cancelled() {
            CopyOperationState::Cancelled
        } else if self.progress.failed == 0 && self.progress.skipped == 0 {
            CopyOperationState::Completed
        } else if self.progress.files_completed > 0
            || self.progress.directories_created > 0
            || self.progress.skipped > 0
        {
            CopyOperationState::CompletedWithErrors
        } else {
            CopyOperationState::Failed
        };
        self.progress.message = self
            .progress
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        if self.progress.skipped > 0 {
            self.progress.message.push_str(&format!(
                "\n{} entries or subtrees skipped; source data retained.",
                self.progress.skipped
            ));
        }
        if self.cancel.is_cancelled() {
            self.progress
                .message
                .push_str("\nCopy cancelled; already published files are retained.");
        }
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = self.progress.clone();
        self.progress.clone()
    }
}
enum Action {
    Keep,
    Done,
    Directory(Arc<dyn TransferDirectory>),
    Conflict(TransferEntry),
}
