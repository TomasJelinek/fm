//! Namespace grants use complete native metadata and confirmed ownership boundaries.
use super::*;
use std::{
    cell::RefCell,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

#[path = "copy_namespace_admission_tests.rs"]
mod admission;

thread_local! {
    static AFTER_RESERVATION_SNAPSHOT: RefCell<Option<Box<dyn FnOnce()>>> = const {
        RefCell::new(None)
    };
}

pub(super) fn after_reservation_snapshot() {
    let hook = AFTER_RESERVATION_SNAPSHOT.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

fn id(value: u64) -> TransferIdentity {
    TransferIdentity {
        device: 7,
        object: (value as u128).to_le_bytes(),
    }
}
fn parent(value: u64, ancestors: &[u64]) -> Arc<TransferNamespaceParent> {
    Arc::new(TransferNamespaceParent {
        identity: id(value),
        lookup_domain: Arc::from(&b"exact-native-rules"[..]),
        ancestors: ancestors.iter().copied().map(id).collect(),
    })
}
fn entry(
    parent: Arc<TransferNamespaceParent>,
    key: &[u8],
    object: Option<u64>,
    subtree: bool,
) -> TransferNamespaceScope {
    TransferNamespaceScope {
        parent,
        lookup_key: Arc::from(key),
        object: object.map(id),
        extent: if subtree {
            TransferNamespaceExtent::Subtree
        } else {
            TransferNamespaceExtent::Entry
        },
    }
}
struct Control {
    probe: Mutex<Result<TransferNamespaceProbe, FsError>>,
    calls: AtomicUsize,
    dropped: AtomicUsize,
    panic: AtomicBool,
    panic_drop: AtomicBool,
    drop_registry: Mutex<Option<Weak<Inner>>>,
    block: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
}
struct Pin(Arc<Control>);
impl TransferNamespacePin for Pin {
    fn refresh(&self, _: &Cancellation) -> Result<TransferNamespaceProbe, FsError> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        if self.0.panic.swap(false, Ordering::SeqCst) {
            panic!("native metadata refresh unwound");
        }
        let block = self.0.block.lock().unwrap().take();
        if let Some((entered, release)) = block {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
        self.0.probe.lock().unwrap().clone()
    }
}
impl Drop for Pin {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
        if let Some(registry) = self
            .0
            .drop_registry
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
        {
            assert!(
                registry.state.try_lock().is_ok(),
                "native pin dropped under registry mutex"
            );
        }
        if self.0.panic_drop.swap(false, Ordering::SeqCst) {
            panic!("native pin destructor unwound");
        }
    }
}
fn pin(scopes: Vec<TransferNamespaceScope>) -> (Arc<Control>, Arc<dyn TransferNamespacePin>) {
    let control = Arc::new(Control {
        probe: Mutex::new(Ok(TransferNamespaceProbe::Known(scopes.into()))),
        calls: AtomicUsize::new(0),
        dropped: AtomicUsize::new(0),
        panic: AtomicBool::new(false),
        panic_drop: AtomicBool::new(false),
        drop_registry: Mutex::new(None),
        block: Mutex::new(None),
    });
    (control.clone(), Arc::new(Pin(control)))
}
struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
fn block(control: &Control) -> (std::sync::mpsc::Receiver<()>, Release) {
    let (entered, receive) = std::sync::mpsc::channel();
    let (send, release) = std::sync::mpsc::channel();
    *control.block.lock().unwrap() = Some((entered, release));
    (receive, Release(Some(send)))
}
struct Job {
    claim: Option<Claim>,
    owners: Option<NativeOwners>,
    io: Option<Arc<Permit>>,
}
impl Job {
    fn new(
        registry: &Registry,
        sequence: u64,
        roots: Vec<(
            TransferNamespaceAccess,
            Option<Arc<dyn TransferNamespacePin>>,
        )>,
    ) -> Self {
        let (claim, owners) = registry.new_job(sequence).unwrap();
        let io = Arc::new(
            registry
                .inner
                .io
                .acquire(midnight_work_budget::Admission::Shared, || false)
                .unwrap(),
        );
        claim
            .plan_union(roots, io.clone(), &Cancellation::default())
            .unwrap();
        Self {
            claim: Some(claim),
            owners: Some(owners),
            io: Some(io),
        }
    }
    fn claim(&self) -> &Claim {
        self.claim.as_ref().unwrap()
    }
    fn grant(&self) -> bool {
        self.claim().try_grant(&Cancellation::default()).unwrap()
    }
    fn close(&mut self) {
        if let Some(claim) = self.claim.take() {
            if claim.begin_closing().is_err() {
                std::mem::forget(self.owners.take());
                std::mem::forget(self.io.take());
                std::mem::forget(claim);
                return;
            }
            drop(self.owners.take());
            if claim.complete_after_drop().is_err() {
                std::mem::forget(self.io.take());
                std::mem::forget(claim);
                return;
            }
            drop(self.io.take());
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.close();
    }
}
fn registry() -> Registry {
    Registry::new_with_io(Arc::new(midnight_work_budget::Gate::new(
        midnight_work_budget::MAX_IO_LIMIT,
    )))
}
fn one(
    registry: &Registry,
    sequence: u64,
    access: TransferNamespaceAccess,
    scope: TransferNamespaceScope,
) -> Job {
    let (_, pin) = pin(vec![scope]);
    Job::new(registry, sequence, vec![(access, Some(pin))])
}
use TransferNamespaceAccess::{Read, Write};
/// A granted, disjoint peer. Other claims' committed facts are then observable, so each
/// of their finishes re-validates every pin; without one, only announced pins refresh.
fn observer(registry: &Registry) -> Job {
    let observer = one(
        registry,
        1_000,
        Read,
        entry(parent(900, &[900, 0]), b"observer", None, false),
    );
    assert!(observer.grant());
    observer
}

#[test]
fn subtree_child_object_announcement_blocks_external_hardlink_until_owner_close() {
    let registry = registry();
    let (_, root) = pin(vec![entry(parent(1, &[1, 0]), b"tree-a", Some(10), true)]);
    let mut owner = Job::new(&registry, 1, vec![(Write, Some(root.clone()))]);
    assert!(owner.grant());
    let mut transaction = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(root), &Cancellation::default())
            .unwrap()
    );
    let (child_control, child) = pin(vec![entry(
        parent(10, &[10, 1, 0]),
        b"child",
        Some(50),
        false,
    )]);
    *child_control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    assert!(
        transaction
            .reserve_private_child(Some(child), &Cancellation::default())
            .unwrap()
    );
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);

    // The unrelated parent proves that only the child's physical identity can
    // connect this hardlink to the already owned subtree's private child.
    let reader = one(
        &registry,
        2,
        Read,
        entry(parent(20, &[20, 0]), b"external-hardlink", Some(50), false),
    );
    assert!(
        !reader.grant(),
        "subtree coverage must retain the new child object exclusion"
    );
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 0);
    owner.close();
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 1);
    assert!(
        reader.grant(),
        "the hardlink reader may enter after actual owner close"
    );
}

#[test]
fn distinct_child_pin_stays_observed_even_with_existing_write_object_coverage() {
    let registry = registry();
    let (_, root) = pin(vec![entry(parent(1, &[1, 0]), b"tree-a", Some(10), true)]);
    let (_, existing_object) = pin(vec![entry(
        parent(20, &[20, 0]),
        b"owned-hardlink",
        Some(50),
        false,
    )]);
    let mut owner = Job::new(
        &registry,
        1,
        vec![(Write, Some(root.clone())), (Write, Some(existing_object))],
    );
    assert!(owner.grant());
    let mut transaction = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(root), &Cancellation::default())
            .unwrap()
    );
    let (duplicate_control, duplicate) = pin(vec![entry(
        parent(10, &[10, 1, 0]),
        b"child",
        Some(50),
        false,
    )]);
    *duplicate_control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    assert!(
        transaction
            .reserve_private_child(Some(duplicate), &Cancellation::default())
            .unwrap()
    );
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);
    assert_eq!(owner.claim().alias_count(), 1);
    assert_eq!(duplicate_control.dropped.load(Ordering::SeqCst), 0);
    let reader = one(
        &registry,
        2,
        Read,
        entry(parent(30, &[30, 0]), b"third-hardlink", Some(50), false),
    );
    assert!(
        !reader.grant(),
        "the existing Write object still excludes this reader"
    );
    owner.close();
    assert_eq!(duplicate_control.dropped.load(Ordering::SeqCst), 1);
    assert!(reader.grant());
}

