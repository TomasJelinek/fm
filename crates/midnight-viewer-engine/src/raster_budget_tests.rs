use super::*;
use std::sync::{Barrier, mpsc};
use std::thread;

const MIB: usize = 1024 * 1024;

#[test]
fn reservations_precede_allocation_and_roll_back_when_painting_fails() {
    let budget = Arc::new(Budget::default());
    let owner = budget.owner();
    let held = budget
        .reserve_strip(owner, MAX_STRIP_BYTES - MIB)
        .expect("a viewer's budget");
    assert!(
        budget.reserve_strip(owner, 2 * MIB).is_none(),
        "concurrent strips must reserve before painting"
    );
    let failed = budget.reserve_strip(owner, MIB).unwrap();
    drop(failed);
    assert!(
        budget.reserve_strip(owner, MIB).is_some(),
        "a strip whose painting failed releases its reservation"
    );
    drop(held);
    assert_eq!(budget.strip_bytes(owner), 0);
}

#[test]
fn body_and_gutter_owners_both_hold_the_bytes_without_the_budget_retaining_pixels() {
    let budget = Arc::new(Budget::default());
    let owner = budget.owner();
    let body: Arc<[u8]> = vec![1; 16].into();
    let gutter: Arc<[u8]> = vec![2; 8].into();
    budget
        .reserve_strip(owner, 24)
        .unwrap()
        .commit(&body, Some(&gutter))
        .unwrap();
    assert_eq!(Arc::strong_count(&body), 1);
    assert_eq!(Arc::strong_count(&gutter), 1);
    assert_eq!(budget.strip_bytes(owner), 24);
    drop(body);
    assert_eq!(
        budget.strip_bytes(owner),
        24,
        "a frontend-held gutter still owns strip storage"
    );
    drop(gutter);
    assert_eq!(budget.strip_bytes(owner), 0);
}

#[test]
fn a_strip_larger_than_its_reservation_is_refused() {
    let budget = Arc::new(Budget::default());
    let owner = budget.owner();
    let body: Arc<[u8]> = vec![1; 32].into();
    assert!(
        budget
            .reserve_strip(owner, 16)
            .unwrap()
            .commit(&body, None)
            .is_err()
    );
    assert_eq!(budget.strip_bytes(owner), 0);
}

#[test]
fn each_viewer_and_all_viewers_have_a_byte_budget_until_their_pixels_are_dropped() {
    let service = Arc::new(Budget::default());
    let owners: Vec<u64> = (0..MAX_TOTAL_STRIP_BYTES / MAX_STRIP_BYTES + 1)
        .map(|_| service.owner())
        .collect();
    let mut held = Vec::new();
    for owner in &owners[..owners.len() - 1] {
        let pixels: Arc<[u8]> = vec![0; 8].into();
        let reservation = service.reserve_strip(*owner, MAX_STRIP_BYTES - 8).unwrap();
        // The reservation's bytes are charged although the pixels are smaller.
        reservation.commit(&pixels, None).unwrap();
        held.push(pixels);
        assert!(
            service.reserve_strip(*owner, 16).is_none(),
            "one viewer's strips stay within its budget"
        );
    }
    // Four viewers hold all but 32 bytes of the shared budget.
    let last = owners[owners.len() - 1];
    assert!(
        service.reserve_strip(last, 64).is_none(),
        "all viewers' strips stay within theirs"
    );
    // Reopening windows (new owners) does not reset live storage.
    for _ in 0..1000 {
        assert!(service.reserve_strip(service.owner(), 64).is_none());
    }
    drop(held.pop());
    assert!(service.reserve_strip(last, 64).is_some());
    drop(held);
    service.reap();
    assert_eq!(service.strip_bytes(owners[0]), 0);
}

#[test]
fn concurrent_reservations_never_exceed_the_viewer_budget() {
    let budget = Arc::new(Budget::default());
    let owner = budget.owner();
    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let (send, receive) = mpsc::channel();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let budget = budget.clone();
            let barrier = barrier.clone();
            let send = send.clone();
            thread::spawn(move || {
                barrier.wait();
                let reservation = budget.reserve_strip(owner, MAX_STRIP_BYTES / 3);
                send.send(reservation.is_some()).unwrap();
                // Keep the reservation until every thread has tried.
                barrier.wait();
                drop(reservation);
            })
        })
        .collect();
    drop(send);
    for worker in workers {
        worker.join().unwrap();
    }
    let admitted = receive.iter().filter(|admitted| *admitted).count();
    assert_eq!(admitted, 3, "three thirds of the budget fit at once");
    assert_eq!(budget.strip_bytes(owner), 0);
}

#[test]
fn owner_zero_is_never_admitted() {
    let budget = Arc::new(Budget::default());
    assert!(budget.reserve_strip(0, 1).is_none());
}
