#[path = "../src/find_worker.rs"]
mod find_worker;

use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn worker_search(
    service: &dyn FindService,
    request: &FindRequest,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(FindUpdate) -> bool,
) -> FindSummary {
    let location = find_worker::CurrentDirectory::default();
    let summary = find_worker::search_with_location(
        service,
        None,
        request,
        cancel,
        emit,
        &location,
        std::time::SystemTime::now(),
    );
    assert!(
        location
            .get()
            .is_none_or(|path| path.as_os_str().as_encoded_bytes().len() <= 1024 * 1024)
    );
    summary
}

fn execute(request: &FindRequest) -> (FindSummary, Vec<Arc<FindResult>>) {
    let mut results = Vec::new();
    let summary = worker_search(
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
fn wildcard_search_accepts_long_attachment_style_roots_without_content_queries() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary
        .path()
        .join("Attachments")
        .join("ooa-f42904ca-f324-4174-9491-2a49c75c6b4c")
        .join("ab71df72c4608dcdcd654370c8ef2f7f543140c45f2c8be1c623a9684fec2404")
        .join("long-native-folder-".repeat(8));
    assert!(root.as_os_str().as_encoded_bytes().len() > 260);
    fs::create_dir_all(root.join("nested")).unwrap();
    let expected = [root.join("top.txt"), root.join("nested/deep.dat")];
    for path in &expected {
        fs::write(path, []).unwrap();
    }
    let query = request(vec![root]);
    assert_eq!(query.masks, "*.*");
    assert!(query.recursive);
    assert!(query.content.is_none());
    let (summary, results) = execute(&query);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    let files: Vec<_> = results
        .iter()
        .filter(|result| result.entry.kind == EntryKind::File)
        .map(|result| result.entry.path.clone())
        .collect();
    assert_eq!(files.len(), 2);
    for path in expected {
        assert!(files.contains(&path));
    }
}

#[test]
fn regex_and_typed_expression_filter_captured_metadata_without_pruning_traversal() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("nonmatching-directory");
    fs::create_dir(&directory).unwrap();
    let deep = directory.join("deep.txt");
    fs::write(&deep, b"large enough").unwrap();
    fs::write(temporary.path().join("small.txt"), b"x").unwrap();
    let mut query = request(vec![temporary.path().to_owned()]);
    query.name_mode = FilterMode::Regex;
    query.masks = r"^deep\.txt$".into();
    let (summary, results) = execute(&query);
    assert_eq!(summary.completion, FindCompletion::Completed);
    assert_eq!(names(&results).as_slice(), std::slice::from_ref(&deep));
    query.name_mode = FilterMode::Expression;
    query.masks = "kind = file and size >= 4B".into();
    let (summary, results) = execute(&query);
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(names(&results), [deep]);
    query.masks = "size > unknownFunction()".into();
    let (summary, results) = execute(&query);
    assert_eq!(summary.completion, FindCompletion::Failed);
    assert!(results.is_empty());
    assert_eq!(summary.progress.errors, 1);
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
    // BOM-marked UTF-16 is supported; its NUL code point is valid but nonmatching.
    fs::write(temporary.path().join("valid-utf16.txt"), [0xff, 0xfe, 0, 0]).unwrap();
    // Keep one genuinely malformed UTF-8 source to test incomplete/error reporting.
    fs::write(
        temporary.path().join("malformed.txt"),
        [0xef, 0xbb, 0xbf, 0xff],
    )
    .unwrap();
    let mut query = request(vec![temporary.path().to_owned()]);
    query.masks = "*.txt".into();
    query.content = Some(FindContentQuery {
        text: "STRASSE 文档".into(),
        case_sensitive: false,
        whole_words: false,
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
        whole_words: false,
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
    let summary = worker_search(&LocalFileSystem::new(None), &query, &cancel, &mut |_| {
        panic!("Cancelled search emitted output")
    });
    assert_eq!(summary.completion, FindCompletion::Cancelled);
    let summary = worker_search(
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
    let summary = worker_search(
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

/// A root of matching files whose source logs every `next_child` call, so a
/// test sees which batches were published before traversal continued.
mod logged {
    use midnight_core::*;
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    pub type Log = Arc<Mutex<Vec<String>>>;
    pub struct Files {
        pub count: usize,
        pub log: Log,
    }
    struct Directory {
        entry: FindEntry,
        next: usize,
        count: usize,
        log: Log,
    }
    struct File(FindEntry);
    fn entry(path: PathBuf, kind: EntryKind, object: u8) -> FindEntry {
        FindEntry {
            path,
            kind,
            metadata: EntryMetadata::default(),
            identity: TransferIdentity {
                device: 1,
                object: [object; 16],
            },
            revision: FindRevision([0; 6]),
        }
    }
    impl FindService for Files {
        fn open_find_source(
            &self,
            path: &Path,
            _: &Cancellation,
        ) -> Result<Box<dyn FindSource>, FsError> {
            Ok(Box::new(Directory {
                entry: entry(path.to_owned(), EntryKind::Directory, 0),
                next: 0,
                count: self.count,
                log: self.log.clone(),
            }))
        }
    }
    impl FindSource for Directory {
        fn entry(&self) -> &FindEntry {
            &self.entry
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            Ok(())
        }
        fn next_child(&mut self, _: &Cancellation) -> Result<Option<FindChild>, FsError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("child {}", self.next));
            if self.next == self.count {
                return Ok(None);
            }
            self.next += 1;
            let path = self.entry.path.join(format!("file-{}", self.next));
            let file = entry(path, EntryKind::File, self.next as u8);
            Ok(Some(FindChild::Entry(Box::new(File(file)))))
        }
        fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
            panic!("a directory has no content stream")
        }
    }
    impl FindSource for File {
        fn entry(&self) -> &FindEntry {
            &self.0
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            Ok(())
        }
        fn next_child(&mut self, _: &Cancellation) -> Result<Option<FindChild>, FsError> {
            Ok(None)
        }
        fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
            panic!("a name search reads no content")
        }
    }
}

fn logged_search(progress_interval: std::time::Duration) -> Vec<String> {
    let log = logged::Log::default();
    let service = logged::Files {
        count: 3,
        log: log.clone(),
    };
    let summary = find_worker::search_with_progress_interval(
        &service,
        None,
        &request(vec![std::env::current_dir().unwrap()]),
        &Cancellation::default(),
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                log.lock().unwrap().push(format!("batch {}", batch.len()));
            }
            true
        },
        &find_worker::CurrentDirectory::default(),
        std::time::SystemTime::now(),
        progress_interval,
    );
    assert_eq!(summary.completion, FindCompletion::Completed);
    assert_eq!(summary.progress.results, 3);
    log.lock().unwrap().clone()
}