#[test]
fn distinct_child_pin_retains_changed_object_despite_unchanged_original_object() {
    let registry = registry();
    let (_, root) = pin(vec![entry(parent(1, &[1, 0]), b"tree-a", Some(10), true)]);
    let (_, existing_object) = pin(vec![entry(
        parent(20, &[20, 0]),
        b"original-x",
        Some(50),
        false,
    )]);
    let mut owner = Job::new(
        &registry,
        1,
        vec![(Write, Some(root.clone())), (Write, Some(existing_object))],
    );
    assert!(owner.grant());
    let mut transaction = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(root), &Cancellation::default())
            .unwrap()
    );
    let child_parent = parent(10, &[10, 1, 0]);
    let (child_control, child) = pin(vec![entry(child_parent.clone(), b"child", Some(50), false)]);
    assert!(
        transaction
            .reserve_private_child(Some(child), &Cancellation::default())
            .unwrap()
    );
    // This child binding changes from X to Y. The separately retained original
    // X root never changes, so refreshing it cannot discover this new object.
    *child_control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
        vec![entry(child_parent, b"child", Some(60), false)].into(),
    ));
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);
    let reader_y = one(
        &registry,
        2,
        Read,
        entry(parent(30, &[30, 0]), b"external-y", Some(60), false),
    );
    let reader_x = one(
        &registry,
        3,
        Read,
        entry(parent(40, &[40, 0]), b"external-x", Some(50), false),
    );
    assert!(
        !reader_y.grant(),
        "the independent observer must retain changed object Y"
    );
    assert!(
        !reader_x.grant(),
        "original object X remains independently excluded"
    );
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 0);
    owner.close();
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 1);
    assert!(reader_y.grant());
    assert!(reader_x.grant());
}

#[test]
fn private_alias_identity_mismatch_retains_incoming_pin_and_quarantines() {
    assert_corrupt_alias_reservation_is_quarantined(AliasCorruption::Lookup);
}

#[test]
fn private_alias_entry_collision_retains_incoming_pin_and_quarantines() {
    assert_corrupt_alias_reservation_is_quarantined(AliasCorruption::Entry);
}

#[test]
fn private_alias_identity_insertion_collision_retains_all_pins_and_quarantines() {
    assert_corrupt_alias_reservation_is_quarantined(AliasCorruption::Insertion);
}

#[derive(Clone, Copy)]
enum AliasCorruption {
    Lookup,
    Entry,
    Insertion,
}

fn assert_corrupt_alias_reservation_is_quarantined(corruption: AliasCorruption) {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let (root_control, root) = pin(vec![entry(shared.clone(), b"output", None, false)]);
    let mut owner = Job::new(&registry, 1, vec![(Write, Some(root))]);
    assert!(owner.grant());
    let (stored_control, stored) = pin(vec![entry(shared.clone(), b"stored", Some(50), false)]);
    let mut first = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        first
            .reserve_private_child(Some(stored), &Cancellation::default())
            .unwrap()
    );
    first.finish().unwrap();
    let store = owner
        .owners
        .as_ref()
        .unwrap()
        .store
        .as_ref()
        .unwrap()
        .clone();
    let (incoming_control, incoming) = pin(vec![entry(shared, b"incoming", Some(60), false)]);
    let identity = Arc::as_ptr(&incoming) as *const () as usize;
    let existing = *store.data.lock().unwrap().aliases.keys().next().unwrap();
    let mut change = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let result = match corruption {
        AliasCorruption::Lookup => {
            store
                .data
                .lock()
                .unwrap()
                .alias_identity
                .insert(identity, existing);
            change.reserve_private_child(Some(incoming), &Cancellation::default())
        }
        AliasCorruption::Entry => {
            store.data.lock().unwrap().next_alias = existing - 1;
            change.reserve_private_child(Some(incoming), &Cancellation::default())
        }
        AliasCorruption::Insertion => {
            // Corrupt only after the initial exact-Arc lookup, while the native
            // probe is outside both host locks. Release before any assertion.
            let (entered, release) = block(&incoming_control);
            thread::scope(|threads| {
                let reservation = threads.spawn(|| {
                    change.reserve_private_child(Some(incoming), &Cancellation::default())
                });
                let entered = entered.recv_timeout(Duration::from_secs(8));
                if entered.is_ok() {
                    store
                        .data
                        .lock()
                        .unwrap()
                        .alias_identity
                        .insert(identity, existing);
                }
                drop(release);
                let result = reservation.join().unwrap();
                assert!(
                    entered.is_ok(),
                    "reservation did not reach the controlled native probe"
                );
                result
            })
        }
    };
    assert!(result.is_err());
    assert_eq!(incoming_control.dropped.load(Ordering::SeqCst), 0);
    assert!(owner.claim().is_quarantined());
    assert_eq!(registry.state(1), FactsState::Quarantined);
    assert!(change.finish().is_err());
    assert_eq!(registry.state(1), FactsState::Quarantined);
    drop(store);
    owner.close();
    assert_eq!(incoming_control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(stored_control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(root_control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(registry.inner.io.snapshot().active_shared, 1);
}

#[test]
fn repeated_exact_private_arc_reuses_live_alias_and_cleanup_removes_identity_index() {
    let registry = registry();
    let _observer = observer(&registry);
    let shared = parent(1, &[1, 0]);
    let mut owner = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"output", None, false),
    );
    assert!(owner.grant());
    let (control, alias) = pin(vec![entry(shared.clone(), b"temporary", Some(50), false)]);
    *control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    let mut first = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        first
            .reserve_private_child(Some(alias.clone()), &Cancellation::default())
            .unwrap()
    );
    assert!(
        first
            .reserve_private_child(Some(alias.clone()), &Cancellation::default())
            .unwrap()
    );
    first.finish().unwrap();
    assert_eq!(owner.claim().alias_count(), 1);
    let mut second = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        second
            .reserve_private_child(Some(alias), &Cancellation::default())
            .unwrap()
    );
    second.finish().unwrap();
    assert_eq!(owner.claim().alias_count(), 1);
    assert_eq!(control.dropped.load(Ordering::SeqCst), 0);
    {
        let store = owner.owners.as_ref().unwrap().store.as_ref().unwrap();
        assert_eq!(store.data.lock().unwrap().alias_identity.len(), 1);
    }
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(Arc::from([])));
    owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(owner.claim().alias_count(), 0);
    assert_eq!(control.dropped.load(Ordering::SeqCst), 1);
    {
        let store = owner.owners.as_ref().unwrap().store.as_ref().unwrap();
        assert!(store.data.lock().unwrap().alias_identity.is_empty());
    }

    let (replacement_control, replacement) =
        pin(vec![entry(shared, b"temporary", Some(60), false)]);
    let mut replacement_change = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        replacement_change
            .reserve_private_child(Some(replacement), &Cancellation::default())
            .unwrap()
    );
    replacement_change.finish().unwrap();
    assert_eq!(owner.claim().alias_count(), 1);
    {
        let store = owner.owners.as_ref().unwrap().store.as_ref().unwrap();
        assert_eq!(store.data.lock().unwrap().alias_identity.len(), 1);
    }
    let reader = one(
        &registry,
        2,
        Read,
        entry(
            parent(20, &[20, 0]),
            b"replacement-hardlink",
            Some(60),
            false,
        ),
    );
    assert!(!reader.grant());
    owner.close();
    assert_eq!(replacement_control.dropped.load(Ordering::SeqCst), 1);
    assert!(reader.grant());
}

#[test]
fn existing_read_object_does_not_cover_a_write_child_announcement() {
    let registry = registry();
    let (_, root) = pin(vec![entry(parent(1, &[1, 0]), b"tree-a", Some(10), true)]);
    let (_, read_object) = pin(vec![entry(
        parent(20, &[20, 0]),
        b"read-hardlink",
        Some(50),
        false,
    )]);
    let mut owner = Job::new(
        &registry,
        1,
        vec![(Write, Some(root.clone())), (Read, Some(read_object))],
    );
    assert!(owner.grant());
    let mut transaction = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(root), &Cancellation::default())
            .unwrap()
    );
    let (child_control, child) = pin(vec![entry(
        parent(10, &[10, 1, 0]),
        b"child",
        Some(50),
        false,
    )]);
    assert!(
        transaction
            .reserve_private_child(Some(child), &Cancellation::default())
            .unwrap()
    );
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);
    let reader = one(
        &registry,
        2,
        Read,
        entry(parent(30, &[30, 0]), b"third-hardlink", Some(50), false),
    );
    assert!(
        !reader.grant(),
        "a Read object fact cannot replace a new Write exclusion"
    );
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 0);
    owner.close();
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 1);
    assert!(reader.grant());
}

#[test]
fn absent_subtree_child_observer_retains_object_materialized_at_finish() {
    let registry = registry();
    let (_, root) = pin(vec![entry(parent(1, &[1, 0]), b"tree-a", Some(10), true)]);
    let mut owner = Job::new(&registry, 1, vec![(Write, Some(root.clone()))]);
    assert!(owner.grant());
    let mut transaction = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(root), &Cancellation::default())
            .unwrap()
    );
    let child_parent = parent(10, &[10, 1, 0]);
    let (child_control, child) = pin(vec![entry(child_parent.clone(), b"child", None, false)]);
    *child_control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    assert!(
        transaction
            .reserve_private_child(Some(child), &Cancellation::default())
            .unwrap()
    );
    // The already announced child is created by this transaction. Its independent
    // observer must survive the initial absence and report the new object at finish.
    *child_control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
        vec![entry(child_parent, b"child", Some(50), false)].into(),
    ));
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);
    let reader = one(
        &registry,
        2,
        Read,
        entry(parent(20, &[20, 0]), b"external-hardlink", Some(50), false),
    );
    assert!(
        !reader.grant(),
        "finish must retain the newly materialized object's exclusion"
    );
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 0);
    owner.close();
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 1);
    assert!(reader.grant());
}

