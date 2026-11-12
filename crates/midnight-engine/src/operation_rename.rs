//! One-file Rename conflicts reuse native Move publication and the shared overwrite choices.
use super::*;
use std::{ffi::OsString, io, path::Path};

#[derive(Default)]
pub(super) struct ConflictState {
    next: AtomicU64,
    pending: Mutex<Option<(u64, std::sync::mpsc::SyncSender<CopyChoice>)>>,
}
impl ConflictState {
    pub(super) fn resolve(&self, decision: CopyDecision) {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((version, sender)) = pending.as_ref()
            && *version == decision.version
        {
            let _ = sender.try_send(decision.choice);
        }
    }
    fn ask(
        &self,
        mut conflict: CopyConflict,
        outbox: &Outbox,
        cancel: &Cancellation,
    ) -> CopyChoice {
        let Ok(previous) = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        else {
            return CopyChoice::Cancel;
        };
        conflict.version = previous + 1;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((conflict.version, sender));
        let sent = outbox.blocking_send(
            Event::Operation(OperationEvent::RenameConflict(Arc::new(conflict))),
            cancel,
        );
        let choice = if sent {
            loop {
                if cancel.is_cancelled() {
                    break CopyChoice::Cancel;
                }
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(choice) => break choice,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break CopyChoice::Cancel,
                }
            }
        } else {
            CopyChoice::Cancel
        };
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
        choice
    }
}
fn error(path: &Path, kind: io::ErrorKind, text: &str) -> FsError {
    FsError::from_io("Rename", path, io::Error::new(kind, text))
}
fn validate(
    prepared: &PreparedRenameReplacement,
    original: &MutationItem,
    name: &std::ffi::OsStr,
) -> Result<(), FsError> {
    let source = prepared.movement.capture.entry();
    let target = prepared.target.entry();
    if original.kind != EntryKind::File
        || source.item.kind != EntryKind::File
        || target.item.kind != EntryKind::File
        || source.item.path != original.path
        || prepared.movement.source.entry().identity != source.identity
        || target.identity == source.identity
        || original.path.parent() != Some(prepared.directory.path())
        || target.item.path != original.path.with_file_name(name)
        || !prepared
            .directory
            .supports_native_move_replacement(source.identity)
    {
        return Err(error(
            &original.path,
            io::ErrorKind::Unsupported,
            "Recoverable Rename replacement requires distinct pinned regular files in the same supported directory.",
        ));
    }
    Ok(())
}
fn recovery_message(recovery: &[TransferRecovery]) -> String {
    if recovery.is_empty() {
        return String::new();
    }
    let mut text = String::from("Original items retained for recovery:");
    for item in recovery.iter().take(16) {
        text.push_str(&format!("\n{}", item.path.display()));
    }
    if recovery.len() > 16 {
        text.push_str("\nAdditional recovery locations were not included in this bounded summary.");
    }
    text
}
fn completed(progress: &mut OperationProgress, destination: std::path::PathBuf) {
    progress.completed = 1;
    progress.processed_entries = 1;
    progress.renamed_to = Some(destination);
}
fn failed(progress: &mut OperationProgress, failure: FsError, cancel: &Cancellation) {
    progress.failed = u64::from(failure.kind != io::ErrorKind::Interrupted);
    progress.message = failure.to_string();
    progress.errors.push(failure);
    progress.state = if cancel.is_cancelled() {
        OperationState::Cancelled
    } else {
        OperationState::Failed
    };
}

pub(super) fn execute(job: worker::Job, outbox: Outbox) -> OperationProgress {
    let mut progress = job.progress();
    let target = &job.targets[0];
    let mut name: OsString = job
        .name
        .as_ref()
        .expect("validated Rename basename")
        .clone();
    progress.current_path = Some(target.item().path.clone());
    loop {
        if job.cancel.is_cancelled() {
            progress.state = OperationState::Cancelled;
            break;
        }
        match target.rename(&name, &job.cancel) {
            Ok(destination) => {
                completed(&mut progress, destination);
                progress.state = OperationState::Completed;
                break;
            }
            Err(failure)
                if failure.kind == io::ErrorKind::AlreadyExists
                    && target.item().kind == EntryKind::File => {}
            Err(failure) => {
                failed(&mut progress, failure, &job.cancel);
                break;
            }
        }
        let prepared = match target
            .prepare_rename_replacement(&name, &job.cancel)
            .and_then(|prepared| {
                validate(&prepared, target.item(), &name)?;
                Ok(prepared)
            }) {
            Ok(prepared) => prepared,
            Err(failure) => {
                failed(&mut progress, failure, &job.cancel);
                break;
            }
        };
        let conflict = CopyConflict {
            id: progress.id,
            version: 0,
            source: prepared.movement.capture.entry().item.clone(),
            target: prepared.target.entry().item.clone(),
            destination: target.item().path.with_file_name(&name),
            can_replace: true,
            can_merge: false,
        };
        match job.rename_conflicts.ask(conflict, &outbox, &job.cancel) {
            CopyChoice::Cancel => {
                job.cancel.cancel();
                progress.state = OperationState::Cancelled;
                break;
            }
            CopyChoice::Skip | CopyChoice::SkipAll => {
                progress.state = OperationState::Cancelled;
                progress.message =
                    "Rename skipped; the source and existing destination were retained.".into();
                break;
            }
            CopyChoice::Rename(alternate) => {
                name = alternate;
                continue;
            }
            CopyChoice::ReplaceRecoverable { .. } => {}
            _ => {
                job.cancel.cancel();
                progress.state = OperationState::Cancelled;
                break;
            }
        }
        let result = prepared.movement.capture.try_native_move(
            prepared.directory.as_ref(),
            &name,
            Some((
                prepared.target.clone(),
                TransferReplacementPolicy::RecoverableCurrentEntry,
            )),
            &job.cancel,
        );
        match result {
            Ok(TransferNativeMoveOutcome::Moved {
                destination,
                recovery,
                ..
            }) => {
                completed(&mut progress, destination);
                progress.state = OperationState::Completed;
                progress.message = recovery_message(&recovery);
            }
            Ok(TransferNativeMoveOutcome::RecoveryRequired {
                destination,
                recovery,
                error,
            }) => {
                if let Some(destination) = destination {
                    completed(&mut progress, destination);
                }
                failed(&mut progress, error, &job.cancel);
                if progress.completed != 0 {
                    progress.state = OperationState::CompletedWithErrors;
                }
                let retained = recovery_message(&recovery);
                if !retained.is_empty() {
                    progress.message.push_str(&format!("\n{retained}"));
                }
            }
            Ok(_) => failed(
                &mut progress,
                error(
                    &target.item().path,
                    io::ErrorKind::Unsupported,
                    "The pinned native Rename replacement could not be performed; no Copy fallback was attempted.",
                ),
                &job.cancel,
            ),
            Err(failure) if failure.kind == io::ErrorKind::AlreadyExists => {
                // No mutation occurred. Never transfer consent to a changed target.
                drop(prepared);
                continue;
            }
            Err(failure) => failed(&mut progress, failure, &job.cancel),
        }
        break;
    }
    progress
}

#[cfg(test)]
#[path = "operation_rename_tests.rs"]
mod tests;
