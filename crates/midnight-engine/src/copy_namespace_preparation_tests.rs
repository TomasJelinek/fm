use super::*;
use std::sync::atomic::AtomicBool;

struct Pin {
    fail: Arc<AtomicBool>,
}
impl TransferNamespacePin for Pin {
    fn refresh(&self, _: &Cancellation) -> Result<TransferNamespaceProbe, FsError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(error(
                Path::new("captured-source"),
                io::ErrorKind::NotConnected,
                "metadata read failed",
            ))
        } else {
            Ok(TransferNamespaceProbe::Unknown)
        }
    }
}
struct ClaimOwner {
    claim: Option<crate::copy_namespace::Claim>,
    native: Option<crate::copy_namespace::NativeOwners>,
    io: Arc<midnight_work_budget::Permit>,
}
impl ClaimOwner {
    fn new(
        registry: &crate::copy_namespace::Registry,
        gate: &Arc<midnight_work_budget::Gate>,
        sequence: u64,
    ) -> Self {
        let (claim, native) = registry.new_job(sequence).unwrap();
        let io = Arc::new(
            gate.acquire(midnight_work_budget::Admission::Shared, || false)
                .unwrap(),
        );
        Self {
            claim: Some(claim),
            native: Some(native),
            io,
        }
    }
    fn poll(
        &self,
        plan: &mut NamespacePlan,
        pins: Vec<(
            TransferNamespaceAccess,
            Option<Arc<dyn TransferNamespacePin>>,
        )>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        plan.poll(self.claim.as_ref().unwrap(), pins, self.io.clone(), cancel)
    }
}
impl Drop for ClaimOwner {
    fn drop(&mut self) {
        // Runs while an assertion unwinds, too: a second panic would abort the binary.
        if let Some(claim) = self.claim.take() {
            if claim.begin_closing().is_err() {
                std::mem::forget(self.native.take());
                std::mem::forget(claim);
                return;
            }
            drop(self.native.take());
            if claim.complete_after_drop().is_err() {
                std::mem::forget(claim);
            }
        }
    }
}
fn registry() -> (
    Arc<midnight_work_budget::Gate>,
    crate::copy_namespace::Registry,
) {
    let gate = Arc::new(midnight_work_budget::Gate::new(2));
    let registry = crate::copy_namespace::Registry::new_with_io(gate.clone());
    (gate, registry)
}
fn roots(
    pin: Option<Arc<dyn TransferNamespacePin>>,
) -> Vec<(
    TransferNamespaceAccess,
    Option<Arc<dyn TransferNamespacePin>>,
)> {
    vec![
        (TransferNamespaceAccess::Read, pin),
        (TransferNamespaceAccess::Write, None),
    ]
}

#[test]
fn failed_first_union_refresh_retries_the_installed_union_without_reinstalling() {
    let (gate, registry) = registry();
    let owner = ClaimOwner::new(&registry, &gate, 1);
    let fail = Arc::new(AtomicBool::new(true));
    let pin: Arc<dyn TransferNamespacePin> = Arc::new(Pin { fail: fail.clone() });
    let mut plan = NamespacePlan::default();
    let error = owner
        .poll(
            &mut plan,
            roots(Some(pin.clone())),
            &Cancellation::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::NotConnected);
    assert!(
        plan.installed,
        "failed refresh already transferred the immutable union into its owner"
    );
    fail.store(false, Ordering::SeqCst);
    assert!(
        owner
            .poll(&mut plan, roots(Some(pin)), &Cancellation::default())
            .unwrap()
    );
}

#[test]
fn valid_unknown_geometry_waits_for_conflicting_authority_and_then_grants() {
    let (gate, registry) = registry();
    let earlier = ClaimOwner::new(&registry, &gate, 1);
    let later = ClaimOwner::new(&registry, &gate, 2);
    let mut first_plan = NamespacePlan::default();
    let mut later_plan = NamespacePlan::default();
    assert!(
        earlier
            .poll(&mut first_plan, roots(None), &Cancellation::default())
            .unwrap()
    );
    assert!(
        !later
            .poll(&mut later_plan, roots(None), &Cancellation::default())
            .unwrap()
    );
    assert!(later_plan.installed);
    drop(earlier);
    assert!(
        later
            .poll(&mut later_plan, roots(None), &Cancellation::default())
            .unwrap()
    );
}

#[test]
fn retry_revalidation_error_is_not_reinterpreted_as_opaque_grant() {
    let (gate, registry) = registry();
    let owner = ClaimOwner::new(&registry, &gate, 1);
    let fail = Arc::new(AtomicBool::new(false));
    let pin: Arc<dyn TransferNamespacePin> = Arc::new(Pin { fail: fail.clone() });
    let mut plan = NamespacePlan::default();
    assert!(
        owner
            .poll(
                &mut plan,
                roots(Some(pin.clone())),
                &Cancellation::default()
            )
            .unwrap()
    );
    fail.store(true, Ordering::SeqCst);
    assert_eq!(
        owner
            .poll(
                &mut plan,
                roots(Some(pin.clone())),
                &Cancellation::default()
            )
            .unwrap_err()
            .kind,
        io::ErrorKind::NotConnected
    );
    fail.store(false, Ordering::SeqCst);
    assert!(
        owner
            .poll(&mut plan, roots(Some(pin)), &Cancellation::default())
            .unwrap()
    );
}

#[test]
fn cancelled_namespace_wait_never_returns_write_permission() {
    let (gate, registry) = registry();
    let owner = ClaimOwner::new(&registry, &gate, 1);
    let mut plan = NamespacePlan::default();
    assert!(
        owner
            .poll(&mut plan, roots(None), &Cancellation::default())
            .unwrap()
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        owner
            .poll(&mut plan, roots(None), &cancel)
            .unwrap_err()
            .kind,
        io::ErrorKind::Interrupted
    );
}