#[test]
fn child_subtree_announcement_requires_subtree_not_entry_object_coverage() {
    let registry = registry();
    let (_, root) = pin(vec![entry(parent(1, &[1, 0]), b"tree-a", Some(10), true)]);
    let (_, entry_only) = pin(vec![entry(
        parent(20, &[20, 0]),
        b"original-directory-binding",
        Some(50),
        false,
    )]);
    let mut owner = Job::new(
        &registry,
        1,
        vec![(Write, Some(root.clone())), (Write, Some(entry_only))],
    );
    assert!(owner.grant());
    let mut transaction = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(root), &Cancellation::default())
            .unwrap()
    );
    let (child_control, child) = pin(vec![entry(
        parent(10, &[10, 1, 0]),
        b"child-directory",
        Some(50),
        true,
    )]);
    *child_control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    assert!(
        transaction
            .reserve_private_child(Some(child), &Cancellation::default())
            .unwrap()
    );
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);
    // A retained original binding and current binding may identify the same
    // directory. This descendant uses the original ancestry outside tree-a.
    let descendant = one(
        &registry,
        2,
        Read,
        entry(parent(50, &[50, 20, 0]), b"descendant", Some(60), false),
    );
    assert!(
        !descendant.grant(),
        "Write Entry object coverage cannot protect descendants"
    );
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 0);
    owner.close();
    assert_eq!(child_control.dropped.load(Ordering::SeqCst), 1);
    assert!(descendant.grant());
}

#[test]
fn same_parent_distinct_files_and_disjoint_subtrees_grant_together() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"first", Some(10), false),
    );
    let second = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"second", Some(11), false),
    );
    let tree = one(&registry, 3, Write, entry(shared, b"tree", Some(12), true));
    assert!(first.grant());
    assert!(second.grant());
    assert!(tree.grant());
}

#[test]
fn subtree_ancestry_conflicts_in_both_directions_and_reciprocal_union_is_atomic() {
    let registry = registry();
    let root = parent(1, &[1, 0]);
    let (_, a) = pin(vec![entry(root.clone(), b"A", Some(10), true)]);
    let (_, b) = pin(vec![entry(root.clone(), b"B", Some(20), true)]);
    let first = Job::new(
        &registry,
        1,
        vec![(Read, Some(a.clone())), (Write, Some(b.clone()))],
    );
    let reciprocal = Job::new(&registry, 2, vec![(Read, Some(b)), (Write, Some(a))]);
    let child = one(
        &registry,
        3,
        Read,
        entry(parent(20, &[20, 1, 0]), b"child", Some(21), false),
    );
    assert!(first.grant());
    assert!(!reciprocal.grant());
    assert!(!child.grant());
    let nested_registry = Registry::new_with_io(Arc::new(midnight_work_budget::Gate::new(
        midnight_work_budget::MAX_IO_LIMIT,
    )));
    let nested = one(
        &nested_registry,
        1,
        Write,
        entry(parent(10, &[10, 1, 0]), b"leaf", Some(11), false),
    );
    let outer = one(&nested_registry, 2, Read, entry(root, b"A", Some(10), true));
    assert!(nested.grant());
    assert!(!outer.grant());
}

#[test]
fn differing_lookup_domains_for_same_parent_never_prove_disjointness() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let mut different = (*shared).clone();
    different.lookup_domain = Arc::from(&b"other-rules"[..]);
    let first = one(&registry, 1, Write, entry(shared, b"one", None, false));
    let second = one(
        &registry,
        2,
        Write,
        entry(Arc::new(different), b"two", None, false),
    );
    assert!(first.grant());
    assert!(!second.grant());
}

#[test]
fn unknown_roots_use_executable_opaque_lane_without_upgrading_any_gate() {
    let registry = registry();
    let opaque = Job::new(&registry, 1, vec![(Write, None)]);
    let peer = one(
        &registry,
        2,
        Read,
        entry(parent(1, &[1, 0]), b"leaf", Some(2), false),
    );
    assert!(opaque.grant());
    assert!(!peer.grant());
    let transaction = opaque
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Opaque);
}

#[test]
fn oldest_ready_conflicter_precedes_younger_conflict_while_disjoint_bypasses() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let mut active = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"held", None, false),
    );
    assert!(active.grant());
    let earlier = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"held", None, false),
    );
    let younger = one(
        &registry,
        3,
        Write,
        entry(shared.clone(), b"held", None, false),
    );
    let disjoint = one(&registry, 4, Write, entry(shared, b"other", None, false));
    assert!(!earlier.grant());
    assert!(disjoint.grant());
    active.close();
    assert!(!younger.grant());
    assert!(earlier.grant());
}

#[test]
fn older_multi_scope_waiter_blocks_younger_on_its_free_scope_but_disjoint_bypasses() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let mut active = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"x", None, false),
    );
    assert!(active.grant());
    let (_, x) = pin(vec![entry(shared.clone(), b"x", None, false)]);
    let (_, y) = pin(vec![entry(shared.clone(), b"y", None, false)]);
    let mut earlier = Job::new(&registry, 2, vec![(Write, Some(x)), (Write, Some(y))]);
    let younger = one(
        &registry,
        3,
        Write,
        entry(shared.clone(), b"y", None, false),
    );
    let disjoint = one(&registry, 4, Write, entry(shared, b"z", None, false));
    assert!(!earlier.grant());
    assert!(
        !younger.grant(),
        "the older complete union reserves priority on y while x is held"
    );
    assert!(disjoint.grant(), "priority must not serialize unrelated z");
    active.close();
    assert!(
        !younger.grant(),
        "the younger request cannot win the newly runnable older union"
    );
    assert!(earlier.grant());
    assert!(!younger.grant());
    earlier.close();
    assert!(
        younger.grant(),
        "actual retirement releases the older union while z remains active"
    );
}

#[test]
fn unplanned_earlier_record_and_held_data_read_do_not_block_independent_grant() {
    let registry = registry();
    let (unplanned, owners) = registry.new_job(1).unwrap();
    let first = one(
        &registry,
        2,
        Read,
        entry(parent(1, &[1, 0]), b"held-read", Some(2), false),
    );
    assert!(first.grant());
    assert_eq!(registry.state(2), FactsState::Stable);
    let second = one(
        &registry,
        3,
        Write,
        entry(parent(1, &[1, 0]), b"other", None, false),
    );
    assert!(second.grant());
    unplanned.begin_closing().unwrap();
    drop(owners);
    unplanned.complete_after_drop().unwrap();
}

#[test]
fn private_alias_collision_returns_false_before_create_and_materialization_stays_claimed() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"output", None, false),
    );
    let peer = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"chosen-temp", None, true),
    );
    assert!(first.grant());
    assert!(peer.grant());
    let mut transaction = first
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (declined, collision) = pin(vec![entry(shared.clone(), b"chosen-temp", None, true)]);
    *declined.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    assert!(
        !transaction
            .reserve_private_child(Some(collision), &Cancellation::default())
            .unwrap()
    );
    assert_eq!(declined.dropped.load(Ordering::SeqCst), 1);
    let (control, alias) = pin(vec![entry(shared.clone(), b"fresh-temp", None, true)]);
    assert!(
        transaction
            .reserve_private_child(Some(alias), &Cancellation::default())
            .unwrap()
    );
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
        vec![entry(shared, b"fresh-temp", Some(50), true)].into(),
    ));
    transaction.finish().unwrap();
    let child = one(
        &registry,
        3,
        Write,
        entry(parent(50, &[50, 1, 0]), b"child", None, false),
    );
    assert!(!child.grant());
}

#[test]
fn certified_empty_private_alias_drops_pin_and_removes_random_name_history() {
    let registry = registry();
    let _observer = observer(&registry);
    let shared = parent(1, &[1, 0]);
    let first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"output", None, false),
    );
    assert!(first.grant());
    let mut transaction = first
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (control, alias) = pin(vec![entry(shared.clone(), b"temporary", Some(30), true)]);
    *control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    assert!(
        transaction
            .reserve_private_child(Some(alias), &Cancellation::default())
            .unwrap()
    );
    transaction.finish().unwrap();
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(Arc::from([])));
    first
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(control.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(first.claim().alias_count(), 0);
    let second = one(&registry, 2, Write, entry(shared, b"temporary", None, true));
    assert!(second.grant());
}

