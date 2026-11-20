//! Compare contents and Include subfolders over an in-memory search service.
use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

#[derive(Clone)]
enum Node {
    Dir,
    File(Vec<u8>, u64),
    Link,
}

#[derive(Default)]
struct Tree {
    nodes: Mutex<BTreeMap<PathBuf, (Node, u64)>>,
    unreadable: Mutex<Vec<PathBuf>>,
    /// Content reads wait here until their cancellation token fires.
    blocking: std::sync::atomic::AtomicBool,
    /// Content reads; `cancel_after` cancels its token after that many.
    reads: AtomicUsize,
    cancel_after: Mutex<Option<(usize, Cancellation)>>,
    /// Bumps a file's revision the first time it is read (a concurrent writer).
    written_on_read: Mutex<Option<PathBuf>>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Tree>);

impl Fake {
    fn dir(&self, path: &str) -> &Self {
        self.insert(path, Node::Dir);
        self
    }
    fn file(&self, path: &str, bytes: &[u8], seconds: u64) -> &Self {
        self.insert(path, Node::File(bytes.to_vec(), seconds));
        self
    }
    fn link(&self, path: &str) -> &Self {
        self.insert(path, Node::Link);
        self
    }
    fn insert(&self, path: &str, node: Node) {
        let mut nodes = self.0.nodes.lock().unwrap();
        let generation = nodes.len() as u64;
        nodes.insert(PathBuf::from(path), (node, generation));
    }
    fn unreadable(&self, path: &str) {
        self.0.unreadable.lock().unwrap().push(path.into());
    }
    fn entry(&self, path: &Path) -> Result<FindEntry, FsError> {
        let nodes = self.0.nodes.lock().unwrap();
        let Some((node, generation)) = nodes.get(path) else {
            return Err(find_error(path, io::ErrorKind::NotFound, "missing"));
        };
        let object = {
            let mut object = [0; 16];
            let index = nodes.keys().position(|key| key == path).unwrap() as u64;
            object[..8].copy_from_slice(&index.to_le_bytes());
            object
        };
        let (kind, size, seconds) = match node {
            Node::Dir => (EntryKind::Directory, None, 0),
            Node::File(bytes, seconds) => (EntryKind::File, Some(bytes.len() as u64), *seconds),
            Node::Link => (EntryKind::Symlink, None, 0),
        };
        Ok(FindEntry {
            path: path.to_owned(),
            kind,
            metadata: EntryMetadata {
                size,
                modified: Some(UNIX_EPOCH + Duration::from_secs(seconds)),
                ..Default::default()
            },
            identity: TransferIdentity { device: 1, object },
            revision: FindRevision([size.unwrap_or(0), seconds, *generation, 0, 0, 0]),
        })
    }
    fn displayed(&self, root: &str) -> Vec<Entry> {
        let children = self.children(Path::new(root));
        children
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let facts = self.entry(path).unwrap();
                let kind = if facts.kind == EntryKind::Symlink {
                    EntryKind::Symlink
                } else {
                    facts.kind
                };
                let mut entry = Entry::new(EntryId(index as u64 + 1), path.clone(), kind);
                entry.metadata = facts.metadata;
                entry
            })
            .collect()
    }
    fn children(&self, path: &Path) -> Vec<PathBuf> {
        self.0
            .nodes
            .lock()
            .unwrap()
            .keys()
            .filter(|key| key.parent() == Some(path))
            .cloned()
            .collect()
    }
}

impl FindService for Fake {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        if cancel.is_cancelled() {
            return Err(find_error(path, io::ErrorKind::Interrupted, "cancelled"));
        }
        if self
            .0
            .unreadable
            .lock()
            .unwrap()
            .iter()
            .any(|item| item == path)
        {
            return Err(find_error(
                path,
                io::ErrorKind::PermissionDenied,
                "Permission denied",
            ));
        }
        let entry = self.entry(path)?;
        if entry.kind == EntryKind::Symlink {
            return Err(find_error(path, io::ErrorKind::Unsupported, "link"));
        }
        Ok(Box::new(Source {
            fake: self.clone(),
            entry,
            children: None,
            offset: 0,
        }))
    }
}

