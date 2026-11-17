use super::*;
use midnight_core::{CopyOperationState, CopyProgress, OperationId, TransferIntent};
use std::{future::Future, path::PathBuf, sync::mpsc, task::Poll, thread};

#[test]
fn snapshots_include_only_earlier_unfinished_reservations_and_never_self() {
    let mut registry = Registry::default();
    let (first, first_wait) = registry.register().unwrap();
    let (second, second_wait) = registry.register().unwrap();
    let (third, third_wait) = registry.register().unwrap();
    assert_eq!(
        (first.sequence(), second.sequence(), third.sequence()),
        (1, 2, 3)
    );
    assert!(first_wait.is_finished());
    assert!(!second_wait.is_finished());
    assert!(!third_wait.is_finished());
    first.finish();
    assert!(
        second_wait.is_finished(),
        "self and future admissions cannot extend the snapshot"
    );
    assert!(!third_wait.is_finished());
    second.finish();
    assert!(third_wait.is_finished());
    third.finish();
}

#[test]
fn dropping_an_unconfirmed_owner_does_not_finish_or_prune_its_latch() {
    let mut registry = Registry::default();
    let (first, _) = registry.register().unwrap();
    let (second, wait) = registry.register().unwrap();
    drop(first);
    assert!(!wait.is_finished());
    second.finish();
    let (third, later) = registry.register().unwrap();
    assert!(
        !later.is_finished(),
        "registry must retain the vanished owner's unfinished state"
    );
    assert_eq!(registry.pending.len(), 2);
    third.finish();
}

#[test]
fn strong_pending_registry_is_bounded_and_only_confirmed_finish_returns_room() {
    let mut registry = Registry::default();
    let mut owners = Vec::new();
    for _ in 0..crate::copy_operations::MAX_JOBS {
        owners.push(registry.register().unwrap().0);
    }
    assert!(registry.register().is_err());
    drop(owners.pop().unwrap());
    assert!(
        registry.register().is_err(),
        "unconfirmed loss is not reusable capacity"
    );
    owners.pop().unwrap().finish();
    let (replacement, wait) = registry.register().unwrap();
    assert_eq!(
        replacement.sequence(),
        crate::copy_operations::MAX_JOBS as u64 + 1
    );
    assert!(!wait.is_finished());
    assert_eq!(registry.pending.len(), crate::copy_operations::MAX_JOBS);
    replacement.finish();
    for owner in owners {
        owner.finish();
    }
}

#[test]
fn exhausted_sequence_refuses_without_wrapping_or_registering_a_latch() {
    let mut registry = Registry {
        last: u64::MAX - 1,
        ..Registry::default()
    };
    let (last, _) = registry.register().unwrap();
    assert_eq!(last.sequence(), u64::MAX);
    last.finish();
    assert!(registry.register().is_err());
    assert!(registry.register().is_err());
    assert_eq!(registry.last, u64::MAX);
    assert!(registry.pending.is_empty());
}

struct HeldDrop {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Drop for HeldDrop {
    fn drop(&mut self) {
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(4)).unwrap();
    }
}

#[test]
fn retained_input_destructor_must_return_before_owner_commit_releases_waiters() {
    let mut registry = Registry::default();
    let (first, _) = registry.register().unwrap();
    let (second, wait) = registry.register().unwrap();
    let (entered, entered_rx) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let worker = thread::spawn(move || {
        drop(HeldDrop {
            entered,
            release: released,
        });
        first.finish();
    });
    entered_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    let pending_during_destructor = !wait.is_finished();
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(pending_during_destructor);
    assert!(wait.is_finished());
    second.finish();
}

#[tokio::test]
async fn cancelled_wait_does_not_finish_predecessors_or_require_their_completion() {
    let mut registry = Registry::default();
    let (first, _) = registry.register().unwrap();
    let (second, wait) = registry.register().unwrap();
    let cancel = Cancellation::default();
    let waiting = wait.wait(&cancel);
    tokio::pin!(waiting);
    std::future::poll_fn(|context| {
        assert!(waiting.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    cancel.cancel();
    assert!(
        !tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
    );
    assert!(!wait.is_finished());
    first.finish();
    second.finish();
}

#[tokio::test]
async fn explicit_completion_wakes_an_already_parked_waiter() {
    let mut registry = Registry::default();
    let (first, _) = registry.register().unwrap();
    let (second, wait) = registry.register().unwrap();
    let cancel = Cancellation::default();
    let waiting = wait.wait(&cancel);
    tokio::pin!(waiting);
    std::future::poll_fn(|context| {
        assert!(waiting.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    first.finish();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
    );
    second.finish();
}

#[tokio::test]
async fn failed_and_cancelled_retirement_finish_without_satisfying_the_success_fence() {
    for state in [CopyOperationState::Failed, CopyOperationState::Cancelled] {
        let mut registry = Registry::default();
        let (first, _) = registry.register().unwrap();
        let (second, wait) = registry.register().unwrap();
        let fence = crate::copy_fence::CopyFence::default();
        let receipt = fence.register();
        let success_wait = fence.snapshot();
        let mut progress =
            CopyProgress::queued(OperationId(1), PathBuf::new(), 1, TransferIntent::Copy);
        progress.state = state;
        receipt.finish(&progress);
        assert!(
            !wait.is_finished(),
            "an outcome is not a teardown acknowledgement"
        );
        first.finish();
        let cancel = Cancellation::default();
        assert!(wait.wait(&cancel).await);
        assert!(
            success_wait
                .wait(&cancel, &progress.destination_directory)
                .await
                .is_err()
        );
        second.finish();
    }
}
