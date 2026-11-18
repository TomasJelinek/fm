//! One shared policy store with final destruction on the existing cleanup worker.
use crate::retirement::{RetirementDomain, RetirementLease, WorkerOwned};
use midnight_core::{
    AcceptedFileComparatorPolicy, ExternalCompareSessionCredit, FileComparatorPolicyStore,
    FileComparatorPreferences, FileComparatorSnapshot,
};
use std::{fmt, ops::Deref, path::PathBuf, sync::Arc};

#[cfg(test)]
#[path = "comparator_policy_owner_tests.rs"]
mod tests;

/// Guarded policy source suitable for dormant UI composition owners.
/// Clones keep final store destruction on the existing retirement worker, even
/// after Engine and the async runtime have stopped. No filesystem service is held.
#[derive(Clone)]
pub struct ComparisonPolicySource {
    policies: ComparatorPolicies,
    retirement: RetirementDomain,
}

/// A non-Clone comparison session pin with runtime-independent final retirement.
/// Keep this in worker state, never Session/UI output, capture or process tracking.
pub struct ComparisonPolicy {
    payload: WorkerOwned<AcceptedPolicyPayload>,
}

// Field order keeps the SAME session credit occupied through actual policy Drop.
struct AcceptedPolicyPayload {
    policy: Option<AcceptedFileComparatorPolicy>,
    _session: Arc<dyn ExternalCompareSessionCredit>,
}

/// Provider-free refusal before any comparison work or accepted policy escapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonPolicyPinError {
    /// The ID or exact raw-preferences Arc no longer matches the published set.
    PolicyChanged,
    /// No actual cleanup slot was available; this is not a stale-policy retry.
    RetirementBusy,
}

impl fmt::Display for ComparisonPolicyPinError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::PolicyChanged => "Comparator settings changed before this comparison opened.",
            Self::RetirementBusy => "Comparison cleanup capacity is occupied.",
        })
    }
}
impl std::error::Error for ComparisonPolicyPinError {}

impl ComparisonPolicySource {
    pub(crate) fn new(policies: ComparatorPolicies, retirement: RetirementDomain) -> Self {
        Self {
            policies,
            retirement,
        }
    }

    /// Short metadata-only snapshot; never compiles, captures or discovers content.
    pub fn snapshot(&self) -> FileComparatorSnapshot {
        self.policies.snapshot()
    }

    /// Call only after acquiring the existing two-session admission credit.
    /// Pass a clone of that SAME credit. Its final reference is retained until
    /// actual policy destruction; it must contain no policy or service ownership.
    /// Definite refusals return unused ownership synchronously, allowing a single
    /// immediate stale-snapshot retry without waiting for the cleanup worker.
    pub fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<ComparisonPolicy, ComparisonPolicyPinError> {
        let lease = self
            .retirement
            .try_reserve()
            .map_err(|_| ComparisonPolicyPinError::RetirementBusy)?;
        let mut payload = lease.own(AcceptedPolicyPayload {
            policy: None,
            _session: session,
        });
        match self.policies.pin(expected) {
            Ok(policy) => {
                // No allocation, callback or cancellation boundary after acceptance.
                payload.policy = Some(policy);
                Ok(ComparisonPolicy { payload })
            }
            Err(_) => {
                // Certify emptiness WHILE armed. Only this metadata-only branch
                // may use into_parts on the caller: no compiled owner was acquired.
                // An unexpected unwind above still retires the armed payload.
                assert!(payload.policy.is_none(), "refused pin must remain empty");
                let (empty, lease) = payload.into_parts();
                drop(empty);
                drop(lease);
                Err(ComparisonPolicyPinError::PolicyChanged)
            }
        }
    }
}

impl ComparisonPolicy {
    fn accepted(&self) -> &AcceptedFileComparatorPolicy {
        self.payload
            .policy
            .as_ref()
            .expect("successful pin owns its policy")
    }

    /// Raw metadata for this exact accepted generation, without a compiled owner.
    pub fn snapshot(&self) -> FileComparatorSnapshot {
        self.accepted().snapshot()
    }

    /// The exact raw rules corresponding to this pin's immutable matcher.
    pub fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        self.accepted().preferences()
    }

    /// Match both original paths on the comparison worker, including manual retries.
    pub fn select(&self, original_paths: &[PathBuf; 2]) -> Option<usize> {
        self.accepted().select(original_paths)
    }
}

/// All Engine/preferences owners clone this wrapper, never its inner store Arc.
/// Its single startup reservation survives asynchronously outliving settings work.
#[derive(Clone)]
pub(crate) struct ComparatorPolicies(Arc<WorkerOwned<Arc<FileComparatorPolicyStore>>>);

impl ComparatorPolicies {
    pub(crate) fn new(lease: RetirementLease, store: Arc<FileComparatorPolicyStore>) -> Self {
        Self(Arc::new(lease.own(store)))
    }

    /// Backend composition only. External owners must honor Core's final-drop
    /// contract; internal Engine/preferences workers must clone Self instead.
    pub(crate) fn for_composition(&self) -> Arc<FileComparatorPolicyStore> {
        Arc::clone(&**self.0)
    }

    #[cfg(test)]
    pub(crate) fn testing(store: Arc<FileComparatorPolicyStore>) -> Self {
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        Self::new(domain.try_reserve().unwrap(), store)
    }
}

impl Deref for ComparatorPolicies {
    type Target = FileComparatorPolicyStore;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
