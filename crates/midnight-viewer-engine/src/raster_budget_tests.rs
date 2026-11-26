use super::*;
use std::sync::{Barrier, mpsc};
use std::thread;

const MIB: usize = 1024 * 1024;

#[test]
fn reservations_precede_allocation_and_roll_back_when_painting_fails() {
    let budget = Arc::new(Budget::default());
    let owner = budget.owner();
    let held = budget
        .reserve_strip(owner, MAX_STRIP_BYTES - MIB, StripUse::Ahead)
        .expect("a viewer's budget");
    assert!(
        budget
            .reserve_strip(owner, 2 * MIB, StripUse::Ahead)
            .is_none(),
        "concurrent strips must reserve before painting"
    );
    let failed = budget.reserve_strip(owner, MIB, StripUse::Ahead).unwrap();
    drop(failed);
    assert!(
        budget.reserve_strip(owner, MIB, StripUse::Ahead).is_some(),
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
        .reserve_strip(owner, 24, StripUse::Ahead)
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

/// A strip's `Weak` keeps its allocation, pixels included, after its last owner dropped
/// it: until a reap or the next reservation forgets it.
#[test]
fn a_released_strip_is_tracked_until_it_is_reaped() {
    let budget = Arc::new(Budget::default());
    let owner = budget.owner();
    let body: Arc<[u8]> = vec![1; 16].into();
    budget
        .reserve_strip(owner, 16, StripUse::Ahead)
        .unwrap()
        .commit(&body, None)
        .unwrap();
    drop(body);
    assert_eq!(budget.entries(), 1, "nothing reaped the released strip yet");
    budget.reap();
    assert_eq!(budget.entries(), 0);
}

#[test]
fn a_strip_larger_than_its_reservation_is_refused() {
    let budget = Arc::new(Budget::default());
    let owner = budget.owner();
    let body: Arc<[u8]> = vec![1; 32].into();
    assert!(
        budget
            .reserve_strip(owner, 16, StripUse::Ahead)
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
        let reservation = service
            .reserve_strip(*owner, MAX_STRIP_BYTES - 8, StripUse::Ahead)
            .unwrap();
        // The reservation's bytes are charged although the pixels are smaller.
        reservation.commit(&pixels, None).unwrap();
        held.push(pixels);
        assert!(
            service.reserve_strip(*owner, 16, StripUse::Ahead).is_none(),
            "one viewer's strips stay within its budget"
        );
    }
    // Four viewers hold all but 32 bytes of the shared budget.
    let last = owners[owners.len() - 1];
    assert!(
        service.reserve_strip(last, 64, StripUse::Ahead).is_none(),
        "all viewers' strips stay within theirs"
    );
    // Reopening windows (new owners) does not reset live storage.
    for _ in 0..1000 {
        assert!(
            service
                .reserve_strip(service.owner(), 64, StripUse::Ahead)
                .is_none()
        );
    }
    drop(held.pop());
    assert!(service.reserve_strip(last, 64, StripUse::Ahead).is_some());
    drop(held);
    service.reap();
    assert_eq!(service.strip_bytes(owners[0]), 0);
}

/// However many viewers are open, each can reserve the strips under its window within its
/// own budget; only strips painted ahead of the window wait for the shared total.
#[test]
fn twenty_viewers_each_show_their_window_while_strips_ahead_stay_within_the_total() {
    let budget = Arc::new(Budget::default());
    let owners: Vec<u64> = (0..20).map(|_| budget.owner()).collect();
    let mut held = Vec::new();
    for owner in &owners {
        let pixels: Arc<[u8]> = vec![0; 8].into();
        budget
            .reserve_strip(*owner, MAX_STRIP_BYTES / 2, StripUse::Shown)
            .expect("every viewer shows its window")
            .commit(&pixels, None)
            .unwrap();
        held.push(pixels);
    }
    // Twenty half budgets are far beyond the shared total: nothing more is painted ahead.
    for owner in &owners {
        assert!(budget.reserve_strip(*owner, MIB, StripUse::Ahead).is_none());
    }
    // A viewer's own budget still bounds what it shows.
    assert!(
        budget
            .reserve_strip(owners[0], MAX_STRIP_BYTES / 2 + 1, StripUse::Shown)
            .is_none()
    );
    // Closing viewers frees the total for strips ahead again.
    held.truncate(MAX_TOTAL_STRIP_BYTES / (MAX_STRIP_BYTES / 2) - 1);
    assert!(
        budget
            .reserve_strip(owners[0], MIB, StripUse::Ahead)
            .is_some()
    );
    drop(held);
    budget.reap();
    assert_eq!(budget.entries(), 0);
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
                let reservation = budget.reserve_strip(owner, MAX_STRIP_BYTES / 3, StripUse::Ahead);
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
    assert!(budget.reserve_strip(0, 1, StripUse::Ahead).is_none());
}
