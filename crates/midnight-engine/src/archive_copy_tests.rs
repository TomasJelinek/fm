use super::*;
use std::collections::HashSet;

#[derive(Debug)]
struct MemoryMount {
    nodes: Vec<ArchiveNode>,
    payload: Vec<u8>,
    changed: AtomicBool,
}
impl ArchiveMount for MemoryMount {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        if let Some(owner) = Arc::into_inner(self) {
            drop(owner);
        }
        Ok(())
    }
    fn id(&self) -> u64 {
        77
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.changed.load(Ordering::Acquire) {
            return Err(error(
                Path::new("container"),
                io::ErrorKind::InvalidData,
                "Source changed",
            ));
        }
        Ok(())
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.validate()?;
        Ok(self
            .nodes
            .iter()
            .filter(|node| node.parent == directory)
            .cloned()
            .collect())
    }
    fn read(&self, _: u64, offset: u64, length: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        self.validate()?;
        let start = (offset as usize).min(self.payload.len());
        Ok(self.payload[start..(start + length as usize).min(self.payload.len())].to_vec())
    }
}
fn node(id: u64, parent: u64, name: &[u8], kind: EntryKind) -> ArchiveNode {
    ArchiveNode {
        modified: None,
        id: VirtualEntryId {
            mount: 77,
            entry: id,
        },
        parent,
        raw_name: name.into(),
        display_name: String::from_utf8_lossy(name).into_owned(),
        kind,
        size: (kind == EntryKind::File).then_some(7),
        readable: kind == EntryKind::File,
        access: Default::default(),
    }
}
fn input(mount: Arc<MemoryMount>, destination: &Path) -> ArchiveCopyRequest {
    let entries: Vec<_> = mount
        .nodes
        .iter()
        .filter(|node| node.parent == 1)
        .cloned()
        .map(Entry::from_archive)
        .collect();
    ArchiveCopyRequest {
        start_policy: midnight_core::TransferStartPolicy::Immediate,
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        location: ArchiveLocation {
            mount: Some(mount),
            address: ArchiveAddress {
                nested: Vec::new(),
                format: None,
                provider: "fixture".into(),
                container: destination.join("source.zip"),
                source_version: vec![1],
                components: vec![],
            },
            directory: 1,
            trail: vec![],
        },
        selected: Arc::new(entries.iter().map(|entry| entry.id).collect::<HashSet<_>>()),
        view: Arc::new(midnight_core::DirectoryIndex::from_entries(&entries)),
        entries: entries.into(),
        fallback: None,
        destination: destination.into(),
        metadata: midnight_core::TransferMetadataPolicy::ContentOnly,
    }
}

#[tokio::test]
async fn filtered_archive_copy_excludes_previously_selected_hidden_members() {
    let temp = fixture();
    let archive = mount(vec![
        node(2, 1, b"visible", EntryKind::File),
        node(3, 1, b"hidden", EntryKind::File),
    ]);
    let mut request = input(archive, temp.path());
    request.view = Arc::new(midnight_core::DirectoryIndex::from_entries(
        &request.entries[..1],
    ));
    let mut h = Harness::native();
    h.controller.enqueue_archive(request).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 1);
    assert_eq!(fs::read(temp.path().join("visible")).unwrap(), b"payload");
    assert!(!temp.path().join("hidden").exists());
    h.close().await;
}

/// A mount whose `locked` member needs the archive's password, which was not entered.
#[derive(Debug)]
struct LockedMount {
    inner: Arc<MemoryMount>,
    locked: u64,
}
impl ArchiveMount for LockedMount {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        if let Some(owner) = Arc::into_inner(self) {
            drop(owner);
        }
        Ok(())
    }
    fn id(&self) -> u64 {
        self.inner.id()
    }
    fn validate(&self) -> Result<(), FsError> {
        self.inner.validate()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.inner.list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        if entry == self.locked {
            return Err(midnight_core::archive_password_required(Path::new(
                "source.zip",
            )));
        }
        self.inner.read(entry, offset, length, cancel)
    }
}

/// A Copy out of an archive whose password prompt was declined goes on: it never asks
/// about a member that needs the password, skips it with that reason, copies the other
/// members and ends Completed with errors (the `viewers_passwords` acceptance journey).
#[tokio::test]
async fn a_copy_skips_members_that_need_a_password_that_was_not_entered() {
    let temp = fixture();
    let memory = mount(vec![
        node(2, 1, b"first.txt", EntryKind::File),
        node(3, 1, b"locked.txt", EntryKind::File),
        node(4, 1, b"last.txt", EntryKind::File),
    ]);
    let mut request = input(memory.clone(), temp.path());
    request.location.mount = Some(Arc::new(LockedMount {
        inner: memory,
        locked: 3,
    }));
    let mut h = Harness::native();
    h.controller.enqueue_archive(request).await;
    let done = loop {
        match h.next().await {
            CopyEvent::Failure(failure) => {
                panic!("the copy asked about a member: {}", failure.error)
            }
            CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
            _ => {}
        }
    };
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.files_completed, done.skipped), (2, 1), "{done:?}");
    assert!(
        done.errors
            .iter()
            .any(midnight_core::is_archive_password_required),
        "the skipped member keeps its reason: {done:?}"
    );
    assert_eq!(fs::read(temp.path().join("first.txt")).unwrap(), b"payload");
    assert_eq!(fs::read(temp.path().join("last.txt")).unwrap(), b"payload");
    assert!(!temp.path().join("locked.txt").exists());
    h.close().await;
}
fn mount(nodes: Vec<ArchiveNode>) -> Arc<MemoryMount> {
    Arc::new(MemoryMount {
        nodes,
        payload: b"payload".to_vec(),
        changed: AtomicBool::new(false),
    })
}

