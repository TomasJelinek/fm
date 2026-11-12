use crate::size_warnings::{ReportLog, ReportRange};
use midnight_core::{Cancellation, FolderSize, FsError};
use std::{io, path::Path, sync::Arc};

pub(super) struct ScanReport<'a> {
    log: &'a ReportLog,
    range: Arc<ReportRange>,
}
impl<'a> ScanReport<'a> {
    pub(super) fn new(log: &'a ReportLog) -> Self {
        Self {
            log,
            range: log.mark().report(),
        }
    }
    pub(super) fn issue(
        &self,
        size: &mut FolderSize,
        path: &Path,
        operation: &str,
        kind: io::ErrorKind,
        reason: &str,
    ) {
        size.errors = size.errors.saturating_add(1);
        // Debug formatting preserves otherwise non-Unicode native path units.
        self.log
            .record(format_args!("{path:?}"), operation, kind, reason);
    }
    pub(super) fn provider_error(
        &self,
        size: &mut FolderSize,
        error: &FsError,
        cancel: &Cancellation,
    ) {
        if error.kind != io::ErrorKind::Interrupted || !cancel.is_cancelled() {
            self.issue(
                size,
                &error.path,
                error.operation,
                error.kind,
                &error.message,
            );
        }
    }
    pub(super) fn publish(&self, size: &mut FolderSize) {
        size.warnings = self.range.handle();
    }
    pub(super) fn finish(&self, size: &mut FolderSize, cancelled: bool) {
        self.range.finish(cancelled);
        size.finished = !cancelled;
        self.publish(size);
    }
}
impl Drop for ScanReport<'_> {
    fn drop(&mut self) {
        self.range.finish(true);
    }
}