#[test]
fn ordinary_finish_error_latches_opaque_diagnostic_without_erasing_native_success() {
    let registry = registry();
    let _observer = observer(&registry);
    let (control, native) = pin(vec![entry(parent(1, &[1, 0]), b"output", None, false)]);
    let first = Job::new(&registry, 1, vec![(Write, Some(native))]);
    assert!(first.grant());
    let result = midnight_platform_common::transfer_namespace::with_transfer_namespace_changes(
        first.claim().changes(),
        || {
            midnight_platform_common::transfer_namespace::with_transfer_namespace_change(
                &Cancellation::default(),
                |_| {
                    *control.probe.lock().unwrap() =
                        Err(failure(io::ErrorKind::PermissionDenied, "refresh refused"));
                    Ok("native publication committed")
                },
            )
        },
    );
    assert_eq!(result.operation.unwrap(), "native publication committed");
    assert!(result.refresh.is_err());
    assert_eq!(registry.state(1), FactsState::Opaque);
    assert!(!first.claim().is_quarantined());
    assert_eq!(first.claim().take_diagnostics().len(), 1);
}

#[test]
fn uncommitted_transaction_quarantines_and_closing_cannot_clear_it() {
    let registry = registry();
    let first = one(
        &registry,
        1,
        Write,
        entry(parent(1, &[1, 0]), b"one", None, false),
    );
    assert!(first.grant());
    drop(
        first
            .claim()
            .changes()
            .begin(&Cancellation::default())
            .unwrap(),
    );
    assert!(first.claim().is_quarantined());
    assert_eq!(registry.state(1), FactsState::Quarantined);
    assert!(first.claim().begin_closing().is_err());
    // The quarantined record is never removed: a later claim fails at once with an
    // actionable error instead of waiting forever for a grant.
    let second = one(
        &registry,
        2,
        Write,
        entry(parent(9, &[9, 0]), b"unrelated", None, false),
    );
    let refused = second
        .claim()
        .try_grant(&Cancellation::default())
        .unwrap_err();
    assert!(is_blocked_by_quarantine(&refused), "{refused}");
    assert!(refused.message.contains("Restart"));
    assert_eq!(registry.state(2), FactsState::Stable);
}

fn poll_once<F: std::future::Future>(future: std::pin::Pin<&mut F>) -> bool {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    future.poll(&mut context).is_ready()
}

/// Run one empty turn of `job`, as a native step that announces nothing would.
fn empty_turn(job: &Job) {
    job.claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap()
        .finish()
        .unwrap();
}
/// Reserve a fresh private child named `key` in `parent` for `job` and finish the turn.
fn reserve_once(job: &Job, parent: &Arc<TransferNamespaceParent>, key: &[u8]) -> bool {
    let mut change = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (_, fresh) = pin(vec![entry(parent.clone(), key, None, false)]);
    let reserved = change
        .reserve_private_child(Some(fresh), &Cancellation::default())
        .unwrap();
    change.finish().unwrap();
    reserved
}

/// The production wait for a refused change (`blockers_changed`) is not ended by the
/// refused transaction's own finish or by any later turn of the refused job: only the
/// refusing peer can clear it. A peer latched opaque clears only by its removal.
#[test]
fn refused_job_own_turns_never_end_its_wait_for_the_refusing_peer() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let mut first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"one", None, false),
    );
    let second = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"two", None, false),
    );
    assert!(first.grant() && second.grant());
    registry.inner.opaque(
        1,
        failure(io::ErrorKind::Other, "source handles still open"),
    );
    clear_peer_refusal();
    assert!(!reserve_once(&second, &shared, b".me-0001"));
    let Some(Refusal::Peer(blockers)) = take_peer_refusal() else {
        panic!("the peer refusal must be recorded");
    };
    assert_eq!(take_peer_refusal(), None, "a refusal is taken once");
    assert!(
        !blockers.can_clear_in_place(),
        "a latched peer clears only by its removal"
    );
    assert_eq!(
        blockers.hold(),
        Hold::Unknown,
        "it refuses whatever is touched"
    );
    let watch = second.claim().watch();
    let cleared = watch.blockers_changed(&blockers);
    tokio::pin!(cleared);
    assert!(
        !poll_once(cleared.as_mut()),
        "the refused job's own transaction finish ended its wait for the opaque peer"
    );
    empty_turn(&second);
    assert!(
        !poll_once(cleared.as_mut()),
        "a later own turn ended the wait"
    );
    empty_turn(&first);
    assert!(
        !poll_once(cleared.as_mut()),
        "a latched peer's turn cannot clear its refusal"
    );
    first.close();
    assert!(poll_once(cleared.as_mut()));
}

/// An intersecting (not latched) peer can clear its refusal by any change of its own
/// record, such as its next turn; the refused job's turns still never end the wait.
#[test]
fn intersecting_peer_refusal_waits_for_that_peers_next_change() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"one", None, false),
    );
    let second = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"two", None, false),
    );
    assert!(first.grant() && second.grant());
    clear_peer_refusal();
    // The second job announces the first job's own slot.
    assert!(!reserve_once(&second, &shared, b"one"));
    let Some(Refusal::Peer(blockers)) = take_peer_refusal() else {
        panic!("the intersecting peer's refusal must be recorded");
    };
    assert!(blockers.can_clear_in_place());
    assert_eq!(blockers.hold(), Hold::Overlap);
    let watch = second.claim().watch();
    let cleared = watch.blockers_changed(&blockers);
    tokio::pin!(cleared);
    empty_turn(&second);
    assert!(!poll_once(cleared.as_mut()));
    empty_turn(&first);
    assert!(poll_once(cleared.as_mut()));
}

/// A refused grant (`Grant::Conflict`) waits only for a grant, removal or quarantine:
/// ordinary turns of the conflicting peer do not end it.
#[test]
fn refused_grant_waits_only_for_a_structural_change() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let mut first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"one", None, false),
    );
    assert!(first.grant());
    let second = one(&registry, 2, Write, entry(shared, b"one", None, false));
    let Grant::Conflict(seen, Hold::Overlap) =
        second.claim().admit(&Cancellation::default()).unwrap()
    else {
        panic!("an intersecting candidate must be refused");
    };
    let watch = second.claim().watch();
    let structural = watch.changed(seen, true);
    tokio::pin!(structural);
    assert!(!poll_once(structural.as_mut()));
    empty_turn(&first);
    assert!(!poll_once(structural.as_mut()));
    first.close();
    assert!(poll_once(structural.as_mut()));
}

/// The record describes the last reservation only: a refused fresh name replaced by an
/// accepted one in the same step is not reported as a refusal.
#[test]
fn accepted_reservation_replaces_an_earlier_refusal_of_the_same_step() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"one", None, false),
    );
    let second = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"two", None, false),
    );
    assert!(first.grant() && second.grant());
    let mut change = second
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (_, taken) = pin(vec![entry(shared.clone(), b"one", None, false)]);
    let (_, free) = pin(vec![entry(shared.clone(), b".me-0004", None, false)]);
    assert!(
        !change
            .reserve_private_child(Some(taken), &Cancellation::default())
            .unwrap()
    );
    assert!(
        change
            .reserve_private_child(Some(free), &Cancellation::default())
            .unwrap()
    );
    change.finish().unwrap();
    assert_eq!(take_peer_refusal(), None);
}

#[test]
fn refusal_by_a_quarantined_peer_is_reported_as_endless() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let first = one(
        &registry,
        1,
        Write,
        entry(shared.clone(), b"one", None, false),
    );
    let second = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"two", None, false),
    );
    assert!(first.grant() && second.grant());
    first.claim().quarantine();
    clear_peer_refusal();
    let mut change = second
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (_, fresh) = pin(vec![entry(shared, b".me-0002", None, false)]);
    assert!(
        !change
            .reserve_private_child(Some(fresh), &Cancellation::default())
            .unwrap()
    );
    change.finish().unwrap();
    assert_eq!(take_peer_refusal(), Some(Refusal::Quarantined));
}