#[test]
fn partial_batches_follow_the_progress_cadence_while_traversal_continues() {
    // Production publishes at most four progress samples per second.
    assert_eq!(
        find_worker::PROGRESS_INTERVAL,
        std::time::Duration::from_millis(250)
    );
    // A due sample publishes the matches found so far, below one 64-row batch,
    // before the next child is read.
    assert_eq!(
        logged_search(std::time::Duration::ZERO),
        [
            "child 0", "batch 1", "child 1", "batch 1", "child 2", "batch 1", "child 3"
        ]
    );
    // Before the interval elapses, partial matches wait for the end.
    assert_eq!(
        logged_search(std::time::Duration::from_secs(3600)),
        ["child 0", "child 1", "child 2", "child 3", "batch 3"]
    );
}

/// A synthetic native tree: `folders` folders of `files` empty files, every node with its
/// own identity and modification time; the `one_byte` file holds one byte instead. Files
/// reopen by path for duplicate checks. A `provider_locale` formats every node the
/// provider opens (the former Windows provider).
mod synthetic {
    use midnight_core::*;
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
        time::{Duration, SystemTime},
    };

    #[derive(Clone)]
    pub struct Tree {
        pub folders: usize,
        pub files: usize,
        pub one_byte: Option<PathBuf>,
        pub provider_locale: Option<Arc<dyn LocaleService>>,
    }
    #[derive(Clone, Copy)]
    enum Level {
        Root,
        Folder(usize),
        File,
    }
    struct Node {
        tree: Tree,
        entry: FindEntry,
        level: Level,
        next: usize,
    }
    impl Tree {
        fn node(&self, path: PathBuf, level: Level) -> Node {
            let (kind, index) = match level {
                Level::Root => (EntryKind::Directory, 0),
                Level::Folder(folder) => (EntryKind::Directory, 1 + folder),
                Level::File => (EntryKind::File, self.file_index(&path).unwrap()),
            };
            let index = index as u64;
            let mut object = [0; 16];
            object[..8].copy_from_slice(&index.to_le_bytes());
            let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + index * 61);
            let size = (kind == EntryKind::File)
                .then(|| u64::from(self.one_byte.as_ref().is_some_and(|file| *file == path)));
            Node {
                tree: self.clone(),
                entry: FindEntry {
                    path,
                    kind,
                    metadata: EntryMetadata {
                        size,
                        modified: Some(modified),
                        localized_modified: self
                            .provider_locale
                            .as_ref()
                            .and_then(|locale| locale.format_datetime(modified)),
                        ..Default::default()
                    },
                    identity: TransferIdentity { device: 1, object },
                    revision: FindRevision([index; 6]),
                },
                level,
                next: 0,
            }
        }
        /// `.../folder-F/file-I.txt` is node `1 + folders + F * files + I`.
        fn file_index(&self, path: &Path) -> Option<usize> {
            let number = |path: &Path, prefix: &str, suffix: &str| {
                path.file_name()?
                    .to_str()?
                    .strip_prefix(prefix)?
                    .strip_suffix(suffix)?
                    .parse::<usize>()
                    .ok()
            };
            let file = number(path, "file-", ".txt")?;
            let folder = number(path.parent()?, "folder-", "")?;
            Some(1 + self.folders + folder * self.files + file)
        }
    }
    impl FindService for Tree {
        fn open_find_source(
            &self,
            path: &Path,
            _: &Cancellation,
        ) -> Result<Box<dyn FindSource>, FsError> {
            let level = if self.file_index(path).is_some() {
                Level::File
            } else {
                Level::Root
            };
            Ok(Box::new(self.node(path.to_owned(), level)))
        }
    }
    impl FindSource for Node {
        fn entry(&self) -> &FindEntry {
            &self.entry
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            Ok(())
        }
        fn next_child(&mut self, _: &Cancellation) -> Result<Option<FindChild>, FsError> {
            let (count, path, level) = match self.level {
                Level::Root => (
                    self.tree.folders,
                    format!("folder-{}", self.next),
                    Level::Folder(self.next),
                ),
                Level::Folder(_) => (
                    self.tree.files,
                    format!("file-{}.txt", self.next),
                    Level::File,
                ),
                Level::File => (0, String::new(), Level::File),
            };
            if self.next == count {
                return Ok(None);
            }
            self.next += 1;
            let child = self.tree.node(self.entry.path.join(path), level);
            Ok(Some(FindChild::Entry(Box::new(child))))
        }
        fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
            panic!("synthetic file contents are never read")
        }
    }
}

