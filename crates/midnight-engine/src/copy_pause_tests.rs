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

/// Two jobs paused before any work: resuming one runs exactly that job to completion
/// (a paused job holds no disk slot), while the other stays paused until resumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_and_move_pause_before_work_resume_only_exact_job_and_finish_normally() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        let (_temp, parent) = moves::protected_fixture();
        let first = parent.join("pause-first");
        let second = parent.join("pause-second");
        let destination = parent.join("pause-destination");
        fs::write(&first, b"content retained through pause").unwrap();
        fs::write(&second, b"resumed alone").unwrap();
        fs::create_dir(&destination).unwrap();
        let mut h = Harness::native();
        let gate = h
            .mutation
            .clone()
            .acquire_many_owned(crate::transfer_leases::NAMESPACE_WEIGHT)
            .await
            .unwrap();
        for (id, source) in [(1, &first), (2, &second)] {
            let request = request(id, std::slice::from_ref(source), &destination);
            match intent {
                TransferIntent::Copy => h.controller.enqueue(request).await,
                TransferIntent::Move => h.controller.enqueue_move(request).await,
            }
            h.controller.set_paused(OperationId(id), true).await;
        }
        drop(gate);
        for id in [1, 2] {
            let stopped = paused(&mut h, id).await;
            assert_eq!(stopped.copied_bytes, 0);
            assert_eq!(stopped.throughput.transfer_per_second, 0.0);
        }
        assert!(first.exists() && second.exists());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        h.controller.set_paused(OperationId(2), false).await;
        let resumed = h.terminal(2).await;
        assert_eq!(resumed.state, CopyOperationState::Completed, "{resumed:?}");
        let held = snapshot(&h.controller.registry.lock().unwrap()[&OperationId(1)].status);
        assert!(held.paused && held.copied_bytes == 0, "{held:?}");
        assert!(!destination.join("pause-first").exists());
        h.controller.set_paused(OperationId(1), false).await;
        let done = h.terminal(1).await;
        h.close().await;
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert!(!done.pause_requested && !done.paused);
        assert_eq!(
            fs::read(destination.join("pause-first")).unwrap(),
            b"content retained through pause"
        );
        assert_eq!(
            fs::read(destination.join("pause-second")).unwrap(),
            b"resumed alone"
        );
        assert_eq!(first.exists(), intent == TransferIntent::Copy);
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

/// WIN-1: a job paused in the middle of a file still reports that file's size and the
/// bytes written so far, from which its window's Details show "Current file written:
/// X / Y bytes"; the pause is taken at the step boundary after the blocked read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copy_paused_in_the_middle_of_a_file_reports_its_size_and_progress() {
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
        read_before_block: true,
    })));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    // The second read blocks: the first 256 KiB block is written.
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    h.controller.set_paused(OperationId(1), true).await;
    released.store(true, Ordering::Release);
    let stopped = paused(&mut h, 1).await;
    assert_eq!(stopped.phase, CopyOperationPhase::Copying, "{stopped:?}");
    assert_eq!(stopped.current_file_size, Some(600_000), "{stopped:?}");
    assert!(
        (1..600_000).contains(&stopped.current_file_copied),
        "{stopped:?}"
    );
    assert_eq!(stopped.current_file_copied, stopped.copied_bytes);
    h.controller.cancel(OperationId(1));
    let done = h.terminal(1).await;
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Cancelled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmation_waits_for_a_safe_checkpoint_and_preserves_manual_pause() {
    use CopyCancelConfirmationAction::{Begin, Confirm, Dismiss};

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
    let id = OperationId(1);
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();

    h.controller.cancel_confirmation(id, 10, Begin).await;
    h.controller.cancel_confirmation(id, 10, Confirm).await;
    loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.pause_requested
        {
            assert!(!progress.paused);
            assert_eq!(progress.cancel_confirmation_ready, None);
            break;
        }
    }
    assert!(
        !h.controller.registry.lock().unwrap()[&id]
            .cancel
            .is_cancelled()
    );
    released.store(true, Ordering::Release);

    loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.cancel_confirmation_ready == Some(10)
        {
            assert!(progress.paused);
            assert!(progress.copied_bytes > 0);
            break;
        }
    }
    assert!(h.io.available_permits() > 0);

    h.controller.set_paused(id, true).await;
    h.controller.cancel_confirmation(id, 10, Dismiss).await;
    h.controller.cancel_confirmation(id, 10, Begin).await;
    assert!(
        h.controller.registry.lock().unwrap()[&id]
            .cancel
            .is_paused()
    );

    h.controller.cancel_confirmation(id, 11, Begin).await;
    h.controller.cancel_confirmation(id, 10, Dismiss).await;
    h.controller.cancel_confirmation(id, 10, Confirm).await;
    loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.cancel_confirmation_ready == Some(11)
        {
            assert!(progress.paused);
            break;
        }
    }

    h.controller.cancel_confirmation(id, 11, Confirm).await;
    let done = h.terminal(1).await;
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert!(!done.paused && !done.pause_requested);
    assert_eq!(done.cancel_confirmation_ready, None);
    assert_eq!(fs::read(&source).unwrap().len(), 600_000);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
}
