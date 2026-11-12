//! Last capture/service ownership must survive independent actor cancellation.
use super::*;
use crate::checksum::retirement_fixture::{Provider, captured};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc as channel,
    },
    thread,
};

#[tokio::test(flavor = "current_thread")]
async fn checksum_retirement_covers_dequeued_and_queued_captures_when_adapter_aborts() {
    let _quota = checksum::test_quota_fixture().await;
    let (sender, _events) = mpsc::channel(1);
    assert!(
        sender
            .try_send(Event::Checksum(Box::new(ChecksumEvent {
                job: ChecksumJobId(99),
                generation: ChecksumGeneration(1),
                update: ChecksumUpdate::Queued,
            })))
            .is_ok()
    );
    let mut controller = Controller::new(
        Retirement::testing(),
        Some(Arc::new(Provider(None))),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        Arc::new(Semaphore::new(1)),
    );
    let resolved = Arc::new(AtomicUsize::new(0));
    let (first, first_drop) = channel::channel();
    controller
        .start_captured(captured(1, first, resolved.clone()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while controller.input.capacity() != 32 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (second, second_drop) = channel::channel();
    controller
        .start_captured(captured(2, second, resolved.clone()))
        .await
        .unwrap();
    assert_eq!(
        controller.input.capacity(),
        31,
        "second capture remains queued"
    );
    assert_eq!(
        resolved.load(Ordering::SeqCst),
        0,
        "first capture waits on full outbox"
    );
    let task = controller.task.take().unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(controller);
    let threads = [
        first_drop.recv_timeout(Duration::from_secs(5)).unwrap(),
        second_drop.recv_timeout(Duration::from_secs(5)).unwrap(),
    ];
    assert!(
        threads
            .iter()
            .all(|thread| *thread != thread::current().id()),
        "captured provider dropped on executor: {threads:?}"
    );
    checksum::test_wait_for_quota_idle().await;
}

#[test]
fn checksum_retirement_covers_service_when_runtime_stops_before_actor_is_polled() {
    let (dropped, received) = channel::channel();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let controller = {
        let _entered = runtime.enter();
        let (sender, _events) = mpsc::channel(1);
        Controller::new(
            Retirement::testing(),
            Some(Arc::new(Provider(Some(dropped)))),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            Arc::new(Semaphore::new(1)),
        )
    };
    drop(runtime);
    drop(controller);
    let dropped_on = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(
        dropped_on,
        thread::current().id(),
        "runtime dropped final service inline"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn checksum_retirement_covers_completed_unconsumed_preparation_output() {
    let _quota = checksum::test_quota_fixture().await;
    let resources = Retirement::testing();
    let lease = resources.domain.try_reserve().unwrap();
    let service = Service::new(resources.service, Some(Arc::new(Provider(None))));
    let (dropped, received) = channel::channel();
    let resolved = Arc::new(AtomicUsize::new(0));
    let request = captured(1, dropped, resolved.clone());
    let retention = checksum::reserve(8 * 1024 * 1024, request.limits.max_retained_bytes).unwrap();
    let start = lease.own(Start {
        capture: Some(Capture::Captured(request)),
        key: (ChecksumJobId(1), ChecksumGeneration(1)),
        cancel: Cancellation::default(),
        retention: Some(retention),
        service,
    });
    let task = tokio::task::spawn_blocking(move || prepare(start));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(resolved.load(Ordering::SeqCst), 1);
    assert!(
        received.try_recv().is_err(),
        "prepared output retains the capture"
    );
    drop(task);
    let dropped_on = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(
        dropped_on,
        thread::current().id(),
        "completed output dropped capture inline"
    );
    checksum::test_wait_for_quota_idle().await;
}

#[tokio::test(flavor = "current_thread")]
async fn checksum_retirement_keeps_rejected_capture_slot_and_quota_until_drop_finishes() {
    use crate::checksum::retirement_fixture::{DropGate, ReleaseGate, captured_gated};
    let _quota = checksum::test_quota_fixture().await;
    let resources = Retirement::testing();
    let domain = resources.domain.clone();
    let (sender, _events) = mpsc::channel(1);
    let controller = Arc::new(Controller::new(
        resources,
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        Arc::new(Semaphore::new(1)),
    ));
    let gate = DropGate::new();
    let _release = ReleaseGate(gate.clone());
    let (dropped, received) = channel::channel();
    let request = captured_gated(
        1,
        dropped,
        Arc::new(AtomicUsize::new(0)),
        Some(gate.clone()),
    );
    let rejecting = controller.clone();
    let pending = tokio::spawn(async move { rejecting.start_captured(request).await });
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    assert_ne!(
        received.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    assert!(
        !pending.is_finished(),
        "rejection must acknowledge actual destruction"
    );
    assert!(
        controller.intake.try_lock().is_err(),
        "single rejection intake remains occupied"
    );
    let mut spare = Vec::new();
    while let Ok(lease) = domain.try_reserve() {
        spare.push(lease);
    }
    assert_eq!(
        spare.len(),
        RETIREMENT_CAPACITY - 2,
        "service and capture slots stay occupied"
    );
    let mut quota = Vec::new();
    while let Ok(retention) = checksum::reserve(0, 0) {
        quota.push(retention);
    }
    assert_eq!(
        quota.len(),
        MAX_CHECKSUM_JOBS - 1,
        "capture retains its admitted quota"
    );
    gate.release();
    let error = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert!(domain.try_reserve().is_ok(), "actual drop returns the slot");
    assert!(
        checksum::reserve(0, 0).is_ok(),
        "actual drop returns the checksum quota"
    );
    drop(quota);
    drop(spare);
    Arc::try_unwrap(controller).ok().unwrap().shutdown().await;
}

#[test]
fn checksum_retirement_covers_unstarted_blocking_preparation_at_runtime_shutdown() {
    use crate::checksum::retirement_fixture::{DropGate, ReleaseGate};
    let resources = Retirement::testing();
    let lease = resources.domain.try_reserve().unwrap();
    let service = Service::new(resources.service, Some(Arc::new(Provider(None))));
    let (dropped, received) = channel::channel();
    let resolved = Arc::new(AtomicUsize::new(0));
    let start = lease.own(Start {
        capture: Some(Capture::Captured(captured(1, dropped, resolved.clone()))),
        key: (ChecksumJobId(1), ChecksumGeneration(1)),
        cancel: Cancellation::default(),
        retention: Some(ChecksumRetention::new(())),
        service,
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let gate = DropGate::new();
    let _release = ReleaseGate(gate.clone());
    let (entered, started) = channel::channel();
    let (blocker, preparing) = {
        let _entered = runtime.enter();
        let blocking = gate.clone();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            blocking.wait();
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        (blocker, tokio::task::spawn_blocking(move || prepare(start)))
    };
    assert!(
        !preparing.is_finished(),
        "the sole blocking worker is occupied"
    );
    // Bounded shutdown alone may still run queued blocking work. Explicitly
    // cancel this gated, unstarted handle to exercise that ownership boundary.
    preparing.abort();
    runtime.shutdown_timeout(Duration::from_millis(10));
    gate.release();
    drop(blocker);
    drop(preparing);
    let dropped_on = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        resolved.load(Ordering::SeqCst),
        0,
        "queued preparation never ran"
    );
    assert_ne!(
        dropped_on,
        thread::current().id(),
        "runtime dropped queued capture inline"
    );
}
