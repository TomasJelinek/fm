use super::*;

async fn paused(h: &mut Harness) -> Arc<OperationProgress> {
    loop {
        if let OperationEvent::Progress(progress) = h.next().await
            && progress.paused
        {
            assert!(progress.pause_requested);
            return progress;
        }
    }
}
#[tokio::test]
async fn confirmed_delete_waits_for_exact_resume_before_calling_native_service() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.set_paused(h.request.id, true).await;
    h.controller.confirm(h.request.id, None).await;
    paused(&mut h).await;
    assert!(h.calls.deleted.lock().unwrap().is_empty());
    h.controller.set_paused(OperationId(999), false).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(h.calls.deleted.lock().unwrap().is_empty());
    h.controller.set_paused(h.request.id, false).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Completed);
    assert!(!done.paused && !done.pause_requested);
    assert_eq!(h.calls.deleted.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn cancelled_paused_delete_retires_without_running_and_stale_resume_is_ignored() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Trash));
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.set_paused(h.request.id, true).await;
    h.controller.confirm(h.request.id, None).await;
    paused(&mut h).await;
    h.controller.cancel(h.request.id);
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Cancelled);
    assert!(!done.paused && !done.pause_requested);
    h.controller.set_paused(h.request.id, false).await;
    assert!(h.calls.deleted.lock().unwrap().is_empty());
    assert!(h.controller.registry.lock().unwrap().is_empty());
}
