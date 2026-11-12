use super::*;
use std::sync::{Barrier, mpsc};
use std::thread;

fn reserve_remaining(budget: &Arc<Budget>, count: usize) -> Vec<Reservation> {
    (0..count)
        .map(|_| budget.reserve().expect("available raster slot"))
        .collect()
}

#[test]
fn reservations_precede_allocation_and_rollback_when_rendering_fails() {
    let budget = Arc::new(Budget::default());
    let mut slots = reserve_remaining(&budget, MAX_SNAPSHOTS);
    assert!(
        budget.reserve().is_none(),
        "concurrent allocations must reserve before rendering"
    );
    drop(slots.pop());
    let recovered = budget
        .reserve()
        .expect("failed rendering releases its reservation");
    assert!(budget.reserve().is_none());
    drop((slots, recovered));
    assert_eq!(
        reserve_remaining(&budget, MAX_SNAPSHOTS).len(),
        MAX_SNAPSHOTS
    );
}

#[test]
fn body_and_gutter_owners_both_hold_the_slot_without_the_budget_retaining_pixels() {
    let budget = Arc::new(Budget::default());
    let occupied = reserve_remaining(&budget, MAX_SNAPSHOTS - 1);
    let body: Arc<[u8]> = vec![1; 16].into();
    let gutter: Arc<[u8]> = vec![2; 8].into();
    budget
        .reserve()
        .unwrap()
        .commit(&body, Some(&gutter))
        .unwrap();
    assert_eq!(Arc::strong_count(&body), 1);
    assert_eq!(Arc::strong_count(&gutter), 1);
    assert!(budget.reserve().is_none());
    drop(body);
    assert!(
        budget.reserve().is_none(),
        "a frontend-held gutter still owns raster storage"
    );
    drop(gutter);
    assert!(budget.reserve().is_some());
    drop(occupied);
}

#[test]
fn cloned_body_owners_survive_worker_close_and_many_new_contexts() {
    let service = Arc::new(Budget::default());
    let mut retired = Vec::new();
    for _ in 0..MAX_SNAPSHOTS {
        let worker_budget = service.clone();
        let body: Arc<[u8]> = vec![3; 16].into();
        worker_budget
            .reserve()
            .unwrap()
            .commit(&body, None)
            .unwrap();
        retired.push(body.clone());
        drop((body, worker_budget));
    }
    for _ in 0..10_000 {
        let replacement_worker = service.clone();
        assert!(
            replacement_worker.reserve().is_none(),
            "reset/reopen must not reset live storage"
        );
    }
    drop(retired.pop());
    let replacement: Arc<[u8]> = vec![4; 32].into();
    service
        .reserve()
        .unwrap()
        .commit(&replacement, None)
        .unwrap();
    assert!(service.reserve().is_none());
    drop((retired, replacement));
    assert_eq!(
        reserve_remaining(&service, MAX_SNAPSHOTS).len(),
        MAX_SNAPSHOTS
    );
}

#[test]
fn reservations_are_atomic_across_concurrent_viewer_workers() {
    let budget = Arc::new(Budget::default());
    let attempts = MAX_SNAPSHOTS + 4;
    let start = Arc::new(Barrier::new(attempts + 1));
    let finish = Arc::new(Barrier::new(attempts + 1));
    let (sender, receiver) = mpsc::channel();
    let workers: Vec<_> = (0..attempts)
        .map(|_| {
            let (budget, start, finish, sender) = (
                budget.clone(),
                start.clone(),
                finish.clone(),
                sender.clone(),
            );
            thread::spawn(move || {
                start.wait();
                let reservation = budget.reserve();
                sender.send(reservation.is_some()).unwrap();
                finish.wait();
                drop(reservation);
            })
        })
        .collect();
    start.wait();
    let admitted = (0..attempts).filter(|_| receiver.recv().unwrap()).count();
    // Always release and join workers before asserting, including on a regression.
    finish.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        admitted, MAX_SNAPSHOTS,
        "reserved allocations share one service-wide cap"
    );
    assert_eq!(
        reserve_remaining(&budget, MAX_SNAPSHOTS).len(),
        MAX_SNAPSHOTS
    );
}

#[test]
fn oversized_body_plus_gutter_is_rejected_and_reservation_is_released() {
    let budget = Arc::new(Budget::default());
    let body: Arc<[u8]> = vec![0; MAX_SNAPSHOT_BYTES].into();
    let gutter: Arc<[u8]> = vec![0; 4].into();
    assert!(
        budget
            .reserve()
            .unwrap()
            .commit(&body, Some(&gutter))
            .is_err()
    );
    assert_eq!(
        reserve_remaining(&budget, MAX_SNAPSHOTS).len(),
        MAX_SNAPSHOTS
    );
    budget.reserve().unwrap().commit(&body, None).unwrap();
    let remaining = reserve_remaining(&budget, MAX_SNAPSHOTS - 1);
    assert!(budget.reserve().is_none());
    drop((body, remaining));
    assert_eq!(
        reserve_remaining(&budget, MAX_SNAPSHOTS).len(),
        MAX_SNAPSHOTS
    );
}

