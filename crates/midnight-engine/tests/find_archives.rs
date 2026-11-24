//! Search inside archives over a fake provider session: member names, contents,
//! folders, archives and members beyond any count, cancellation (also of a slow
//! provider) and release.
#[allow(dead_code, reason = "this copy exercises the archive seam only")]
#[path = "../src/find_worker.rs"]
mod find_worker;

use find_worker::{ArchiveSearch, archive::OpenedArchive};
use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime},
};

/// The provider call that works through a huge archive until Stop cancels it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slow {
    Open,
    List,
    Read,
}

/// How a slow provider call reports its work: it says when it starts, and records
/// whether the search's cancellation, rather than its own ten-second patience, ended it.
#[derive(Debug, Default)]
struct SlowWork {
    started: Option<mpsc::SyncSender<Slow>>,
    stopped: AtomicBool,
}
impl SlowWork {
    fn until_cancelled(&self, slow: Slow, cancel: &Cancellation) -> FsError {
        if let Some(started) = &self.started {
            let _ = started.try_send(slow);
        }
        let begun = Instant::now();
        while !cancel.is_cancelled() {
            if begun.elapsed() > Duration::from_secs(10) {
                return find_error(
                    Path::new(""),
                    std::io::ErrorKind::TimedOut,
                    "the provider was never cancelled",
                );
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.stopped.store(true, Ordering::SeqCst);
        find_error(Path::new(""), std::io::ErrorKind::Interrupted, "cancelled")
    }
}

#[derive(Debug)]
struct FakeMount {
    nodes: Vec<(u64, ArchiveNode, Vec<u8>)>,
    reads: Arc<AtomicUsize>,
    cancel_on_read: Option<Cancellation>,
    slow: Option<(Slow, Arc<SlowWork>)>,
}

fn node(id: u64, parent: u64, name: &str, kind: EntryKind, size: Option<u64>) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId {
            mount: 7,
            entry: id,
        },
        parent,
        raw_name: name.as_bytes().into(),
        display_name: name.into(),
        kind,
        size,
        readable: true,
        modified: Some(ArchiveTimestamp::Local {
            year: 2024,
            month: 5,
            day: 6,
            hour: 7,
            minute: 8,
            second: 9,
        }),
        access: Default::default(),
    }
}

impl FakeMount {
    /// Four members, and `extra` empty files `extra-<n>.bin` and `folders` empty folders
    /// `folder-<n>` beside them in the root.
    fn new(reads: Arc<AtomicUsize>, extra: usize, folders: usize) -> Self {
        let file = |id, parent, name: &str, bytes: &str| {
            (
                parent,
                node(id, parent, name, EntryKind::File, Some(bytes.len() as u64)),
                bytes.as_bytes().to_vec(),
            )
        };
        let mut nodes = vec![
            (1, node(2, 1, "docs", EntryKind::Directory, None), vec![]),
            file(3, 1, "readme.md", "hello archive needle"),
            file(4, 2, "guide.txt", "nothing here"),
            file(5, 2, "notes.md", "a needle in docs"),
        ];
        nodes.extend((0..extra as u64).map(|n| file(10 + n, 1, &format!("extra-{n}.bin"), "")));
        nodes.extend((0..folders as u64).map(|n| {
            let name = format!("folder-{n}");
            (
                1,
                node(1 << 32 | n, 1, &name, EntryKind::Directory, None),
                vec![],
            )
        }));
        Self {
            nodes,
            reads,
            cancel_on_read: None,
            slow: None,
        }
    }
    fn slow(&self, call: Slow, cancel: &Cancellation) -> Option<FsError> {
        let (slow, work) = self.slow.as_ref()?;
        (*slow == call).then(|| work.until_cancelled(call, cancel))
    }
}

