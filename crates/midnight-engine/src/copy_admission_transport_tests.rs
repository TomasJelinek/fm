use super::*;
use crate::engine_command_admission_fixture as fixture;
use crate::engine_commands::{CommandReceiver, CommandSender, Message};
use fixture::{Case, Release, retire_refused_sync, wait_async, wait_sync};
use std::{sync::mpsc as sync, thread};

const OBSERVE: Duration = Duration::from_secs(8);

struct WorkerPause {
    release: Option<sync::Sender<()>>,
    returned: sync::Receiver<()>,
}
impl WorkerPause {
    fn finish(mut self) -> bool {
        let _ = self.release.take().unwrap().send(());
        self.returned.recv_timeout(OBSERVE).is_ok()
    }
}
impl Drop for WorkerPause {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}
fn paused_retirement() -> (crate::retirement::RetirementDomain, WorkerPause) {
    struct HeldMetadata {
        entered: sync::Sender<()>,
        release: sync::Receiver<()>,
        returned: sync::Sender<()>,
    }
    impl Drop for HeldMetadata {
        fn drop(&mut self) {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(15)).unwrap();
            let _ = self.returned.send(());
        }
    }
    // One unrelated metadata-only job occupies the fixed domain worker. All32
    // Copy payload leases remain available; rollback cannot rely on that worker.
    let domain = crate::retirement::RetirementDomain::try_start(33, 0).unwrap();
    let (entered, observed) = sync::channel();
    let (release, released) = sync::channel();
    let (returned, acknowledged) = sync::channel();
    drop(domain.try_reserve().unwrap().own(HeldMetadata {
        entered,
        release: released,
        returned,
    }));
    observed.recv_timeout(OBSERVE).unwrap();
    (
        domain,
        WorkerPause {
            release: Some(release),
            returned: acknowledged,
        },
    )
}
fn all_payload_leases_returned(domain: &crate::retirement::RetirementDomain) -> bool {
    let leases: Vec<_> = (0..32)
        .filter_map(|_| domain.try_reserve_nowait().ok())
        .collect();
    leases.len() == 32
}

fn transport(
    domain: crate::retirement::RetirementDomain,
) -> (Admission, CommandSender, CommandReceiver) {
    let admission = Admission::new(domain);
    let inbox = crate::retirement::RetirementDomain::try_start(1, 0)
        .unwrap()
        .try_reserve()
        .unwrap();
    let (sender, receiver) = CommandSender::channel(8, admission.clone(), inbox);
    (admission, sender, receiver)
}
fn capture_identity(command: &Command) -> std::sync::Weak<dyn CapturedTransferSources> {
    match command {
        Command::EnqueueCapturedTransfer(request) => Arc::downgrade(&request.sources),
        _ => unreachable!(),
    }
}
fn reclaim_full(
    result: Result<(), mpsc::error::TrySendError<Command>>,
    original: &std::sync::Weak<dyn CapturedTransferSources>,
    id: u64,
) -> bool {
    match result {
        Err(mpsc::error::TrySendError::Full(command)) => {
            let same = match &command {
                Command::EnqueueCapturedTransfer(request) => original
                    .upgrade()
                    .is_some_and(|owner| Arc::ptr_eq(&owner, &request.sources)),
                _ => false,
            };
            let reclaimed = retire_refused_sync(command);
            same && reclaimed == Some(id)
        }
        Err(error) => {
            retire_refused_sync(error.into_inner());
            false
        }
        Ok(()) => false,
    }
}
fn empty(admission: &Admission) -> bool {
    let state = admission
        .0
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state.slots.is_empty() && state.controls.is_empty()
}

