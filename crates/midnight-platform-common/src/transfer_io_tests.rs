use super::*;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread,
    time::Duration,
};

#[test]
fn exclusive_move_call_borrows_the_saturated_configured_shared_owner() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    let _deadline = Deadline::new(&cancel);
    let observed = with_transfer_io_permit_from(&gate, &owner, || {
        acquire_transfer_exclusive_call(&cancel, Path::new("move"))
    })
    .unwrap();
    let marker = observed.expect("a Move call must borrow the lifetime Shared permit");
    assert!(marker.owned.is_none());
    assert_eq!(gate.snapshot().active_shared, 1);
    assert_eq!(gate.snapshot().queued, 0);
    drop(marker);
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(owner);
    assert!(gate.try_acquire(Admission::Exclusive).is_some());
}

#[test]
fn standalone_move_call_remains_exclusive_until_its_marker_drops() {
    let gate = Arc::new(Gate::new(2));
    let cancel = Cancellation::default();
    let _deadline = Deadline::new(&cancel);
    let marker = acquire_exclusive_with_gate(&gate, &cancel, Path::new("standalone move")).unwrap();
    assert!(marker.owned.is_some());
    assert!(gate.snapshot().exclusive);
    assert_eq!(gate.snapshot().active_shared, 0);
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(marker);
    assert!(!gate.snapshot().exclusive);
    let first = gate.try_acquire(Admission::Shared).unwrap();
    let second = gate.try_acquire(Admission::Shared).unwrap();
    drop((first, second));
}

#[test]
fn cancelled_move_calls_refuse_borrowing_and_standalone_admission() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    with_transfer_io_permit_from(&gate, &owner, || {
        assert!(
            matches!(acquire_transfer_exclusive_call(&cancel, Path::new("borrowed move")),
            Err(error) if error.kind == io::ErrorKind::Interrupted)
        );
    })
    .unwrap();
    assert!(
        matches!(acquire_exclusive_with_gate(&gate, &cancel, Path::new("standalone move")),
        Err(error) if error.kind == io::ErrorKind::Interrupted)
    );
    assert_eq!(gate.snapshot().active_shared, 1);
    assert_eq!(gate.snapshot().queued, 0);
}

#[test]
fn cancelled_queued_standalone_move_does_not_revoke_the_shared_owner() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    let queued_gate = gate.clone();
    let queued_cancel = cancel.clone();
    let (sent, received) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result =
            acquire_exclusive_with_gate(&queued_gate, &queued_cancel, Path::new("queued move"));
        let _ = sent.send(result.map(|permit| {
            drop(permit);
        }));
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while gate.snapshot().queued == 0 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    let was_queued = gate.snapshot().queued == 1;
    cancel.cancel();
    let cancelled = received.recv_timeout(Duration::from_secs(2));
    let retained = gate.snapshot().active_shared;
    // Release the ordinary owner before checking observations, including timeout.
    drop(owner);
    worker.join().unwrap();
    assert!(was_queued);
    assert!(matches!(cancelled.unwrap(), Err(error) if error.kind == io::ErrorKind::Interrupted));
    assert_eq!(retained, 1);
    assert_eq!(gate.snapshot().queued, 0);
    assert!(gate.try_acquire(Admission::Exclusive).is_some());
}