#[test]
fn known_disjoint_closing_peer_does_not_refuse_private_reservation() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let (peer_control, peer_pin) = pin(vec![entry(shared.clone(), b"first.bin", Some(10), false)]);
    let mut peer = Job::new(&registry, 1, vec![(Write, Some(peer_pin))]);
    let mut owner = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"second.bin", None, false),
    );
    assert!(peer.grant());
    assert!(owner.grant());
    peer.claim().begin_closing().unwrap();
    assert_eq!(registry.state(1), FactsState::Closing);
    assert_eq!(peer_control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(registry.inner.io.snapshot().active_shared, 2);

    let (candidate_control, candidate) = pin(vec![entry(shared, b"second.bin", None, false)]);
    *candidate_control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    let mut transaction = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let reserved = transaction
        .reserve_private_child(Some(candidate), &Cancellation::default())
        .unwrap();
    transaction.finish().unwrap();
    assert!(
        reserved,
        "known disjoint peer retirement must not refuse the owned publication name"
    );
    assert_eq!(candidate_control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(peer_control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(registry.inner.io.snapshot().active_shared, 2);

    // Closing must keep its cleanup turn and its actual owners until confirmed retirement.
    peer.claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(registry.state(1), FactsState::Closing);
    peer.close();
    assert_eq!(peer_control.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(registry.inner.io.snapshot().active_shared, 1);
    owner.close();
    assert_eq!(candidate_control.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(registry.inner.io.snapshot().active_shared, 0);
}

#[test]
fn peer_completion_after_private_reservation_snapshot_does_not_refuse_disjoint_name() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let (peer_control, peer_pin) = pin(vec![entry(shared.clone(), b"first.bin", Some(10), false)]);
    let mut peer = Job::new(&registry, 1, vec![(Write, Some(peer_pin))]);
    let mut owner = one(
        &registry,
        2,
        Write,
        entry(shared.clone(), b"second.bin", None, false),
    );
    assert!(peer.grant());
    assert!(owner.grant());
    let (candidate_control, candidate) = pin(vec![entry(shared, b"second.bin", None, false)]);
    *candidate_control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    let changes = owner.claim().changes();

    let reserved = thread::scope(|threads| {
        let (entered, observed) = std::sync::mpsc::channel();
        let (continue_send, continue_receive) = std::sync::mpsc::channel();
        // Release the worker before scoped joining even if a main-thread assertion unwinds.
        let release = Release(Some(continue_send));
        let worker = threads.spawn(move || {
            AFTER_RESERVATION_SNAPSHOT.with(|slot| {
                let mut slot = slot.borrow_mut();
                assert!(slot.is_none());
                *slot = Some(Box::new(move || {
                    entered.send(()).unwrap();
                    continue_receive.recv().unwrap();
                }));
            });
            let mut transaction = changes.begin(&Cancellation::default()).unwrap();
            let reserved = transaction
                .reserve_private_child(Some(candidate), &Cancellation::default())
                .unwrap();
            transaction.finish().unwrap();
            reserved
        });
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let epoch = {
            let state = registry.inner.state.lock().unwrap();
            assert_eq!(state.turn, Some(2));
            state.revision
        };
        assert_eq!(registry.inner.io.snapshot().active_shared, 2);
        peer.close();
        assert_eq!(peer_control.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(registry.inner.io.snapshot().active_shared, 1);
        {
            let state = registry.inner.state.lock().unwrap();
            assert_eq!(state.turn, Some(2));
            assert!(!state.records.contains_key(&1));
            assert!(state.revision > epoch);
        }
        drop(release);
        worker.join().unwrap()
    });
    assert!(
        reserved,
        "completion of a disjoint peer after the snapshot must not refuse publication"
    );
    assert_eq!(candidate_control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(registry.inner.io.snapshot().active_shared, 1);
    owner.close();
    assert_eq!(candidate_control.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(registry.inner.io.snapshot().active_shared, 0);
}

#[test]
fn private_reservation_rechecks_new_peer_obstructions_after_snapshot() {
    #[derive(Clone, Copy, Debug)]
    enum Obstruction {
        OpaqueFacts,
        LatchedOpaque,
        Quarantined,
        Conflicting,
    }

    for obstruction in [
        Obstruction::OpaqueFacts,
        Obstruction::LatchedOpaque,
        Obstruction::Quarantined,
        Obstruction::Conflicting,
    ] {
        let registry = registry();
        let shared = parent(1, &[1, 0]);
        let (peer_control, peer_pin) =
            pin(vec![entry(shared.clone(), b"first.bin", Some(10), false)]);
        let mut peer = Job::new(&registry, 1, vec![(Write, Some(peer_pin))]);
        let mut owner = one(
            &registry,
            2,
            Write,
            entry(shared.clone(), b"second.bin", None, false),
        );
        assert!(peer.grant());
        assert!(owner.grant());
        let replacement = match obstruction {
            Obstruction::OpaqueFacts | Obstruction::Conflicting => {
                let probe = if matches!(obstruction, Obstruction::OpaqueFacts) {
                    TransferNamespaceProbe::Unknown
                } else {
                    TransferNamespaceProbe::Known(
                        vec![entry(shared.clone(), b"second.bin", None, false)].into(),
                    )
                };
                Some(Arc::new(
                    Facts::build(vec![PinFact {
                        root: Some(0),
                        alias: None,
                        access: Write,
                        probe,
                    }])
                    .unwrap(),
                ))
            }
            Obstruction::LatchedOpaque | Obstruction::Quarantined => None,
        };
        let (candidate_control, candidate) = pin(vec![entry(shared, b"second.bin", None, false)]);
        *candidate_control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
        let changes = owner.claim().changes();

        let outcome = thread::scope(|threads| {
            let (entered, observed) = std::sync::mpsc::channel();
            let (continue_send, continue_receive) = std::sync::mpsc::channel();
            let release = Release(Some(continue_send));
            let worker = threads.spawn(move || {
                AFTER_RESERVATION_SNAPSHOT.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    assert!(slot.is_none());
                    *slot = Some(Box::new(move || {
                        entered.send(()).unwrap();
                        continue_receive.recv().unwrap();
                    }));
                });
                let mut transaction = changes.begin(&Cancellation::default()).unwrap();
                let result =
                    transaction.reserve_private_child(Some(candidate), &Cancellation::default());
                let finished = transaction.finish();
                (result, finished)
            });
            observed.recv_timeout(Duration::from_secs(5)).unwrap();
            let original = {
                let mut state = registry.inner.state.lock().unwrap();
                assert_eq!(state.turn, Some(2));
                let record = state.records.get_mut(&1).unwrap();
                assert!(record.granted);
                // Change only cached host metadata at the validation boundary;
                // the admitted claim, native pins and actual I/O owners remain intact.
                let original = (
                    record.state,
                    record.opaque_latched,
                    record.facts.clone(),
                    record.quarantined.load(Ordering::Acquire),
                );
                match obstruction {
                    Obstruction::OpaqueFacts => {
                        record.state = FactsState::Opaque;
                    }
                    Obstruction::LatchedOpaque => {
                        record.state = FactsState::Opaque;
                        record.opaque_latched = true;
                    }
                    Obstruction::Quarantined => {
                        record.state = FactsState::Quarantined;
                        record.quarantined.store(true, Ordering::Release);
                    }
                    Obstruction::Conflicting => {}
                }
                if let Some(replacement) = replacement {
                    record.facts = replacement;
                }
                Inner::bump(&mut state, 1).unwrap();
                original
            };
            drop(release);
            let outcome = worker.join();
            // Restore synthetic metadata before interpreting any worker assertion
            // or error, so fixture teardown never clears a genuine native failure.
            {
                let mut state = registry
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let record = state.records.get_mut(&1).unwrap();
                record.state = original.0;
                record.opaque_latched = original.1;
                record.facts = original.2;
                record.quarantined.store(original.3, Ordering::Release);
                Inner::bump(&mut state, 1).unwrap();
            }
            outcome
        });
        let (reserved, finished) = outcome.unwrap();
        finished.unwrap();
        assert!(
            !matches!(reserved, Ok(true)),
            "{obstruction:?} introduced after the snapshot was ignored: {reserved:?}"
        );
        assert_eq!(owner.claim().alias_count(), 0);
        assert_eq!(candidate_control.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(peer_control.dropped.load(Ordering::SeqCst), 0);
        assert_eq!(registry.inner.io.snapshot().active_shared, 2);
        peer.close();
        assert_eq!(peer_control.dropped.load(Ordering::SeqCst), 1);
        assert_eq!(registry.inner.io.snapshot().active_shared, 1);
        owner.close();
        assert_eq!(registry.inner.io.snapshot().active_shared, 0);
    }
}

#[test]
fn one_hundred_thousand_sibling_metadata_roots_use_indexes_and_shared_parent() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let roots = (0..100_000u64)
        .map(|value| {
            entry(
                shared.clone(),
                &value.to_le_bytes(),
                Some(value + 10),
                false,
            )
        })
        .collect();
    let (_, native) = pin(roots);
    let first = Job::new(&registry, 1, vec![(Read, Some(native))]);
    assert!(first.grant());
    let second = one(
        &registry,
        2,
        Write,
        entry(shared, b"independent", None, false),
    );
    assert!(second.grant());
    let state = registry.inner.state.lock().unwrap();
    let facts = &state.records[&1].facts;
    assert_eq!(facts.index.slots.len(), 100_000);
    assert_eq!(facts.index.parents.len(), 1);
}

#[test]
fn native_probe_unwind_quarantines_strong_pin_and_real_io_owner() {
    let registry = registry();
    let (control, native) = pin(vec![entry(parent(1, &[1, 0]), b"one", Some(2), false)]);
    let mut first = Job::new(&registry, 1, vec![(Write, Some(native))]);
    control.panic.store(true, Ordering::SeqCst);
    assert!(first.claim().try_grant(&Cancellation::default()).is_err());
    assert!(first.claim().is_quarantined());
    first.close();
    assert_eq!(control.dropped.load(Ordering::SeqCst), 0);
    assert_eq!(registry.inner.io.snapshot().active_shared, 1);
}

#[test]
fn selected_empty_root_is_rejected_and_mutation_cannot_begin_before_grant() {
    let registry = registry();
    let (claim, owners) = registry.new_job(1).unwrap();
    assert!(claim.changes().begin(&Cancellation::default()).is_err());
    let io = Arc::new(
        registry
            .inner
            .io
            .acquire(midnight_work_budget::Admission::Shared, || false)
            .unwrap(),
    );
    let (_, empty) = pin(Vec::new());
    assert!(
        claim
            .plan_union(
                vec![(Write, Some(empty))],
                io.clone(),
                &Cancellation::default()
            )
            .is_err()
    );
    assert_eq!(registry.state(1), FactsState::Opaque);
    claim.begin_closing().unwrap();
    drop(owners);
    claim.complete_after_drop().unwrap();
    drop(io);
}

#[test]
fn unknown_private_alias_requires_already_granted_opaque_exclusion() {
    let registry = registry();
    let known = one(
        &registry,
        1,
        Write,
        entry(parent(1, &[1, 0]), b"one", None, false),
    );
    assert!(known.grant());
    let mut change = known
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        change
            .reserve_private_child(None, &Cancellation::default())
            .is_err()
    );
    change.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);
    drop(known);
    let opaque = Job::new(&registry, 2, vec![(Write, None)]);
    assert!(opaque.grant());
    let mut change = opaque
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        change
            .reserve_private_child(None, &Cancellation::default())
            .unwrap()
    );
    change.finish().unwrap();
    assert_eq!(registry.state(2), FactsState::Opaque);
}

#[test]
fn native_pin_destructor_unwind_never_certifies_close_or_releases_io() {
    let registry = registry();
    let (control, native) = pin(vec![entry(parent(1, &[1, 0]), b"one", Some(2), false)]);
    let mut first = Job::new(&registry, 1, vec![(Write, Some(native))]);
    assert!(first.grant());
    control.panic_drop.store(true, Ordering::SeqCst);
    first.close();
    assert_eq!(control.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(registry.state(1), FactsState::Quarantined);
    assert_eq!(registry.inner.io.snapshot().active_shared, 1);
}

#[test]
fn ordinary_native_failure_and_late_cancel_finish_the_turn_without_quarantine() {
    let registry = registry();
    let first = one(
        &registry,
        1,
        Write,
        entry(parent(1, &[1, 0]), b"one", None, false),
    );
    assert!(first.grant());
    let cancel = Cancellation::default();
    let result = midnight_platform_common::transfer_namespace::with_transfer_namespace_changes(
        first.claim().changes(),
        || {
            midnight_platform_common::transfer_namespace::with_transfer_namespace_change::<()>(
                &cancel,
                |_| {
                    cancel.cancel();
                    Err(failure(
                        io::ErrorKind::AlreadyExists,
                        "native no-replace collision",
                    ))
                },
            )
        },
    );
    assert_eq!(
        result.operation.unwrap_err().kind,
        io::ErrorKind::AlreadyExists
    );
    assert!(result.refresh.is_ok());
    assert!(!first.claim().is_quarantined());
    assert_eq!(registry.state(1), FactsState::Stable);
}

#[test]
fn announced_root_and_private_alias_lifecycles_do_not_rescan_or_rebuild_all_roots() {
    const ROOTS: usize = 512;
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let roots: Vec<_> = (0..ROOTS)
        .map(|index| {
            pin(vec![entry(
                shared.clone(),
                &index.to_le_bytes(),
                None,
                true,
            )])
        })
        .collect();
    let mut job = Job::new(
        &registry,
        1,
        roots
            .iter()
            .map(|(_, pin)| (Write, Some(pin.clone())))
            .collect(),
    );
    assert!(job.grant());
    let baseline_calls: usize = roots
        .iter()
        .map(|(control, _)| control.calls.load(Ordering::SeqCst))
        .sum();
    let base = registry.inner.state.lock().unwrap().records[&1]
        .facts
        .clone();
    let base_visits = base.index.visits;
    for (index, (control, original)) in roots.iter().enumerate() {
        let object = 10_000 + index as u64;
        let mut transaction = job
            .claim()
            .changes()
            .begin(&Cancellation::default())
            .unwrap();
        assert!(
            transaction
                .reserve_private_child(Some(original.clone()), &Cancellation::default())
                .unwrap()
        );
        let (alias_control, alias) = pin(vec![entry(
            shared.clone(),
            format!("temporary-{index}").as_bytes(),
            None,
            true,
        )]);
        assert!(
            transaction
                .reserve_private_child(Some(alias.clone()), &Cancellation::default())
                .unwrap()
        );
        *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
            vec![entry(
                shared.clone(),
                &index.to_le_bytes(),
                Some(object),
                true,
            )]
            .into(),
        ));
        *alias_control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
            vec![entry(
                shared.clone(),
                format!("temporary-{index}").as_bytes(),
                Some(100_000 + index as u64),
                true,
            )]
            .into(),
        ));
        transaction.finish().unwrap();
        let descendant = Facts::build(vec![PinFact {
            root: Some(0),
            alias: None,
            access: Write,
            probe: TransferNamespaceProbe::Known(
                vec![entry(
                    parent(object, &[object, 1, 0]),
                    b"child",
                    None,
                    false,
                )]
                .into(),
            ),
        }])
        .unwrap();
        assert!(
            base.conflicts(&descendant),
            "materialized subtree must be visible before another admission refresh"
        );
        let mut cleanup = job
            .claim()
            .changes()
            .begin(&Cancellation::default())
            .unwrap();
        assert!(
            cleanup
                .reserve_private_child(Some(original.clone()), &Cancellation::default())
                .unwrap()
        );
        assert!(
            cleanup
                .reserve_private_child(Some(alias), &Cancellation::default())
                .unwrap()
        );
        *alias_control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(Arc::from([])));
        cleanup.finish().unwrap();
        assert_eq!(job.claim().alias_count(), 0);
        assert_eq!(alias_control.dropped.load(Ordering::SeqCst), 1);
        let current = registry.inner.state.lock().unwrap().records[&1]
            .facts
            .clone();
        assert!(
            Arc::ptr_eq(&base, &current),
            "incremental finish must retain the immutable base"
        );
        assert!(current.overlay.lock().unwrap().aliases.is_empty());
    }
    let calls: usize = roots
        .iter()
        .map(|(control, _)| control.calls.load(Ordering::SeqCst))
        .sum();
    assert!(
        calls - baseline_calls <= ROOTS * 8,
        "selected roots were rescanned for unrelated lifecycles"
    );
    assert_eq!(base.index.visits, base_visits);
    assert!(
        base.overlay.lock().unwrap().index.visits <= ROOTS * 16,
        "the touched overlay was rebuilt from all prior roots"
    );
    drop(roots);
    job.close();
}