struct Source {
    fake: Fake,
    entry: FindEntry,
    children: Option<std::vec::IntoIter<PathBuf>>,
    offset: usize,
}

impl FindSource for Source {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self
            .fake
            .entry(&self.entry.path)?
            .matches_capture(&self.entry)
        {
            Ok(())
        } else {
            Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidData,
                "changed",
            ))
        }
    }
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError> {
        if cancel.is_cancelled() {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        let fake = self.fake.clone();
        let children = self
            .children
            .get_or_insert_with(|| fake.children(&self.entry.path).into_iter());
        let Some(path) = children.next() else {
            return Ok(None);
        };
        if self.fake.0.unreadable.lock().unwrap().contains(&path) {
            return Ok(Some(FindChild::Failed(find_error(
                &path,
                io::ErrorKind::PermissionDenied,
                "Permission denied",
            ))));
        }
        let entry = self.fake.entry(&path)?;
        if entry.kind == EntryKind::Symlink {
            return Ok(Some(FindChild::Skipped {
                path,
                reason: FindSkipReason::Link,
            }));
        }
        Ok(Some(FindChild::Entry(Box::new(Source {
            fake: self.fake.clone(),
            entry,
            children: None,
            offset: 0,
        }))))
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        let reads = self.fake.0.reads.fetch_add(1, Ordering::SeqCst) + 1;
        while self.fake.0.blocking.load(Ordering::SeqCst) && !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some((after, token)) = &*self.fake.0.cancel_after.lock().unwrap()
            && reads > *after
        {
            token.cancel();
        }
        if cancel.is_cancelled() {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::Interrupted,
                "cancelled",
            ));
        }
        let written = self.fake.0.written_on_read.lock().unwrap().take();
        if let Some(path) = written {
            if path == self.entry.path {
                let mut nodes = self.fake.0.nodes.lock().unwrap();
                if let Some((_, generation)) = nodes.get_mut(&path) {
                    *generation += 1000;
                }
            } else {
                *self.fake.0.written_on_read.lock().unwrap() = Some(path);
            }
        }
        let nodes = self.fake.0.nodes.lock().unwrap();
        let Some((Node::File(bytes, _), _)) = nodes.get(&self.entry.path) else {
            return Err(find_error(
                &self.entry.path,
                io::ErrorKind::InvalidInput,
                "not a file",
            ));
        };
        let rest = &bytes[self.offset.min(bytes.len())..];
        // Short reads on purpose: callers must fill their own buffers.
        let count = rest.len().min(buffer.len()).min(5);
        buffer[..count].copy_from_slice(&rest[..count]);
        self.offset += count;
        Ok(count)
    }
}

fn request(fake: &Fake, options: CompareOptions) -> Arc<CompareRequest> {
    let sides = [(PaneId::Left, "/left"), (PaneId::Right, "/right")].map(|(pane, root)| {
        let entries = fake.displayed(root);
        CompareSide {
            tree: None,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            source: VisibleDirectory {
                pane,
                navigation: NavigationId(1),
                path: root.into(),
                entries: entries.into(),
            },
            view_revision: 0,
            selection_revision: SelectionRevision(0),
        }
    });
    Arc::new(CompareRequest {
        id: CompareId(1),
        sides,
        options,
    })
}

fn deep(
    fake: &Fake,
    request: Arc<CompareRequest>,
    cancel: &Cancellation,
) -> (Result<CompareResult, FsError>, Vec<CompareProgress>) {
    let mut samples = Vec::new();
    let mut report = |sample: &CompareProgress| samples.push(sample.clone());
    let result = compute::compare(
        request,
        &|| cancel.is_cancelled(),
        Some(compute::Deep {
            find: fake,
            cancel,
            report: &mut report,
        }),
    );
    (result, samples)
}

