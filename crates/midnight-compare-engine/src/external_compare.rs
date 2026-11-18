//! Guarded external admission with one bounded typed outcome observer.
use crate::{ExternalCompareChoiceOutcome, FileChoice, Mailbox, Output};
use midnight_core::{
    Cancellation, ExternalCompareOutcome, ExternalCompareProcess, ExternalCompareSessionCredit,
    FileComparatorProgram, FsError,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, RecvTimeoutError},
    },
    time::Duration,
};

/// Synchronous host admission, invoked only on an admitted comparison worker.
/// Implementations prepare through the Engine's guarded native authority and
/// real command sender. Ok means admission only; Err means no command admitted.
/// Neither policy ownership nor content discovery/reads belong in this boundary.
pub trait ExternalCompareLauncher: Send + Sync + 'static {
    fn submit(
        &self,
        process: ExternalCompareProcess,
        original_paths: [PathBuf; 2],
        session: Arc<dyn ExternalCompareSessionCredit>,
        cancel: Cancellation,
        reply: mpsc::SyncSender<ExternalCompareOutcome>,
    ) -> Result<(), FsError>;
}

pub(crate) enum Attempt {
    /// Generation was superseded before the atomic external admission fence.
    Superseded,
    /// Session was closed; Engine still owns any admitted guarded request/child.
    Closed,
    Choice(FileChoice),
    Started,
}

pub(crate) fn compare(
    program: &FileComparatorProgram,
    paths: &[PathBuf; 2],
    generation: u64,
    output: &Mailbox,
    launcher: Option<&Arc<dyn ExternalCompareLauncher>>,
    credit: &Arc<dyn ExternalCompareSessionCredit>,
    cancel: Cancellation,
) -> Attempt {
    if !output.begin_external(generation) {
        return if cancel.is_cancelled() {
            Attempt::Closed
        } else {
            Attempt::Superseded
        };
    }
    let process = match program.process_for(paths) {
        Ok(process) => process,
        Err(message) => return not_started(paths, message.to_owned()),
    };
    let Some(launcher) = launcher else {
        return not_started(paths, "External program integration is unavailable.".into());
    };
    let (reply, received) = mpsc::sync_channel(1);
    // No policy, source capture or compiled matcher is passed to submit.
    let submitted = catch_unwind(AssertUnwindSafe(|| {
        launcher.submit(
            process,
            paths.clone(),
            credit.clone(),
            cancel.clone(),
            reply,
        )
    }));
    match submitted {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return not_started(paths, error.to_string()),
        Err(payload) => {
            // Submit may have admitted before unwinding. Publish ambiguity FIRST:
            // payload destruction is arbitrary host code and may itself panic.
            // Sticky Unknown prevents the existing outer worker catch/finish from
            // erasing this result if that secondary destruction fails.
            let choice = external_choice(
                paths,
                "Program launch outcome is uncertain; do not retry automatically.".into(),
                ExternalCompareChoiceOutcome::Unknown,
            );
            output.put(generation, Output::ChooseFiles(choice.clone()));
            // This runs on the existing comparator worker, under its existing
            // outer unwind boundary. Nothing is forgotten or retained globally.
            drop(payload);
            return Attempt::Choice(choice);
        }
    }
    loop {
        // Closing does not wait for cleanup or child exit. The existing Engine
        // front door observes this same flag and keeps its own guarded credits.
        if cancel.is_cancelled() {
            return Attempt::Closed;
        }
        match received.recv_timeout(Duration::from_millis(30)) {
            Ok(ExternalCompareOutcome::Started {
                pid,
                tracking_warning,
            }) => {
                output.put(
                    generation,
                    Output::ExternalStarted {
                        pid,
                        tracking_warning,
                    },
                );
                return Attempt::Started;
            }
            Ok(ExternalCompareOutcome::NotStarted(error)) => {
                return not_started(paths, error.to_string());
            }
            Ok(ExternalCompareOutcome::Unknown { message }) => return unknown(paths, message),
            Err(RecvTimeoutError::Disconnected) => {
                return unknown(
                    paths,
                    "Program launch outcome is unavailable; do not retry automatically.".into(),
                );
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn not_started(paths: &[PathBuf; 2], message: String) -> Attempt {
    Attempt::Choice(external_choice(
        paths,
        message,
        ExternalCompareChoiceOutcome::NotStarted,
    ))
}

fn unknown(paths: &[PathBuf; 2], message: String) -> Attempt {
    Attempt::Choice(external_choice(
        paths,
        message,
        ExternalCompareChoiceOutcome::Unknown,
    ))
}

fn external_choice(
    paths: &[PathBuf; 2],
    message: String,
    outcome: ExternalCompareChoiceOutcome,
) -> FileChoice {
    FileChoice {
        paths: paths.clone(),
        message,
        is_error: true,
        external_outcome: Some(outcome),
    }
}
