//! A dequeued checksum still owns its capture while waiting for the process worker.
use super::retirement_fixture::{Provider, captured};
use super::*;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc as channel,
    },
    thread,
};

#[tokio::test(flavor = "current_thread")]
async fn checksum_retirement_covers_backend_work_pending_on_worker_admission() {
    let _quota = test_quota_fixture().await;
    let busy = workers().acquire_owned().await.unwrap();
    let mut controller = Controller::new(Retirement::testing(), Arc::new(Provider(None)));
    let lease = controller
        .retirement
        .as_ref()
        .unwrap()
        .try_reserve()
        .unwrap();
    let (dropped, received) = channel::channel();
    let resolved = Arc::new(AtomicUsize::new(0));
    let input = captured(1, dropped, resolved.clone());
    let request = ChecksumRequest::new(
        input.job,
        input.generation,
        vec![
            std::env::current_dir()
                .unwrap()
                .join("_build/checksum-lifetime-source"),
        ],
    );
    let retention = reserve(
        request.validate().unwrap(),
        request.limits.max_retained_bytes,
    )
    .unwrap();
    controller
        .admit_prepared(lease.own(Work::calculate(
            request,
            Cancellation::default(),
            retention,
            Some(input.sources),
            controller._service.clone(),
        )))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while controller.requests.capacity() != MAX_CHECKSUM_JOBS {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        resolved.load(Ordering::SeqCst),
        0,
        "native execution remains gated"
    );
    let task = controller.task.take().unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(controller);
    drop(busy);
    let dropped_on = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(
        dropped_on,
        thread::current().id(),
        "backend Work dropped final capture inline"
    );
    test_wait_for_quota_idle().await;
}