fn selected(result: &CompareResult) -> [Vec<String>; 2] {
    std::array::from_fn(|side| {
        let mut names: Vec<_> = result.request.sides[side]
            .source
            .entries
            .iter()
            .filter(|entry| result.selections[side].contains_path(&entry.path))
            .map(|entry| entry.name.to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    })
}

fn options(content: bool, recursive: bool) -> CompareOptions {
    CompareOptions {
        by_content: content,
        recursive,
        ..CompareOptions::default()
    }
}

fn twins() -> Fake {
    let fake = Fake::default();
    fake.dir("/left").dir("/right");
    fake
}

#[test]
fn equal_size_and_time_with_different_bytes_selects_both_files_only_with_contents() {
    let fake = twins();
    fake.file("/left/same.txt", b"identical bytes", 100)
        .file("/right/same.txt", b"identical bytes", 100)
        .file("/left/differ.txt", b"0123456789abc", 100)
        .file("/right/differ.txt", b"0123456789abd", 100);
    let metadata = compute(request(&fake, options(false, false)), &|| false).unwrap();
    assert_eq!(selected(&metadata), [Vec::<String>::new(), vec![]]);

    let cancel = Cancellation::default();
    let (result, samples) = deep(&fake, request(&fake, options(true, false)), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [vec!["differ.txt".to_owned()], vec!["differ.txt".to_owned()]]
    );
    assert_eq!(result.content_differences, 1);
    assert_eq!(result.content_files, 2);
    assert_eq!(result.content_bytes, 2 * (15 + 13));
    assert_eq!(result.errors, 0);
    let contents = samples
        .iter()
        .find(|sample| sample.phase == ComparePhase::Contents)
        .expect("the content phase reports its totals when it starts");
    assert_eq!(contents.content_files_total, 2);
    assert_eq!(contents.content_bytes_total, 2 * (15 + 13));
}

#[test]
fn contents_skip_pairs_the_metadata_already_separates() {
    let fake = twins();
    // Sizes differ: never read. With size off, both are selected.
    fake.file("/left/longer.txt", b"abcdef", 100)
        .file("/right/longer.txt", b"abc", 100)
        // Times differ: the newer is selected without reading.
        .file("/left/newer.txt", b"same", 500)
        .file("/right/newer.txt", b"same", 100);
    let cancel = Cancellation::default();
    let (result, _) = deep(&fake, request(&fake, options(true, false)), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [
            vec!["longer.txt".to_owned(), "newer.txt".to_owned()],
            vec![]
        ]
    );
    assert_eq!(result.content_files, 0, "no pair needed reading");
    let only_contents = CompareOptions {
        by_size: false,
        by_modified: false,
        ..options(true, false)
    };
    let (result, _) = deep(&fake, request(&fake, only_contents), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [vec!["longer.txt".to_owned()], vec!["longer.txt".to_owned()]]
    );
    assert_eq!(result.content_files, 1, "only newer.txt has equal sizes");
}

#[test]
fn subfolders_select_the_folder_on_the_side_that_differs() {
    let fake = twins();
    fake.dir("/left/equal")
        .dir("/right/equal")
        .file("/left/equal/a.txt", b"a", 1)
        .file("/right/equal/a.txt", b"a", 1)
        .dir("/left/extra")
        .dir("/right/extra")
        .dir("/left/extra/deeper")
        .dir("/right/extra/deeper")
        .file("/left/extra/deeper/only-left.txt", b"x", 1)
        .dir("/left/newer")
        .dir("/right/newer")
        .file("/left/newer/n.txt", b"n", 1)
        .file("/right/newer/n.txt", b"n", 50)
        .dir("/left/one-sided");
    let flat = compute(request(&fake, CompareOptions::default()), &|| false).unwrap();
    assert_eq!(selected(&flat), [Vec::<String>::new(), vec![]]);

    let cancel = Cancellation::default();
    let (result, _) = deep(&fake, request(&fake, options(false, true)), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [
            vec!["extra".to_owned(), "one-sided".to_owned()],
            vec!["newer".to_owned()]
        ]
    );
    assert_eq!(result.differing_folders, 2);
    assert_eq!(result.errors, 0);
    assert!(!result.limited);
}

#[test]
fn subfolder_contents_and_links_are_compared_inside_folders() {
    let fake = twins();
    fake.dir("/left/data")
        .dir("/right/data")
        .file("/left/data/same.bin", b"0000", 7)
        .file("/right/data/same.bin", b"0000", 7)
        .file("/left/data/changed.bin", b"1111", 7)
        .file("/right/data/changed.bin", b"1112", 7)
        .link("/left/data/link")
        .link("/right/data/link")
        .dir("/left/links")
        .dir("/right/links")
        .link("/left/links/only-left");
    let cancel = Cancellation::default();
    let (result, _) = deep(&fake, request(&fake, options(false, true)), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [vec!["links".to_owned()], vec![]],
        "a link on one side only is a difference; bytes are not read"
    );
    assert_eq!(
        result.skipped, 1,
        "the link on both sides is matched, not followed"
    );

    let (result, _) = deep(&fake, request(&fake, options(true, true)), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [
            vec!["data".to_owned(), "links".to_owned()],
            vec!["data".to_owned()]
        ]
    );
    assert_eq!(result.content_differences, 1);
    assert_eq!(result.content_files, 2);
}

#[test]
fn unreadable_entries_select_their_folders_on_both_sides_with_a_diagnostic() {
    let fake = twins();
    fake.dir("/left/locked")
        .dir("/right/locked")
        .dir("/left/partial")
        .dir("/right/partial")
        .file("/left/partial/a", b"a", 1)
        .file("/right/partial/a", b"a", 1)
        .file("/left/partial/hidden", b"h", 1)
        .file("/right/partial/hidden", b"h", 1);
    fake.unreadable("/right/locked");
    fake.unreadable("/left/partial/hidden");
    let cancel = Cancellation::default();
    let (result, _) = deep(&fake, request(&fake, options(false, true)), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [
            vec!["locked".to_owned(), "partial".to_owned()],
            vec!["locked".to_owned(), "partial".to_owned()]
        ]
    );
    assert_eq!(result.errors, 2);
    assert_eq!(result.diagnostics.len(), 2);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|error| error.path == Path::new("/right/locked"))
    );
}

