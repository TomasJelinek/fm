#[path = "../src/find_worker.rs"]
mod find_worker;

use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs::{self, File, FileTimes},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

fn fixture() -> tempfile::TempDir {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/operations-find/backend");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(fs::canonicalize(root).unwrap()).unwrap()
}

fn request(root: &Path) -> FindRequest {
    FindRequest::new(FindSessionId(1), FindGeneration(1), vec![root.to_owned()])
}

fn execute_at(query: &FindRequest, at: SystemTime) -> (FindSummary, Vec<Arc<FindResult>>) {
    let mut results = Vec::new();
    let location = find_worker::CurrentDirectory::default();
    let summary = find_worker::search_with_location(
        &LocalFileSystem::new(None),
        query,
        &Cancellation::default(),
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                results.extend(batch.iter().cloned());
            }
            true
        },
        &location,
        at,
    );
    assert!(location.get().is_some());
    for result in &results {
        find_worker::validate_result(
            &LocalFileSystem::new(None),
            &result.entry,
            &Cancellation::default(),
        )
        .unwrap();
    }
    (summary, results)
}

fn names(results: &[Arc<FindResult>]) -> Vec<PathBuf> {
    let mut names: Vec<_> = results
        .iter()
        .map(|row| row.entry.path.file_name().unwrap().into())
        .collect();
    names.sort();
    names
}

#[test]
fn modified_minute_and_hour_use_one_inclusive_submission_interval() {
    let fixture = fixture();
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    for (name, age) in [
        ("now.txt", 0),
        ("minute.txt", 60),
        ("hour.txt", 3600),
        ("old.txt", 3601),
    ] {
        let file = File::create(fixture.path().join(name)).unwrap();
        file.set_times(FileTimes::new().set_modified(at - Duration::from_secs(age)))
            .unwrap();
    }
    File::create(fixture.path().join("future.txt"))
        .unwrap()
        .set_times(FileTimes::new().set_modified(at + Duration::from_secs(1)))
        .unwrap();
    let mut query = request(fixture.path());
    query.modified_within = Some(Duration::from_secs(60));
    let (summary, rows) = execute_at(&query, at);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(
        names(&rows),
        [PathBuf::from("minute.txt"), PathBuf::from("now.txt")]
    );
    query.modified_within = Some(Duration::from_secs(3600));
    assert_eq!(
        names(&execute_at(&query, at).1),
        [
            PathBuf::from("hour.txt"),
            PathBuf::from("minute.txt"),
            PathBuf::from("now.txt")
        ]
    );
}

#[test]
fn duplicate_groups_require_equal_bytes_and_distinct_identities_across_roots() {
    let fixture = fixture();
    fs::create_dir(fixture.path().join("nested")).unwrap();
    let bytes = [0xff, 0, 1, 2, 3];
    fs::write(fixture.path().join("a.bin"), bytes).unwrap();
    fs::write(fixture.path().join("nested/other.name"), bytes).unwrap();
    fs::write(fixture.path().join("same-size.bin"), [0xff, 0, 1, 2, 4]).unwrap();
    fs::write(fixture.path().join("unique.bin"), [7; 21]).unwrap();
    fs::write(fixture.path().join("empty-a"), []).unwrap();
    fs::write(fixture.path().join("empty-b"), []).unwrap();
    fs::hard_link(
        fixture.path().join("a.bin"),
        fixture.path().join("alias.bin"),
    )
    .unwrap();
    let mut query = request(fixture.path());
    query.roots.push(fixture.path().join("nested"));
    query.duplicates = true;
    let (summary, rows) = execute_at(&query, SystemTime::now());
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(rows.len(), 4);
    assert_eq!(summary.progress.duplicate_groups, 2);
    assert_eq!(summary.progress.duplicate_candidates, 6);
    assert!(
        rows.iter()
            .all(|row| row.duplicate_group.is_some() && row.duplicate_group_size == 2)
    );
    for group in [1, 2] {
        let members: Vec<_> = rows
            .iter()
            .filter(|row| row.duplicate_group == Some(group))
            .collect();
        assert_eq!(members.len(), 2);
        assert_ne!(members[0].entry.identity, members[1].entry.identity);
        assert_eq!(
            fs::read(&members[0].entry.path).unwrap(),
            fs::read(&members[1].entry.path).unwrap()
        );
    }
    assert_eq!(
        summary.progress.content_bytes,
        summary.progress.duplicate_bytes
    );
    assert!(summary.progress.duplicate_bytes >= 3 * bytes.len() as u64);
}