impl ArchiveMount for FakeMount {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        Ok(())
    }
    fn id(&self) -> u64 {
        7
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        if let Some(error) = self.slow(Slow::List, cancel) {
            return Err(error);
        }
        if cancel.is_cancelled() {
            return Err(find_error(
                Path::new(""),
                std::io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        Ok(self
            .nodes
            .iter()
            .filter(|(parent, _, _)| *parent == directory)
            .map(|(_, node, _)| node.clone())
            .collect())
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(token) = &self.cancel_on_read {
            token.cancel();
        }
        // Staging a huge member: the first read of a member decodes all of it.
        if let Some(error) = self.slow(Slow::Read, cancel) {
            return Err(error);
        }
        if cancel.is_cancelled() {
            return Err(find_error(
                Path::new(""),
                std::io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        let (_, _, bytes) = self
            .nodes
            .iter()
            .find(|(_, node, _)| node.id.entry == entry)
            .expect("known member");
        let start = (offset as usize).min(bytes.len());
        // Short provider reads on purpose.
        let end = (start + length as usize).min(bytes.len()).min(start + 4);
        Ok(bytes[start..end].to_vec())
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

#[derive(Default)]
struct FakeArchives {
    opened: AtomicUsize,
    released: AtomicUsize,
    reads: Arc<AtomicUsize>,
    cancel_on_read: Option<Cancellation>,
    /// The archive with this file name has that many more files in its root.
    huge: Option<(&'static str, usize)>,
    /// The archive with this file name has that many more folders in its root.
    folders: Option<(&'static str, usize)>,
    slow: Option<(Slow, Arc<SlowWork>)>,
}

impl ArchiveSearch for FakeArchives {
    fn open(
        &self,
        archive: &FindEntry,
        cancel: &Cancellation,
    ) -> Option<Result<OpenedArchive, FsError>> {
        archive_format(&archive.path)?;
        // Opening a huge archive scans all of it before the first listing.
        if let Some((Slow::Open, work)) = &self.slow {
            return Some(Err(work.until_cancelled(Slow::Open, cancel)));
        }
        self.opened.fetch_add(1, Ordering::SeqCst);
        let count = |named: Option<(&str, usize)>| {
            named
                .filter(|(name, _)| archive.path.file_name() == Some(std::ffi::OsStr::new(name)))
                .map_or(0, |(_, count)| count)
        };
        let mut mount = FakeMount::new(self.reads.clone(), count(self.huge), count(self.folders));
        mount.cancel_on_read = self.cancel_on_read.clone();
        mount.slow = self.slow.clone();
        Some(Ok(OpenedArchive {
            mount: Arc::new(mount),
            root: ArchiveAddress {
                provider: "fake".into(),
                format: Some(ArchiveFormat::Zip),
                container: archive.path.clone(),
                source_version: vec![1],
                components: Vec::new(),
            },
        }))
    }
    fn release(&self, mount: Arc<dyn ArchiveMount>) -> Result<(), FsError> {
        self.released.fetch_add(1, Ordering::SeqCst);
        mount.release_on_worker()
    }
}

fn fixture() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().unwrap();
    fs::write(
        temporary.path().join("bundle.zip"),
        b"PK fake archive bytes",
    )
    .unwrap();
    fs::write(temporary.path().join("plain.txt"), b"no match").unwrap();
    temporary
}

fn run(
    archives: Option<&dyn ArchiveSearch>,
    request: &FindRequest,
    cancel: &Cancellation,
) -> (FindSummary, Vec<Arc<FindResult>>) {
    let mut results = Vec::new();
    let summary = find_worker::search_with_archives(
        &LocalFileSystem::new(None),
        archives,
        None,
        request,
        cancel,
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                results.extend(batch.iter().cloned());
            }
            true
        },
        &find_worker::CurrentDirectory::default(),
        SystemTime::now(),
    );
    (summary, results)
}

fn shown(results: &[Arc<FindResult>]) -> Vec<PathBuf> {
    let mut paths: Vec<_> = results
        .iter()
        .map(|result| result.entry.path.clone())
        .collect();
    paths.sort();
    paths
}

#[test]
fn member_names_match_inside_archives_with_their_folder_address() {
    let temporary = fixture();
    let root = temporary.path();
    let archives = FakeArchives::default();
    let mut request = FindRequest::new(FindSessionId(1), FindGeneration(1), vec![root.into()]);
    request.masks = "*.md".into();
    let (summary, results) = run(Some(&archives), &request, &Cancellation::default());
    assert!(results.is_empty(), "archives are searched only on request");
    assert_eq!(summary.progress.archives, 0);

    request.archives = true;
    let (summary, results) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(
        summary.completion,
        FindCompletion::Completed,
        "{:?}",
        summary.diagnostics
    );
    let archive = root.join("bundle.zip");
    assert_eq!(
        shown(&results),
        [
            archive.join("docs").join("notes.md"),
            archive.join("readme.md")
        ]
    );
    assert_eq!(
        (summary.progress.archives, summary.progress.archive_members),
        (1, 4)
    );
    let notes = results
        .iter()
        .find(|result| result.entry.path.ends_with("notes.md"))
        .unwrap();
    let member = notes.archive.as_ref().expect("an archive member");
    assert_eq!(member.folder.container, archive);
    assert_eq!(member.folder.components, vec![b"docs".to_vec()]);
    assert_eq!(member.name, b"notes.md");
    assert!(member.folder.validate().is_ok());
    assert_eq!(member.container.path, archive);
    assert_eq!(
        notes
            .entry
            .metadata
            .localized_modified
            .as_ref()
            .map(|time| time.date.as_str()),
        Some("2024-05-06"),
        "a zone-less archive time is shown as stored"
    );
    assert_eq!(
        archives.released.load(Ordering::SeqCst),
        archives.opened.load(Ordering::SeqCst),
        "every session is released on the worker"
    );
}

#[test]
fn archive_search_honors_explicit_scan_budgets_without_limiting_default_searches() {
    let temporary = fixture();
    let root = temporary.path();
    fs::remove_file(root.join("plain.txt")).unwrap();
    let archives = FakeArchives::default();
    let mut request = FindRequest::new(FindSessionId(1), FindGeneration(1), vec![root.into()]);
    request.archives = true;
    request.masks = "*.md".into();
    request.limits.max_nodes = 2;
    let (summary, _) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert_eq!(summary.progress.examined, 2);
    assert_eq!(summary.progress.archive_members, 1);
    assert!(summary.reached_budgets.contains(&FindBudget::Nodes));

    request.limits.max_nodes = 0;
    let (summary, results) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(summary.completion, FindCompletion::Completed, "{summary:?}");
    assert_eq!(summary.progress.archive_members, 4);
    assert_eq!(results.len(), 2);
    assert!(!summary.reached_budgets.contains(&FindBudget::Nodes));
}

#[test]
fn member_contents_are_read_through_the_provider() {
    let temporary = fixture();
    let root = temporary.path();
    let archives = FakeArchives::default();
    let mut request = FindRequest::new(FindSessionId(1), FindGeneration(1), vec![root.into()]);
    request.archives = true;
    request.content = Some(FindContentQuery {
        text: "needle".into(),
        case_sensitive: true,
        whole_words: true,
        regex: false,
    });
    let (summary, results) = run(Some(&archives), &request, &Cancellation::default());
    let archive = root.join("bundle.zip");
    assert_eq!(
        shown(&results),
        [
            archive.join("docs").join("notes.md"),
            archive.join("readme.md")
        ]
    );
    assert!(summary.progress.content_bytes > 0);
    assert!(
        archives.reads.load(Ordering::SeqCst) > 3,
        "short reads are continued"
    );
    request.content = Some(FindContentQuery {
        text: r"^a needle\b".into(),
        case_sensitive: true,
        whole_words: false,
        regex: true,
    });
    let (_, results) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(shown(&results), [archive.join("docs").join("notes.md")]);
}

#[test]
fn every_archive_and_member_is_searched_however_many_there_are() {
    // More archives than the 1,000 a search once opened, one of them with more
    // members than the 100,000 an archive once listed.
    const ARCHIVES: usize = 1_001;
    const EXTRA: usize = 100_001;
    let temporary = tempfile::tempdir().unwrap();
    for archive in 0..ARCHIVES {
        fs::write(
            temporary.path().join(format!("a{archive:04}.zip")),
            b"PK fake archive bytes",
        )
        .unwrap();
    }
    let archives = FakeArchives {
        huge: Some(("a0500.zip", EXTRA)),
        ..FakeArchives::default()
    };
    let mut request = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![temporary.path().into()],
    );
    request.archives = true;
    request.masks = "readme.md;extra-100000.bin".into();
    let (summary, results) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(
        summary.completion,
        FindCompletion::Completed,
        "{:?} {:?}",
        summary.reached_budgets,
        summary.diagnostics
    );
    assert!(summary.reached_budgets.is_empty());
    assert_eq!(summary.progress.archives, ARCHIVES as u64);
    assert_eq!(
        summary.progress.archive_members,
        (ARCHIVES * 4 + EXTRA) as u64
    );
    assert_eq!(
        results.len(),
        ARCHIVES + 1,
        "every readme.md and the last extra"
    );
    assert!(
        results
            .iter()
            .any(|result| result.entry.path.ends_with("a0500.zip/extra-100000.bin"))
    );
    assert_eq!(archives.opened.load(Ordering::SeqCst), ARCHIVES);
    assert_eq!(archives.released.load(Ordering::SeqCst), ARCHIVES);
}

#[test]
fn folders_waiting_in_an_archive_count_against_retained_memory_until_listed() {
    let temporary = tempfile::tempdir().unwrap();
    for archive in 0..300 {
        fs::write(
            temporary.path().join(format!("a{archive:03}.zip")),
            b"PK fake archive bytes",
        )
        .unwrap();
    }
    let mut request = FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![temporary.path().into()],
    );
    request.archives = true;
    request.masks = "nothing matches this".into();
    request.limits.max_retained_bytes = 16 * 1024;
    // Each archive's folders give their bytes back once listed, so 300 archives fit.
    let archives = FakeArchives::default();
    let (summary, _) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(
        summary.completion,
        FindCompletion::Completed,
        "{:?}",
        summary.reached_budgets
    );
    // 2,000 folders waiting at once in one archive do not.
    let archives = FakeArchives {
        folders: Some(("a150.zip", 2_000)),
        ..FakeArchives::default()
    };
    let (summary, _) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert_eq!(summary.reached_budgets, [FindBudget::RetainedBytes]);
    assert_eq!(
        archives.released.load(Ordering::SeqCst),
        archives.opened.load(Ordering::SeqCst)
    );
    // Within the default budget the same archive is searched completely.
    request.limits = FindLimits::default();
    let (summary, _) = run(Some(&archives), &request, &Cancellation::default());
    assert_eq!(summary.completion, FindCompletion::Completed);
    assert_eq!(summary.progress.archive_members, 300 * 4 + 2_000);
}

#[test]
fn stop_reaches_an_archive_while_it_opens_lists_or_stages_a_member() {
    for slow in [Slow::Open, Slow::List, Slow::Read] {
        let temporary = fixture();
        let (started, working) = mpsc::sync_channel(1);
        let work = Arc::new(SlowWork {
            started: Some(started),
            ..SlowWork::default()
        });
        let archives = Arc::new(FakeArchives {
            slow: Some((slow, work.clone())),
            ..FakeArchives::default()
        });
        let mut request = FindRequest::new(
            FindSessionId(1),
            FindGeneration(1),
            vec![temporary.path().into()],
        );
        request.archives = true;
        request.content = Some(FindContentQuery {
            text: "needle".into(),
            case_sensitive: false,
            whole_words: false,
            regex: false,
        });
        let cancel = Cancellation::default();
        let search = {
            let (archives, cancel) = (archives.clone(), cancel.clone());
            std::thread::spawn(move || run(Some(archives.as_ref()), &request, &cancel).0)
        };
        assert_eq!(working.recv_timeout(Duration::from_secs(10)).unwrap(), slow);
        let stopped = Instant::now();
        cancel.cancel();
        let summary = search.join().unwrap();
        assert!(
            stopped.elapsed() < Duration::from_secs(5),
            "{slow:?}: Stop took {:?}",
            stopped.elapsed()
        );
        assert_eq!(summary.completion, FindCompletion::Cancelled, "{slow:?}");
        assert!(
            work.stopped.load(Ordering::SeqCst),
            "{slow:?}: the provider call saw the search's own cancellation"
        );
        assert_eq!(
            archives.released.load(Ordering::SeqCst),
            archives.opened.load(Ordering::SeqCst),
            "{slow:?}: every opened session is released"
        );
    }
}

#[test]
fn a_missing_provider_is_reported_once() {
    let temporary = fixture();
    let root = temporary.path();
    let mut request = FindRequest::new(FindSessionId(1), FindGeneration(1), vec![root.into()]);
    request.archives = true;
    let (summary, results) = run(None, &request, &Cancellation::default());
    assert_eq!(summary.completion, FindCompletion::Incomplete);
    assert!(results.iter().all(|result| result.archive.is_none()));
    assert_eq!(summary.progress.errors, 1, "said once, not per archive");
    assert!(
        summary.diagnostics[0]
            .message
            .contains("Search inside archives is unavailable")
    );
}

#[test]
fn stop_during_a_member_read_ends_the_search_and_releases_the_session() {
    let temporary = fixture();
    let root = temporary.path();
    let cancel = Cancellation::default();
    let archives = FakeArchives {
        cancel_on_read: Some(cancel.clone()),
        ..FakeArchives::default()
    };
    let mut request = FindRequest::new(FindSessionId(1), FindGeneration(1), vec![root.into()]);
    request.archives = true;
    request.content = Some(FindContentQuery {
        text: "needle".into(),
        case_sensitive: false,
        whole_words: false,
        regex: false,
    });
    let (summary, _) = run(Some(&archives), &request, &cancel);
    assert_eq!(summary.completion, FindCompletion::Cancelled);
    assert_eq!(archives.reads.load(Ordering::SeqCst), 1);
    assert_eq!(archives.released.load(Ordering::SeqCst), 1);
}
