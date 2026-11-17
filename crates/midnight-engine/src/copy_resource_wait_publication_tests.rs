//! Resource waits share the job heartbeat, including retry after a full result queue.
use super::*;
use std::{future::Future, pin::Pin, task::Poll};

async fn assert_pending<T>(mut pending: Pin<&mut impl Future<Output = T>>) {
    std::future::poll_fn(|context| {
        assert!(pending.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
}

fn next_progress(events: &mut mpsc::Receiver<Event>) -> Arc<CopyProgress> {
    match events.try_recv().expect("expected one progress snapshot") {
        Event::Copy(CopyEvent::Progress(progress)) => progress,
        event => panic!("unexpected event: {event:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn resource_wait_clear_retries_after_full_outbox_without_a_second_heartbeat() {
    let progress =
        CopyProgress::queued(OperationId(1), Default::default(), 1, TransferIntent::Copy);
    let status = Arc::new(Mutex::new(progress));
    status.lock().unwrap().state = CopyOperationState::Preparing;
    let tracker = Mutex::new(Tracker::default());
    let budget = Arc::new(Semaphore::new(0));
    let cancel = Cancellation::default();
    let (sender, mut events) = mpsc::channel(1);
    let outbox = CopyOutbox::unblocked(Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    });
    let pending = acquire_resources(&budget, &cancel, &status, &outbox, Some(&tracker));
    tokio::pin!(pending);
    assert_pending(pending.as_mut()).await;
    assert_eq!(
        status.lock().unwrap().wait_reason,
        Some(TransferWaitReason::Resources)
    );
    assert!(
        events.try_recv().is_err(),
        "wait entry bypassed the heartbeat"
    );

    // The acquisition future cannot emit its own progress at or between job ticks.
    for _ in 0..2 {
        tokio::time::advance(CADENCE).await;
        assert_pending(pending.as_mut()).await;
        assert!(events.try_recv().is_err(), "duplicate wait heartbeat");
    }
    publish_progress_tick(&outbox, &status, sample_throughput(&status, &tracker));
    // Keep this delivered wait snapshot in the one-slot queue while the wait clears.
    assert_eq!(events.len(), 1);
    budget.add_permits(1);
    let permit = tokio::time::timeout(Duration::from_secs(1), pending.as_mut())
        .await
        .expect("released resource wait did not finish")
        .expect("released resource permit was lost");
    assert_eq!(status.lock().unwrap().wait_reason, None);
    assert_eq!(budget.available_permits(), 0);
    publish_progress_tick(&outbox, &status, sample_throughput(&status, &tracker));
    assert_eq!(
        next_progress(&mut events).wait_reason,
        Some(TransferWaitReason::Resources)
    );

    // Inactive preparation produces no throughput changes to rescue a lost clear.
    tokio::time::advance(CADENCE).await;
    publish_progress_tick(&outbox, &status, sample_throughput(&status, &tracker));
    let cleared = next_progress(&mut events);
    assert_eq!(cleared.wait_reason, None);
    assert_eq!(cleared.state, CopyOperationState::Preparing);
    assert_eq!(cleared.discovery.state, DiscoveryState::NotStarted);
    assert!(cleared.throughput.samples.is_empty());
    assert!(cleared.verification_throughput.samples.is_empty());
    publish_progress_tick(&outbox, &status, sample_throughput(&status, &tracker));
    // The timer also publishes unchanged preparation state as a heartbeat.
    assert_eq!(next_progress(&mut events).wait_reason, None);
    assert!(
        events.try_recv().is_err(),
        "one tick published duplicate progress"
    );
    drop(permit);
    assert_eq!(budget.available_permits(), 1);
}
