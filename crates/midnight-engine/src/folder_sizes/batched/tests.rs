use super::*;
use std::{sync::mpsc, thread};

#[path = "fixtures.rs"]
mod fixtures;

#[test]
fn conservative_io_reserves_all_four_slots_and_cancelled_waiter_exits() {
    let budget = Arc::new(IoBudget::isolated());
    let held = budget
        .acquire_io(Admission::Exclusive, &Cancellation::default())
        .expect("Initial I/O permit");
    let cancel = Cancellation::default();
    let waiter_cancel = cancel.clone();
    let waiter_budget = budget.clone();
    let (started, entered) = mpsc::channel();
    let waiter = thread::spawn(move || {
        started.send(()).unwrap();
        waiter_budget
            .acquire_io(Admission::Shared, &waiter_cancel)
            .is_some()
    });
    entered.recv().unwrap();
    cancel.cancel();
    assert!(!waiter.join().unwrap());
    drop(held);
    assert!(
        budget
            .acquire_io(Admission::Exclusive, &Cancellation::default())
            .is_some()
    );
}

#[test]
fn a_local_classification_query_does_not_wait_for_an_existing_ssd_read() {
    let budget = Arc::new(IoBudget::isolated());
    let _read = budget
        .acquire_io(Admission::Shared, &Cancellation::default())
        .expect("SSD read permit");
    assert!(
        budget
            .acquire_classification(&Cancellation::default())
            .is_some()
    );
}