#[test]
#[expect(
    clippy::result_large_err,
    reason = "These transport fixtures must preserve the original Command and its capture ownership."
)]
fn state_contention_returns_original_capture_and_restores_capacity_before_cleanup() {
    let case = Case::new();
    let (domain, paused_worker) = paused_retirement();
    let (admission, sender, receiver) = transport(domain.clone());
    let (command, probe) = case.capture(1, false, false);
    let identity = capture_identity(&command);
    let (entered, armed) = sync::channel();
    let (release, released) = sync::channel();
    let (done, observed) = sync::channel();
    let worker = thread::spawn(move || {
        let result = faults::with(
            faults::Point::AfterArmBeforeLock,
            faults::Action::Pause {
                entered,
                release: released,
            },
            || sender.try_send(command),
        );
        done.send(()).unwrap();
        result
    });
    let staged = armed.recv_timeout(Duration::from_secs(2)).is_ok();
    let locked = admission.0.state.lock().unwrap();
    let _ = release.send(());
    let returned_while_locked = observed.recv_timeout(Duration::from_secs(2)).is_ok();
    let restored = admission.capacity().available_permits() == 32;
    let leases_restored = all_payload_leases_returned(&domain);
    let untouched = probe.calls() == 0 && !probe.closed();
    drop(locked);
    let result = worker.join().unwrap();
    let pristine = empty(&admission);
    let intact = reclaim_full(result, &identity, 1);
    let mut next = admission.reserve().unwrap();
    let no_hidden_latch = next.owned.resources_mut().take_predecessors().is_finished();
    drop(next);
    drop(receiver);
    let worker_released = paused_worker.finish();
    assert!(
        staged
            && returned_while_locked
            && restored
            && leases_restored
            && untouched
            && pristine
            && intact
            && no_hidden_latch
            && worker_released
    );
    assert!(probe.checked_close());
}

#[test]
#[expect(
    clippy::result_large_err,
    reason = "These transport fixtures must preserve the original Command and its capture ownership."
)]
fn fence_contention_rolls_back_without_publishing_finished_or_holding_state_mutex() {
    let case = Case::new();
    let (domain, paused_worker) = paused_retirement();
    let (admission, sender, receiver) = transport(domain.clone());
    let (command, probe) = case.capture(1, false, false);
    let identity = capture_identity(&command);
    let fence = admission.fence();
    let (entered, armed) = sync::channel();
    let (release, released) = sync::channel();
    let (done, observed) = sync::channel();
    let worker = thread::spawn(move || {
        let result = faults::with(
            faults::Point::AfterArmBeforeLock,
            faults::Action::Pause {
                entered,
                release: released,
            },
            || sender.try_send(command),
        );
        done.send(()).unwrap();
        result
    });
    let staged = armed.recv_timeout(Duration::from_secs(2)).is_ok();
    let locked = fence.try_registration().unwrap();
    let _ = release.send(());
    let returned_while_locked = observed.recv_timeout(Duration::from_secs(2)).is_ok();
    let state_free = admission.0.state.try_lock().is_ok();
    let restored = admission.capacity().available_permits() == 32;
    let leases_restored = all_payload_leases_returned(&domain);
    let untouched = probe.calls() == 0 && !probe.closed();
    drop(locked);
    let result = worker.join().unwrap();
    let pristine = empty(&admission);
    let intact = reclaim_full(result, &identity, 1);
    let mut next = admission.reserve().unwrap();
    let no_hidden_latch = next.owned.resources_mut().take_predecessors().is_finished();
    drop(next);
    drop(receiver);
    let worker_released = paused_worker.finish();
    assert!(
        staged
            && returned_while_locked
            && state_free
            && restored
            && leases_restored
            && untouched
            && pristine
            && intact
            && no_hidden_latch
            && worker_released
    );
}

#[test]
fn unavailable_actual_retirement_lease_refuses_without_a_finished_latch() {
    let case = Case::new();
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let held = domain.try_reserve().unwrap();
    let (admission, sender, receiver) = transport(domain);
    let (command, probe) = case.capture(1, false, false);
    let identity = capture_identity(&command);
    let result = sender.try_send(command);
    let restored = admission.capacity().available_permits() == 32;
    let untouched = probe.calls() == 0 && !probe.closed();
    let pristine = empty(&admission);
    let intact = reclaim_full(result, &identity, 1);
    drop(held);
    let mut next = admission.reserve().unwrap();
    let no_hidden_latch = next.owned.resources_mut().take_predecessors().is_finished();
    drop(next);
    drop(receiver);
    assert!(restored && untouched && pristine && intact && no_hidden_latch);
}

