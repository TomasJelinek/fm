use super::*;

async fn paused(h: &mut Harness, id: u64) -> Arc<CopyProgress> {
    loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.id == OperationId(id)
            && progress.paused
        {
            assert!(progress.pause_requested);
            return progress;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_and_move_pause_before_work_resume_only_exact_job_and_finish_normally() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let (_temp, parent) = moves::protected_fixture();
        let source = parent.join("pause-source");
        let destination = parent.join("pause-destination");
        fs::write(&source, b"content retained through pause").unwrap();
        fs::create_dir(&destination).unwrap();
        let mut h = Harness::native();
        let gate = h.mutation.clone().acquire_owned().await.unwrap();
        let request = request(1, std::slice::from_ref(&source), &destination);
        match intent {
            TransferIntent::Copy => h.controller.enqueue(request).await,
            TransferIntent::Move => h.controller.enqueue_move(request).await,
        }
        h.controller.set_paused(OperationId(1), true).await;
        drop(gate);
        let stopped = paused(&mut h, 1).await;
        assert_eq!(stopped.copied_bytes, 0);
        assert_eq!(stopped.throughput.transfer_per_second, 0.0);
        assert!(source.exists());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        h.controller.set_paused(OperationId(999), false).await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        h.controller.set_paused(OperationId(1), false).await;
        let done = h.terminal(1).await;
        h.close().await;
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert!(!done.pause_requested && !done.paused);
        assert_eq!(
            fs::read(destination.join("pause-source")).unwrap(),
            b"content retained through pause"
        );
        assert_eq!(source.exists(), intent == TransferIntent::Copy);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_flight_copy_is_not_falsely_acknowledged_and_cancel_cleans_paused_stage() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, vec![42; 600_000]).unwrap();
    fs::create_dir(&destination).unwrap();
    let entered = Arc::new(Notify::new());
    let released = Arc::new(AtomicBool::new(false));
    let _release = ReleaseOnDrop(released.clone());
    let mut h = Harness::new(Some(Arc::new(BlockingService {
        inner: midnight_platform::LocalFileSystem::new(None),
        entered: entered.clone(),
        released: released.clone(),
        prepared: Arc::new(AtomicUsize::new(0)),
        read_before_block: false,
    })));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    h.controller.set_paused(OperationId(1), true).await;
    loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.pause_requested
        {
            assert!(!progress.paused, "native call has not returned yet");
            break;
        }
    }
    assert_eq!(h.io.available_permits(), 0);
    released.store(true, Ordering::Release);
    let stopped = paused(&mut h, 1).await;
    assert!(stopped.copied_bytes > 0);
    assert_eq!(stopped.throughput.transfer_per_second, 0.0);
    h.controller.cancel(OperationId(1));
    let done = h.terminal(1).await;
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert!(!done.paused && !done.pause_requested);
    assert_eq!(fs::read(&source).unwrap().len(), 600_000);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
}
