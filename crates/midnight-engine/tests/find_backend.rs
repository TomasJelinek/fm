#[path = "../src/find_worker.rs"]
mod find_worker;

use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn execute(request: &FindRequest) -> (FindSummary, Vec<Arc<FindResult>>) {
    let mut results = Vec::new();
    let summary = find_worker::search(
        &LocalFileSystem::new(None),
        request,
        &Cancellation::default(),
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                results.extend(batch.iter().cloned());
            }
            true
        },
    );
    (summary, results)
}
fn request(roots: Vec<PathBuf>) -> FindRequest {
    FindRequest::new(FindSessionId(1), FindGeneration(1), roots)
}
fn names(results: &[Arc<FindResult>]) -> Vec<PathBuf> {
    let mut paths: Vec<_> = results
        .iter()
        .map(|result| result.entry.path.clone())
        .collect();
    paths.sort();
    paths
}

#[test]
fn multiple_native_roots_masks_recursion_and_stable_ids() {
    let temporary = tempfile::tempdir().unwrap();
    let one = temporary.path().join("one");
    let two = temporary.path().join("two");
    fs::create_dir_all(one.join("nested")).unwrap();
    fs::create_dir(&two).unwrap();
    fs::write(one.join("visible 文档.txt"), "one").unwrap();
    fs::write(one.join("nested/deep.txt"), "two").unwrap();
    fs::write(one.join("ignore.tmp"), "three").unwrap();
    fs::write(two.join("other.TXT"), "four").unwrap();
    let mut query = request(vec![one.clone(), two.clone()]);
    query.masks = "*.txt".into();
    let (summary, results) = execute(&query);
    assert_eq!(summary.completion, FindCompletion::Completed);
    assert_eq!(
        names(&results),
        vec![
            one.join("nested/deep.txt"),
            one.join("visible 文档.txt"),
            two.join("other.TXT")
        ]
    );
    for (index, result) in results.iter().enumerate() {
        assert_eq!(result.id, FindResultId(index as u64 + 1));
    }
    query.recursive = false;
    let (summary, results) = execute(&query);
    assert_eq!(summary.completion, FindCompletion::Completed);
    assert_eq!(results.len(), 2);
    query.name_case_sensitive = true;
    assert_eq!(execute(&query).1.len(), 1);
}

#[test]
fn strict_utf8_bom_unicode_folding_and_matches_crossing_chunks() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("large.txt");
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend(vec![b'x'; 65_531]);
    bytes.extend("Straße 文档\n".as_bytes());
    fs::write(&path, &bytes).unwrap();
    fs::write(temporary.path().join("malformed.txt"), [0xff, 0xfe, 0, 0]).unwrap();
    let mut query = request(vec![temporary.path().to_owned()]);
    query.masks = "*.txt".into();
    query.content = Some(FindContentQuery {
        text: "STRASSE 文档".into(),
        case_sensitive: false,
    });
    let (summary, results) = execute(&query);
    assert_eq!(names(&results), vec![path]);
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert_eq!(summary.progress.errors, 1);
    assert!(
        summary
            .diagnostics
            .iter()
            .any(|error| error.kind == std::io::ErrorKind::InvalidData)
    );
    query.content.as_mut().unwrap().case_sensitive = true;
    assert!(execute(&query).1.is_empty());
}

#[test]
fn result_node_depth_retention_and_content_limits_report_incomplete() {
    let temporary = tempfile::tempdir().unwrap();
    for index in 0..10 {
        fs::write(temporary.path().join(format!("{index}.txt")), "needle").unwrap();
    }
    let mut query = request(vec![temporary.path().to_owned()]);
    query.limits.max_results = 2;
    let (summary, results) = execute(&query);
    assert_eq!(results.len(), 2);
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert!(summary.reached_budgets.contains(&FindBudget::Results));
    query.limits = FindLimits::default();
    query.limits.max_nodes = 2;
    let (summary, _) = execute(&query);
    assert!(summary.progress.examined <= 2);
    assert!(summary.reached_budgets.contains(&FindBudget::Nodes));
    query.limits = FindLimits::default();
    query.limits.max_retained_bytes = 1;
    assert!(
        execute(&query)
            .0
            .reached_budgets
            .contains(&FindBudget::RetainedBytes)
    );
    query.limits = FindLimits::default();
    query.limits.max_content_bytes_per_file = 3;
    query.content = Some(FindContentQuery {
        text: "needle".into(),
        case_sensitive: true,
    });
    let (summary, results) = execute(&query);
    assert!(results.is_empty());
    assert_eq!(summary.progress.limited_files, 10);
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    query.limits = FindLimits::default();
    query.limits.max_content_bytes = 3;
    assert!(
        execute(&query)
            .0
            .reached_budgets
            .contains(&FindBudget::TotalContent)
    );
    query.content = None;
    query.limits = FindLimits::default();
    query.limits.max_depth = 1;
    fs::create_dir(temporary.path().join("nested")).unwrap();
    fs::write(temporary.path().join("nested/hidden.txt"), "deeper").unwrap();
    assert!(
        execute(&query)
            .0
            .reached_budgets
            .contains(&FindBudget::Depth)
    );
}

