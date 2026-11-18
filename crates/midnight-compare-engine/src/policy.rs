//! Comparison-specific policy ownership. Engine supplies production implementations;
//! no Core API, raw policy-store builder or generic retirement transport is added.
use midnight_core::{
    ExternalCompareSessionCredit, FileComparatorPreferences, FileComparatorSnapshot,
};
use std::{fmt, path::PathBuf, sync::Arc};

/// Refusal before an owned comparison-policy pin is returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileComparatorPinError {
    /// The expected policy ID or exact raw-preferences Arc is no longer current.
    /// Only this error permits the caller's one metadata refresh and retry.
    PolicyChanged,
    /// No actual backend retirement capacity is currently available for a pin.
    /// Map to `OpenError::Unavailable`, never to a stale-policy refresh.
    RetirementBusy,
}

impl fmt::Display for FileComparatorPinError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::PolicyChanged => {
                "Comparator settings changed before this comparison opened. Try again."
            }
            Self::RetirementBusy => "Comparison settings are temporarily unavailable.",
        })
    }
}

impl std::error::Error for FileComparatorPinError {}

/// A backend-owned source of metadata snapshots and guarded historical pins.
///
/// `Service::with_comparator_policies(self, Arc<dyn FileComparatorPolicySource>)`
/// retains this source rather than an Arc to the raw Core policy store. Production
/// sources must guard final store destruction on any calling thread and may outlive
/// Engine/runtime/UI without retaining EngineServices, TaskLifetime or a session.
pub trait FileComparatorPolicySource: Send + Sync + 'static {
    /// Short metadata-only access. Never compile, capture, discover or perform I/O.
    fn snapshot(&self) -> FileComparatorSnapshot;

    /// Atomically accept BOTH `expected.id` and the exact preferences Arc identity.
    /// Value-equal reconstructed preferences are not an accepted launch snapshot.
    ///
    /// The caller first admits the existing maximum-two session counter and
    /// completes fallible local setup. `session` is that SAME counter-only credit;
    /// neither policy nor paths/captures/process state may be stored inside it.
    ///
    /// Before accepting a policy, implementations reserve actual backend cleanup
    /// capacity and arm every owner required for non-fallible successful handoff.
    /// The returned non-Clone owner must remain safe to drop on ANY calling thread,
    /// including UI, an uninvoked blocking closure and after runtime shutdown.
    /// Backend final policy destruction precedes release of its retained credit.
    ///
    /// Both errors acquired no accepted policy. Release the unused credit and
    /// certified-empty reservation synchronously, without queuing no-op cleanup.
    /// A stale second open must not prevent one immediate refreshed retry while
    /// retirement work is blocked. Never synchronously destroy an accepted policy
    /// on an error path; that guarantee belongs to the already armed Engine owner.
    fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<Box<dyn OwnedFileComparatorPolicy>, FileComparatorPinError>;
}

/// One non-Clone historical policy retained exclusively by a comparison worker.
///
/// No accepted handle or compiled Arc can be extracted or cloned through this
/// interface. Snapshot/preferences expose raw metadata only. Do not place this
/// owner in Session, Mailbox, FileChoice, launch commands, captures or process
/// tracking. Manual A/B retries borrow the SAME owner even after settings change.
///
/// Implementations guarantee backend final retirement when dropped on ANY
/// calling thread. Only `select` is worker-only; an unstarted/cancelled closure
/// may drop this owner elsewhere. Engine implements that guard, not Service's
/// runtime-dependent `retire` helper or a compare-engine cleanup worker.
pub trait OwnedFileComparatorPolicy: Send + 'static {
    /// Metadata from this accepted historical policy, never the live source.
    fn snapshot(&self) -> FileComparatorSnapshot;

    /// The SAME accepted raw Arc used by snapshot and selected rule indices.
    fn preferences(&self) -> &Arc<FileComparatorPreferences>;

    /// Worker-only first enabled rule matching BOTH original native basenames.
    /// A selected explicit Automatic row is terminal; never continue to later
    /// rules. The result indexes this owner's preferences, not current settings.
    /// The matcher performs no capture, filesystem access or decoder discovery.
    fn select(&self, original_paths: &[PathBuf; 2]) -> Option<usize>;
}

/// Exact launch identity; raw metadata value equality is insufficient here.
pub(crate) fn same_snapshot(
    expected: &FileComparatorSnapshot,
    current: &FileComparatorSnapshot,
) -> bool {
    expected.id == current.id && Arc::ptr_eq(&expected.preferences, &current.preferences)
}

/// Compatibility for Service instances that have NO configured policy source.
///
/// Only the exact process-default ID AND shared raw Arc are policy-free Automatic.
/// Any other launch must return OpenError::Unavailable, never PolicyChanged and
/// never a refresh to a default that discards a configured rule. When a source IS
/// installed, pin through it even if its snapshot happens to be this default.
pub(crate) fn is_policy_free_default(expected: &FileComparatorSnapshot) -> bool {
    same_snapshot(expected, &FileComparatorSnapshot::default())
}