#[tokio::test]
async fn successful_send_is_already_in_the_next_actual_reservation_snapshot() {
    let case = Case::new();
    let domain = crate::retirement::RetirementDomain::try_start(32, 0).unwrap();
    let (admission, sender, mut receiver) = transport(domain.clone());
    let (command, probe) = case.capture(1, false, true);
    let _release = Release(probe.clone());
    let sent = match sender.try_send(command) {
        Ok(()) => true,
        Err(error) => {
            retire_refused_sync(error.into_inner());
            false
        }
    };
    let mut next = admission.reserve().unwrap();
    let predecessors = next.owned.resources_mut().take_predecessors();
    let earlier_pending = !predecessors.is_finished();
    let message = tokio::time::timeout(OBSERVE, receiver.inbox.recv())
        .await
        .ok()
        .flatten();
    let published = matches!(&message, Some(Message::GuardedCopy(admitted)) if admitted.request.id == OperationId(1));
    let mut conversion_pending = false;
    let mut cancellation_preserved = false;
    match message {
        Some(Message::GuardedCopy(mut admitted)) => {
            let original_cancel = admitted.request.resources_mut().pending_cancel().unwrap();
            let mut input = ownership::convert_submission(admitted.request);
            conversion_pending = !predecessors.is_finished() && !probe.close_entered();
            original_cancel.cancel();
            cancellation_preserved = input
                .resources_mut()
                .pending_cancel()
                .is_some_and(|cancel| cancel.is_cancelled());
            drop(input);
        }
        other => drop(other),
    }
    let held = wait_async(OBSERVE, || probe.close_entered()).await;
    let held_authority = !probe.closed()
        && !predecessors.is_finished()
        && admission.capacity().available_permits() == 30;
    probe.release();
    let closed = wait_async(OBSERVE, || probe.closed()).await;
    let finished = wait_async(OBSERVE, || predecessors.is_finished()).await;
    let first_returned =
        wait_async(OBSERVE, || admission.capacity().available_permits() == 31).await;
    drop(next);
    let all_returned = wait_async(OBSERVE, || admission.capacity().available_permits() == 32).await;
    let leases_returned = wait_async(OBSERVE, || all_payload_leases_returned(&domain)).await;
    drop(receiver);
    assert!(sent && published && earlier_pending && closed && finished);
    assert_eq!(probe.calls(), 0);
    assert!(probe.checked_close());
    assert!(
        conversion_pending
            && cancellation_preserved
            && held
            && held_authority
            && first_returned
            && all_returned
            && leases_returned
    );
    assert_eq!(admission.capacity().available_permits(), 32);
    assert!(!probe.timed_out());
}