#[test]
fn incremental_mutations_probe_only_announced_aliases_and_preserve_cold_hardlinks() {
    const HISTORY: usize = 128;
    let registry = registry();
    let directory = parent(100, &[100, 1, 0]);
    let (root_control, root) = pin(vec![entry(parent(1, &[1, 0]), b"output", Some(100), true)]);
    let mut owner = Job::new(&registry, 1, vec![(Write, Some(root.clone()))]);
    assert!(owner.grant());
    let mut aliases: Vec<_> = (0..HISTORY)
        .map(|index| {
            pin(vec![entry(
                directory.clone(),
                &index.to_le_bytes(),
                Some(1000 + index as u64),
                false,
            )])
        })
        .collect();
    let mut initial = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        initial
            .reserve_private_child(Some(root.clone()), &Cancellation::default())
            .unwrap()
    );
    for (_, alias) in &aliases {
        assert!(
            initial
                .reserve_private_child(Some(alias.clone()), &Cancellation::default())
                .unwrap()
        );
    }
    initial.finish().unwrap();
    let cold_calls = aliases[7].0.calls.load(Ordering::SeqCst);
    let cold_reader = Facts::build(vec![PinFact {
        root: Some(0),
        alias: None,
        access: Read,
        probe: TransferNamespaceProbe::Known(
            vec![entry(parent(200, &[200, 0]), b"outside", Some(1007), false)].into(),
        ),
    }])
    .unwrap();
    for turn in 0..16_u64 {
        let before = root_control.calls.load(Ordering::SeqCst)
            + aliases
                .iter()
                .map(|(control, _)| control.calls.load(Ordering::SeqCst))
                .sum::<usize>();
        let mut change = owner
            .claim()
            .changes()
            .begin(&Cancellation::default())
            .unwrap();
        assert!(
            change
                .reserve_private_child(Some(root.clone()), &Cancellation::default())
                .unwrap()
        );
        for (_, alias) in &aliases[..2] {
            assert!(
                change
                    .reserve_private_child(Some(alias.clone()), &Cancellation::default())
                    .unwrap()
            );
        }
        let key = format!("new-{turn}");
        let (control, alias) = pin(vec![entry(directory.clone(), key.as_bytes(), None, false)]);
        assert!(
            change
                .reserve_private_child(Some(alias.clone()), &Cancellation::default())
                .unwrap()
        );
        // The new absent slot is materialized only after its observer is retained.
        *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
            vec![entry(
                directory.clone(),
                key.as_bytes(),
                Some(9000 + turn),
                false,
            )]
            .into(),
        ));
        *aliases[0].0.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
            vec![entry(
                directory.clone(),
                &0_usize.to_le_bytes(),
                Some(5000 + turn),
                false,
            )]
            .into(),
        ));
        change.finish().unwrap();
        aliases.push((control, alias));
        let after = root_control.calls.load(Ordering::SeqCst)
            + aliases
                .iter()
                .map(|(control, _)| control.calls.load(Ordering::SeqCst))
                .sum::<usize>();
        assert_eq!(
            after - before,
            8,
            "one root and three aliases are probed at announcement and finish"
        );
        assert_eq!(aliases[7].0.calls.load(Ordering::SeqCst), cold_calls);
        let facts = registry.inner.state.lock().unwrap().records[&1]
            .facts
            .clone();
        assert!(
            facts.conflicts(&cold_reader),
            "partial updates must preserve untouched object claims"
        );
        assert_eq!(owner.claim().alias_count(), HISTORY + turn as usize + 1);
    }
    let reader = one(
        &registry,
        2,
        Read,
        entry(parent(200, &[200, 0]), b"outside", Some(1007), false),
    );
    assert!(!reader.grant());
    assert_eq!(
        aliases[7].0.calls.load(Ordering::SeqCst),
        cold_calls,
        "admission compares committed facts and never probes a peer's aliases"
    );
    drop(aliases);
    drop(root);
    owner.close();
    assert!(reader.grant());
}

