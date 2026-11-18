//! One current compiled policy, borrowed save proofs, and explicit session pins.
use super::{FileComparatorPreferences, MAX_FILE_COMPARATOR_MATCHER_BYTES};
use crate::FsError;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileComparatorPolicyId(u64);

/// UI/events/queued launches own raw metadata only, never the compiled policy.
#[derive(Clone, Debug)]
pub struct FileComparatorSnapshot {
    pub id: FileComparatorPolicyId,
    pub preferences: Arc<FileComparatorPreferences>,
}

fn default_preferences() -> Arc<FileComparatorPreferences> {
    static DEFAULT: OnceLock<Arc<FileComparatorPreferences>> = OnceLock::new();
    DEFAULT.get_or_init(Arc::default).clone()
}

impl Default for FileComparatorSnapshot {
    fn default() -> Self {
        Self {
            id: FileComparatorPolicyId::default(),
            preferences: default_preferences(),
        }
    }
}

/// Implemented by the backend compiler, never by presentation or a plugin API.
/// The immutable matcher must correspond exactly to preferences(), including disabled rules.
pub trait CompiledFileComparatorPolicy: Send + Sync + 'static {
    fn preferences(&self) -> &Arc<FileComparatorPreferences>;
    /// First enabled pattern matching BOTH original lossless Unicode basenames, or None.
    fn select(&self, original_paths: &[PathBuf; 2]) -> Option<usize>;
    /// Aggregate configured program plus DFA ceilings, not process memory usage.
    fn matcher_budget_bytes(&self) -> usize;
}

struct PolicySet {
    raw: Arc<FileComparatorPreferences>,
    compiled: Arc<dyn CompiledFileComparatorPolicy>,
}

struct Published {
    id: FileComparatorPolicyId,
    set: Arc<PolicySet>,
}

/// Backend-worker candidate. Its constructor accepts an already validated compiler result.
/// Only the serialized settings worker may hold a replacement; drop it there on save failure.
pub struct PreparedFileComparatorPolicy {
    id: Option<FileComparatorPolicyId>,
    set: Arc<PolicySet>,
}

impl PreparedFileComparatorPolicy {
    /// `compiled` must have passed raw and semantic validation with <=16 MiB matcher ceilings.
    /// This is a host-backend contract; Core performs no pattern compilation.
    pub fn new(compiled: Arc<dyn CompiledFileComparatorPolicy>) -> Self {
        Self {
            id: None,
            set: Arc::new(PolicySet {
                raw: compiled.preferences().clone(),
                compiled,
            }),
        }
    }

    /// Attach an ID reserved by the sole settings writer BEFORE persistence.
    pub fn with_id(mut self, id: FileComparatorPolicyId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn validation_proof(&self) -> ComparatorValidationProof<'_> {
        ComparatorValidationProof { set: &self.set }
    }
}

/// A non-Clone session pin, or the serialized settings writer's temporary reuse handle.
/// Never store this in UI/events/launch queues/process tracking or cleanup quarantine.
pub struct AcceptedFileComparatorPolicy {
    id: FileComparatorPolicyId,
    set: Arc<PolicySet>,
}

impl AcceptedFileComparatorPolicy {
    pub fn snapshot(&self) -> FileComparatorSnapshot {
        FileComparatorSnapshot {
            id: self.id,
            preferences: self.set.raw.clone(),
        }
    }

    pub fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.set.raw
    }

    pub fn select(&self, original_paths: &[PathBuf; 2]) -> Option<usize> {
        if original_paths
            .iter()
            .any(|path| path.file_name().and_then(|name| name.to_str()).is_none())
        {
            return None;
        }
        self.set.compiled.select(original_paths).filter(|index| {
            self.set
                .raw
                .rules
                .get(*index)
                .is_some_and(|rule| rule.enabled)
        })
    }

    pub fn validation_proof(&self) -> ComparatorValidationProof<'_> {
        ComparatorValidationProof { set: &self.set }
    }
}

impl fmt::Debug for AcceptedFileComparatorPolicy {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("AcceptedFileComparatorPolicy")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Borrowed evidence for synchronous prepared persistence; no owned compiled reference escapes.
pub struct ComparatorValidationProof<'a> {
    set: &'a PolicySet,
}