/// Preserve times and attributes gives copied members, folders included, the modified
/// time their archive records; without it they get the time of the copy.
#[tokio::test]
async fn preserving_gives_copied_members_their_recorded_modified_time() {
    let recorded = |seconds: i64, nanoseconds: u32| {
        (
            Some(ArchiveTimestamp::Utc {
                seconds,
                nanoseconds,
            }),
            std::time::UNIX_EPOCH + Duration::new(seconds as u64, nanoseconds),
        )
    };
    let (file_time, file_expected) = recorded(1_049_522_828, 765_432_100);
    let (folder_time, folder_expected) = recorded(981_173_106, 0);
    let (inner_time, inner_expected) = recorded(1_100_000_000, 500_000_000);
    for metadata in [
        TransferMetadataPolicy::PreserveTimesAndAttributes,
        TransferMetadataPolicy::ContentOnly,
    ] {
        let temp = fixture();
        let mut dated = node(2, 1, b"dated", EntryKind::File);
        dated.modified = file_time;
        let mut folder = node(3, 1, b"folder", EntryKind::Directory);
        folder.modified = folder_time;
        let mut inner = node(4, 3, b"inner", EntryKind::File);
        inner.modified = inner_time;
        let mut request = input(mount(vec![dated, folder, inner]), temp.path());
        request.metadata = metadata;
        let mut h = Harness::native();
        h.controller.enqueue_archive(request).await;
        let done = h.terminal(1).await;
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert_eq!(done.metadata_not_preserved, 0, "{done:?}");
        let time = |path: &Path| fs::metadata(path).unwrap().modified().unwrap();
        let preserved = metadata == TransferMetadataPolicy::PreserveTimesAndAttributes;
        for (path, expected) in [
            (temp.path().join("dated"), file_expected),
            (temp.path().join("folder"), folder_expected),
            (temp.path().join("folder").join("inner"), inner_expected),
        ] {
            assert_eq!(time(&path) == expected, preserved, "{path:?}");
        }
        assert_eq!(fs::read(temp.path().join("dated")).unwrap(), b"payload");
        h.close().await;
    }
}

/// Required modified time stays refused for archive members.
#[tokio::test]
async fn required_modified_time_is_refused_for_archive_members() {
    let temp = fixture();
    let mut request = input(
        mount(vec![node(2, 1, b"member", EntryKind::File)]),
        temp.path(),
    );
    request.metadata = TransferMetadataPolicy::RequireModifiedTime;
    let mut h = Harness::native();
    h.controller.enqueue_archive(request).await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
    assert!(!temp.path().join("member").exists());
    h.close().await;
}

#[tokio::test]
async fn queued_archive_copy_preserves_sample_verification_choice() {
    let temp = fixture();
    let mut file = node(2, 1, b"file", EntryKind::File);
    file.size = Some(400_000);
    let archive = Arc::new(MemoryMount {
        nodes: vec![file],
        payload: vec![23; 400_000],
        changed: AtomicBool::new(false),
    });
    let mut request = input(archive, temp.path());
    request.verification = TransferVerification::SizeAndSamples;
    let mut harness = Harness::native();
    harness.controller.enqueue_archive(request).await;
    let done = harness.terminal(1).await;
    harness.close().await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.copied_bytes, 400_000);
    assert_eq!(done.verification, TransferVerification::SizeAndSamples);
    assert_eq!(done.verified_bytes, 3 * TRANSFER_SAMPLE_BYTES as u64);
    assert_eq!(
        fs::read(temp.path().join("file")).unwrap(),
        vec![23; 400_000]
    );
}

#[test]
fn invalidated_archive_does_not_report_completed_root() {
    let temp = fixture();
    let mount = mount(vec![node(2, 1, b"file", EntryKind::File)]);
    let input = input(mount.clone(), temp.path());
    let cancel = Cancellation::default();
    let roots = archive_source::resolve(&input, &cancel).unwrap();
    let status = Arc::new(Mutex::new(initial(&input.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare_archive(temp.path().into(), roots, &service, cancel, status).unwrap();
    mount.changed.store(true, Ordering::Release);
    for _ in 0..100 {
        let step = job.step();
        assert!(job.take_completed_root().is_none());
        if let worker::Step::Failure(ref failure) = step {
            assert!(!failure.can_retry);
            assert!(job.accept_decision(CopyDecision {
                id: failure.id,
                version: failure.version,
                choice: CopyChoice::Skip,
            }));
        }
        if matches!(step, worker::Step::Done) {
            assert!(job.finish().failed > 0);
            assert!(!temp.path().join("file").exists());
            return;
        }
    }
    panic!("failed archive did not finish");
}

#[tokio::test]
async fn completed_archive_root_retains_provider_identity() {
    let temp = fixture();
    let archive = mount(vec![
        node(2, 1, b"folder", EntryKind::Directory),
        node(3, 2, b"child", EntryKind::File),
        node(4, 1, b"file", EntryKind::File),
    ]);
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(archive, temp.path()))
        .await;
    let mut completed = Vec::new();
    loop {
        match h.next().await {
            CopyEvent::RootsCompleted { sources, .. } => {
                for source in sources {
                    let CompletedCopySource::Archive(id) = source else {
                        panic!("unexpected native identity")
                    };
                    completed.push(id);
                }
            }
            CopyEvent::Progress(p) if p.state.is_terminal() => {
                assert_eq!(p.state, CopyOperationState::Completed);
                break;
            }
            CopyEvent::Conflict(_) => panic!("unexpected conflict"),
            _ => {}
        }
    }
    assert_eq!(
        completed,
        vec![
            VirtualEntryId {
                mount: 77,
                entry: 2
            },
            VirtualEntryId {
                mount: 77,
                entry: 4
            }
        ]
    );
    assert_eq!(
        fs::read(temp.path().join("folder/child")).unwrap(),
        b"payload"
    );
    h.close().await;
}

#[tokio::test]
async fn hidden_fallback_and_entirely_hidden_selection_never_extract() {
    let temp = fixture();
    for selected in [false, true] {
        let archive = mount(vec![node(2, 1, b"hidden", EntryKind::File)]);
        let mut request = input(archive, temp.path());
        request.view = Arc::new(midnight_core::DirectoryIndex::from_entries(&[]));
        request.fallback = Some(EntryId(2));
        if !selected {
            request.selected = Arc::new(HashSet::new());
        }
        let mut h = Harness::native();
        h.controller.enqueue_archive(request).await;
        let done = h.terminal(1).await;
        assert_eq!(done.state, CopyOperationState::Failed, "{done:?}");
        assert!(!temp.path().join("hidden").exists());
        h.close().await;
    }
}

#[tokio::test]
async fn recursive_archive_copy_uses_independent_verified_native_publication() {
    let temp = fixture();
    let archive = mount(vec![
        node(2, 1, "日本語".as_bytes(), EntryKind::Directory),
        node(3, 2, "čau.txt".as_bytes(), EntryKind::File),
    ]);
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(archive, temp.path()))
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!((done.copied_bytes, done.verified_bytes), (7, 7));
    assert_eq!(done.directories_created, 1);
    assert_eq!(
        fs::read(temp.path().join("日本語/čau.txt")).unwrap(),
        b"payload"
    );
    h.close().await;
}

#[tokio::test]
async fn duplicate_archive_names_keep_first_file_and_require_conflict_decision() {
    let temp = fixture();
    let archive = mount(vec![
        node(2, 1, b"same", EntryKind::File),
        node(3, 1, b"same", EntryKind::File),
    ]);
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(archive, temp.path()))
        .await;
    let conflict = h.conflict(1).await;
    assert_eq!(fs::read(temp.path().join("same")).unwrap(), b"payload");
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Skip,
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.files_completed, 1);
    assert_eq!(done.skipped, 1);
    assert_eq!(done.verified_bytes, 7);
    h.close().await;
}

