//! A deliberately invalid, metadata-only submission exercises the closed handoff's Err unwind.
use super::Submission;
use crate::copy_operations::ownership::{self, Resources, Scope};
use midnight_core::{Command, OperationId};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use tokio::sync::Semaphore;

#[test]
fn unexpected_non_copy_conversion_retains_original_finished_and_quarantines_capacity() {
    let capacity = Arc::new(Semaphore::new(1));
    let mutation = Arc::new(Semaphore::new(1));
    let permit = capacity.clone().try_acquire_owned().unwrap();
    let mutation_permit = mutation.clone().try_acquire_owned().unwrap();
    let fence = crate::copy_fence::CopyFence::default();
    let mut registry = crate::copy_finished::Registry::default();
    let (finished, predecessors) = registry.register().unwrap();
    let observed = finished.observer();
    let (_successor_owner, successor) = registry.register().unwrap();
    let mut resources = Resources::admission(fence.register(), finished, permit, predecessors);
    resources.mutation = Some(mutation_permit);
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    // Private descendant access constructs the impossible transport input without any new API.
    // ListVolumes is pure metadata: no file, provider, native handle or arbitrary destructor.
    let submission = Submission {
        command: Some(Command::ListVolumes),
        id: OperationId(1),
    };
    let owned = domain
        .try_reserve()
        .unwrap()
        .own(Scope::new(submission, resources));
    assert!(!observed.is_finished() && !successor.is_finished());
    assert_eq!(capacity.available_permits(), 0);
    assert_eq!(mutation.available_permits(), 0);

    let panic = catch_unwind(AssertUnwindSafe(|| {
        drop(ownership::convert_submission(owned));
    }))
    .expect_err("a non-Copy command must trip the guarded Copy invariant");
    assert_eq!(
        panic.downcast_ref::<&str>().copied(),
        Some("guarded Copy contains a Copy command")
    );
    // Observe original publication/capacity, not a copied private quarantine/SUCCESS flag.
    assert!(
        !observed.is_finished(),
        "unwind must not commit the original FINISHED owner"
    );
    assert!(
        !successor.is_finished(),
        "its actual successor must keep waiting"
    );
    assert_eq!(
        capacity.available_permits(),
        0,
        "unknown teardown retains admission"
    );
    assert_eq!(
        mutation.available_permits(),
        0,
        "unknown teardown retains mutation authority"
    );
}