impl ComparatorValidationProof<'_> {
    /// Raw VALUE equality is intentional: UserPreferences owns values, not a snapshot Arc.
    pub fn matches(&self, preferences: &FileComparatorPreferences) -> bool {
        self.set.raw.as_ref() == preferences
            && self.set.compiled.matcher_budget_bytes() <= MAX_FILE_COMPARATOR_MATCHER_BYTES
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileComparatorPolicyChanged;

impl fmt::Display for FileComparatorPolicyChanged {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("Comparator settings changed before this comparison opened. Try again.")
    }
}

impl std::error::Error for FileComparatorPolicyChanged {}

struct State {
    current: Published,
    last_reserved: u64,
}

/// One current set, no history map. The sole publisher is the serialized preferences worker.
/// Session callers must acquire the existing two-session credit before pinning.
/// Final store/candidate destruction belongs to a backend worker.
pub struct FileComparatorPolicyStore {
    state: Mutex<State>,
}

impl Default for FileComparatorPolicyStore {
    fn default() -> Self {
        let raw = default_preferences();
        Self {
            state: Mutex::new(State {
                current: Published {
                    id: FileComparatorPolicyId::default(),
                    set: Arc::new(PolicySet {
                        raw: raw.clone(),
                        compiled: Arc::new(AutomaticPolicy(raw)),
                    }),
                },
                last_reserved: 0,
            }),
        }
    }
}

impl FileComparatorPolicyStore {
    pub fn snapshot(&self) -> FileComparatorSnapshot {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        FileComparatorSnapshot {
            id: state.current.id,
            preferences: state.current.set.raw.clone(),
        }
    }

    /// ID AND raw Arc identity are compared atomically with publication.
    pub fn pin(
        &self,
        expected: &FileComparatorSnapshot,
    ) -> Result<AcceptedFileComparatorPolicy, FileComparatorPolicyChanged> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.current.id != expected.id
            || !Arc::ptr_eq(&state.current.set.raw, &expected.preferences)
        {
            return Err(FileComparatorPolicyChanged);
        }
        Ok(AcceptedFileComparatorPolicy {
            id: state.current.id,
            set: state.current.set.clone(),
        })
    }

    /// Sole-writer helper: unchanged saves reuse exact current raw Arc, ID and validation proof.
    /// The worker drops this temporary handle before starting its next replacement operation.
    pub fn reuse_for_save(
        &self,
        preferences: &FileComparatorPreferences,
    ) -> Option<AcceptedFileComparatorPolicy> {
        let current = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            AcceptedFileComparatorPolicy {
                id: state.current.id,
                set: state.current.set.clone(),
            }
        };
        (current.preferences().as_ref() == preferences).then_some(current)
    }

    /// Reserve monotonically before writing. Abandoned candidates leave harmless ID gaps.
    pub fn prepare_id(&self) -> Result<FileComparatorPolicyId, FsError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let next = state.last_reserved.checked_add(1).ok_or_else(|| {
            FsError::from_io(
                "prepare comparator policy",
                Path::new(""),
                io::Error::other("Comparator policy identifiers are exhausted."),
            )
        })?;
        state.last_reserved = next;
        Ok(FileComparatorPolicyId(next))
    }

    /// Sole-writer commit after successful persistence, with no fallible allocation or I/O.
    /// The candidate must carry this writer's reserved ID via with_id before saving.
    /// Panics only on a host programming error: publishing an unreserved/stale candidate.
    pub fn publish(&self, candidate: PreparedFileComparatorPolicy) -> FileComparatorSnapshot {
        let id = candidate
            .id
            .expect("comparator candidate needs a reserved ID before persistence");
        let snapshot = FileComparatorSnapshot {
            id,
            preferences: candidate.set.raw.clone(),
        };
        let displaced = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            assert!(
                id.0 > state.current.id.0 && id.0 <= state.last_reserved,
                "comparator publication requires this writer's newer reserved ID"
            );
            std::mem::replace(
                &mut state.current,
                Published {
                    id,
                    set: candidate.set,
                },
            )
        };
        // The expensive/foreign compiler destructor may reenter snapshot(); never hold its lock.
        drop(displaced);
        snapshot
    }
}

struct AutomaticPolicy(Arc<FileComparatorPreferences>);

impl CompiledFileComparatorPolicy for AutomaticPolicy {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.0
    }
    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        None
    }
    fn matcher_budget_bytes(&self) -> usize {
        0
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