#[tokio::test]
async fn archive_copy_rejects_traversal_links_changed_mount_and_stale_selection() {
    for (name, kind) in [
        (b"../outside".as_slice(), EntryKind::File),
        (b"link", EntryKind::Symlink),
    ] {
        let temp = fixture();
        let mut h = Harness::native();
        h.controller
            .enqueue_archive(input(mount(vec![node(2, 1, name, kind)]), temp.path()))
            .await;
        assert_eq!(h.terminal(1).await.state, CopyOperationState::Failed);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        h.close().await;
    }
    let temp = fixture();
    let archive = mount(vec![node(2, 1, b"safe", EntryKind::File)]);
    let mut h = Harness::native();
    let mut request = input(archive.clone(), temp.path());
    request.selected = Arc::new(HashSet::from([EntryId(999)]));
    h.controller.enqueue_archive(request).await;
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Failed);
    archive.changed.store(true, Ordering::Release);
    let mut request = input(archive, temp.path());
    request.id = OperationId(2);
    h.controller.enqueue_archive(request).await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Failed);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    h.close().await;
}

#[tokio::test]
async fn archive_copy_shares_admission_capacity_and_cancellation() {
    let temp = fixture();
    let archive = mount(vec![node(2, 1, b"safe", EntryKind::File)]);
    let mut h = Harness::native();
    let reservations: Vec<_> = (0..32).map(|_| h.controller.reserve().unwrap()).collect();
    h.controller
        .enqueue_archive(input(archive.clone(), temp.path()))
        .await;
    assert_eq!(
        h.terminal(1).await.errors[0].kind,
        io::ErrorKind::WouldBlock
    );
    drop(reservations);
    h.wait_capacity(MAX_JOBS).await;
    let gate = h
        .mutation
        .clone()
        .acquire_many_owned(crate::transfer_leases::NAMESPACE_WEIGHT)
        .await
        .unwrap();
    let mut request = input(archive, temp.path());
    request.id = OperationId(2);
    let ready = h.reserve_ready().await;
    h.enqueue_reserved_input(request.into(), ready).await;
    h.controller.cancel(OperationId(2));
    drop(gate);
    let cancelled = h.terminal(2).await;
    assert_eq!(
        cancelled.state,
        CopyOperationState::Cancelled,
        "{cancelled:?}"
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    h.close().await;
}

#[derive(Debug)]
struct ReidentifiedMount {
    inner: Arc<MemoryMount>,
    identity: u64,
}
impl ArchiveMount for ReidentifiedMount {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        if let Some(owner) = Arc::into_inner(self) {
            return owner.inner.release_on_worker();
        }
        Ok(())
    }
    fn id(&self) -> u64 {
        self.identity
    }
    fn validate(&self) -> Result<(), FsError> {
        self.inner.validate()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.inner.list(directory, cancel).map(|nodes| {
            nodes
                .into_iter()
                .map(|mut node| {
                    node.id.mount = self.identity;
                    node
                })
                .collect()
        })
    }
    fn read(
        &self,
        member: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.inner.read(member, offset, length, cancel)
    }
}
#[test]
fn captured_archive_roots_share_budget_but_keep_full_mount_identity() {
    let temp = fixture();
    let first = mount(vec![node(2, 1, b"first", EntryKind::Directory)]);
    let second = Arc::new(ReidentifiedMount {
        inner: mount(vec![node(2, 1, b"second", EntryKind::Directory)]),
        identity: 88,
    });
    let mut first_location = input(first, temp.path()).location;
    let mut second_location = first_location.clone();
    second_location.mount = Some(second.clone());
    first_location.directory = 2;
    second_location.directory = 2;
    let first_node = first_location
        .mount
        .as_ref()
        .unwrap()
        .list(1, &Cancellation::default())
        .unwrap()
        .remove(0);
    let second_node = second.list(1, &Cancellation::default()).unwrap().remove(0);
    let first_target = crate::providers::ArchiveTarget {
        location: first_location,
        member: None,
        original: Some(first_node),
    };
    let second_target = crate::providers::ArchiveTarget {
        location: second_location,
        member: None,
        original: Some(second_node),
    };
    let budget = archive_source::traversal();
    let first =
        archive_source::captured_root(&first_target, budget.clone(), &Cancellation::default())
            .unwrap();
    let second =
        archive_source::captured_root(&second_target, budget, &Cancellation::default()).unwrap();
    assert_eq!(
        first.virtual_identity(),
        Some(VirtualEntryId {
            mount: 77,
            entry: 2
        })
    );
    assert_eq!(
        second.virtual_identity(),
        Some(VirtualEntryId {
            mount: 88,
            entry: 2
        })
    );
}