#[test]
fn one_registry_enforces_each_worker_limit_and_keeps_closed_worker_pixels_charged() {
    let service = Arc::new(Budget::default());
    let first_owner = service.owner();
    let mut first = Vec::new();
    for _ in 0..3 {
        first.push(
            service
                .reserve_for(first_owner)
                .expect("worker has three render slots"),
        );
    }
    assert!(
        service.reserve_for(first_owner).is_none(),
        "one worker cannot consume four slots"
    );
    let second_owner = service.owner();
    assert_ne!(first_owner, second_owner);
    let second = service
        .reserve_for(second_owner)
        .expect("another worker still has capacity");
    let body: Arc<[u8]> = vec![7; 16].into();
    first.pop().unwrap().commit(&body, None).unwrap();
    assert!(
        service.reserve_for(first_owner).is_none(),
        "committed and reserved slots both count"
    );
    drop(first);
    let first_replacement = service.reserve_for(first_owner).unwrap();
    let first_last = service.reserve_for(first_owner).unwrap();
    assert!(service.reserve_for(first_owner).is_none());
    drop((first_replacement, first_last, second));

    // Closing a worker drops its owner value, not pixel allocations held by pending outputs.
    let mut retired = vec![body];
    for _ in 1..MAX_SNAPSHOTS {
        let replacement_owner = service.owner();
        let pixels: Arc<[u8]> = vec![8; 16].into();
        service
            .reserve_for(replacement_owner)
            .unwrap()
            .commit(&pixels, None)
            .unwrap();
        retired.push(pixels);
    }
    let new_owner = service.owner();
    assert!(
        service.reserve_for(new_owner).is_none(),
        "closed workers still share the global cap"
    );
    drop(retired.pop());
    let new_reservation = service.reserve_for(new_owner).unwrap();
    assert!(
        service.reserve_for(new_owner).is_none(),
        "only one old allocation was reclaimed"
    );
    drop((new_reservation, retired));
    let slots: Vec<_> = (0..3)
        .map(|_| service.reserve_for(new_owner).unwrap())
        .collect();
    assert!(service.reserve_for(new_owner).is_none());
    drop(slots);
}

#[test]
fn retirement_reaps_dead_weak_mappings_without_reopening_or_disturbing_live_owners() {
    let budget = Arc::new(Budget::default());
    let reservation = budget.reserve().unwrap();
    let live_body: Arc<[u8]> = vec![9; 16].into();
    budget.reserve().unwrap().commit(&live_body, None).unwrap();
    let body: Arc<[u8]> = vec![1; 32].into();
    let gutter: Arc<[u8]> = vec![2; 16].into();
    budget
        .reserve()
        .unwrap()
        .commit(&body, Some(&gutter))
        .unwrap();
    assert_eq!(Arc::weak_count(&body), 1);
    assert_eq!(Arc::weak_count(&gutter), 1);
    drop(body);
    budget.reap();
    assert_eq!(
        Arc::weak_count(&gutter),
        1,
        "one surviving pixel owner retains both mappings"
    );
    assert_eq!(Arc::weak_count(&live_body), 1);
    drop(gutter);
    budget.reap();
    {
        let slots = budget.slots.lock().unwrap();
        assert_eq!(
            slots
                .iter()
                .filter(|slot| matches!(slot, Slot::Reserved(_)))
                .count(),
            1
        );
        let live: Vec<_> = slots
            .iter()
            .filter_map(|slot| match slot {
                Slot::Live { body, gutter, .. } => Some((body, gutter)),
                _ => None,
            })
            .collect();
        assert_eq!(
            live.len(),
            1,
            "dead Weak slice mappings must be dropped without another reservation"
        );
        assert!(Weak::ptr_eq(live[0].0, &Arc::downgrade(&live_body)));
        assert!(live[0].1.is_none());
    }
    assert_eq!(Arc::weak_count(&live_body), 1);
    drop(live_body);
    budget.reap();
    assert!(
        budget
            .slots
            .lock()
            .unwrap()
            .iter()
            .all(|slot| !matches!(slot, Slot::Live { .. }))
    );
    drop(reservation);
    assert!(
        budget
            .slots
            .lock()
            .unwrap()
            .iter()
            .all(|slot| matches!(slot, Slot::Vacant))
    );
}
