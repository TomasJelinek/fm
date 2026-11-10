use super::*;

#[tokio::test]
async fn short_delete_event_retains_items_not_bytes() {
    for mode in [DeleteMode::Permanent, DeleteMode::Trash] {
        let mut h = Harness::new(MutationKind::Delete(mode));
        h.controller.prepare(h.request.clone()).await;
        h.preview().await;
        h.controller.confirm(h.request.id, None).await;
        let done = h.finished().await;
        assert_eq!(done.state, OperationState::Completed);
        assert_eq!(done.throughput.items_per_second, 0.0);
        assert!(
            done.throughput
                .samples
                .iter()
                .any(|sample| sample.items_per_second > 0.0)
        );
        assert!(done.throughput.samples.iter().all(
            |sample| sample.transfer_per_second == 0.0 && sample.verification_per_second == 0.0
        ));
    }
}

#[tokio::test]
async fn blocked_delete_emits_zero_samples_until_the_native_call_returns() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Trash));
    h.calls.blocked.store(true, Ordering::Release);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.confirm(h.request.id, None).await;
    tokio::time::timeout(Duration::from_secs(2), h.calls.entered.notified())
        .await
        .unwrap();
    let sampled = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let OperationEvent::Progress(progress) = h.next().await
                && !progress.throughput.samples.is_empty()
            {
                break progress;
            }
        }
    })
    .await;
    h.calls.blocked.store(false, Ordering::Release);
    let done = h.finished().await;
    let sampled = sampled.expect("blocked Delete must emit observed zero activity");
    assert_eq!(sampled.state, OperationState::Running);
    assert_eq!(sampled.throughput.items_per_second, 0.0);
    assert_eq!(done.throughput.items_per_second, 0.0);
    assert!(
        done.throughput
            .samples
            .iter()
            .any(|sample| sample.items_per_second > 0.0)
    );
}

#[tokio::test]
async fn cancelled_delete_keeps_late_native_completion_in_history() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Trash));
    h.calls.blocked.store(true, Ordering::Release);
    h.calls.complete_after_cancel.store(true, Ordering::Release);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.confirm(h.request.id, None).await;
    tokio::time::timeout(Duration::from_secs(2), h.calls.entered.notified())
        .await
        .unwrap();
    h.controller.cancel(h.request.id);
    loop {
        if let OperationEvent::Progress(progress) = h.next().await
            && progress.state == OperationState::CancelRequested
            && !progress.throughput.samples.is_empty()
        {
            assert_eq!(progress.throughput.items_per_second, 0.0);
            break;
        }
    }
    h.calls.blocked.store(false, Ordering::Release);
    let done = h.finished().await;
    assert_eq!(done.processed_entries, 1);
    assert_eq!(done.throughput.items_per_second, 0.0);
    assert!(
        done.throughput
            .samples
            .iter()
            .any(|sample| sample.items_per_second > 0.0)
    );
}

#[tokio::test]
async fn delete_speed_settles_to_zero_after_reported_work_then_a_native_stall() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    h.calls
        .blocked_after_progress
        .store(true, Ordering::Release);
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.confirm(h.request.id, None).await;
    let mut observed_activity = false;
    loop {
        if let OperationEvent::Progress(progress) = h.next().await {
            observed_activity |= progress.throughput.items_per_second > 0.0;
            if observed_activity && progress.throughput.items_per_second == 0.0 {
                assert_eq!(progress.processed_entries, 1);
                assert_eq!(progress.state, OperationState::Running);
                break;
            }
        }
    }
    h.calls
        .blocked_after_progress
        .store(false, Ordering::Release);
    assert_eq!(h.finished().await.state, OperationState::Completed);
}
