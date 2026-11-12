//! Frozen tree failures; diagnostic text never grants provider/path authority.
use crate::size_warnings::ReportLog;
use midnight_core::{Cancellation, GeneratedTextSource, TreeDiagnostic};
use std::{fmt, io, sync::Arc};

pub(crate) fn log(arena: &crate::ReportArena) -> ReportLog {
    ReportLog::with_title(
        arena,
        std::env::temp_dir().join("MidnightExplorer-size-warnings"),
        "Folder listing error",
    )
}

/// One log/file per tree generation. Only complete immutable record ranges are
/// published, so later failures/retries cannot change an opened description.
pub(crate) fn report(
    log: &ReportLog,
    context: impl fmt::Display,
    message: &str,
) -> Arc<dyn GeneratedTextSource> {
    let range = log.mark().report();
    log.record(context, "Tree operation", io::ErrorKind::Other, message);
    range.finish(false);
    match range.snapshot(&Cancellation::default()) {
        Ok(source) => source,
        Err(error) => Arc::new(Unavailable(format!(
            "Full diagnostic report unavailable.\n{}\nThe bounded panel error remains available.\n",
            TreeDiagnostic::new(&error.to_string()).as_str(),
        ))),
    }
}

/// Terminal errors have bounded admission/retention through their existing
/// command/event owner. No live tree is required for construction failures.
pub(crate) fn diagnostic(
    arena: &crate::ReportArena,
    error: &(impl fmt::Display + ?Sized),
) -> TreeDiagnostic {
    let message = error.to_string();
    let source = report(&log(arena), "Captured tree operation", &message);
    TreeDiagnostic::with_details(&message, source)
}

/// A refresh visits at most the retained tree's bounded node set. Keep only
/// those frozen source owners, not copied path/reason strings or new spool files.
pub(crate) fn composed(sources: Vec<Arc<dyn GeneratedTextSource>>) -> Arc<dyn GeneratedTextSource> {
    let mut sections = Vec::with_capacity(sources.len());
    let mut length = 0_u64;
    for source in sources {
        let Some(next) = length.checked_add(source.length()) else {
            return Arc::new(Unavailable(
                "Full refresh report unavailable: combined report length overflowed.\n".into(),
            ));
        };
        if next != length {
            sections.push((length, source));
        }
        length = next;
    }
    Arc::new(Composed { sections, length })
}

#[derive(Debug)]
struct Composed {
    sections: Vec<(u64, Arc<dyn GeneratedTextSource>)>,
    length: u64,
}
impl GeneratedTextSource for Composed {
    fn length(&self) -> u64 {
        self.length
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        if offset >= self.length || output.is_empty() {
            return Ok(0);
        }
        let index = self.sections.partition_point(|(start, _)| *start <= offset) - 1;
        let (start, source) = &self.sections[index];
        source.read_at(offset - start, output)
    }
}

#[derive(Debug)]
struct Unavailable(String);
impl GeneratedTextSource for Unavailable {
    fn length(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        let offset = offset.min(self.length()) as usize;
        let bytes = &self.0.as_bytes()[offset..];
        let count = bytes.len().min(output.len());
        output[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(source: &dyn GeneratedTextSource) -> String {
        let mut bytes = vec![0; source.length().try_into().unwrap()];
        let mut offset = 0;
        while offset < bytes.len() {
            let end = (offset + 16 * 1024).min(bytes.len());
            let count = source
                .read_at(offset as u64, &mut bytes[offset..end])
                .unwrap();
            assert!(count > 0);
            offset += count;
        }
        assert_eq!(source.read_at(source.length(), &mut [0; 1]).unwrap(), 0);
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn terminal_diagnostic_preserves_long_unicode_tail_and_snapshot_identity() {
        let reason = format!("{} unique terminal tail", "東京 failure ".repeat(20_000));
        let diagnostic = diagnostic(&crate::ReportArena::testing(), &reason);
        assert!(diagnostic.as_str().len() <= TreeDiagnostic::MAX_BYTES);
        assert!(!diagnostic.as_str().contains("unique terminal tail"));
        let source = diagnostic.details().unwrap().clone();
        assert_eq!(diagnostic, diagnostic.clone());
        drop(diagnostic);
        assert!(text(source.as_ref()).contains(&reason));
    }

    #[test]
    fn unavailable_storage_retains_specific_summary_and_explicit_missing_detail_count() {
        let temporary = tempfile::tempdir().unwrap();
        let blocked = temporary.path().join("not-a-directory");
        std::fs::write(&blocked, b"occupied").unwrap();
        let log = ReportLog::with_title(
            &crate::ReportArena::testing(),
            blocked.join("report"),
            "Folder listing error",
        );
        let source = report(&log, "東京 folder", "Permission denied by provider");
        let diagnostic = TreeDiagnostic::with_details("Permission denied by provider", source);
        assert_eq!(diagnostic.as_str(), "Permission denied by provider");
        let text = text(diagnostic.details().unwrap().as_ref());
        assert!(text.contains("Observed issues: 1"));
        assert!(text.contains("Stored details: 0"));
        assert!(text.contains("Report storage failed"));
        assert!(text.contains("1 issue details could not be stored"));
        assert!(!text.contains("Counting continued"));
    }
}
