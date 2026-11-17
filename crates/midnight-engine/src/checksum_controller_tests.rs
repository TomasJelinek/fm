use super::*;

struct StopOnDrop(Cancellation);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[tokio::test]
async fn cancelling_one_job_unblocks_full_batch_delivery_without_global_shutdown() {
    let (sender, _receiver) = mpsc::channel(1);
    let key = (ChecksumJobId(900), ChecksumGeneration(1));
    sender
        .try_send(ChecksumEvent {
            job: key.0,
            generation: key.1,
            update: ChecksumUpdate::Started,
        })
        .unwrap();
    let stop = Cancellation::default();
    let _release = StopOnDrop(stop.clone());
    let cancel = Cancellation::default();
    let worker_cancel = cancel.clone();
    let entered = Arc::new(tokio::sync::Notify::new());
    let worker_entered = entered.clone();
    let worker_stop = stop.clone();
    let blocked = tokio::task::spawn_blocking(move || {
        worker_entered.notify_one();
        send_update(
            &sender,
            key,
            ChecksumUpdate::Progress(ChecksumProgress::default()),
            &worker_stop,
            &worker_cancel,
        )
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    cancel.cancel();
    assert!(
        !tokio::time::timeout(Duration::from_secs(5), blocked)
            .await
            .unwrap()
            .unwrap()
    );
    assert!(
        !stop.is_cancelled(),
        "Cancelling one job must not close other windows."
    );
}

struct NoReads;
impl TransferService for NoReads {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(worker::failure(path, io::ErrorKind::Unsupported, "idle"))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(worker::failure(path, io::ErrorKind::Unsupported, "idle"))
    }
}

#[tokio::test(start_paused = true)]
async fn idle_backend_wakes_for_shutdown_without_a_polling_timer() {
    let controller = Controller::new(Retirement::testing(), Arc::new(NoReads));
    tokio::task::yield_now().await; // The actor reaches its idle wait.
    // Paused time advances only to fire a timer: an event-driven actor needs none.
    let started = tokio::time::Instant::now();
    controller.shutdown().await;
    assert_eq!(tokio::time::Instant::now(), started);
}
