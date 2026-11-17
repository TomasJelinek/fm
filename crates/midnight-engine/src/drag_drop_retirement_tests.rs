use super::*;
#[path = "drag_drop_retirement_fixture.rs"]
mod fixture;
use fixture::{Case, Copies, DropHarness, Release};

const OBSERVE: Duration = Duration::from_secs(8);
const QUIET: Duration = Duration::from_millis(350);

async fn accepted_file_retirement(fill_then_cancel: bool) {
    let case = Case::new();
    let _release = Release(case.trace.clone());
    let mut copies = Copies::new(case.service());
    let mut drag = DropHarness::new(copies.controller.admission(), &case);
    let token = drag.execute().await;
    // This is an awaited worker drain of the actual retired Source, not a timed
    // guess or Arc count. Only Accepted's sealed registration now owns the file.
    drag.drain_old_gesture().await;
    let before = copies.controller.capacity().available_permits();
    let held_io = fill_then_cancel.then(|| copies.io.clone().try_acquire_owned().unwrap());
    if fill_then_cancel {
        drag.controller
            .accept(
                token,
                OperationId(1),
                TransferVerification::default(),
                &copies.controller,
            )
            .await;
    }
    let b = if fill_then_cancel { 2 } else { 1 };
    copies.controller.enqueue(case.request(b)).await;
    let waiting = copies.wait_earlier(b).await;
    let extra = tokio::time::timeout(OBSERVE, async {
        let mut extra = Vec::new();
        for _ in 0..30 {
            loop {
                match copies.controller.reserve() {
                    Ok(reservation) => {
                        extra.push(reservation);
                        break;
                    }
                    Err(error) if error.kind == io::ErrorKind::WouldBlock => {
                        // Native admission deliberately refuses transient metadata contention.
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("unexpected reservation failure: {error:?}"),
                }
            }
        }
        extra
    })
    .await
    .expect("available native drop capacity must become reservable");
    let saturated = copies.controller.capacity().available_permits() == 0
        && copies.controller.reserve().is_err();
    let early_read = case.trace.wait_read(QUIET).await;
    let rejected = if fill_then_cancel {
        copies.controller.cancel(OperationId(1));
        None
    } else {
        drag.controller.reject(token);
        Some(drag.submit_backlog().await)
    };
    drop(held_io);
    let file_held = case.trace.wait_entered(OBSERVE).await;
    let held_capacity = copies.controller.capacity().available_permits();
    let idle_disk = copies.io.available_permits();
    let while_held_read = case.trace.wait_read(QUIET).await;
    let closed_early = case.trace.closed();
    // A generic retirement ACK may precede Copy's nested sealed cleanup. Native
    // completion is witnessed only by the actual file Drop and B's real read.
    case.trace.release();
    drop(extra);
    let b_read = case.trace.wait_read(OBSERVE).await;
    let closed = case.trace.wait_closed(OBSERVE).await;
    if let Some(retired) = rejected {
        tokio::time::timeout(OBSERVE, retired)
            .await
            .unwrap()
            .unwrap();
    }
    let terminal = copies.terminal(b).await;
    let a_terminal = if fill_then_cancel {
        copies.terminal(1).await
    } else {
        None
    };
    let returned = copies.wait_capacity(32).await;
    drag.close().await;
    copies.close().await;

    assert_eq!(
        before, 31,
        "DropAccepted must already own the real admission slot"
    );
    assert!(waiting && saturated && file_held);
    assert!(!early_read && !while_held_read && !closed_early);
    assert_eq!(
        held_capacity, 0,
        "the held native file must retain A's slot"
    );
    assert_eq!(
        idle_disk, 1,
        "an unfilled/cancelled A must not fake an I/O permit hold"
    );
    assert!(closed && b_read && returned);
    assert!(case.trace.close_precedes_read());
    assert!(case.trace.checked_last_release());
    assert!(!case.trace.timed_out());
    let terminal = terminal.expect("B must produce a terminal event after cleanup");
    assert_eq!(
        terminal.state,
        CopyOperationState::Completed,
        "{terminal:?}"
    );
    assert!(terminal.errors.is_empty());
    assert_eq!(terminal.verified_bytes, case.bytes.len() as u64);
    assert_eq!(std::fs::read(case.output()).unwrap(), case.bytes);
    if fill_then_cancel {
        assert_eq!(a_terminal.unwrap().state, CopyOperationState::Cancelled);
        assert!(!case.destination.join("accepted-a").exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_drop_rejection_keeps_capacity_and_finished_until_actual_file_close() {
    accepted_file_retirement(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_drop_fill_then_cancel_keeps_waiting_successor_out_of_native_io() {
    accepted_file_retirement(true).await;
}

#[test]
fn accepted_drop_abandonment_after_runtime_shutdown_closes_file_on_domain_worker() {
    let case = Case::new();
    let _release = Release(case.trace.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (drag, token, capacity) = runtime.block_on(async {
        let admission = copy_operations::Admission::default();
        let capacity = admission.capacity();
        let mut drag = DropHarness::new(admission, &case);
        let token = drag.execute().await;
        drag.drain_old_gesture().await;
        drag.stop_forwarder().await;
        (drag, token, capacity)
    });
    drop(runtime);
    // Remove the actual Accepted value, including its original sealed owner.
    // No synthetic retain_provider hook and no cloned token/registration is used.
    let accepted = drag
        .controller
        .pending
        .lock()
        .unwrap()
        .remove(&token)
        .unwrap();
    let abandoning = std::thread::spawn(move || {
        let thread = std::thread::current().id();
        drop(accepted);
        thread
    });
    let file_held = case.trace.wait_entered_sync(OBSERVE);
    let held_capacity = capacity.available_permits();
    let closed_early = case.trace.closed();
    case.trace.release();
    let closed = case.trace.wait_closed_sync(OBSERVE);
    let abandoning_thread = abandoning.join().unwrap();
    let returned = fixture::wait_sync(OBSERVE, || capacity.available_permits() == 32);
    drag.controller.shutdown();
    drop(drag);

    assert!(file_held && closed && returned);
    assert_eq!(held_capacity, 31);
    assert!(!closed_early);
    assert_ne!(case.trace.drop_thread(), Some(abandoning_thread));
    assert!(case.trace.checked_last_release());
    assert!(!case.trace.timed_out());
}