#[test]
fn duplicate_result_and_candidate_limits_are_explicit_and_keep_whole_groups() {
    let fixture = fixture();
    for name in ["a", "b", "c"] {
        fs::write(fixture.path().join(name), b"identical").unwrap();
    }
    let mut query = request(fixture.path());
    query.duplicates = true;
    query.limits.max_results = 2;
    let (summary, rows) = execute_at(&query, SystemTime::now());
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert!(summary.reached_budgets.contains(&FindBudget::Results));
    assert!(
        rows.is_empty(),
        "A three-member group must not be presented as a two-member group"
    );
    query.limits = FindLimits::default();
    query.limits.max_duplicate_candidates = 1;
    let (summary, rows) = execute_at(&query, SystemTime::now());
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert!(
        summary
            .reached_budgets
            .contains(&FindBudget::DuplicateCandidates)
    );
    assert!(rows.is_empty());
    query.limits = FindLimits::default();
    query.limits.max_content_bytes = 1;
    let (summary, rows) = execute_at(&query, SystemTime::now());
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert!(summary.reached_budgets.contains(&FindBudget::TotalContent));
    assert!(rows.is_empty());
}

#[test]
fn large_duplicate_groups_publish_atomically_and_refused_delivery_is_cancelled() {
    let fixture = fixture();
    for index in 0..70 {
        fs::write(fixture.path().join(format!("copy-{index}")), []).unwrap();
    }
    let mut query = request(fixture.path());
    query.duplicates = true;
    for (accept, stop_after) in [(true, false), (false, false), (true, true)] {
        let mut batches = Vec::new();
        let cancel = Cancellation::default();
        let summary = find_worker::search_with_location(
            &LocalFileSystem::new(None),
            &query,
            &cancel,
            &mut |update| {
                if let FindUpdate::Batch(batch) = update {
                    batches.push(batch.len());
                    assert!(batch.iter().all(|row| row.duplicate_group_size == 70));
                    if stop_after {
                        cancel.cancel();
                    }
                    return accept;
                }
                true
            },
            &find_worker::CurrentDirectory::default(),
            SystemTime::now(),
        );
        assert_eq!(batches, [70]);
        assert_eq!(
            summary.completion,
            if accept && !stop_after {
                FindCompletion::Completed
            } else {
                FindCompletion::Cancelled
            }
        );
        assert_eq!(
            summary.progress.results,
            if accept { 70 } else { 0 },
            "Rejected groups must not count as visible results"
        );
    }
}

#[test]
fn content_name_and_recent_filters_apply_before_duplicate_grouping() {
    let fixture = fixture();
    for (name, bytes) in [
        ("a.txt", "needle"),
        ("b.txt", "needle"),
        ("c.bin", "needle"),
        ("old.txt", "needle"),
        ("other.txt", "absent"),
    ] {
        fs::write(fixture.path().join(name), bytes).unwrap();
    }
    let at = SystemTime::now();
    File::options()
        .write(true)
        .open(fixture.path().join("old.txt"))
        .unwrap()
        .set_times(FileTimes::new().set_modified(at - Duration::from_secs(120)))
        .unwrap();
    let mut query = request(fixture.path());
    query.duplicates = true;
    query.masks = "*.txt".into();
    query.modified_within = Some(Duration::from_secs(60));
    query.content = Some(FindContentQuery {
        text: "needle".into(),
        case_sensitive: true,
        whole_words: false,
    });
    let (summary, rows) = execute_at(&query, at);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(
        names(&rows),
        [PathBuf::from("a.txt"), PathBuf::from("b.txt")]
    );
}

#[test]
#[ignore = "manual performance fixture; writes 2,000 small native files under _build/qa"]
fn duplicate_pruning_performance_fixture() {
    let fixture = fixture();
    for index in 0..2000 {
        fs::write(
            fixture.path().join(format!("unique-{index}.bin")),
            vec![42; index],
        )
        .unwrap();
    }
    for index in 0..4 {
        fs::write(
            fixture.path().join(format!("duplicate-{index}.bin")),
            [17; 4096],
        )
        .unwrap();
    }
    let mut query = request(fixture.path());
    query.duplicates = true;
    let started = Instant::now();
    let (summary, rows) = execute_at(&query, SystemTime::now());
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(rows.len(), 4);
    assert_eq!(
        summary.progress.duplicate_bytes,
        4096 * 10,
        "Singleton size buckets must not read content"
    );
    eprintln!(
        "Find duplicate fixture: {} entries, {} content bytes, {:?}",
        summary.progress.examined,
        summary.progress.duplicate_bytes,
        started.elapsed()
    );
}