/// `count` seven-byte files in the archive's root, or (`folder`) in its folder `flat`.
fn many(count: u64, folder: bool) -> Arc<MemoryMount> {
    let parent = if folder { 2 } else { 1 };
    let mut nodes = Vec::with_capacity(count as usize + 1);
    if folder {
        nodes.push(node(2, 1, b"flat", EntryKind::Directory));
    }
    nodes.extend((0..count).map(|index| {
        node(
            index + 3,
            parent,
            format!("{index:06}.txt").as_bytes(),
            EntryKind::File,
        )
    }));
    mount(nodes)
}

fn perf_files() -> u64 {
    std::env::var("MIDNIGHT_PERF_FILES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_001)
}

/// Manual measurement (WIN-8): the copy actor over an in-memory archive, with a UI
/// that takes every event at once. Run with TEMP on the volume to measure:
/// `cargo test -p midnight-engine --lib archive_copy_rate_through_the_copy_actor -- --ignored --nocapture --test-threads=1`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual measurement: copies MIDNIGHT_PERF_FILES archive members"]
async fn archive_copy_rate_through_the_copy_actor() {
    let count = perf_files();
    for folder in [true, false] {
        let temp = fixture();
        let mut h = Harness::native();
        let started = std::time::Instant::now();
        h.controller
            .enqueue_archive(input(many(count, folder), temp.path()))
            .await;
        let (mut events, mut notices) = (0u64, 0u64);
        let progress = loop {
            events += 1;
            match h.next().await {
                CopyEvent::RootsCompleted { sources, .. } => notices += sources.len() as u64,
                CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
                CopyEvent::Conflict(_) | CopyEvent::Failure(_) => panic!("the copy stopped"),
                _ => {}
            }
        };
        let elapsed = started.elapsed();
        assert_eq!(progress.files_completed, count, "{progress:?}");
        println!(
            "PERF actor {}: {count} files in {:.2} s = {:.0} files/s, {events} events, \
             {notices} root notices",
            if folder { "folder" } else { "selection" },
            elapsed.as_secs_f64(),
            count as f64 / elapsed.as_secs_f64(),
        );
        h.close().await;
    }
}

/// Manual measurement (WIN-8): Cancel of a selection whose UI took no events while it
/// was copied, as a UI that cannot keep up. Counts what reaches the UI before the
/// terminal progress, and the files written after Cancel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual measurement: copies up to 6,000 archive members"]
async fn archive_selection_cancel_while_the_ui_takes_no_events() {
    let temp = fixture();
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(many(6_000, false), temp.path()))
        .await;
    let files = || fs::read_dir(temp.path()).unwrap().count();
    // Until the copy has 4,500 files, or stalls because nothing takes its notices.
    let (mut last, mut still) = (0, std::time::Instant::now());
    let started = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let now = files();
        assert!(
            started.elapsed() < Duration::from_secs(600),
            "the copy did not start"
        );
        if now >= 4_500 || (now > 0 && now == last && still.elapsed() > Duration::from_secs(5)) {
            break;
        }
        if now != last {
            (last, still) = (now, std::time::Instant::now());
        }
    }
    let at_cancel = files();
    let cancelled = std::time::Instant::now();
    h.controller.cancel(OperationId(1));
    let (mut events, mut notices) = (0u64, 0u64);
    let progress = loop {
        events += 1;
        match h.next().await {
            CopyEvent::RootsCompleted { sources, .. } => notices += sources.len() as u64,
            CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
            _ => {}
        }
    };
    println!(
        "PERF cancel: {at_cancel} files when cancelled, {} at the end; the terminal progress \
         came after {events} events ({notices} root notices), {:.0} ms after Cancel",
        files(),
        cancelled.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(progress.state, CopyOperationState::Cancelled);
    h.close().await;
}

/// The published members' names (a member's private stage and the job's staging folder
/// excluded).
fn published(folder: &Path) -> std::collections::BTreeSet<String> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with(".me-") && !name.starts_with(".midnight-stage-"))
        .collect()
}