#[test]
fn a_file_written_while_it_is_read_is_selected_as_not_verified() {
    let fake = twins();
    fake.file("/left/busy.log", b"same bytes", 3)
        .file("/right/busy.log", b"same bytes", 3);
    *fake.0.written_on_read.lock().unwrap() = Some("/right/busy.log".into());
    let cancel = Cancellation::default();
    let (result, _) = deep(&fake, request(&fake, options(true, false)), &cancel);
    let result = result.unwrap();
    assert_eq!(
        selected(&result),
        [vec!["busy.log".to_owned()], vec!["busy.log".to_owned()]]
    );
    assert_eq!(result.content_differences, 0);
    assert_eq!((result.errors, result.unknown), (1, 1));
}

#[test]
fn cancelling_during_a_content_read_returns_no_partial_result() {
    let fake = twins();
    let bytes = vec![7; 4096];
    fake.file("/left/large", &bytes, 1)
        .file("/right/large", &bytes, 1);
    let cancel = Cancellation::default();
    *fake.0.cancel_after.lock().unwrap() = Some((3, cancel.clone()));
    let (result, _) = deep(&fake, request(&fake, options(true, false)), &cancel);
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(
        fake.0.reads.load(Ordering::SeqCst) < 10,
        "reading stops at once"
    );
}

#[test]
fn contents_or_subfolders_without_the_search_service_fail_clearly() {
    let fake = twins();
    fake.file("/left/a", b"a", 1).file("/right/a", b"a", 1);
    for (content, recursive) in [(true, false), (false, true)] {
        let error = compute(request(&fake, options(content, recursive)), &|| false).unwrap_err();
        assert!(error.message.contains("native search service"), "{error:?}");
    }
}

