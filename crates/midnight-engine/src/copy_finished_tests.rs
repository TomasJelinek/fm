use super::*;
use midnight_core::{CopyOperationState, CopyProgress, OperationId, TransferIntent};
use std::{future::Future, path::PathBuf, task::Poll};

#[test]
fn unpublished_owner_and_preparation_do_not_publish_or_consume_a_sequence() {
    let mut registry = Registry::default();
    let unpublished = Owner::unpublished();
    assert_eq!(unpublished.sequence(), 0);
    assert_eq!(registry.prepare_sequence(), Ok(1));
    assert_eq!(registry.prepare_sequence(), Ok(1));
    assert!(registry.pending.is_empty());
    assert_eq!(registry.last, 0);
    drop(unpublished);
    let (first, earlier) = registry.register().unwrap();
    assert_eq!(first.sequence(), 1);
    assert!(earlier.is_finished());
    first.finish();
}

#[test]
fn prepared_publication_reuses_storage_and_captures_only_earlier_owners() {
    let mut registry = Registry::default();
    let registry_storage = registry.pending.as_ptr();
    let mut first = Owner::unpublished();
    let first_identity = Arc::as_ptr(&first.0);
    let mut first_wait = Predecessors::with_capacity();
    let first_storage = first_wait.0.as_ptr();
    let sequence = registry.prepare_sequence().unwrap();
    registry.publish_prepared(sequence, &mut first, &mut first_wait);
    let mut second = Owner::unpublished();
    let mut second_wait = Predecessors::with_capacity();
    let second_storage = second_wait.0.as_ptr();
    let sequence = registry.prepare_sequence().unwrap();
    registry.publish_prepared(sequence, &mut second, &mut second_wait);
    let (third, third_wait) = registry.register().unwrap();
    assert_eq!(registry.pending.as_ptr(), registry_storage);
    assert_eq!(first_wait.0.as_ptr(), first_storage);
    assert_eq!(second_wait.0.as_ptr(), second_storage);
    assert_eq!(Arc::as_ptr(&registry.pending[0]), first_identity);
    assert_eq!(second_wait.0.len(), 1);
    assert_eq!(
        (first.sequence(), second.sequence(), third.sequence()),
        (1, 2, 3)
    );
    assert!(first_wait.is_finished());
    assert!(!second_wait.is_finished() && !third_wait.is_finished());
    first.finish();
    assert!(second_wait.is_finished());
    assert!(!third_wait.is_finished());
    second.finish();
    assert!(third_wait.is_finished());
    third.finish();
}

#[test]
fn prepare_refuses_full_or_exhausted_registry_without_publishing() {
    let mut registry = Registry::default();
    let mut owners = Vec::new();
    for _ in 0..crate::copy_operations::MAX_JOBS {
        owners.push(registry.register().unwrap().0);
    }
    let storage = registry.pending.as_ptr();
    let last = registry.last;
    assert!(registry.prepare_sequence().is_err());
    assert_eq!(registry.last, last);
    assert_eq!(registry.pending.len(), crate::copy_operations::MAX_JOBS);
    owners.pop().unwrap().finish();
    assert_eq!(registry.prepare_sequence(), Ok(last + 1));
    assert_eq!(registry.last, last);
    assert_eq!(registry.pending.as_ptr(), storage);
    for owner in owners {
        owner.finish();
    }
    // The last sequence value is usable; only the next one is refused, without wrapping.
    registry.last = u64::MAX - 1;
    let (final_owner, _) = registry.register().unwrap();
    assert_eq!(final_owner.sequence(), u64::MAX);
    final_owner.finish();
    assert!(registry.prepare_sequence().is_err());
    assert!(registry.register().is_err());
    assert_eq!(registry.last, u64::MAX);
    assert!(registry.pending.is_empty());
}

#[test]
fn observer_only_confirms_explicit_finish_and_never_owner_loss() {
    let mut registry = Registry::default();
    let (owner, _) = registry.register().unwrap();
    let observer = owner.observer();
    drop(owner);
    assert!(!observer.is_finished());
    let (confirmed, _) = registry.register().unwrap();
    let confirmed_observer = confirmed.observer();
    confirmed.finish();
    assert!(confirmed_observer.is_finished());
    registry.prepare_sequence().unwrap();
    assert!(confirmed_observer.is_finished());
    assert!(!observer.is_finished());
    assert_eq!(registry.pending.len(), 1);
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