/// Records every date/time it formats: through `inner`, or as whole seconds.
#[derive(Default)]
struct RecordingLocale {
    inner: Option<Arc<dyn LocaleService>>,
    formatted: Mutex<Vec<SystemTime>>,
}
impl LocaleService for RecordingLocale {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(NumberLocale::default())
    }
    fn format_datetime(&self, value: SystemTime) -> Option<LocalizedDateTime> {
        self.formatted.lock().unwrap().push(value);
        match &self.inner {
            Some(inner) => inner.format_datetime(value),
            None => Some(seconds(value)),
        }
    }
}
fn seconds(value: SystemTime) -> LocalizedDateTime {
    let seconds = value.duration_since(UNIX_EPOCH).unwrap().as_secs();
    LocalizedDateTime {
        date: format!("day {}", seconds / 86_400),
        time: format!("second {}", seconds % 86_400),
    }
}

fn synthetic_search(
    tree: &synthetic::Tree,
    locale: Option<&dyn LocaleService>,
    query: &FindRequest,
) -> (FindSummary, Vec<Arc<FindResult>>) {
    let mut results = Vec::new();
    let summary = find_worker::search_with_location(
        tree,
        locale,
        query,
        &Cancellation::default(),
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                results.extend(batch.iter().cloned());
            }
            true
        },
        &find_worker::CurrentDirectory::default(),
        SystemTime::now(),
    );
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    (summary, results)
}

fn sorted_times<'a>(times: impl IntoIterator<Item = &'a SystemTime>) -> Vec<SystemTime> {
    let mut times: Vec<_> = times.into_iter().copied().collect();
    times.sort();
    times
}