/// Cancel stops the copy itself at once (WIN-8): no member is written once the job has
/// observed Cancel, however many selected roots remain, and the job ends within a bound.
#[test]
fn a_cancelled_selection_writes_no_member_after_cancel() {
    let temp = fixture();
    let input = input(many(2_000, false), temp.path());
    let cancel = Cancellation::default();
    let roots = archive_source::resolve(&input, &cancel).unwrap();
    let status = Arc::new(Mutex::new(initial(&input.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job = worker::Job::prepare_archive(
        temp.path().into(),
        roots,
        &service,
        cancel.clone(),
        status.clone(),
    )
    .unwrap();
    while status.lock().unwrap().files_completed < 20 {
        assert!(matches!(job.step(), worker::Step::Continue));
    }
    // Midway through the next member: its stage exists, nothing more is published.
    assert!(matches!(job.step(), worker::Step::Continue));
    cancel.cancel();
    let before = published(temp.path());
    let cancelled = std::time::Instant::now();
    while !matches!(job.step(), worker::Step::Done) {}
    let progress = job.finish();
    let ended = cancelled.elapsed();
    assert_eq!(
        published(temp.path()),
        before,
        "a member was written after Cancel"
    );
    assert_eq!(progress.files_completed, before.len() as u64);
    assert!(
        ended < Duration::from_secs(2),
        "the job ended {ended:?} after Cancel, with 1,979 roots left"
    );
}

/// A UI that takes no events while a selection is copied, as the application's while it
/// refreshes a panel of 100,001 members (WIN-8): after Cancel, the job's end reaches it
/// after a few events, not after one notice per copied file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_selection_reaches_a_waiting_ui_after_a_few_events() {
    const UI_EVENTS: usize = 32;
    let temp = fixture();
    let mut h = Harness::native_with_events(UI_EVENTS);
    h.controller
        .enqueue_archive(input(many(3_000, false), temp.path()))
        .await;
    let started = std::time::Instant::now();
    while published(temp.path()).len() < 600 {
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "600 files were not copied"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    h.controller.cancel(OperationId(1));
    let (mut events, mut roots) = (0, 0);
    let progress = loop {
        match h.next().await {
            CopyEvent::RootsCompleted { sources, .. } => {
                events += 1;
                roots += sources.len();
            }
            CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
            _ => events += 1,
        }
    };
    assert_eq!(progress.state, CopyOperationState::Cancelled);
    assert_eq!(
        roots as u64, progress.completed_roots,
        "every copied root was announced"
    );
    assert!(roots >= 600, "{roots} roots announced");
    // The UI's channel, plus the waiting notices joined, plus a few progress updates.
    let bound = UI_EVENTS + roots.div_ceil(delivery::MAX_ROOTS_PER_NOTICE) + 8;
    assert!(
        events <= bound,
        "{events} events (for {roots} copied roots) came before the job's end; at most {bound}"
    );
    h.close().await;
}

/// What interrupts a batch of steps while one is in flight (`Job::steps`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Interrupt {
    /// Nothing: the batch runs on to its boundary (the positive control).
    None,
    /// The copy actor stops awaiting the call: aborted or dropped (`Awaited`).
    Abort,
    Cancel,
    Pause,
}

/// A mount that runs its hook as the first member read is made, inside a step.
struct HookedMount {
    inner: Arc<MemoryMount>,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl std::fmt::Debug for HookedMount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("HookedMount")
    }
}
impl ArchiveMount for HookedMount {
    fn id(&self) -> u64 {
        self.inner.id()
    }
    fn validate(&self) -> Result<(), FsError> {
        self.inner.validate()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.inner.list(directory, cancel)
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        let hook = self.hook.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        self.inner.read(entry, offset, length, cancel)
    }
}

/// One `Job::steps` call over a selection of three small members, interrupted as the
/// step that reads the first member runs. Returns the count of steps begun when that
/// read ran (its step is the one in flight), the count when the call returned, and the
/// members published by then.
fn interrupted_batch(interrupt: Interrupt) -> (u64, u64, usize) {
    let temp = fixture();
    let memory = many(3, false);
    let mut request = input(memory.clone(), temp.path());
    let cancel = Cancellation::default().pausable_child();
    let awaited = Arc::new(Mutex::new(Some(worker::Awaited::new())));
    let flag = awaited.lock().unwrap().as_ref().unwrap().flag();
    let in_flight = Arc::new(AtomicU64::new(0));
    let hook: Box<dyn FnOnce() + Send> = {
        let (cancel, awaited, in_flight) = (cancel.clone(), awaited.clone(), in_flight.clone());
        Box::new(move || {
            in_flight.store(worker::STEPS.with(std::cell::Cell::get), Ordering::Release);
            match interrupt {
                Interrupt::None => {}
                Interrupt::Abort => drop(awaited.lock().unwrap().take()),
                Interrupt::Cancel => cancel.cancel(),
                Interrupt::Pause => cancel.set_pause_requested(true),
            }
        })
    };
    request.location.mount = Some(Arc::new(HookedMount {
        inner: memory,
        hook: Mutex::new(Some(hook)),
    }));
    let roots = archive_source::resolve(&request, &cancel).unwrap();
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job = worker::Job::prepare_archive(
        temp.path().into(),
        roots,
        &service,
        cancel.clone(),
        status.clone(),
    )
    .unwrap();
    // A limit no batch reaches: only the boundaries under test end the call.
    job.steps_within(&flag, Duration::from_secs(600));
    let ended = worker::STEPS.with(std::cell::Cell::get);
    let published = published(temp.path()).len();
    cancel.cancel();
    while !matches!(job.step(), worker::Step::Done) {}
    drop(job.finish());
    (in_flight.load(Ordering::Acquire), ended, published)
}

/// Without an interrupt a batch runs past the step that reads a small member, to the
/// member's completed root: the control for the three tests below.
#[test]
fn a_batch_of_steps_runs_a_small_member_to_its_completed_root() {
    let (in_flight, ended, published) = interrupted_batch(Interrupt::None);
    assert!(
        ended > in_flight,
        "the batch stopped at the read's step {in_flight}"
    );
    assert_eq!(published, 1);
}

/// An aborted copy actor (its `Awaited` dropped while a step runs) ends the batch after
/// that step: no later step runs, and nothing is published.
#[test]
fn an_aborted_actor_ends_a_batch_after_the_step_in_flight() {
    let (in_flight, ended, published) = interrupted_batch(Interrupt::Abort);
    assert_eq!(ended, in_flight, "a step ran after the one in flight");
    assert_eq!(published, 0);
}

/// Cancel and pause end a batch at the next step boundary.
#[test]
fn cancel_and_pause_end_a_batch_at_the_next_step_boundary() {
    for interrupt in [Interrupt::Cancel, Interrupt::Pause] {
        let (in_flight, ended, published) = interrupted_batch(interrupt);
        assert_eq!(
            ended, in_flight,
            "{interrupt:?}: a step ran after the one in flight"
        );
        assert_eq!(published, 0, "{interrupt:?}");
    }
}

/// Manual measurement (WIN-8): pause and Cancel latency of a selection copy through the
/// copy actor while its steps run in batches of at most `STEP_BUDGET`.
/// `cargo test -p midnight-engine --lib archive_copy_pause_and_cancel_latency -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual measurement: copies up to 3,000 archive members"]
async fn archive_copy_pause_and_cancel_latency() {
    let temp = fixture();
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(many(3_000, false), temp.path()))
        .await;
    let files = || published(temp.path()).len();
    while files() < 500 {
        h.next().await;
    }
    let requested = std::time::Instant::now();
    h.controller.set_paused(OperationId(1), true).await;
    let paused = loop {
        if let CopyEvent::Progress(update) = h.next().await
            && update.paused
        {
            break requested.elapsed();
        }
    };
    let at_pause = files();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let while_paused = files() - at_pause;
    h.controller.set_paused(OperationId(1), false).await;
    while files() < 1_500 {
        h.next().await;
    }
    let requested = std::time::Instant::now();
    h.controller.cancel(OperationId(1));
    let done = loop {
        if let CopyEvent::Progress(update) = h.next().await
            && update.state.is_terminal()
        {
            break update;
        }
    };
    let cancelled = requested.elapsed();
    println!(
        "PERF pause: the paused progress {:.1} ms after the request; {while_paused} files \
         in the 500 ms after it. Cancel: {:?} {:.1} ms after the request",
        paused.as_secs_f64() * 1000.0,
        done.state,
        cancelled.as_secs_f64() * 1000.0
    );
    h.close().await;
}

