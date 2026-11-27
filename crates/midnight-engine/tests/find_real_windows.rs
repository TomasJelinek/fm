#![cfg(windows)]
#[path = "../src/find_worker.rs"]
mod find_worker;
#[path = "support/qa_fixture.rs"]
mod qa_fixture;

use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{fs, os::windows::fs::OpenOptionsExt, sync::Arc, time::SystemTime};

fn run(query: &FindRequest) -> (FindSummary, Vec<Arc<FindResult>>) {
    let mut rows = Vec::new();
    let location = find_worker::CurrentDirectory::default();
    let native = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let summary = find_worker::search_with_location(
        &native,
        Some(&native),
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
        // Ordinary and duplicate results show the panel listing's native date and time.
        let listed = native.entry_metadata(&row.entry.path).unwrap();
        assert!(listed.localized_modified.is_some());
        assert_eq!(
            row.entry.metadata.localized_modified,
            listed.localized_modified
        );
    }
    (summary, rows)
}

#[test]
fn actual_windows_read_sharing_failure_is_incomplete_and_a_new_search_recovers() {
    let fixture = qa_fixture::Folder::new(&qa_fixture::operations_find(), "find-read-error-");
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
        regex: false,
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

/// A share whose server reports no file ID and index 0 for every entry (the WebDAV
/// redirector, some SMB servers): recursive Find enters every folder and duplicate
/// search compares every file. Before, each subfolder was refused as an identity cycle
/// and every file after the first was dropped as another name of it.
#[test]
fn a_share_without_file_ids_is_searched_recursively_and_for_duplicates() {
    use midnight_platform_windows::test_network_ids::{Response, Scope};
    let fixture = qa_fixture::Folder::new(&qa_fixture::operations_find(), "find-zero-ids-");
    let nested = fixture.path().join("first").join("second");
    fs::create_dir_all(&nested).unwrap();
    fs::write(fixture.path().join("top.txt"), b"same bytes").unwrap();
    fs::write(nested.join("deep.txt"), b"same bytes").unwrap();
    fs::write(nested.join("other.txt"), b"different!").unwrap();
    let _scope = Scope::new(fixture.path(), Response::ZeroIndex).unwrap();
    let mut query = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![fixture.path().to_owned()],
    );
    let (summary, rows) = run(&query);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(summary.progress.errors, 0, "{:?}", summary.diagnostics);
    let mut names: Vec<_> = rows
        .iter()
        .map(|row| row.entry.path.file_name().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["deep.txt", "first", "other.txt", "second", "top.txt"]
    );
    assert!(rows.iter().all(|row| !row.entry.identity.is_known()));
    query.duplicates = true;
    let (summary, rows) = run(&query);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    let mut duplicates: Vec<_> = rows
        .iter()
        .map(|row| {
            (
                row.entry.path.file_name().unwrap().to_owned(),
                row.duplicate_group_size,
            )
        })
        .collect();
    duplicates.sort();
    assert_eq!(duplicates, [("deep.txt".into(), 2), ("top.txt".into(), 2)]);
}
