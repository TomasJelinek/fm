//! Accepted settings acknowledgements carry raw metadata, never a compiled-policy owner.
use super::{FileComparatorDiagnostic, FileComparatorSnapshot};
use crate::{FsError, UserPreferences};
use std::fmt;

#[derive(Clone, Debug, Default)]
pub struct AcceptedPreferences {
    pub preferences: UserPreferences,
    pub comparators: FileComparatorSnapshot,
}

#[derive(Clone, Debug)]
pub struct PreferencesApplyError {
    pub error: FsError,
    /// Indexed against this transaction's submitted rule order, not a newer UI draft.
    pub comparator: Option<FileComparatorDiagnostic>,
}

impl From<FsError> for PreferencesApplyError {
    fn from(error: FsError) -> Self {
        Self {
            error,
            comparator: None,
        }
    }
}

impl From<FileComparatorDiagnostic> for PreferencesApplyError {
    fn from(diagnostic: FileComparatorDiagnostic) -> Self {
        Self {
            error: diagnostic.clone().into(),
            comparator: Some(diagnostic),
        }
    }
}

impl fmt::Display for PreferencesApplyError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, output)
    }
}

impl std::error::Error for PreferencesApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
