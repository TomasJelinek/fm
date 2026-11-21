//! Copy-out takes archive folders and selections of any size: nothing limits how many
//! members it copies or how many bytes their names take, while a member met twice
//! still fails.
use super::*;
use crate::archive_shared::testing::SharedMount;

fn member(id: u64, parent: u64, kind: EntryKind, name: Vec<u8>) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId {
            mount: 62,
            entry: id,
        },
        parent,
        display_name: String::from_utf8_lossy(&name).into_owned(),
        raw_name: name.into(),
        kind,
        size: (kind == EntryKind::File).then_some(0),
        modified: None,
        readable: kind == EntryKind::File,
        access: Default::default(),
    }
}

/// The members, in listing order, that Copy takes from a folder listing `rows`.
fn copied(rows: Vec<ArchiveNode>) -> Result<Vec<u64>, FsError> {
    let mount = SharedMount::new(62);
    mount.insert(2, rows);
    let mut folder = Source::new(
        MountReference::new(Arc::new(mount)),
        member(2, 1, EntryKind::Directory, b"folder".to_vec()),
        0,
        traversal(),
    )?;
    let mut members = Vec::new();
    while let Some(child) = folder.next_child(&Cancellation::default())? {
        members.push(child.virtual_identity().map_or(0, |id| id.entry));
    }
    Ok(members)
}

#[test]
fn a_folder_beyond_the_former_copy_budgets_is_copied_completely() {
    // Copy once refused more than 100,000 members, and more than 8 MiB of names.
    let many = (3..100_004)
        .map(|id| member(id, 2, EntryKind::File, format!("m{id}").into_bytes()))
        .collect();
    assert_eq!(copied(many).unwrap(), (3..100_004).collect::<Vec<_>>());
    // 65 members whose raw names and labels take 64 KiB each: 8 MiB and 128 KiB.
    let named = (3..68)
        .map(|id| {
            let mut name = format!("{id:03}").into_bytes();
            name.resize(64 * 1024, b'n');
            member(id, 2, EntryKind::File, name)
        })
        .collect();
    assert_eq!(copied(named).unwrap(), (3..68).collect::<Vec<_>>());
}

#[test]
fn a_member_listed_twice_still_fails_the_copy() {
    let twice = vec![
        member(3, 2, EntryKind::File, b"a".to_vec()),
        member(3, 2, EntryKind::File, b"a".to_vec()),
    ];
    let error = copied(twice).unwrap_err();
    assert!(error.message.contains("cyclic"), "{error}");
}

/// One-byte files `m000002`, `m000003`, ... in the archive's root folder.
fn files(count: u64) -> (SharedMount, Vec<ArchiveNode>) {
    let mount = SharedMount::new(62);
    let rows: Vec<_> = (2..count + 2)
        .map(|id| ArchiveNode {
            size: Some(1),
            ..member(id, 1, EntryKind::File, format!("m{id:06}").into_bytes())
        })
        .collect();
    mount.insert(1, rows.clone());
    for id in 2..count + 2 {
        mount.insert_content(id, b"x");
    }
    (mount, rows)
}

