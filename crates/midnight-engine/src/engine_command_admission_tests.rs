use crate::engine_command_admission_fixture as fixture;
use fixture::{Case, Release, retire_refused, wait_async, wait_sync};
use midnight_core::*;
use std::time::Duration;
use tokio::sync::mpsc;

const OBSERVE: Duration = Duration::from_secs(8);

#[tokio::test]
async fn unpolled_sender_refuses_thirty_third_copy_intact_without_capture_callbacks() {
    let case = Case::new();
    let (mut engine, mut events) = case.engine(128, true);
    let (sender, receiver) = engine.command_channel(64).unwrap();
    let run = engine.run(receiver);
    for id in 1..=32 {
        sender.try_send(case.invalid_copy(id)).unwrap();
    }
    let (command, probe) = case.capture(33, false, false);
    let result = sender.try_send(command);
    let no_callback = probe.calls() == 0 && !probe.closed();
    let no_event = events.try_recv().is_err();
    let refused = match result {
        Err(mpsc::error::TrySendError::Full(command)) => Some(command),
        Err(mpsc::error::TrySendError::Closed(command)) => {
            retire_refused(command).await;
            None
        }
        Ok(()) => None,
    };
    let intact = if let Some(command) = refused {
        retire_refused(command).await == Some(33)
    } else {
        false
    };
    // Native handoff controls use a prior reservation; they must not take #33.
    let native_control = sender
        .try_send(Command::AcceptDrop {
            token: DropToken(9),
            id: OperationId(34),
            verification: TransferVerification::default(),
        })
        .is_ok();
    let task = tokio::spawn(run);
    let mut rejected = std::collections::HashSet::new();
    let drained = tokio::time::timeout(OBSERVE, async {
        while rejected.len() < 32 {
            let Some(event) = events.recv().await else {
                return false;
            };
            if let Event::Copy(CopyEvent::Progress(progress)) = event
                && progress.state.is_terminal()
                && (!(1..=32).contains(&progress.id.0)
                    || progress
                        .errors
                        .first()
                        .is_none_or(|error| error.kind != std::io::ErrorKind::InvalidInput)
                    || !rejected.insert(progress.id.0))
            {
                return false;
            }
        }
        true
    })
    .await
    .unwrap_or(false);
    // A refused #33 must not leave an unfinished latch behind. A real native
    // successor that explicitly waits must still copy after the32 failed jobs.
    let mut successor = Some(case.valid_copy_waiting(35));
    let sent = tokio::time::timeout(OBSERVE, async {
        loop {
            match sender.try_send(successor.take().unwrap()) {
                Ok(()) => return true,
                Err(mpsc::error::TrySendError::Full(command)) => successor = Some(command),
                Err(mpsc::error::TrySendError::Closed(command)) => {
                    successor = Some(command);
                    return false;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false);
    let terminal = if sent {
        fixture::terminal(&mut events, 35).await
    } else {
        None
    };
    let shutdown = sender.try_send(Command::Shutdown).is_ok();
    let joined = tokio::time::timeout(OBSERVE, task).await;
    assert!(no_callback, "no_callback");
    assert!(no_event, "no_event");
    assert!(intact, "intact");
    assert!(native_control, "native_control");
    assert!(drained, "drained");
    assert!(sent, "sent");
    assert!(shutdown, "shutdown");
    assert!(joined.is_ok_and(|result| result.is_ok()));
    assert_eq!(terminal.unwrap().state, CopyOperationState::Completed);
    assert!(case.copied_bytes_match());
    assert!(probe.checked_close());
}

#[tokio::test]
async fn full_and_closed_command_fifo_return_capture_ownership_without_getters() {
    let case = Case::new();
    let (mut engine, _events) = case.engine(8, false);
    let (sender, receiver) = engine.command_channel(1).unwrap();
    let run = engine.run(receiver);
    sender
        .try_send(Command::CancelCopy(OperationId(90)))
        .unwrap();
    let (command, full_probe) = case.capture(1, false, false);
    let full = match sender.try_send(command) {
        Err(mpsc::error::TrySendError::Full(command)) => Some(command),
        Err(mpsc::error::TrySendError::Closed(command)) => {
            retire_refused(command).await;
            None
        }
        Ok(()) => None,
    };
    let full_untouched = full_probe.calls() == 0 && !full_probe.closed();
    let full_intact = if let Some(command) = full {
        retire_refused(command).await == Some(1)
    } else {
        false
    };
    drop(run);
    let closed_signal = wait_async(OBSERVE, || sender.is_closed()).await;
    let (command, closed_probe) = case.capture(2, false, false);
    let closed = match sender.try_send(command) {
        Err(mpsc::error::TrySendError::Closed(command)) => Some(command),
        Err(mpsc::error::TrySendError::Full(command)) => {
            retire_refused(command).await;
            None
        }
        Ok(()) => None,
    };
    let closed_untouched = closed_probe.calls() == 0 && !closed_probe.closed();
    let closed_intact = if let Some(command) = closed {
        retire_refused(command).await == Some(2)
    } else {
        false
    };
    assert!(full_untouched, "full_untouched");
    assert!(full_intact, "full_intact");
    assert!(closed_signal, "closed_signal");
    assert!(closed_untouched, "closed_untouched");
    assert!(closed_intact, "closed_intact");
    assert!(full_probe.checked_close() && closed_probe.checked_close());
}

#[tokio::test]
async fn async_send_and_both_permit_forms_cannot_bypass_copy_admission() {
    let case = Case::new();
    let (mut engine, _events) = case.engine(8, false);
    let (sender, receiver) = engine.command_channel(64).unwrap();
    let run = engine.run(receiver);
    for id in 1..=32 {
        sender.try_send(case.invalid_copy(id)).unwrap();
    }
    let (command, asynchronous) = case.capture(33, false, false);
    let async_result = tokio::time::timeout(OBSERVE, sender.send(command)).await;
    let async_refused = match async_result {
        Ok(Err(mpsc::error::TrySendError::Full(command))) => {
            retire_refused(command).await == Some(33)
        }
        Ok(Err(mpsc::error::TrySendError::Closed(command))) => {
            retire_refused(command).await;
            false
        }
        _ => false,
    };
    let permit = tokio::time::timeout(OBSERVE, sender.reserve())
        .await
        .unwrap()
        .unwrap();
    let (command, borrowed) = case.capture(34, false, false);
    let borrowed_refused = match permit.try_send(command) {
        Err(mpsc::error::TrySendError::Full(command)) => retire_refused(command).await == Some(34),
        Err(mpsc::error::TrySendError::Closed(command)) => {
            retire_refused(command).await;
            false
        }
        Ok(()) => false,
    };
    let permit = tokio::time::timeout(OBSERVE, sender.clone().reserve_owned())
        .await
        .unwrap()
        .unwrap();
    let (command, owned) = case.capture(35, false, false);
    let owned_refused = match permit.try_send(command) {
        Err(mpsc::error::TrySendError::Full(command)) => retire_refused(command).await == Some(35),
        Err(mpsc::error::TrySendError::Closed(command)) => {
            retire_refused(command).await;
            false
        }
        Ok(()) => false,
    };
    drop(run);
    let closed = wait_async(OBSERVE, || sender.is_closed()).await;
    assert!(async_refused, "async_refused");
    assert!(borrowed_refused, "borrowed_refused");
    assert!(owned_refused, "owned_refused");
    assert!(closed, "closed");
    for probe in [asynchronous, borrowed, owned] {
        assert_eq!(probe.calls(), 0);
        assert!(probe.checked_close());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn queued_before_run_copy_then_cancel_preserves_control_fifo() {
    let case = Case::new();
    let (mut engine, mut events) = case.engine(64, true);
    let (sender, receiver) = engine.command_channel(8).unwrap();
    let run = engine.run(receiver);
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
    let untouched = probe.calls() == 0;
    sender
        .try_send(Command::CancelCopy(OperationId(1)))
        .unwrap();
    let task = tokio::spawn(run);
    let terminal = fixture::terminal(&mut events, 1).await;
    let closed = wait_async(OBSERVE, || probe.closed()).await;
    let shutdown = sender.try_send(Command::Shutdown).is_ok();
    probe.release();
    let joined = tokio::time::timeout(OBSERVE, task).await;
    assert!(admitted, "admitted");
    assert!(untouched, "untouched");
    assert!(closed, "closed");
    assert!(shutdown, "shutdown");
    assert!(joined.is_ok_and(|result| result.is_ok()));
    assert_eq!(terminal.unwrap().state, CopyOperationState::Cancelled);
    assert!(probe.all_callbacks_off(std::thread::current().id()));
    assert!(probe.checked_close() && !probe.timed_out());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_rejection_outbox_does_not_block_cancel_or_shutdown_in_real_loop() {
    let case = Case::new();
    let (mut engine, mut events) = case.engine(1, true);
    let (sender, receiver) = engine.command_channel(8).unwrap();
    let run = engine.run(receiver);
    let (command, probe) = case.capture(1, true, true);
    let _release = Release(probe.clone());
    let admitted = match sender.try_send(command) {
        Ok(()) => true,
        Err(error) => {
            probe.release();
            retire_refused(error.into_inner()).await;
            false
        }
    };
    // One immutable event fills the actual Engine Outbox before it starts.
    case.fill_events();
    let task = tokio::spawn(run);
    let entered = wait_async(OBSERVE, || probe.prepared()).await;
    let invalid_sent = sender.try_send(case.invalid_copy(2)).is_ok();
    let cancel_sent = sender.try_send(Command::CancelCopy(OperationId(1))).is_ok();
    let cancelled = wait_async(OBSERVE, || probe.saw_cancel()).await;
    let held = wait_async(OBSERVE, || probe.close_entered()).await;
    let shutdown_sent = sender.try_send(Command::Shutdown).is_ok();
    let closing = wait_async(OBSERVE, || sender.is_closed()).await;
    let not_closed = !probe.closed();
    // Do not mistake an accepted Shutdown for completed native destruction.
    probe.release();
    let drainer = tokio::spawn(async move { while events.recv().await.is_some() {} });
    let joined = tokio::time::timeout(OBSERVE, task).await;
    case.close_event_sender();
    let drained = tokio::time::timeout(OBSERVE, drainer).await;
    assert!(admitted, "admitted");
    assert!(entered, "entered");
    assert!(invalid_sent, "invalid_sent");
    assert!(cancel_sent, "cancel_sent");
    assert!(cancelled, "cancelled");
    assert!(held, "held");
    assert!(shutdown_sent, "shutdown_sent");
    assert!(closing, "closing");
    assert!(not_closed, "not_closed");
    assert!(
        joined.is_ok_and(|result| result.is_ok()) && drained.is_ok_and(|result| result.is_ok())
    );
    assert!(probe.checked_close() && !probe.timed_out());
}

#[test]
fn admitted_captured_command_abandoned_without_runtime_uses_reserved_native_cleanup() {
    let case = Case::new();
    let (command, probe) = case.capture(1, false, true);
    let _release = Release(probe.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (sender, run, admitted) = {
        let _entered = runtime.enter();
        let (mut engine, _events) = case.engine(8, false);
        let (sender, receiver) = engine.command_channel(8).unwrap();
        let run = engine.run(receiver);
        let admitted = match sender.try_send(command) {
            Ok(()) => true,
            Err(error) => {
                probe.release();
                fixture::retire_refused_sync(error.into_inner());
                false
            }
        };
        (sender, run, admitted)
    };
    drop(runtime);
    let caller = std::thread::current().id();
    drop(run);
    let held = wait_sync(OBSERVE, || probe.close_entered());
    let not_closed = !probe.closed();
    let no_getters = probe.calls() == 0;
    probe.release();
    let closed = wait_sync(OBSERVE, || probe.closed());
    let transport_closed = wait_sync(OBSERVE, || sender.is_closed());
    assert!(admitted, "admitted");
    assert!(held, "held");
    assert!(not_closed, "not_closed");
    assert!(no_getters, "no_getters");
    assert!(closed, "closed");
    assert!(transport_closed, "transport_closed");
    assert_ne!(probe.drop_thread(), Some(caller));
    assert!(probe.checked_close() && !probe.timed_out());
}