struct Deadline {
    done: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Deadline {
    fn new(cancel: &Cancellation) -> Self {
        let (done, receiver) = mpsc::channel();
        let cancel = cancel.clone();
        let worker = thread::spawn(move || {
            if receiver.recv_timeout(Duration::from_secs(4)).is_err() {
                cancel.cancel();
            }
        });
        Self {
            done: Some(done),
            worker: Some(worker),
        }
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            let _ = done.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn saturated_limit_one_borrows_without_queuing_or_returning_the_owner_permit() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    let _deadline = Deadline::new(&cancel);
    with_gate(&gate, &owner, || {
        let first = acquire_with_gate(&gate, &cancel, Path::new("first")).unwrap();
        let second = acquire_with_gate(&gate, &cancel, Path::new("second")).unwrap();
        assert!(first.owned.is_none() && second.owned.is_none());
        assert_eq!(gate.snapshot().active_shared, 1);
        assert_eq!(gate.snapshot().queued, 0);
        drop((first, second));
        assert!(gate.try_acquire(Admission::Shared).is_none());
    })
    .unwrap();
    assert!(gate.try_acquire(Admission::Shared).is_none());
    drop(owner);
    assert!(gate.try_acquire(Admission::Shared).is_some());
}

#[test]
fn nested_success_error_and_unwind_restore_the_outer_borrow_context() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    let _deadline = Deadline::new(&cancel);
    with_gate(&gate, &owner, || {
        let result = with_gate(&gate, &owner, || Err::<(), _>("ordinary failure")).unwrap();
        assert_eq!(result, Err("ordinary failure"));
        let caught = catch_unwind(AssertUnwindSafe(|| {
            with_gate(&gate, &owner, || panic!("borrow context fixture")).unwrap();
        }));
        assert!(caught.is_err());
        assert!(
            acquire_with_gate(&gate, &cancel, Path::new("outer"))
                .unwrap()
                .owned
                .is_none()
        );
    })
    .unwrap();
    drop(owner);
    let standalone = acquire_with_gate(&gate, &cancel, Path::new("after")).unwrap();
    assert!(standalone.owned.is_some());
    assert_eq!(gate.snapshot().active_shared, 1);
}

#[test]
fn unwind_out_of_outermost_context_does_not_leave_admission_bypassed() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let caught = catch_unwind(AssertUnwindSafe(|| {
        with_gate(&gate, &owner, || panic!("outer fixture")).unwrap();
    }));
    assert!(caught.is_err());
    drop(owner);
    let permit =
        acquire_with_gate(&gate, &Cancellation::default(), Path::new("after unwind")).unwrap();
    assert!(permit.owned.is_some());
    assert_eq!(gate.snapshot().active_shared, 1);
}

#[test]
fn wrong_gate_and_exclusive_witnesses_refuse_before_callback() {
    let gate = Arc::new(Gate::new(1));
    let other = Arc::new(Gate::new(1));
    let wrong = other.try_acquire(Admission::Shared).unwrap();
    assert!(with_gate(&gate, &wrong, || panic!("wrong gate callback")).is_err());
    assert!(
        with_transfer_io_permit_from(&gate, &wrong, || panic!("public wrong gate callback"))
            .is_err()
    );
    let exclusive = gate.try_acquire(Admission::Exclusive).unwrap();
    assert!(with_gate(&gate, &exclusive, || panic!("exclusive callback")).is_err());
    assert!(
        with_transfer_io_permit_from(&gate, &exclusive, || panic!("public exclusive callback"))
            .is_err()
    );
    assert!(gate.snapshot().exclusive);
    assert!(configured_gate().is_none());
}

/// A retained permit released for a pause, decision or wait holds no capacity: it never
/// installs a borrow context, so nested calls cannot run unadmitted beside a queued
/// exclusive request. Readmitted, the same permit is accepted again.
#[test]
fn released_witness_refuses_before_callback_until_readmitted() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire_retained(Admission::Shared).unwrap();
    assert!(owner.release());
    assert!(with_gate(&gate, &owner, || panic!("released witness callback")).is_err());
    assert!(
        with_transfer_io_permit_from(&gate, &owner, || panic!("public released callback")).is_err()
    );
    assert!(configured_gate().is_none());
    assert_eq!(gate.snapshot().active_shared, 0);
    assert!(owner.try_reacquire());
    with_gate(&gate, &owner, || {
        assert!(configured_gate().is_some_and(|configured| Arc::ptr_eq(&configured, &gate)));
    })
    .unwrap();
    assert!(configured_gate().is_none());
    assert_eq!(gate.snapshot().active_shared, 1);
}

#[test]
fn cancellation_refuses_borrowed_and_standalone_calls_before_admission() {
    let gate = Arc::new(Gate::new(1));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    with_gate(&gate, &owner, || {
        assert!(matches!(acquire_with_gate(&gate, &cancel, Path::new("borrowed")), Err(error) if error.kind == io::ErrorKind::Interrupted));
    }).unwrap();
    assert!(
        matches!(acquire_with_gate(&gate, &cancel, Path::new("standalone")), Err(error) if error.kind == io::ErrorKind::Interrupted)
    );
    assert_eq!(gate.snapshot().queued, 0);
}