/// Copy all of `rows`, as the panel captured them, from the archive's root folder.
fn request(
    mount: Arc<dyn ArchiveMount>,
    rows: &[ArchiveNode],
    destination: &Path,
) -> ArchiveCopyRequest {
    let entries: Vec<_> = rows
        .iter()
        .cloned()
        .map(midnight_core::Entry::from_archive)
        .collect();
    ArchiveCopyRequest {
        start_policy: midnight_core::TransferStartPolicy::Immediate,
        verification: midnight_core::TransferVerification::default(),
        id: midnight_core::OperationId(1),
        location: midnight_core::ArchiveLocation {
            mount: Some(mount),
            address: midnight_core::ArchiveAddress {
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

fn destination() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-copy-queue/_build/tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("archive-selection-")
        .tempdir_in(root)
        .unwrap()
}

#[test]
fn a_selection_of_more_than_4096_members_is_taken_whole() {
    // Copy-out once refused a selection of more than 4,096 entries.
    let (mount, rows) = files(100_001);
    let temp = destination();
    let roots = resolve(
        &request(Arc::new(mount), &rows, temp.path()),
        &Cancellation::default(),
    )
    .unwrap();
    let ids: Vec<u64> = roots
        .iter()
        .map(|root| root.virtual_identity().map_or(0, |id| id.entry))
        .collect();
    assert_eq!(ids, (2..100_003).collect::<Vec<_>>());
}

/// The whole verified copy job over a selection beyond the former 4,096 entries. Each
/// file is copied, read back and published like any other, about 25 ms each in a debug
/// build, so it runs on request:
/// `cargo test -p midnight-engine --lib a_selection_of_more_than_4096_members_is_copied -- --ignored`
#[test]
#[ignore = "copies 4,097 files through the verified copy job: about two minutes in a debug build"]
fn a_selection_of_more_than_4096_members_is_copied_one_root_at_a_time() {
    let (mount, rows) = files(4_097);
    let temp = destination();
    let request = request(Arc::new(mount), &rows, temp.path());
    let cancel = Cancellation::default();
    let roots = resolve(&request, &cancel).unwrap();
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare_archive(temp.path().into(), roots, &service, cancel, status).unwrap();
    loop {
        match job.step() {
            worker::Step::Done => break,
            worker::Step::Continue => {}
            worker::Step::Conflict(_) | worker::Step::Failure(_) => panic!("the copy stopped"),
        }
    }
    let done = job.finish();
    assert_eq!((done.files_completed, done.failed), (4_097, 0));
    assert_eq!(std::fs::read(temp.path().join("m004098")).unwrap(), b"x");
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 4_097);
}

#[test]
fn captured_archive_roots_beyond_the_native_root_bound_are_prepared() {
    // An inline-tree copy-out of archive members prepares them as captured roots, which
    // took at most 4,096 roots; only native roots, which stay open until copied, count.
    let (mount, rows) = files(MAX_NATIVE_TRANSFER_ROOTS as u64 + 1);
    let temp = destination();
    let request = request(Arc::new(mount), &rows, temp.path());
    let cancel = Cancellation::default();
    let roots = resolve(&request, &cancel).unwrap();
    assert!(roots.iter().all(|root| root.native_identity().is_none()));
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let job = worker::Job::prepare_sources(
        temp.path().into(),
        roots,
        &service,
        cancel,
        status,
        TransferMetadataPolicy::ContentOnly,
    );
    assert!(job.is_ok(), "{:?}", job.err().map(|error| error.message));
}

/// Stop is pressed as the folder's listing returns.
#[derive(Debug)]
struct Stopping {
    inner: SharedMount,
    stop: Cancellation,
}
impl ArchiveMount for Stopping {
    fn id(&self) -> u64 {
        self.inner.id()
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        let rows = self.inner.list(directory, cancel);
        self.stop.cancel();
        rows
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.inner.read(entry, offset, length, cancel)
    }
    fn validate(&self) -> Result<(), FsError> {
        self.inner.validate()
    }
}

#[test]
fn stop_ends_resolving_a_large_selection() {
    let (inner, rows) = files(100_001);
    let stop = Cancellation::default();
    let mount = Arc::new(Stopping {
        inner,
        stop: stop.clone(),
    });
    let temp = destination();
    let error = resolve(&request(mount, &rows, temp.path()), &stop)
        .err()
        .expect("Stop ends the resolution");
    assert_eq!(error.kind, io::ErrorKind::Interrupted, "{error}");
}

/// How much resolving a selection of a million members grows the process, per member:
/// its roots, and the folder listing and lookups it uses meanwhile. Linux only:
/// `cargo test -p midnight-engine --lib archive_selection_memory_per_member -- --ignored --nocapture --test-threads=1`
#[cfg(target_os = "linux")]
#[test]
#[ignore = "manual memory measurement of a million-member selection"]
fn archive_selection_memory_per_member() {
    const MEMBERS: u64 = 1_000_000;
    let resident = || {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| {
                value
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .unwrap()
            * 1024
    };
    let (mount, rows) = files(MEMBERS);
    let temp = destination();
    let request = request(Arc::new(mount), &rows, temp.path());
    let start = resident();
    let roots = resolve(&request, &Cancellation::default()).unwrap();
    let grown = resident().saturating_sub(start);
    assert_eq!(roots.len() as u64, MEMBERS);
    println!(
        "selection: resolving {MEMBERS} selected members grows the process by {:.1} MiB, \
         {} bytes per member",
        grown as f64 / 1048576.0,
        grown / MEMBERS
    );
}