#[test]
fn folder_walk_limits_end_with_unfinished_folders_selected() {
    let fake = twins();
    let mut left = String::from("/left");
    let mut right = String::from("/right");
    for level in 0..=walk_depth_limit() {
        left.push_str(&format!("/d{level}"));
        right.push_str(&format!("/d{level}"));
        fake.dir(&left).dir(&right);
    }
    let cancel = Cancellation::default();
    let (result, _) = deep(&fake, request(&fake, options(false, true)), &cancel);
    let result = result.unwrap();
    assert!(result.limited);
    assert_eq!(
        selected(&result),
        [vec!["d0".to_owned()], vec!["d0".to_owned()]]
    );
}

fn walk_depth_limit() -> usize {
    compute::walk_limits().1
}

#[test]
fn an_expanded_tree_shows_the_differing_subfolders_selected() {
    let root = std::env::current_dir().unwrap();
    let fake = Fake::default();
    for side in ["left", "right"] {
        let base = root.join(side);
        fake.dir(base.to_str().unwrap());
        fake.dir(base.join("folder").to_str().unwrap());
        fake.file(base.join("same.txt").to_str().unwrap(), b"same", 1);
        fake.dir(
            base.join("folder")
                .join(format!("only-{side}-child"))
                .to_str()
                .unwrap(),
        );
    }
    let mut request = super::tree_tests::request();
    Arc::make_mut(&mut request).options = options(false, true);
    let cancel = Cancellation::default();
    let (result, _) = deep(&fake, request.clone(), &cancel);
    let result = result.unwrap();
    for slot in 0..2 {
        let selection = result.tree_replacements[slot].as_ref().unwrap();
        assert!(
            selection.contains_position(0, TreeRowId(40)),
            "the displayed folder contains a difference"
        );
        assert!(
            selection.contains_position(1, TreeRowId(90)),
            "its one-sided child, shown by the expanded tree, is selected too"
        );
    }
    assert_eq!(result.differing_folders, 1);
}

fn controller(
    fake: &Fake,
    capture: &CompareRequest,
) -> (Controller, tokio::sync::mpsc::Receiver<Event>, JoinSet<()>) {
    let (sender, events) = tokio::sync::mpsc::channel(64);
    let mut tasks = JoinSet::new();
    let controller = Controller::with_services(
        super::tests::sources(capture),
        crate::Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        Services {
            find: Some(Arc::new(fake.clone())),
            scope: None,
        },
        &mut tasks,
    );
    (controller, events, tasks)
}

#[tokio::test]
async fn the_controller_reports_content_progress_before_the_result() {
    let fake = twins();
    fake.file("/left/a", b"abc", 1).file("/right/a", b"abd", 1);
    let capture = request(&fake, options(true, false));
    let (controller, mut events, mut tasks) = controller(&fake, &capture);
    controller.compare((*capture).clone());
    let mut progress = None;
    let result = loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            Event::ComparisonProgress {
                id,
                progress: sample,
            } => {
                assert_eq!(id, capture.id);
                progress = Some(sample);
            }
            Event::DirectoriesCompared { id, result } => {
                assert_eq!(id, capture.id);
                break result.unwrap();
            }
            _ => {}
        }
    };
    let progress = progress.expect("a progress sample precedes the result");
    assert_eq!(progress.phase, ComparePhase::Contents);
    assert_eq!(progress.content_files_total, 1);
    assert_eq!(result.content_differences, 1);
    drop(controller);
    while let Some(joined) = tasks.join_next().await {
        joined.unwrap();
    }
}

#[tokio::test]
async fn cancel_stops_a_blocked_content_read_and_publishes_nothing() {
    let fake = twins();
    fake.file("/left/a", b"abc", 1).file("/right/a", b"abc", 1);
    fake.0.blocking.store(true, Ordering::SeqCst);
    let capture = request(&fake, options(true, false));
    let (controller, mut events, mut tasks) = controller(&fake, &capture);
    controller.compare((*capture).clone());
    tokio::time::timeout(Duration::from_secs(10), async {
        while fake.0.reads.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the worker starts reading");
    controller.cancel(capture.id);
    drop(controller);
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(joined) = tasks.join_next().await {
            joined.unwrap();
        }
    })
    .await
    .expect("the blocked read ends with its cancellation");
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, Event::DirectoriesCompared { .. }),
            "a cancelled comparison publishes no result"
        );
    }
}