#[test]
fn standalone_calls_take_shared_capacity_and_release_only_on_drop() {
    let gate = Arc::new(Gate::new(2));
    let cancel = Cancellation::default();
    let _deadline = Deadline::new(&cancel);
    let first = acquire_with_gate(&gate, &cancel, Path::new("one")).unwrap();
    let second = acquire_with_gate(&gate, &cancel, Path::new("two")).unwrap();
    assert_eq!(gate.snapshot().active_shared, 2);
    assert!(!gate.snapshot().exclusive);
    drop(first);
    assert_eq!(gate.snapshot().active_shared, 1);
    drop(second);
    assert_eq!(gate.snapshot().active_shared, 0);
}

#[test]
fn returning_a_marker_does_not_extend_the_synchronous_borrow_context() {
    let gate = Arc::new(Gate::new(2));
    let owner = gate.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    let _deadline = Deadline::new(&cancel);
    let marker = with_gate(&gate, &owner, || {
        acquire_with_gate(&gate, &cancel, Path::new("inside"))
    })
    .unwrap()
    .unwrap();
    assert!(marker.owned.is_none());
    let outside = acquire_with_gate(&gate, &cancel, Path::new("outside")).unwrap();
    assert!(outside.owned.is_some());
    assert_eq!(gate.snapshot().active_shared, 2);
    drop(marker);
    assert_eq!(gate.snapshot().active_shared, 2);
}

#[test]
fn trusted_configured_gate_composition_validates_every_nested_witness() {
    let configured = Arc::new(Gate::new(1));
    let other = Arc::new(Gate::new(1));
    let owner = configured.try_acquire(Admission::Shared).unwrap();
    let foreign = other.try_acquire(Admission::Shared).unwrap();
    let cancel = Cancellation::default();
    let _deadline = Deadline::new(&cancel);
    with_transfer_io_permit_from(&configured, &owner, || {
        with_transfer_io_permit_from(&configured, &owner, || {
            assert!(
                acquire_transfer_call(&cancel, Path::new("configured"))
                    .unwrap()
                    .owned
                    .is_none()
            );
            assert_eq!(configured.snapshot().active_shared, 1);
        })
        .unwrap();
        assert!(
            with_transfer_io_permit_from(&configured, &foreign, || panic!(
                "foreign nested witness"
            ))
            .is_err()
        );
        assert!(
            acquire_transfer_call(&cancel, Path::new("outer survives"))
                .unwrap()
                .owned
                .is_none()
        );
    })
    .unwrap();
    // The configured gate does not escape its synchronous call.
    assert!(configured_gate().is_none());
    assert_eq!(configured.snapshot().active_shared, 1);
    drop(owner);
    assert_eq!(configured.snapshot().active_shared, 0);
}

#[test]
fn nested_configured_gate_identity_is_restored_after_error_and_unwind() {
    let outer = Arc::new(Gate::new(1));
    let inner = Arc::new(Gate::new(1));
    let a = outer.try_acquire(Admission::Shared).unwrap();
    let b = inner.try_acquire(Admission::Shared).unwrap();
    let is_configured = |gate: &Arc<Gate>| {
        configured_gate().is_some_and(|configured| Arc::ptr_eq(&configured, gate))
    };
    with_transfer_io_permit_from(&outer, &a, || {
        assert!(is_configured(&outer));
        let result = with_transfer_io_permit_from(&inner, &b, || {
            assert!(is_configured(&inner));
            assert!(
                with_transfer_io_permit_from(&inner, &a, || panic!(
                    "outer witness accepted inside inner gate"
                ))
                .is_err()
            );
            Err::<(), _>("ordinary result")
        })
        .unwrap();
        assert_eq!(result, Err("ordinary result"));
        assert!(is_configured(&outer));
        let caught = catch_unwind(AssertUnwindSafe(|| {
            with_transfer_io_permit_from(&inner, &b, || panic!("configured unwind")).unwrap();
        }));
        assert!(caught.is_err());
        assert!(is_configured(&outer));
    })
    .unwrap();
    assert!(configured_gate().is_none());
    assert_eq!(outer.snapshot().active_shared, 1);
    assert_eq!(inner.snapshot().active_shared, 1);
}
