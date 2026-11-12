#![cfg(windows)]
#[path = "../src/find_worker.rs"]
mod find_worker;

use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{fs, os::windows::fs::OpenOptionsExt, path::Path, sync::Arc, time::SystemTime};

fn run(query: &FindRequest) -> (FindSummary, Vec<Arc<FindResult>>) {
    let mut rows = Vec::new();
    let location = find_worker::CurrentDirectory::default();
    let native = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let summary = find_worker::search_with_location(
        &native,
        query,
        &cancel,
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
        find_worker::validate_result(&native, &row.entry, &cancel).unwrap();
    }
    (summary, rows)
}

#[test]
fn actual_windows_read_sharing_failure_is_incomplete_and_a_new_search_recovers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_build/qa/operations-find/native-read-error");
    fs::create_dir_all(&root).unwrap();
    let fixture = tempfile::tempdir_in(fs::canonicalize(root).unwrap()).unwrap();
    let locked = fixture.path().join("locked.txt");
    fs::write(&locked, b"needle").unwrap();
    fs::write(fixture.path().join("readable.txt"), b"needle").unwrap();
    // FILE_SHARE_WRITE | FILE_SHARE_DELETE, deliberately excluding READ. Attribute
    // probes can still enumerate the file, while the actual byte open must fail.
    let writer = fs::OpenOptions::new()
        .write(true)
        .share_mode(2 | 4)
        .open(&locked)
        .unwrap();
    let mut query = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![fixture.path().to_owned()],
    );
    query.content = Some(FindContentQuery {
        text: "needle".into(),
        case_sensitive: true,
        whole_words: false,
    });
    let (summary, rows) = run(&query);
    assert_eq!(
        summary.completion,
        FindCompletion::Incomplete,
        "{summary:?}"
    );
    assert_eq!(summary.progress.errors, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(summary.diagnostics[0].path, locked);
    query.content = None;
    query.duplicates = true;
    let (summary, rows) = run(&query);
    assert_eq!(
        summary.completion,
        FindCompletion::Incomplete,
        "{summary:?}"
    );
    assert_eq!(summary.progress.errors, 1);
    assert!(rows.is_empty());
    assert_eq!(summary.diagnostics[0].path, locked);
    drop(writer);
    let (summary, rows) = run(&query);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.duplicate_group_size == 2));
}
