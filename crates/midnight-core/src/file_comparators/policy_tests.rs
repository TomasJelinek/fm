use super::*;

#[test]
fn policy_id_exhaustion_refuses_before_publication_without_changing_current() {
    let store = FileComparatorPolicyStore::default();
    let before = store.snapshot();
    store.state.lock().unwrap().last_reserved = u64::MAX;
    assert!(store.prepare_id().is_err());
    let after = store.snapshot();
    assert_eq!(after.id, before.id);
    assert!(Arc::ptr_eq(&after.preferences, &before.preferences));
}
