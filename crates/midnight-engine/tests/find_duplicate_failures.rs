#[path = "../src/find_worker.rs"]
mod find_worker;

use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs, io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

#[derive(Clone, Copy)]
enum Failure {
    Read,
    ChangedCapture,
    CancelRead,
    ShortRead,
    UnknownModified,
    RepresentativeChanged,
    UnequalCompareStream,
}

struct Service {
    native: LocalFileSystem,
    failure: Failure,
    read_bytes: Arc<AtomicU64>,
    file_opens: AtomicU64,
}

impl FindService for Service {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        let inner = self.native.open_find_source(path, cancel)?;
        let mut entry = inner.entry().clone();
        let opened = if entry.kind == EntryKind::File {
            self.file_opens.fetch_add(1, Ordering::Relaxed) + 1
        } else {
            0
        };
        if entry.kind == EntryKind::File
            && (matches!(self.failure, Failure::ChangedCapture)
                || matches!(self.failure, Failure::RepresentativeChanged) && opened == 6)
        {
            entry.revision.0[0] = entry.revision.0[0].wrapping_add(1);
        }
        Ok(Box::new(Source {
            inner,
            entry,
            failure: self.failure,
            read_bytes: self.read_bytes.clone(),
            unequal: matches!(self.failure, Failure::UnequalCompareStream) && opened == 5,
        }))
    }
}
struct Source {
    inner: Box<dyn FindSource>,
    entry: FindEntry,
    failure: Failure,
    read_bytes: Arc<AtomicU64>,
    unequal: bool,
}
impl FindSource for Source {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError> {
        Ok(self.inner.next_child(cancel)?.map(|child| match child {
            FindChild::Entry(inner) => {
                let mut entry = inner.entry().clone();
                if matches!(self.failure, Failure::UnknownModified) {
                    entry.metadata.modified = None;
                }
                FindChild::Entry(Box::new(Source {
                    entry,
                    inner,
                    failure: self.failure,
                    read_bytes: self.read_bytes.clone(),
                    unequal: false,
                }))
            }
            other => other,
        }))
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        assert!(
            buffer.len() <= 64 * 1024,
            "duplicate reader exceeded its fixed buffer"
        );
        if matches!(self.failure, Failure::Read)
            && self.entry.path.file_name().unwrap() == "broken.bin"
        {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::PermissionDenied,
                "Injected read denial",
            ));
        }
        let length = if matches!(self.failure, Failure::ShortRead) {
            buffer.len().min(7)
        } else {
            buffer.len()
        };
        let read = self.inner.read_chunk(&mut buffer[..length], cancel)?;
        if self.unequal && read != 0 {
            buffer[0] ^= 1;
        }
        self.read_bytes.fetch_add(read as u64, Ordering::Relaxed);
        if matches!(self.failure, Failure::CancelRead) {
            cancel.cancel();
        }
        Ok(read)
    }
}

fn run(failure: Failure) -> (FindSummary, Vec<Arc<FindResult>>, u64) {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/operations-find/failures");
    fs::create_dir_all(&root).unwrap();
    let fixture = tempfile::tempdir_in(fs::canonicalize(root).unwrap()).unwrap();
    for name in ["a.bin", "b.bin", "broken.bin"] {
        fs::write(fixture.path().join(name), vec![0xff; 128 * 1024 + 3]).unwrap();
    }
    let reads = Arc::new(AtomicU64::new(0));
    let service = Service {
        native: LocalFileSystem::new(None),
        failure,
        read_bytes: reads.clone(),
        file_opens: AtomicU64::new(0),
    };
    let mut query = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![fixture.path().to_owned()],
    );
    query.duplicates = true;
    if matches!(failure, Failure::UnknownModified) {
        query.modified_within = Some(std::time::Duration::from_secs(60));
    }
    let mut rows = Vec::new();
    let location = find_worker::CurrentDirectory::default();
    let summary = find_worker::search_with_location(
        &service,
        &query,
        &Cancellation::default(),
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                rows.extend(batch.iter().cloned());
            }
            true
        },
        &location,
        SystemTime::now(),
    );
    assert!(location.get().is_some());
    for row in &rows {
        find_worker::validate_result(&service.native, &row.entry, &Cancellation::default())
            .unwrap();
    }
    (summary, rows, reads.load(Ordering::Relaxed))
}

#[test]
fn simulated_read_denial_reports_incomplete_and_preserves_other_duplicate_members() {
    let (summary, rows, _) = run(Failure::Read);
    assert_eq!(
        summary.completion,
        FindCompletion::Incomplete,
        "{summary:?}"
    );
    assert_eq!(summary.progress.errors, 1);
    assert_eq!(rows.len(), 2);
    assert!(
        summary
            .diagnostics
            .iter()
            .any(|error| error.kind == io::ErrorKind::PermissionDenied)
    );
}

#[test]
fn simulated_replaced_capture_cannot_become_a_duplicate_result() {
    let (summary, rows, read) = run(Failure::ChangedCapture);
    assert_eq!(
        summary.completion,
        FindCompletion::Incomplete,
        "{summary:?}"
    );
    assert_eq!(summary.progress.errors, 3);
    assert!(rows.is_empty());
    assert_eq!(
        read, 0,
        "Identity/revision must be checked before content reads"
    );
}

#[test]
fn simulated_short_native_reads_do_not_truncate_hash_or_byte_comparison() {
    let (summary, rows, _) = run(Failure::ShortRead);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(rows.len(), 3);
    assert_eq!(summary.progress.duplicate_bytes, (128 * 1024 + 3) * 7);
}

#[test]
fn cancellation_after_one_hash_read_does_not_publish_a_partial_group() {
    let (summary, rows, read) = run(Failure::CancelRead);
    assert_eq!(summary.completion, FindCompletion::Cancelled, "{summary:?}");
    assert!(rows.is_empty());
    assert!(read <= 64 * 1024);
}

#[test]
fn unknown_modified_time_is_excluded_from_a_relative_age_search_without_reading_content() {
    let (summary, rows, read) = run(Failure::UnknownModified);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert!(rows.is_empty());
    assert_eq!(read, 0);
}

#[test]
fn representative_change_on_a_later_comparison_discards_earlier_pairs() {
    let (summary, rows, read) = run(Failure::RepresentativeChanged);
    assert_eq!(
        summary.completion,
        FindCompletion::Incomplete,
        "{summary:?}"
    );
    assert_eq!(summary.progress.errors, 1);
    assert!(
        rows.is_empty(),
        "A changed representative cannot leave a previously accepted pair"
    );
    assert_eq!(read, (128 * 1024 + 3) * 5);
}

#[test]
fn simulated_equal_hashes_with_unequal_compare_streams_do_not_form_one_group() {
    let (summary, rows, _) = run(Failure::UnequalCompareStream);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(
        rows.len(),
        2,
        "An equal SHA-256 grouping must still require exact byte equality"
    );
    assert!(rows.iter().all(|row| row.duplicate_group_size == 2));
}
