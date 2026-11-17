use super::*;

#[tokio::test]
async fn a_copy_share_excludes_legacy_consumers_until_actual_release() {
    let gate = namespace_gate();
    let copy = gate.clone().try_acquire_owned().unwrap();
    assert!(
        gate.clone()
            .try_acquire_many_owned(NAMESPACE_WEIGHT)
            .is_err()
    );
    assert!(gate.clone().try_acquire_owned().is_ok());
    drop(copy);
    let legacy = gate
        .clone()
        .acquire_many_owned(NAMESPACE_WEIGHT)
        .await
        .unwrap();
    assert!(gate.clone().try_acquire_owned().is_err());
    drop(legacy);
    assert_eq!(gate.available_permits(), NAMESPACE_WEIGHT as usize);
}

#[tokio::test]
async fn namespace_contention_rolls_back_the_real_io_slot_before_waiting() {
    let disk = Arc::new(Gate::new(1));
    let namespace = namespace_gate();
    let legacy = namespace
        .clone()
        .acquire_many_owned(NAMESPACE_WEIGHT)
        .await
        .unwrap();
    assert!(try_pair(&disk, &namespace).unwrap().is_none());
    assert_eq!(disk.snapshot().active_shared, 0);
    // A legacy namespace owner can still obtain the I/O it needs to finish.
    let legacy_io = disk.try_acquire(Admission::Exclusive).unwrap();
    drop(legacy_io);
    drop(legacy);
    let transfer = try_pair(&disk, &namespace).unwrap().unwrap();
    assert!(transfer.io.belongs_to(&disk));
    assert!(transfer.io.is_shared());
    assert_eq!(disk.snapshot().active_shared, 1);
    drop(transfer);
    assert_eq!(disk.snapshot().available, 1);
}

#[test]
fn configured_slots_bound_active_jobs_without_an_additional_copy_budget() {
    let disk = Arc::new(Gate::new(2));
    let namespace = namespace_gate();
    let first = try_pair(&disk, &namespace).unwrap().unwrap();
    let second = try_pair(&disk, &namespace).unwrap().unwrap();
    assert!(try_pair(&disk, &namespace).unwrap().is_none());
    assert_eq!(disk.snapshot().active_shared, 2);
    assert_eq!(namespace.available_permits(), NAMESPACE_WEIGHT as usize - 2);
    drop(first);
    let next = try_pair(&disk, &namespace).unwrap().unwrap();
    assert_eq!(disk.snapshot().active_shared, 2);
    drop((second, next));
    assert_eq!(disk.snapshot().active_shared, 0);
    assert_eq!(namespace.available_permits(), NAMESPACE_WEIGHT as usize);
}
