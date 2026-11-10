//! Retained diagnostic authority and cached presentation for directory-size issues.
use crate::{Cancellation, FsError, GeneratedTextSource};
use std::{fmt, sync::Arc};

/// Backend-produced summary. Reading this never opens storage or locks a live scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizeWarningSummary {
    pub observed: u64,
    pub stored: u64,
    pub tooltip: Arc<str>,
}

/// An immutable complete-record report boundary, not a pathname to reopen.
/// Snapshot creation, reads and final destruction belong on backend workers.
pub trait SizeWarningSource: fmt::Debug + Send + Sync {
    fn snapshot(&self, cancel: &Cancellation) -> Result<Arc<dyn GeneratedTextSource>, FsError>;
}

#[derive(Clone, Debug)]
pub struct SizeWarningReport(Arc<ReportData>);

#[derive(Debug)]
struct ReportData {
    source: Arc<dyn SizeWarningSource>,
    summary: SizeWarningSummary,
}

impl SizeWarningReport {
    pub fn new(source: Arc<dyn SizeWarningSource>, summary: SizeWarningSummary) -> Self {
        Self(Arc::new(ReportData { source, summary }))
    }

    pub fn summary(&self) -> &SizeWarningSummary {
        &self.0.summary
    }

    /// Call only on a backend worker; a source may touch retained private storage.
    pub fn snapshot(&self, cancel: &Cancellation) -> Result<Arc<dyn GeneratedTextSource>, FsError> {
        self.0.source.snapshot(cancel)
    }
}

// Report equality is retained authority equality, never filesystem access or
// comparison of potentially large generated contents.
impl PartialEq for SizeWarningReport {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (Arc::ptr_eq(&self.0.source, &other.0.source) && self.0.summary == other.0.summary)
    }
}
impl Eq for SizeWarningReport {}