/// The job's private staging folders (`.midnight-stage-*`) directly in `folder`, none
/// while `folder` does not exist yet. Only Windows stages in such a folder; Linux keeps
/// staging beside the destination (`TransferDirectory::stage_job_file`'s default).
#[cfg(windows)]
fn staging_folders(folder: &Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".midnight-stage-")
        })
        .map(|entry| entry.path())
        .collect()
}

/// The staging folder of the job copying into `folder`, once there is one.
#[cfg(windows)]
async fn staging_in(folder: &Path) -> std::path::PathBuf {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let [staging] = staging_folders(folder).as_slice() {
            return staging.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no staging folder appeared in {}",
            folder.display()
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// WIN-8: a Copy that reads the folder another copy stages in waits for that copy as a
/// whole, as before its stages moved into a staging folder there, and then copies what it
/// published: never the staging folder.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_reading_the_folder_an_archive_copy_stages_in_waits_for_the_whole_copy() {
    let temp = fixture();
    let output = temp.path().join("output");
    let copied = temp.path().join("copied");
    fs::create_dir(&output).unwrap();
    fs::create_dir(&copied).unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(many(300, false), &output))
        .await;
    let staging = staging_in(&output).await;
    h.controller.set_paused(OperationId(1), true).await;
    h.wait_status(1, |progress| progress.paused).await;
    assert!(staging.is_dir(), "the paused copy keeps its staging folder");
    h.controller
        .enqueue(request(2, std::slice::from_ref(&output), &copied))
        .await;
    let waiting = h
        .wait_status(2, |progress| {
            matches!(
                progress.wait_reason,
                Some(TransferWaitReason::SameFolders | TransferWaitReason::UnconfirmedFolders)
            )
        })
        .await;
    assert_ne!(waiting.state, CopyOperationState::Running, "{waiting:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let still = h.wait_status(2, |_| true).await;
    assert!(still.wait_reason.is_some(), "{still:?}");
    assert_eq!(still.files_completed, 0);
    assert!(!copied.join("output").exists());
    h.controller.set_paused(OperationId(1), false).await;
    let first = h.terminal(1).await;
    assert_eq!(first.state, CopyOperationState::Completed, "{first:?}");
    assert_eq!(first.files_completed, 300);
    let second = h.terminal(2).await;
    assert_eq!(second.state, CopyOperationState::Completed, "{second:?}");
    assert_eq!(second.files_completed, 300);
    assert!(!staging.exists());
    let names = |folder: &Path| {
        let mut names: Vec<_> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&copied.join("output")), names(&output));
    assert_eq!(names(&output).len(), 300);
    h.close().await;
}

/// WIN-8: a paused copy keeps its staging folder, and once resumed it stages its other
/// files in that same folder; the folder goes when the copy ends.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_archive_copy_keeps_its_staging_folder_and_resumes_in_it() {
    let temp = fixture();
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(many(400, false), temp.path()))
        .await;
    let staging = staging_in(temp.path()).await;
    h.controller.set_paused(OperationId(1), true).await;
    h.wait_status(1, |progress| progress.paused).await;
    let at_pause = published(temp.path()).len();
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(staging_folders(temp.path()), std::slice::from_ref(&staging));
    }
    assert_eq!(
        published(temp.path()).len(),
        at_pause,
        "nothing is copied while paused"
    );
    // Every staging folder seen until the copy ends: only the one from before the pause,
    // still there while the resumed copy publishes more files.
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (folder, staging, stop) = (temp.path().to_owned(), staging.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut seen = std::collections::BTreeSet::from([staging.clone()]);
            let mut resumed = 0;
            while !stop.load(Ordering::Acquire) {
                seen.extend(staging_folders(&folder));
                if published(&folder).len() > at_pause && staging.is_dir() {
                    resumed += 1;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            (seen, resumed)
        })
    };
    h.controller.set_paused(OperationId(1), false).await;
    let done = h.terminal(1).await;
    stop.store(true, Ordering::Release);
    let (seen, resumed) = watcher.join().unwrap();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert!(
        resumed > 0,
        "the resumed copy published beside the kept staging folder"
    );
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(staging_folders(temp.path()).is_empty());
    assert_eq!(published(temp.path()).len(), 400);
    h.close().await;
}

/// WIN-8: Cancel removes the unpublished stage and then the empty staging folder; the
/// files copied before it stay.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_archive_copy_leaves_neither_stages_nor_its_staging_folder() {
    let temp = fixture();
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(many(2_000, false), temp.path()))
        .await;
    let staging = staging_in(temp.path()).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while published(temp.path()).len() < 10 {
        assert!(
            std::time::Instant::now() < deadline,
            "10 files were not copied"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    h.controller.cancel(OperationId(1));
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert!(!staging.exists());
    let left = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        left.iter().all(|name| !name.starts_with('.')),
        "a stage or staging folder remains: {left:?}"
    );
    assert_eq!(left.len() as u64, done.files_completed);
    h.close().await;
}

