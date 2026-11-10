use super::*;
use std::collections::HashSet;

#[derive(Debug)]
struct MemoryMount {
    nodes: Vec<ArchiveNode>,
    payload: Vec<u8>,
    changed: AtomicBool,
}
impl ArchiveMount for MemoryMount {
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
        verification: midnight_core::TransferVerification::default(),
        id: OperationId(1),
        location: ArchiveLocation {
            mount: Some(mount),
            address: ArchiveAddress {
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
fn mount(nodes: Vec<ArchiveNode>) -> Arc<MemoryMount> {
    Arc::new(MemoryMount {
        nodes,
        payload: b"payload".to_vec(),
        changed: AtomicBool::new(false),
    })
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
            CopyEvent::RootCompleted {
                source: CompletedCopySource::Archive(id),
                ..
            } => completed.push(id),
            CopyEvent::Progress(p) if p.state.is_terminal() => {
                assert_eq!(p.state, CopyOperationState::Completed);
                break;
            }
            CopyEvent::RootCompleted { .. } | CopyEvent::Conflict(_) => {
                panic!("unexpected native identity/conflict")
            }
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
    let gate = h.mutation.clone().acquire_owned().await.unwrap();
    let mut request = input(archive, temp.path());
    request.id = OperationId(2);
    h.controller.enqueue_archive(request).await;
    h.controller.cancel(OperationId(2));
    drop(gate);
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Cancelled);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    h.close().await;
}

#[derive(Debug)]
struct ReidentifiedMount {
    inner: Arc<MemoryMount>,
    identity: u64,
}
impl ArchiveMount for ReidentifiedMount {
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
