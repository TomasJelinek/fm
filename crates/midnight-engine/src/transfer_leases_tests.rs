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