/// WIN-8: each copied folder has a staging folder of its own while its files are staged,
/// removed once the folder is copied; the destination keeps its own until the job's end.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn copied_folders_stage_in_folders_of_their_own_removed_with_each_folder() {
    let temp = fixture();
    let mut nodes = vec![
        node(2, 1, b"first", EntryKind::Directory),
        node(3, 1, b"second", EntryKind::Directory),
    ];
    for index in 0..60 {
        let parent = if index % 2 == 0 { 2 } else { 3 };
        nodes.push(node(
            index + 10,
            parent,
            format!("{index:03}.txt").as_bytes(),
            EntryKind::File,
        ));
    }
    nodes.push(node(100, 1, b"root.txt", EntryKind::File));
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(mount(nodes), temp.path()))
        .await;
    let first = staging_in(&temp.path().join("first")).await;
    // While the first folder's files are staged, its destination holds no staging folder.
    assert!(staging_folders(temp.path()).is_empty());
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 61);
    assert!(!first.exists());
    for folder in [
        temp.path().to_owned(),
        temp.path().join("first"),
        temp.path().join("second"),
    ] {
        assert!(staging_folders(&folder).is_empty(), "{}", folder.display());
    }
    assert_eq!(published(&temp.path().join("first")).len(), 30);
    assert_eq!(published(&temp.path().join("second")).len(), 30);
    assert_eq!(fs::read(temp.path().join("root.txt")).unwrap(), b"payload");
    h.close().await;
}

/// Manual measurement (WIN-8): Cancel of a selection of 100,001 archive members early in
/// its copy, with a UI that takes every event at once: how long the job takes to start
/// writing, and to end after Cancel.
/// `cargo test -p midnight-engine --lib archive_selection_of_100001_cancelled_early -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual measurement: selects 100,001 archive members"]
async fn archive_selection_of_100001_cancelled_early() {
    let temp = fixture();
    let mut h = Harness::native();
    let enqueued = std::time::Instant::now();
    h.controller
        .enqueue_archive(input(many(100_001, false), temp.path()))
        .await;
    let files = || published(temp.path()).len();
    while files() == 0 {
        h.next().await;
    }
    let first = enqueued.elapsed();
    while files() < 200 {
        h.next().await;
    }
    let requested = std::time::Instant::now();
    h.controller.cancel(OperationId(1));
    let mut events = 0;
    let done = loop {
        events += 1;
        if let CopyEvent::Progress(update) = h.next().await
            && update.state.is_terminal()
        {
            break update;
        }
    };
    println!(
        "PERF 100,001-member selection: first file {:.1} s after the enqueue; Cancel at {} \
         files ended it ({:?}) {:.0} ms after the request, after {events} events; {} files \
         at the end",
        first.as_secs_f64(),
        done.files_completed,
        done.state,
        requested.elapsed().as_secs_f64() * 1000.0,
        files()
    );
    h.close().await;
}

/// WIN-8: a selection of `members` archive files into a folder where `000010.txt` already
/// exists. The job copies ten files and asks about the 11th; `meanwhile` then changes the
/// folder, and the answer is Cancel. Returns the job's end, what its close probed
/// (`copy_namespace::Claim::revalidate_before_close`) and how long after the answer the
/// job ended.
async fn answered_cancel_at_the_eleventh(
    members: u64,
    meanwhile: impl FnOnce(&Path),
) -> (
    Arc<CopyProgress>,
    crate::copy_namespace::ClosingProbe,
    Duration,
) {
    let temp = fixture();
    fs::write(temp.path().join("000010.txt"), b"kept").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(many(members, false), temp.path()))
        .await;
    let conflict = loop {
        if let CopyEvent::Conflict(conflict) = h.next().await {
            break conflict;
        }
    };
    assert_eq!(
        published(temp.path()).len(),
        11,
        "ten copies beside the existing file"
    );
    meanwhile(temp.path());
    let answered = std::time::Instant::now();
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    let done = loop {
        if let CopyEvent::Progress(progress) = h.next().await
            && progress.state.is_terminal()
        {
            break progress;
        }
    };
    let ended = answered.elapsed();
    let mut closes = h.controller.namespace().closes_for_test();
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.files_completed, 10);
    assert_eq!(closes.len(), 1, "{closes:?}");
    (done, closes.remove(0), ended)
}

/// The destination pins of the first `members` selected archive members: a copy plans a
/// (source, destination) pair per selected item, and an archive member has no source pin.
fn destination_pins(members: usize) -> Vec<usize> {
    (0..members).map(|member| 2 * member + 1).collect()
}

/// WIN-8: a job stopped by Cancel refreshes, as it closes, the pins of the 11 members it
/// took (ten copied and the one it asked about) and its private pins, not those of the
/// whole selection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_selection_closes_with_only_the_members_it_took() {
    let (_, close, _) = answered_cancel_at_the_eleventh(2_000, |_| {}).await;
    assert_eq!(close.roots, destination_pins(11));
    assert_eq!(close.refreshed, 11 + close.aliases, "{close:?}");
}

/// Manual measurement (WIN-8): the same for a selection of 100,001 members.
/// `cargo test -p midnight-engine --lib a_cancelled_selection_of_100001_closes -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual measurement: selects 100,001 archive members"]
async fn a_cancelled_selection_of_100001_closes_with_only_the_members_it_took() {
    let (_, close, ended) = answered_cancel_at_the_eleventh(100_001, |_| {}).await;
    println!(
        "PERF 100,001-member selection cancelled at its 11th member: its close refreshed {} \
         pins ({} roots, {} aliases) in {:.1} ms; the job ended {:.0} ms after the answer",
        close.refreshed,
        close.roots.len(),
        close.aliases,
        close.elapsed.as_secs_f64() * 1000.0,
        ended.as_secs_f64() * 1000.0
    );
    assert_eq!(close.roots, destination_pins(11));
    assert_eq!(close.refreshed, 11 + close.aliases, "{close:?}");
}

/// WIN-8: another program renamed a copied file before the job was cancelled: as the job
/// closes, that file's pin is still probed and the change is reported.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_to_a_copied_member_is_still_reported_as_a_cancelled_job_closes() {
    let (done, close, _) = answered_cancel_at_the_eleventh(2_000, |folder| {
        fs::rename(folder.join("000003.txt"), folder.join("renamed.txt")).unwrap();
    })
    .await;
    assert!(
        done.errors
            .iter()
            .any(|error| error.path.ends_with("000003.txt")),
        "{:?}",
        done.errors
    );
    assert_eq!(close.roots, destination_pins(11));
}