#[tokio::test]
#[expect(
    clippy::result_large_err,
    reason = "These transport fixtures must preserve the original Command and its capture ownership."
)]
async fn two_producers_cannot_publish_a_later_admission_ahead_of_the_locked_first() {
    let case = Case::new();
    let (admission, sender, mut receiver) =
        transport(crate::retirement::RetirementDomain::try_start(32, 0).unwrap());
    let (first, first_probe) = case.capture(1, false, false);
    let (second, second_probe) = case.capture(2, false, false);
    let (entered, observed) = sync::channel();
    let (release, released) = sync::channel();
    let first_sender = sender.clone();
    let worker = thread::spawn(move || {
        faults::with(
            faults::Point::AfterRegisterBeforePublish,
            faults::Action::Pause {
                entered,
                release: released,
            },
            || first_sender.try_send(first),
        )
    });
    let paused = observed.recv_timeout(Duration::from_secs(2)).is_ok();
    let second_result = sender.try_send(second);
    let second = match second_result {
        Err(mpsc::error::TrySendError::Full(command)) => Some(command),
        Err(error) => {
            retire_refused_sync(error.into_inner());
            None
        }
        Ok(()) => None,
    };
    let refused = second.is_some();
    let _ = release.send(());
    let first_sent = match worker.join().unwrap() {
        Ok(()) => true,
        Err(error) => {
            retire_refused_sync(error.into_inner());
            false
        }
    };
    let second_sent = if let Some(second) = second {
        match sender.try_send(second) {
            Ok(()) => true,
            Err(error) => {
                retire_refused_sync(error.into_inner());
                false
            }
        }
    } else {
        false
    };
    let mut first = tokio::time::timeout(OBSERVE, receiver.inbox.recv())
        .await
        .ok()
        .flatten();
    let second = tokio::time::timeout(OBSERVE, receiver.inbox.recv())
        .await
        .ok()
        .flatten();
    let (ordered, predecessors) = match (first.as_mut(), second) {
        (Some(Message::GuardedCopy(first)), Some(Message::GuardedCopy(mut second))) => {
            let ordered = first.request.id == OperationId(1)
                && second.request.id == OperationId(2)
                && first.request.resources_mut().admission_sequence()
                    < second.request.resources_mut().admission_sequence();
            let predecessors = second.request.resources_mut().take_predecessors();
            (ordered, Some((predecessors, second)))
        }
        (_, second) => {
            drop(second);
            (false, None)
        }
    };
    let pending = predecessors
        .as_ref()
        .is_some_and(|(wait, _)| !wait.is_finished());
    drop(first);
    let finished = if let Some((wait, _)) = &predecessors {
        wait_async(OBSERVE, || wait.is_finished()).await
    } else {
        false
    };
    drop(predecessors);
    drop(receiver);
    let closed = wait_async(OBSERVE, || first_probe.closed() && second_probe.closed()).await;
    let returned = wait_async(OBSERVE, || admission.capacity().available_permits() == 32).await;
    assert!(
        paused
            && refused
            && first_sent
            && second_sent
            && ordered
            && pending
            && finished
            && closed
            && returned
    );
    assert!(first_probe.checked_close() && second_probe.checked_close());
}

#[expect(
    clippy::result_large_err,
    reason = "These transport fixtures must preserve the original Command and its capture ownership."
)]
fn panic_edge(point: faults::Point, registered: bool) {
    let case = Case::new();
    let (admission, sender, receiver) =
        transport(crate::retirement::RetirementDomain::try_start(32, 0).unwrap());
    let (command, probe) = case.capture(1, false, true);
    let _release = Release(probe.clone());
    let worker = thread::spawn(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            faults::with(point, faults::Action::Panic, || sender.try_send(command))
        }))
    });
    let panicked = match worker.join().unwrap() {
        Err(_) => true,
        Ok(Err(error)) => {
            probe.release();
            retire_refused_sync(error.into_inner());
            false
        }
        Ok(Ok(())) => false,
    };
    let held = wait_sync(OBSERVE, || probe.close_entered());
    let capacity_held = admission.capacity().available_permits() == 31;
    let state_free = match admission.0.state.try_lock() {
        Ok(_) | Err(std::sync::TryLockError::Poisoned(_)) => true,
        Err(std::sync::TryLockError::WouldBlock) => false,
    };
    let fence_free = admission.fence().try_registration().is_ok();
    let not_closed = !probe.closed();
    let mut next = admission.reserve().unwrap();
    let predecessors = next.owned.resources_mut().take_predecessors();
    let expected_pending = predecessors.is_finished() != registered;
    probe.release();
    let closed = wait_sync(OBSERVE, || probe.closed());
    let finished = wait_sync(OBSERVE, || predecessors.is_finished());
    drop(next);
    drop(receiver);
    let returned = wait_sync(OBSERVE, || admission.capacity().available_permits() == 32);
    assert!(
        panicked
            && held
            && capacity_held
            && state_free
            && fence_free
            && not_closed
            && expected_pending
            && closed
            && finished
            && returned
    );
    assert!(probe.checked_close() && !probe.timed_out());
    assert_eq!(probe.calls(), 0);
}