#[test]
fn touched_existing_empty_alias_is_removed_and_its_slot_can_be_reused() {
    let registry = registry();
    let directory = parent(100, &[100, 1, 0]);
    let (_, root) = pin(vec![entry(parent(1, &[1, 0]), b"output", Some(100), true)]);
    let owner = Job::new(&registry, 1, vec![(Write, Some(root.clone()))]);
    assert!(owner.grant());
    let (control, alias) = pin(vec![entry(
        directory.clone(),
        b"temporary",
        Some(500),
        false,
    )]);
    *control.drop_registry.lock().unwrap() = Some(Arc::downgrade(&registry.inner));
    let mut create = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        create
            .reserve_private_child(Some(root.clone()), &Cancellation::default())
            .unwrap()
    );
    assert!(
        create
            .reserve_private_child(Some(alias.clone()), &Cancellation::default())
            .unwrap()
    );
    create.finish().unwrap();
    assert_eq!(owner.claim().alias_count(), 1);
    // This pin certifies its own completed native removal before reannouncement.
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(Arc::from([])));
    let before = control.calls.load(Ordering::SeqCst);
    let mut removal = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        removal
            .reserve_private_child(Some(root), &Cancellation::default())
            .unwrap()
    );
    assert!(
        removal
            .reserve_private_child(Some(alias), &Cancellation::default())
            .unwrap()
    );
    assert_eq!(
        owner.claim().alias_count(),
        1,
        "announcement alone cannot drop a retained owner"
    );
    removal.finish().unwrap();
    assert_eq!(control.calls.load(Ordering::SeqCst) - before, 2);
    assert_eq!(control.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(owner.claim().alias_count(), 0);
    let (_, replacement) = pin(vec![entry(directory, b"temporary", None, false)]);
    let mut reuse = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        reuse
            .reserve_private_child(Some(replacement), &Cancellation::default())
            .unwrap()
    );
    reuse.finish().unwrap();
    assert_eq!(owner.claim().alias_count(), 1);
}

#[test]
fn fresh_empty_private_and_original_announcements_remain_invalid() {
    let registry = registry();
    let _observer = observer(&registry);
    let (control, root) = pin(vec![entry(parent(1, &[1, 0]), b"output", None, false)]);
    let owner = Job::new(&registry, 1, vec![(Write, Some(root.clone()))]);
    assert!(owner.grant());
    let (_, fresh) = pin(vec![]);
    let mut change = owner
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let error = change
        .reserve_private_child(Some(fresh), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidData);
    assert_eq!(owner.claim().alias_count(), 0);
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(Arc::from([])));
    let error = change
        .reserve_private_child(Some(root), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidData);
    assert!(change.finish().is_err());
    assert!(
        !owner.claim().is_quarantined(),
        "ordinary invalid metadata is not an unwind"
    );
}

#[test]
fn fresh_private_pin_does_not_opt_unmodified_helpers_into_partial_refresh() {
    let registry = registry();
    let _observer = observer(&registry);
    let shared = parent(1, &[1, 0]);
    let (control, native) = pin(vec![entry(shared.clone(), b"selected", None, true)]);
    let job = Job::new(&registry, 1, vec![(Write, Some(native))]);
    assert!(job.grant());
    let before = control.calls.load(Ordering::SeqCst);
    let mut transaction = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (_, alias) = pin(vec![entry(shared.clone(), b"temp", None, true)]);
    assert!(
        transaction
            .reserve_private_child(Some(alias), &Cancellation::default())
            .unwrap()
    );
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
        vec![entry(shared, b"selected", Some(50), true)].into(),
    ));
    transaction.finish().unwrap();
    assert!(control.calls.load(Ordering::SeqCst) > before);
    let descendant = Facts::build(vec![PinFact {
        root: Some(0),
        alias: None,
        access: Write,
        probe: TransferNamespaceProbe::Known(
            vec![entry(parent(50, &[50, 1, 0]), b"child", None, false)].into(),
        ),
    }])
    .unwrap();
    assert!(
        registry.inner.state.lock().unwrap().records[&1]
            .facts
            .conflicts(&descendant)
    );
}

/// A job without a granted peer refreshes only announced pins per turn. Its final
/// revalidation before closing still latches an external change to an untouched pinned
/// binding and records it as the job's diagnostic, as a full turn refresh would have.
/// A claim that was never granted touched nothing and is not revalidated.
#[test]
fn final_revalidation_reports_an_untouched_pin_change_of_a_lone_job() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let (_, touched) = pin(vec![entry(shared.clone(), b"touched", None, false)]);
    let (untouched_control, untouched) =
        pin(vec![entry(shared.clone(), b"untouched", None, false)]);
    let mut job = Job::new(
        &registry,
        1,
        vec![(Write, Some(touched.clone())), (Read, Some(untouched))],
    );
    assert!(job.grant());
    let before = untouched_control.calls.load(Ordering::SeqCst);
    let mut transaction = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(touched), &Cancellation::default())
            .unwrap()
    );
    transaction.finish().unwrap();
    assert_eq!(
        untouched_control.calls.load(Ordering::SeqCst),
        before,
        "a lone job's turn refreshes only what it announced"
    );
    *untouched_control.probe.lock().unwrap() = Err(failure(
        io::ErrorKind::InvalidInput,
        "original namespace parent or ancestry changed",
    ));
    job.claim().revalidate_before_close();
    assert!(untouched_control.calls.load(Ordering::SeqCst) > before);
    // The same persistent failure seen again is reported once.
    job.claim().revalidate_before_close();
    let diagnostics = job.claim().take_diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("ancestry changed"));
    assert_eq!(registry.state(1), FactsState::Opaque);

    let (waiting_control, waiting) = pin(vec![entry(shared, b"touched", None, false)]);
    let ungranted = Job::new(&registry, 2, vec![(Write, Some(waiting))]);
    let planned = waiting_control.calls.load(Ordering::SeqCst);
    ungranted.claim().revalidate_before_close();
    assert_eq!(waiting_control.calls.load(Ordering::SeqCst), planned);
    assert!(ungranted.claim().take_diagnostics().is_empty());
    job.close();
}