#[test]
fn the_engine_locale_formats_each_published_result_once_and_no_other_node() {
    let root = std::env::current_dir().unwrap();
    // The three matches are folder-N/file-7.txt; the first one holds one byte.
    let one_byte = root.join("folder-0").join("file-7.txt");
    let tree = synthetic::Tree {
        folders: 3,
        files: 10,
        one_byte: Some(one_byte.clone()),
        provider_locale: None,
    };
    let mut query = request(vec![root]);
    query.masks = "file-7.txt".into();
    let search = |query: &FindRequest| {
        let locale = RecordingLocale::default();
        let (summary, results) = synthetic_search(&tree, Some(&locale), query);
        assert_eq!(summary.progress.examined, 33);
        for result in &results {
            let metadata = &result.entry.metadata;
            assert_eq!(metadata.localized_modified, metadata.modified.map(seconds));
        }
        let formatted = sorted_times(locale.formatted.lock().unwrap().iter());
        (summary, results, formatted)
    };
    let modified_times = |results: &[Arc<FindResult>]| {
        sorted_times(
            results
                .iter()
                .map(|result| result.entry.metadata.modified.as_ref().unwrap()),
        )
    };
    // An ordinary search formats each of its three results once. The three folders and
    // 27 nonmatching files are examined but never formatted.
    let (_, results, formatted) = search(&query);
    assert_eq!(results.len(), 3);
    let matches = modified_times(&results);
    assert_eq!(formatted, matches);
    // A duplicate search formats only the members of the groups it publishes. The
    // one-byte candidate is alone in its size bucket: it is never read, joins no
    // group and is never formatted.
    query.duplicates = true;
    let (summary, results, formatted) = search(&query);
    assert_eq!(summary.progress.duplicate_candidates, 3);
    assert_eq!(summary.progress.duplicate_groups, 1);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| result.entry.path != one_byte));
    assert_eq!(formatted, modified_times(&results));
    // A group that the result limit omits is not formatted either.
    let mut limited = query.clone();
    limited.limits.max_results = 1;
    let locale = RecordingLocale::default();
    let mut published = 0;
    let summary = find_worker::search_with_location(
        &tree,
        Some(&locale),
        &limited,
        &Cancellation::default(),
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                published += batch.len();
            }
            true
        },
        &find_worker::CurrentDirectory::default(),
        SystemTime::now(),
    );
    assert_eq!(summary.reached_budgets, vec![FindBudget::Results]);
    assert_eq!(summary.progress.duplicate_candidates, 3);
    assert_eq!(published, 0);
    assert!(locale.formatted.lock().unwrap().is_empty());
    // A time the provider formatted is kept, and without a locale results keep the
    // provider's absent value (the UI shows its labelled UTC fallback).
    query.duplicates = false;
    let provider = Arc::new(RecordingLocale::default());
    let provider_formatted = synthetic::Tree {
        provider_locale: Some(provider.clone()),
        ..tree.clone()
    };
    let engine = RecordingLocale::default();
    let (_, results) = synthetic_search(&provider_formatted, Some(&engine), &query);
    assert!(engine.formatted.lock().unwrap().is_empty());
    assert_eq!(provider.formatted.lock().unwrap().len(), 34);
    assert!(results.iter().all(|result| {
        let metadata = &result.entry.metadata;
        metadata.localized_modified == metadata.modified.map(seconds)
    }));
    let (_, results) = synthetic_search(&tree, None, &query);
    assert_eq!(results.len(), 3);
    assert!(
        results
            .iter()
            .all(|result| result.entry.metadata.localized_modified.is_none())
    );
}

/// Manual measurement: a name search of 400 synthetic folders of 625 files with the
/// native locale. `provider formats` formats every node the provider opens, as the
/// Windows provider formerly did; otherwise the engine formats the 400 results only.
#[test]
#[ignore = "manual performance measurement over 250,400 synthetic nodes"]
fn synthetic_name_search_localization_measurement() {
    let native: Arc<dyn LocaleService> = Arc::new(LocalFileSystem::new(None));
    let mut query = request(vec![std::env::current_dir().unwrap()]);
    query.masks = "file-7.txt".into();
    let mut samples = [Vec::new(), Vec::new()];
    for round in 0..7 {
        for provider_formats in [round % 2 == 0, round % 2 != 0] {
            let provider = Arc::new(RecordingLocale {
                inner: Some(native.clone()),
                ..Default::default()
            });
            let engine = RecordingLocale {
                inner: Some(native.clone()),
                ..Default::default()
            };
            let tree = synthetic::Tree {
                folders: 400,
                files: 625,
                one_byte: None,
                provider_locale: provider_formats
                    .then(|| provider.clone() as Arc<dyn LocaleService>),
            };
            let started = Instant::now();
            let (summary, results) = synthetic_search(&tree, Some(&engine), &query);
            let elapsed = started.elapsed();
            assert!(
                results
                    .iter()
                    .all(|result| result.entry.metadata.localized_modified.is_some())
            );
            eprintln!(
                "{}: {elapsed:?}, {} nodes, {} results, {} provider formats, {} engine formats",
                if provider_formats {
                    "provider formats"
                } else {
                    "results only"
                },
                summary.progress.examined,
                results.len(),
                provider.formatted.lock().unwrap().len(),
                engine.formatted.lock().unwrap().len(),
            );
            samples[usize::from(provider_formats)].push(elapsed);
        }
    }
    for (label, mut times) in ["results only", "provider formats"]
        .into_iter()
        .zip(samples)
    {
        times.sort();
        eprintln!(
            "{label}: min {:?}, median {:?}",
            times[0],
            times[times.len() / 2]
        );
    }
}