#[test]
fn after_arm_panic_keeps_capacity_through_file_close_without_publishing_a_latch() {
    panic_edge(faults::Point::AfterArmBeforeLock, false);
}
#[test]
fn after_register_panic_unlocks_both_mutexes_but_retains_finished_until_file_close() {
    panic_edge(faults::Point::AfterRegisterBeforePublish, true);
}

#[test]
fn foreign_receiver_entry_keeps_both_engines_provider_ownership_protected() {
    let first_case = Case::new();
    let second_case = Case::new();
    let (mut first, _first_events) = first_case.engine(8, false);
    let (mut second, _second_events) = second_case.engine(8, false);
    let second_admission = second.copy_admission.clone();
    let (first_sender, first_receiver) = first.command_channel(8).unwrap();
    let (second_sender, second_receiver) = second.command_channel(8).unwrap();
    let (first_command, first_probe) = first_case.capture(1, false, false);
    let (second_command, second_probe) = second_case.capture(1, false, true);
    let _release = Release(second_probe.clone());
    let first_sent = match first_sender.try_send(first_command) {
        Ok(()) => true,
        Err(error) => {
            retire_refused_sync(error.into_inner());
            false
        }
    };
    let second_sent = match second_sender.try_send(second_command) {
        Ok(()) => true,
        Err(error) => {
            second_probe.release();
            retire_refused_sync(error.into_inner());
            false
        }
    };
    drop(first.run(second_receiver));
    let foreign_held = wait_sync(OBSERVE, || second_probe.close_entered());
    let own_receiver_still_owns = !first_probe.closed();
    let capacity_held = second_admission.capacity().available_permits() == 31;
    second_probe.release();
    let foreign_closed = wait_sync(OBSERVE, || second_probe.closed());
    drop(first_receiver);
    drop(second);
    let own_closed = wait_sync(OBSERVE, || first_probe.closed());
    assert!(
        first_sent
            && second_sent
            && foreign_held
            && own_receiver_still_owns
            && capacity_held
            && foreign_closed
            && own_closed
    );
    assert_eq!(first_probe.calls() + second_probe.calls(), 0);
    assert!(first_probe.checked_close() && second_probe.checked_close());
}

#[tokio::test]
async fn outstanding_typed_permit_after_receiver_close_retires_the_accepted_guard() {
    let case = Case::new();
    let (mut engine, _events) = case.engine(8, false);
    let admission = engine.copy_admission.clone();
    let (sender, receiver) = engine.command_channel(8).unwrap();
    let permit = sender.clone().reserve_owned().await.unwrap();
    drop(receiver);
    let transport_closed = wait_async(OBSERVE, || sender.is_closed()).await;
    let (command, probe) = case.capture(1, false, true);
    let _release = Release(probe.clone());
    let accepted = match permit.try_send(command) {
        Ok(()) => true,
        Err(error) => {
            probe.release();
            fixture::retire_refused(error.into_inner()).await;
            false
        }
    };
    let held = wait_async(OBSERVE, || probe.close_entered()).await;
    let capacity_held = admission.capacity().available_permits() == 31;
    let not_closed = !probe.closed();
    probe.release();
    let closed = wait_async(OBSERVE, || probe.closed()).await;
    let returned = wait_async(OBSERVE, || admission.capacity().available_permits() == 32).await;
    drop(engine);
    assert!(
        transport_closed && accepted && held && capacity_held && not_closed && closed && returned
    );
    assert!(probe.checked_close() && !probe.timed_out());
    assert_eq!(probe.calls(), 0);
}