#[test]
fn cancellation_and_refused_output_stop_without_complete_claims() {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(temporary.path().join("one.txt"), "one").unwrap();
    let query = request(vec![temporary.path().to_owned()]);
    let cancel = Cancellation::default();
    cancel.cancel();
    let summary = find_worker::search(&LocalFileSystem::new(None), &query, &cancel, &mut |_| {
        panic!("Cancelled search emitted output")
    });
    assert_eq!(summary.completion, FindCompletion::Cancelled);
    let summary = find_worker::search(
        &LocalFileSystem::new(None),
        &query,
        &Cancellation::default(),
        &mut |_| false,
    );
    assert_eq!(summary.completion, FindCompletion::Cancelled);
    let invalid = request(vec![Path::new("relative").to_owned()]);
    assert_eq!(execute(&invalid).0.completion, FindCompletion::Failed);
}

#[test]
fn result_validation_rejects_changed_or_removed_native_files_and_missing_roots_are_partial() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("captured.txt");
    fs::write(&path, "original").unwrap();
    let mut query = request(vec![temporary.path().to_owned()]);
    let (_, results) = execute(&query);
    let captured = results
        .iter()
        .find(|result| result.entry.path == path)
        .unwrap();
    find_worker::validate_result(
        &LocalFileSystem::new(None),
        &captured.entry,
        &Cancellation::default(),
    )
    .unwrap();
    fs::write(&path, "changed content with a different size").unwrap();
    assert!(
        find_worker::validate_result(
            &LocalFileSystem::new(None),
            &captured.entry,
            &Cancellation::default()
        )
        .is_err()
    );
    fs::remove_file(&path).unwrap();
    assert!(
        find_worker::validate_result(
            &LocalFileSystem::new(None),
            &captured.entry,
            &Cancellation::default()
        )
        .is_err()
    );
    query.roots.push(temporary.path().join("missing directory"));
    let (summary, _) = execute(&query);
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert_eq!(summary.progress.errors, 1);
}

#[test]
fn permission_failures_are_counted_while_diagnostics_remain_bounded() {
    struct DeniedService;
    struct DeniedDirectory {
        entry: FindEntry,
        remaining: u64,
    }
    impl FindService for DeniedService {
        fn open_find_source(
            &self,
            path: &Path,
            _: &Cancellation,
        ) -> Result<Box<dyn FindSource>, FsError> {
            Ok(Box::new(DeniedDirectory {
                remaining: 100,
                entry: FindEntry {
                    path: path.to_owned(),
                    kind: EntryKind::Directory,
                    metadata: EntryMetadata::default(),
                    identity: TransferIdentity {
                        device: 1,
                        object: [0; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
            }))
        }
    }
    impl FindSource for DeniedDirectory {
        fn entry(&self) -> &FindEntry {
            &self.entry
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            Ok(())
        }
        fn next_child(&mut self, _: &Cancellation) -> Result<Option<FindChild>, FsError> {
            if self.remaining == 0 {
                return Ok(None);
            }
            self.remaining -= 1;
            Ok(Some(FindChild::Failed(find_error(
                &self.entry.path.join(format!("denied-{}", self.remaining)),
                std::io::ErrorKind::PermissionDenied,
                "Access denied",
            ))))
        }
        fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
            panic!("A denied directory has no content stream")
        }
    }
    let query = request(vec![std::env::current_dir().unwrap()]);
    let summary = find_worker::search(
        &DeniedService,
        &query,
        &Cancellation::default(),
        &mut |_| true,
    );
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert_eq!(summary.progress.errors, 100);
    assert_eq!(summary.diagnostics.len(), MAX_FIND_DIAGNOSTICS);
    assert!(
        summary
            .diagnostics
            .iter()
            .all(|error| error.kind == std::io::ErrorKind::PermissionDenied)
    );
}
