//! One shared policy store with final destruction on the existing cleanup worker.
use crate::retirement::{RetirementLease, WorkerOwned};
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
}

/// A non-Clone comparison session pin. Keep this in worker state, never Session/UI
/// output, capture or process tracking.
///
/// It needs no cleanup slot of its own, so any number of sessions may pin: the store
/// keeps a reference to every set a pin references (its current set, or a displaced
/// one in its history), and this pin keeps the guarded store alive. Dropping a pin on
/// any thread therefore never runs a compiled policy's destructor; the store's next
/// publication (on the settings writer) or its own retirement (on the cleanup worker)
/// does.
pub struct ComparisonPolicy {
    // Field order: the policy reference goes before the store it relies on.
    policy: AcceptedFileComparatorPolicy,
    _store: ComparatorPolicies,
    _session: Arc<dyn ExternalCompareSessionCredit>,
}

/// Provider-free refusal before any comparison work or accepted policy escapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonPolicyPinError {
    /// The ID or exact raw-preferences Arc no longer matches the published set.
    PolicyChanged,
}

impl fmt::Display for ComparisonPolicyPinError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::PolicyChanged => "Comparator settings changed before this comparison opened.",
        })
    }
}
impl std::error::Error for ComparisonPolicyPinError {}

impl ComparisonPolicySource {
    pub(crate) fn new(policies: ComparatorPolicies) -> Self {
        Self { policies }
    }

    /// Short metadata-only snapshot; never compiles, captures or discovers content.
    pub fn snapshot(&self) -> FileComparatorSnapshot {
        self.policies.snapshot()
    }

    /// Pin the published set `expected` names for one comparison session. Sessions are
    /// not counted; `session` is that session's credit, retained with the pin and
    /// containing no policy or service ownership. A refusal acquired nothing, allowing
    /// a single immediate stale-snapshot retry.
    pub fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<ComparisonPolicy, ComparisonPolicyPinError> {
        let policy = self
            .policies
            .pin(expected)
            .map_err(|_| ComparisonPolicyPinError::PolicyChanged)?;
        Ok(ComparisonPolicy {
            policy,
            _store: self.policies.clone(),
            _session: session,
        })
    }
}

impl ComparisonPolicy {
    fn accepted(&self) -> &AcceptedFileComparatorPolicy {
        &self.policy
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
