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
    {
        // A resume for another ID leaves this job's pause in force; while it holds,
        // the worker cannot reach the native service (no timing window to wait out).
        let registry = h.controller.registry.lock().unwrap();
        let cancel = &registry[&h.request.id].cancel;
        assert!(cancel.pause_requested() && cancel.is_paused());
    }
    assert!(h.calls.deleted.lock().unwrap().is_empty());
    h.controller.set_paused(h.request.id, false).await;
    let done = h.finished().await;
    assert_eq!(done.state, OperationState::Completed);
    assert!(!done.paused && !done.pause_requested);
    assert_eq!(h.calls.deleted.lock().unwrap().len(), 1);
}
/// `WorkerOwned::cancelled` stores Cancelled in the job's shared status before its
/// asynchronous retirement, and that store can precede the actor's pause rescan that
/// observes the cancellation (a disposal polled on another runtime worker, or a paused
/// queued job whose gate wait sees the cancellation first). The rescan must not publish
/// a terminal status, whatever pause flags it still holds: only `finish` reports the
/// terminal state, once, and without pause flags.
#[tokio::test]
async fn a_cancelled_paused_delete_reports_its_terminal_state_once() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    let id = h.request.id;
    h.controller.prepare(h.request.clone()).await;
    h.preview().await;
    h.controller.set_paused(id, true).await;
    {
        let registry = h.controller.registry.lock().unwrap();
        let mut status = registry[&id].status.lock().unwrap();
        assert!(status.pause_requested);
        // Stored before the rescan observes the cancellation, still with the pause.
        status.state = OperationState::Cancelled;
    }
    h.controller.cancel(id);
    h.controller.shutdown();
    let exited = tokio::time::timeout(Duration::from_secs(5), h.tasks.join_next()).await;
    assert!(matches!(exited, Ok(Some(Ok(())))), "{exited:?}");
    let mut terminal = Vec::new();
    while let Ok(event) = h.events.try_recv() {
        if let Event::Operation(OperationEvent::Progress(progress)) = event
            && progress.id == id
            && progress.state.is_terminal()
        {
            terminal.push(progress);
        }
    }
    assert_eq!(terminal.len(), 1, "{terminal:#?}");
    assert_eq!(terminal[0].state, OperationState::Cancelled);
    assert!(!terminal[0].pause_requested && !terminal[0].paused);
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