/// WIN-8: another program created the name of a member the job never took: the job ends
/// Cancelled as before, and its close neither probes that name nor reports the change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_to_a_member_never_taken_is_not_reported_as_a_cancelled_job_closes() {
    let (done, close, _) = answered_cancel_at_the_eleventh(2_000, |folder| {
        fs::write(folder.join("000500.txt"), b"another program").unwrap();
    })
    .await;
    assert!(done.errors.is_empty(), "{:?}", done.errors);
    assert_eq!(close.roots, destination_pins(11));
    assert_eq!(close.refreshed, 11 + close.aliases, "{close:?}");
}

/// An archive whose member `member` (an entry id) cannot be read.
#[derive(Debug)]
struct UnreadableMember {
    inner: Arc<MemoryMount>,
    member: u64,
}
impl ArchiveMount for UnreadableMember {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        if let Some(owner) = Arc::into_inner(self) {
            return owner.inner.release_on_worker();
        }
        Ok(())
    }
    fn id(&self) -> u64 {
        self.inner.id()
    }
    fn validate(&self) -> Result<(), FsError> {
        self.inner.validate()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.inner.list(directory, cancel)
    }
    fn read(
        &self,
        member: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        if member == self.member {
            return Err(error(
                Path::new("container"),
                io::ErrorKind::PermissionDenied,
                "Member unreadable",
            ));
        }
        self.inner.read(member, offset, length, cancel)
    }
}

/// WIN-8: a job stopped by a failure midway (its 11th member cannot be read, and the
/// answer is Cancel) probes, as it closes, the pins of the members it took: a change to a
/// copied one is reported, and one to a member it never took is neither probed nor
/// reported.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_job_stopped_by_a_failure_closes_with_only_the_members_it_took() {
    let temp = fixture();
    let archive = many(2_000, false);
    let mut request = input(archive.clone(), temp.path());
    // `many` gives member `index` the entry id `index + 3`.
    request.location.mount = Some(Arc::new(UnreadableMember {
        inner: archive,
        member: 13,
    }));
    let mut h = Harness::native();
    h.controller.enqueue_archive(request).await;
    let failure = loop {
        if let CopyEvent::Failure(failure) = h.next().await {
            break failure;
        }
    };
    assert_eq!(published(temp.path()).len(), 10);
    fs::rename(
        temp.path().join("000003.txt"),
        temp.path().join("renamed.txt"),
    )
    .unwrap();
    fs::write(temp.path().join("000500.txt"), b"another program").unwrap();
    h.controller
        .decide(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    let done = h.terminal(1).await;
    let closes = h.controller.namespace().closes_for_test();
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert_eq!(done.files_completed, 10);
    assert!(
        !done
            .errors
            .iter()
            .any(|error| error.path.ends_with("000500.txt")),
        "{:?}",
        done.errors
    );
    assert!(
        done.errors
            .iter()
            .any(|error| error.path.ends_with("000003.txt")),
        "{:?}",
        done.errors
    );
    assert_eq!(closes.len(), 1, "{closes:?}");
    assert_eq!(closes[0].roots, destination_pins(11));
}

/// WIN-8: a completed job took every member, so its close probes every pin, as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completed_selection_closes_with_every_member() {
    let temp = fixture();
    let mut h = Harness::native();
    h.controller
        .enqueue_archive(input(many(300, false), temp.path()))
        .await;
    let done = h.terminal(1).await;
    let closes = h.controller.namespace().closes_for_test();
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(closes.len(), 1, "{closes:?}");
    assert_eq!(closes[0].roots, destination_pins(300));
    assert_eq!(closes[0].refreshed, 300 + closes[0].aliases, "{closes:?}");
}

/// Manual measurement (WIN-8): a Copy of a selection of 100,001 archive members to its end,
/// with a UI that takes every event at once: how long the job takes to start writing, how
/// long a file takes across the copy (between the first and the last progress update that
/// count files, which come on a regular cadence), how long the job takes to end after that
/// last update, and how long its namespace close takes of it
/// (`copy_namespace::Claim::revalidate_before_close`).
/// `cargo test -p midnight-engine --lib archive_selection_of_100001_copied -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual measurement: copies 100,001 archive members"]
async fn archive_selection_of_100001_copied() {
    const MEMBERS: u64 = 100_001;
    let temp = fixture();
    let mut h = Harness::native();
    let enqueued = std::time::Instant::now();
    h.controller
        .enqueue_archive(input(many(MEMBERS, false), temp.path()))
        .await;
    let (mut first, mut last) = (None, None);
    let done = loop {
        if let CopyEvent::Progress(update) = h.next().await {
            if update.state.is_terminal() {
                break update;
            }
            if update.files_completed > 0 {
                let counted = (std::time::Instant::now(), update.files_completed);
                first.get_or_insert(counted);
                last = Some(counted);
            }
        }
    };
    let ended = std::time::Instant::now();
    let closes = h.controller.namespace().closes_for_test();
    h.close().await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, MEMBERS);
    assert_eq!(closes.len(), 1, "{closes:?}");
    let ((first, first_files), (last, last_files)) = (
        first.expect("an update counts files"),
        last.expect("an update counts files"),
    );
    println!(
        "PERF 100,001-member selection copied: first file {:.1} s after the enqueue; {:.3} ms \
         per file from {first_files} to {last_files} files ({:.1} s); the job ended {:.0} ms \
         after that; its close refreshed {} pins ({} roots, {} aliases) in {:.0} ms; {:.1} s \
         from the enqueue to the end",
        (first - enqueued).as_secs_f64(),
        (last - first).as_secs_f64() * 1000.0 / (last_files - first_files).max(1) as f64,
        (last - first).as_secs_f64(),
        (ended - last).as_secs_f64() * 1000.0,
        closes[0].refreshed,
        closes[0].roots.len(),
        closes[0].aliases,
        closes[0].elapsed.as_secs_f64() * 1000.0,
        (ended - enqueued).as_secs_f64()
    );
    assert_eq!(closes[0].roots, destination_pins(MEMBERS as usize));
}
#[path = "archive_copy_approximate_tests.rs"]
mod approximate;
#[path = "archive_copy_progress_tests.rs"]
mod progress;
