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

struct Counted {
    raw: Arc<FileComparatorPreferences>,
    dropped: Arc<std::sync::atomic::AtomicUsize>,
}
impl CompiledFileComparatorPolicy for Counted {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.raw
    }
    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        None
    }
    fn matcher_budget_bytes(&self) -> usize {
        1
    }
}
impl Drop for Counted {
    fn drop(&mut self) {
        self.dropped
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}

fn publish_counted(
    store: &FileComparatorPolicyStore,
    dropped: &Arc<std::sync::atomic::AtomicUsize>,
) -> FileComparatorSnapshot {
    let id = store.prepare_id().unwrap();
    store.publish(
        PreparedFileComparatorPolicy::new(Arc::new(Counted {
            raw: Arc::default(),
            dropped: dropped.clone(),
        }))
        .with_id(id),
    )
}

/// Any number of sessions pin; one generation is one shared set. A displaced set stays
/// in the store while pinned, its pins never drop it, and the next publication drops it
/// once no pin references it.
#[test]
fn pinned_generations_are_shared_retained_and_dropped_by_a_later_publication() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let store = FileComparatorPolicyStore::default();
    let dropped = Arc::new(AtomicUsize::new(0));
    let first = publish_counted(&store, &dropped);
    let pins: Vec<_> = (0..6).map(|_| store.pin(&first).unwrap()).collect();
    assert!(
        pins.iter().all(|pin| Arc::ptr_eq(&pin.set, &pins[0].set)),
        "one generation is one shared set"
    );
    let second = publish_counted(&store, &dropped);
    let later = store.pin(&second).unwrap();
    assert!(store.pin(&first).is_err(), "only the current set is pinned");
    assert_eq!(store.pinned_history(), 1);
    drop(pins);
    assert_eq!(
        dropped.load(Ordering::Acquire),
        0,
        "a pin never drops a set"
    );
    let _third = publish_counted(&store, &dropped);
    // The first set is dropped now; the second, still pinned, joins the history.
    assert_eq!(dropped.load(Ordering::Acquire), 1);
    assert_eq!(store.pinned_history(), 1);
    drop(later);
    assert_eq!(dropped.load(Ordering::Acquire), 1);
    drop(store);
    assert_eq!(dropped.load(Ordering::Acquire), 3);
}

/// An unpinned displaced set is dropped by the publication that displaces it.
#[test]
fn an_unpinned_displaced_set_is_dropped_at_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let store = FileComparatorPolicyStore::default();
    let dropped = Arc::new(AtomicUsize::new(0));
    publish_counted(&store, &dropped);
    publish_counted(&store, &dropped);
    assert_eq!(dropped.load(Ordering::Acquire), 1);
    assert_eq!(store.pinned_history(), 0);
}