/// A transfer job planned as copies plan one: a (source, destination) pair per selected
/// item. Item `i` reads `s{i}` in folder 1 and writes `names[i]` in folder 2. Returns the
/// job, the controls of its roots in planned order (item `i`: `2i` and `2i + 1`) and its
/// destination pins.
fn items(
    registry: &Registry,
    sequence: u64,
    names: &[&str],
) -> (Job, Vec<Arc<Control>>, Vec<Arc<dyn TransferNamespacePin>>) {
    let mut controls = Vec::new();
    let mut outputs = Vec::new();
    let mut endpoints = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let (read, source) = pin(vec![entry(
            parent(1, &[1, 0]),
            format!("s{index}").as_bytes(),
            Some(100 + index as u64),
            false,
        )]);
        let (written, output) = pin(vec![entry(
            parent(2, &[2, 0]),
            name.as_bytes(),
            None,
            false,
        )]);
        controls.extend([read, written]);
        endpoints.extend([(Read, Some(source)), (Write, Some(output.clone()))]);
        outputs.push(output);
    }
    let (claim, owners) = registry.new_job(sequence).unwrap();
    let io = Arc::new(
        registry
            .inner
            .io
            .acquire(midnight_work_budget::Admission::Shared, || false)
            .unwrap(),
    );
    claim
        .plan_transfer(endpoints, io.clone(), &Cancellation::default())
        .unwrap();
    let job = Job {
        claim: Some(claim),
        owners: Some(owners),
        io: Some(io),
    };
    (job, controls, outputs)
}
/// One opted-in turn that announces the item's own destination pin, as a native
/// publication of the item announces it.
fn publish(job: &Job, output: &Arc<dyn TransferNamespacePin>) {
    let mut turn = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        turn.reserve_private_child(Some(output.clone()), &Cancellation::default())
            .unwrap()
    );
    turn.finish().unwrap();
}
fn calls(controls: &[Arc<Control>]) -> Vec<usize> {
    controls
        .iter()
        .map(|control| control.calls.load(Ordering::SeqCst))
        .collect()
}
/// The roots whose pins were refreshed since `before`.
fn probed_since(controls: &[Arc<Control>], before: &[usize]) -> Vec<usize> {
    (0..controls.len())
        .filter(|root| controls[*root].calls.load(Ordering::SeqCst) != before[*root])
        .collect()
}
fn changed(name: &str) -> Result<TransferNamespaceProbe, FsError> {
    Err(FsError::from_io(
        "transfer namespace",
        Path::new(name),
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "original namespace leaf binding changed",
        ),
    ))
}

/// WIN-8: a job stopped before its last item (cancelled, or stopped by an error) probes
/// as it closes the pins of the items it started, the current one included, and every
/// root its turns touched: here item 4's destination, which has item 1's name, so
/// publishing item 1 touched it. The pins of the items it never started are not probed.
#[test]
fn a_stopped_job_closes_with_the_items_it_started_and_the_roots_its_turns_touched() {
    let registry = registry();
    let names = ["d0", "same", "d2", "d3", "same", "d5"];
    let (job, controls, outputs) = items(&registry, 1, &names);
    assert!(job.grant());
    let unstarted = job.claim().unstarted();
    unstarted.set(6);
    unstarted.set(5);
    publish(&job, &outputs[0]);
    unstarted.set(4);
    publish(&job, &outputs[1]);
    // Item 2 is the current item when the job stops.
    unstarted.set(3);
    let before = calls(&controls);
    job.claim().revalidate_before_close();
    assert_eq!(probed_since(&controls, &before), [0, 1, 2, 3, 4, 5, 9]);
    let closes = registry.closes_for_test();
    assert_eq!(closes.len(), 1);
    assert_eq!(
        (
            closes[0].sequence,
            closes[0].roots.as_slice(),
            closes[0].aliases,
            closes[0].refreshed
        ),
        (1, [0, 1, 2, 3, 4, 5, 9].as_slice(), 0, 7)
    );
    assert!(job.claim().take_diagnostics().is_empty());
    assert_eq!(registry.state(1), FactsState::Stable);
}

/// WIN-8: as a stopped job closes, an external change to the binding of an item it
/// started (a copied file renamed) is still latched and reported; one of an item it
/// never started (its name created by another program) is neither probed nor reported.
#[test]
fn a_change_is_reported_at_close_for_a_started_item_and_not_for_an_unstarted_one() {
    let registry = registry();
    let (job, controls, outputs) = items(&registry, 1, &["d0", "d1", "d2", "d3"]);
    assert!(job.grant());
    let unstarted = job.claim().unstarted();
    unstarted.set(3);
    publish(&job, &outputs[0]);
    unstarted.set(2);
    publish(&job, &outputs[1]);
    let unstarted_calls = controls[7].calls.load(Ordering::SeqCst);
    *controls[7].probe.lock().unwrap() = changed("d3");
    job.claim().revalidate_before_close();
    let diagnostics = job.claim().take_diagnostics();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(registry.state(1), FactsState::Stable);
    *controls[3].probe.lock().unwrap() = changed("d1");
    job.claim().revalidate_before_close();
    let diagnostics = job.claim().take_diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].path, Path::new("d1"));
    assert_eq!(registry.state(1), FactsState::Opaque);
    assert_eq!(controls[7].calls.load(Ordering::SeqCst), unstarted_calls);
}

/// WIN-8: a job that completed every item touched every root, so its close probes every
/// pin, as before; so does the close of a job that reported no progress.
#[test]
fn a_completed_job_or_one_without_progress_probes_every_pin_at_close() {
    for unstarted in [Some(0), None] {
        let registry = registry();
        let (job, controls, outputs) = items(&registry, 1, &["d0", "d1", "d2"]);
        assert!(job.grant());
        if let Some(unstarted) = unstarted {
            job.claim().unstarted().set(unstarted);
        }
        publish(&job, &outputs[0]);
        let before = calls(&controls);
        job.claim().revalidate_before_close();
        assert_eq!(
            probed_since(&controls, &before),
            (0..6).collect::<Vec<_>>(),
            "{unstarted:?}"
        );
    }
}

/// WIN-8: a turn that did not announce every binding it changed (here a fresh private
/// pin only) may have changed any pin without recording it. Its own finish refreshes
/// every pin, and the job's close then probes every pin too, whatever items it started.
#[test]
fn after_a_turn_that_did_not_announce_every_change_the_close_probes_every_pin() {
    let registry = registry();
    let (job, controls, _) = items(&registry, 1, &["d0", "d1", "d2", "d3"]);
    assert!(job.grant());
    job.claim().unstarted().set(3);
    let mut turn = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (_, fresh) = pin(vec![entry(parent(2, &[2, 0]), b"stage", None, false)]);
    assert!(
        turn.reserve_private_child(Some(fresh), &Cancellation::default())
            .unwrap()
    );
    let before = calls(&controls);
    turn.finish().unwrap();
    assert_eq!(
        probed_since(&controls, &before),
        (0..8).collect::<Vec<_>>(),
        "the turn's finish refreshes every pin"
    );
    let before = calls(&controls);
    job.claim().revalidate_before_close();
    assert_eq!(probed_since(&controls, &before), (0..8).collect::<Vec<_>>());
    let closes = registry.closes_for_test();
    assert_eq!(closes.len(), 1);
    assert_eq!(
        (closes[0].aliases, closes[0].refreshed),
        (1, 9),
        "the alias the turn created is probed too"
    );
}

#[test]
fn intersecting_announcement_is_not_coverage_of_a_larger_alias() {
    let registry = registry();
    let (_, native) = pin(vec![entry(
        parent(1, &[1, 0]),
        b"selected",
        Some(10),
        false,
    )]);
    let mut job = Job::new(&registry, 1, vec![(Write, Some(native.clone()))]);
    assert!(job.grant());
    let mut transaction = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(native.clone()), &Cancellation::default())
            .unwrap()
    );
    let (_, larger) = pin(vec![entry(parent(0, &[0]), b"parent", Some(1), true)]);
    assert!(
        transaction
            .reserve_private_child(Some(larger), &Cancellation::default())
            .unwrap()
    );
    transaction.finish().unwrap();
    assert_eq!(job.claim().alias_count(), 1);
    let peer = one(
        &registry,
        2,
        Write,
        entry(parent(1, &[1, 0]), b"other", None, false),
    );
    assert!(!peer.grant());
    drop(native);
    job.close();
}

#[test]
fn equal_geometry_fresh_pin_does_not_enable_incremental_finish() {
    let registry = registry();
    let _observer = observer(&registry);
    let shared = parent(1, &[1, 0]);
    let first_scope = entry(shared.clone(), b"first", None, false);
    let (_, first) = pin(vec![first_scope.clone()]);
    let (control, second) = pin(vec![entry(shared.clone(), b"second", None, true)]);
    let job = Job::new(
        &registry,
        1,
        vec![(Write, Some(first)), (Write, Some(second))],
    );
    assert!(job.grant());
    let before = control.calls.load(Ordering::SeqCst);
    let mut transaction = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (_, fresh) = pin(vec![first_scope]);
    assert!(
        transaction
            .reserve_private_child(Some(fresh), &Cancellation::default())
            .unwrap()
    );
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
        vec![entry(shared, b"second", Some(50), true)].into(),
    ));
    transaction.finish().unwrap();
    assert!(control.calls.load(Ordering::SeqCst) > before);
    assert_eq!(
        job.claim().alias_count(),
        1,
        "a fresh absent pin must remain observed for future object materialization"
    );
}

#[test]
fn announced_root_becoming_unknown_retains_global_exclusion() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let (control, native) = pin(vec![entry(shared.clone(), b"root", None, true)]);
    let mut job = Job::new(&registry, 1, vec![(Write, Some(native.clone()))]);
    assert!(job.grant());
    let mut transaction = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        transaction
            .reserve_private_child(Some(native.clone()), &Cancellation::default())
            .unwrap()
    );
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Unknown);
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Opaque);
    let peer = one(
        &registry,
        2,
        Write,
        entry(shared, b"otherwise-disjoint", None, false),
    );
    assert!(!peer.grant());
    drop(native);
    job.close();
}
