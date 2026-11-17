//! Accepted invalid IDs still retire and deliver independently of the command loop.
use crate::engine_command_admission_fixture as fixture;
use fixture::{Case, Release, retire_refused, wait_async};
use midnight_core::*;
use std::time::Duration;

#[tokio::test]
async fn accepted_zero_id_delivers_invalid_input_and_does_not_strand_shutdown() {
    let case = Case::new();
    let (mut engine, mut events) = case.engine(16, false);
    let (sender, receiver) = engine.command_channel(8).unwrap();
    let accepted = sender.try_send(case.invalid_copy(0)).is_ok();
    let task = tokio::spawn(engine.run(receiver));
    let terminal = fixture::terminal(&mut events, 0).await;
    let shutdown = sender.try_send(Command::Shutdown).is_ok();
    let joined = tokio::time::timeout(Duration::from_secs(8), task).await;
    assert!(accepted && shutdown && joined.is_ok_and(|result| result.is_ok()));
    assert!(terminal.is_some_and(|progress| {
        progress.state == CopyOperationState::Failed
            && progress.errors.iter().any(|error| {
                error.kind == std::io::ErrorKind::InvalidInput
                    && error.message.contains("operation ID")
            })
    }));
}

#[tokio::test]
async fn replayed_id_retires_without_forging_events_or_removing_original_cancel_control() {
    let case = Case::new();
    let (mut engine, mut events) = case.engine(64, true);
    let (sender, receiver) = engine.command_channel(16).unwrap();
    let (command, probe) = case.capture(1, true, false);
    let _release = Release(probe.clone());
    let admitted = match sender.try_send(command) {
        Ok(()) => true,
        Err(error) => {
            probe.release();
            retire_refused(error.into_inner()).await;
            false
        }
    };
    let task = tokio::spawn(engine.run(receiver));
    let prepared = wait_async(Duration::from_secs(8), || probe.prepared()).await;
    let (mut duplicate_command, replay) = case.capture(1, false, true);
    let _replay_release = Release(replay.clone());
    if let Command::EnqueueCapturedTransfer(request) = &mut duplicate_command {
        request.destination.push("replay-visible-marker");
    }
    let duplicate = match sender.try_send(duplicate_command) {
        Ok(()) => true,
        Err(error) => {
            replay.release();
            retire_refused(error.into_inner()).await;
            false
        }
    };
    let cleanup_entered = wait_async(Duration::from_secs(8), || replay.close_entered()).await;
    let held = !replay.closed();
    replay.release();
    let returned = wait_async(Duration::from_secs(8), || {
        replay.closed()
            && sender.admission.capacity().available_permits()
                == crate::copy_operations::MAX_JOBS - 1
    })
    .await;
    // Returning the same admission means the replay runner/delivery tail ended.
    // The original is still waiting for Cancel, so no id=1 terminal is genuine.
    let mut forged = false;
    while let Ok(event) = events.try_recv() {
        if let Event::Copy(CopyEvent::Progress(progress)) = event {
            forged |= progress.id == OperationId(1)
                && (progress.state.is_terminal()
                    || progress
                        .destination_directory
                        .ends_with("replay-visible-marker"));
        }
    }
    let cancelled = sender.try_send(Command::CancelCopy(OperationId(1))).is_ok();
    let observed = wait_async(Duration::from_secs(8), || probe.saw_cancel()).await;
    probe.release();
    let shutdown = sender.try_send(Command::Shutdown).is_ok();
    let joined = tokio::time::timeout(Duration::from_secs(8), task).await;
    assert!(
        admitted
            && prepared
            && duplicate
            && cleanup_entered
            && held
            && returned
            && cancelled
            && observed
            && shutdown
    );
    assert!(joined.is_ok_and(|result| result.is_ok()));
    assert!(!forged && replay.calls() == 0 && replay.checked_close() && !replay.timed_out());
}
