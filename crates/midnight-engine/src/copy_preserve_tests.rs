//! Preserve times and attributes (`TransferMetadataPolicy::PreserveTimesAndAttributes`)
//! on the native filesystem: files and folders, read-only last, Replace, cross-volume
//! Move, a streamed selection, warnings and the content-only default.
use super::*;
use std::{
    ffi::OsStr,
    fs::{File, FileTimes},
    time::SystemTime,
};

/// 2001-02-03T04:05:06.1234567Z, the created time (Windows).
#[cfg(windows)]
fn created() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(981_173_106, 123_456_700)
}
/// 2003-04-05T06:07:08.7654321Z.
fn modified() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::new(1_049_522_828, 765_432_100)
}
/// One access time per test that reading a source cannot change: on Windows ten minutes
/// ago (NTFS updates an access time only once it is an hour old), on Linux ten minutes
/// from now (`relatime` updates one older than the modified or change time).
fn accessed() -> SystemTime {
    thread_local! {
        static ACCESSED: SystemTime = {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();
            // Whole 100 ns, which every tested filesystem stores exactly.
            let now = SystemTime::UNIX_EPOCH
                + Duration::new(now.as_secs(), now.subsec_nanos() / 100 * 100);
            if cfg!(windows) {
                now - Duration::from_secs(600)
            } else {
                now + Duration::from_secs(600)
            }
        };
    }
    ACCESSED.with(|time| *time)
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::os::windows::fs::{FileTimesExt, MetadataExt, OpenOptionsExt};
    const READ_ONLY: u32 = 0x1;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const ARCHIVE: u32 = 0x20;
    const NOT_INDEXED: u32 = 0x2000;
    const PRESERVED: u32 = READ_ONLY | HIDDEN | SYSTEM | ARCHIVE | NOT_INDEXED;

    /// Windows attributes.
    pub(super) type Attributes = u32;

    pub(super) fn set(path: &Path, attributes: Attributes) {
        let file = File::options()
            .access_mode(0x100) // FILE_WRITE_ATTRIBUTES
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
            .open(path)
            .unwrap();
        file.set_times(
            FileTimes::new()
                .set_created(created())
                .set_accessed(accessed())
                .set_modified(modified()),
        )
        .unwrap();
        drop(file);
        midnight_platform_windows::set_file_attributes(path, attributes).unwrap();
    }

    /// The copy has every preserved time and attribute of `expected`. NTFS with access
    /// updates on (the default on small volumes) moves a folder's access time whenever
    /// it is listed, as the job's discovery lists the source folders before the job reads
    /// them: a folder's access time is then the one it had when the job read it, which
    /// this does not check.
    pub(super) fn assert_preserved(path: &Path, expected: Attributes) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert_eq!(metadata.created().unwrap(), created(), "{path:?}");
        assert_eq!(metadata.modified().unwrap(), modified(), "{path:?}");
        if !metadata.is_dir() {
            assert_eq!(metadata.accessed().unwrap(), accessed(), "{path:?}");
        }
        assert_eq!(
            metadata.file_attributes() & PRESERVED,
            expected & PRESERVED,
            "{path:?}"
        );
    }

    /// Content only: the times are the copy's own and the hidden, system and read-only
    /// attributes are not copied.
    pub(super) fn assert_content_only(path: &Path, source: Attributes) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert_ne!(metadata.modified().unwrap(), modified(), "{path:?}");
        assert_ne!(metadata.created().unwrap(), created(), "{path:?}");
        assert_eq!(
            metadata.file_attributes() & (HIDDEN | SYSTEM | READ_ONLY) & source,
            0,
            "{path:?}"
        );
    }

    pub(super) fn is_read_only(path: &Path) -> bool {
        fs::symlink_metadata(path).unwrap().file_attributes() & READ_ONLY != 0
    }

    pub(super) fn writable(path: &Path) {
        midnight_platform_windows::set_file_attributes(path, 0).unwrap();
    }

    pub(super) const FILE: Attributes = READ_ONLY | HIDDEN;
    pub(super) const OTHER_FILE: Attributes = SYSTEM | NOT_INDEXED;
    pub(super) const FOLDER: Attributes = HIDDEN | ARCHIVE;
    pub(super) const ROOT: Attributes = READ_ONLY | NOT_INDEXED;
}

#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    /// The POSIX mode.
    pub(super) type Attributes = u32;

    pub(super) fn set(path: &Path, mode: Attributes) {
        // The mode first: changing it moves the change time, which must stay earlier
        // than the access time.
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        File::open(path)
            .unwrap()
            .set_times(
                FileTimes::new()
                    .set_accessed(accessed())
                    .set_modified(modified()),
            )
            .unwrap();
    }

    /// Linux sets no birth time: the copy's creation time is its own.
    pub(super) fn assert_preserved(path: &Path, expected: Attributes) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert_eq!(metadata.modified().unwrap(), modified(), "{path:?}");
        assert_eq!(metadata.accessed().unwrap(), accessed(), "{path:?}");
        assert_eq!(metadata.mode() & 0o7777, expected & 0o777, "{path:?}");
    }

    pub(super) fn assert_content_only(path: &Path, source: Attributes) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert_ne!(metadata.modified().unwrap(), modified(), "{path:?}");
        // The usual umask removes group and other write; without one they would match.
        if source & 0o022 != 0 {
            assert_ne!(metadata.mode() & 0o777, source & 0o777, "{path:?}");
        }
    }

    pub(super) fn is_read_only(path: &Path) -> bool {
        fs::symlink_metadata(path).unwrap().mode() & 0o222 == 0
    }

    pub(super) fn writable(path: &Path) {
        let mode = if path.is_dir() { 0o755 } else { 0o644 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Read-only, and group- and other-writable bits that the usual umask removes.
    pub(super) const FILE: Attributes = 0o444;
    pub(super) const OTHER_FILE: Attributes = 0o666;
    pub(super) const FOLDER: Attributes = 0o775;
    pub(super) const ROOT: Attributes = 0o555;
}
use native::{FILE, FOLDER, OTHER_FILE, ROOT};

/// `root/a.txt`, `root/sub/b.txt` and `root/sub/deeper/`, each with known times and
/// attributes, set children first so writing them cannot change their folder's times.
fn tree(parent: &Path) -> PathBuf {
    let root = parent.join("root");
    let sub = root.join("sub");
    let deeper = sub.join("deeper");
    fs::create_dir_all(&deeper).unwrap();
    fs::write(root.join("a.txt"), b"first file").unwrap();
    fs::write(sub.join("b.txt"), b"second file").unwrap();
    for (path, attributes) in [
        (root.join("a.txt"), FILE),
        (sub.join("b.txt"), OTHER_FILE),
        (deeper.clone(), FOLDER),
        (sub.clone(), FOLDER),
        (root.clone(), ROOT),
    ] {
        native::set(&path, attributes);
    }
    root
}

/// Every item of a tree made writable, so that the temporary folder can be removed.
fn release(root: &Path) {
    if !root.exists() {
        return;
    }
    let mut stack = vec![root.to_owned()];
    while let Some(path) = stack.pop() {
        native::writable(&path);
        if path.is_dir() {
            for entry in fs::read_dir(&path).unwrap() {
                stack.push(entry.unwrap().path());
            }
        }
    }
}

fn preserving(
    intent: TransferIntent,
    sources: &[PathBuf],
    destination: &Path,
) -> MetadataTransferRequest {
    MetadataTransferRequest {
        input: MetadataTransferInput::Paths(request(1, sources, destination)),
        intent,
        metadata: TransferMetadataPolicy::PreserveTimesAndAttributes,
    }
}

fn assert_tree_preserved(copy: &Path) {
    native::assert_preserved(&copy.join("a.txt"), FILE);
    native::assert_preserved(&copy.join("sub").join("b.txt"), OTHER_FILE);
    native::assert_preserved(&copy.join("sub").join("deeper"), FOLDER);
    // Written into after they were created, the folders still have their sources' times.
    native::assert_preserved(&copy.join("sub"), FOLDER);
    native::assert_preserved(copy, ROOT);
    assert_eq!(fs::read(copy.join("a.txt")).unwrap(), b"first file");
    assert_eq!(
        fs::read(copy.join("sub").join("b.txt")).unwrap(),
        b"second file"
    );
}

#[tokio::test]
async fn copy_preserves_times_and_attributes_of_files_and_folders_after_their_contents() {
    let (_temp, parent) = moves::protected_fixture();
    let root = tree(&parent);
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut harness = Harness::native();
    harness
        .controller
        .enqueue_metadata(preserving(
            TransferIntent::Copy,
            std::slice::from_ref(&root),
            &destination,
        ))
        .await;
    let done = harness.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        (done.failed, done.metadata_not_preserved),
        (0, 0),
        "{done:?}"
    );
    assert_eq!(done.files_completed, 2);
    assert_eq!(done.directories_created, 3);
    assert_tree_preserved(&destination.join("root"));
    // The sources keep their modified times; reading them may move their access times.
    assert_eq!(
        fs::metadata(root.join("a.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        modified()
    );
    release(&destination);
    release(&root);
    harness.close().await;
}

#[tokio::test]
async fn without_the_option_copy_keeps_content_only() {
    let (_temp, parent) = moves::protected_fixture();
    let root = tree(&parent);
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut harness = Harness::native();
    harness
        .controller
        .enqueue(request(1, std::slice::from_ref(&root), &destination))
        .await;
    let done = harness.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    let copy = destination.join("root");
    native::assert_content_only(&copy.join("a.txt"), FILE);
    native::assert_content_only(&copy.join("sub").join("b.txt"), OTHER_FILE);
    native::assert_content_only(&copy.join("sub"), FOLDER);
    // Linux keeps the source's permission bits masked by the umask, Read-only included.
    #[cfg(windows)]
    assert!(!native::is_read_only(&copy.join("a.txt")));
    release(&destination);
    release(&root);
    harness.close().await;
}

#[tokio::test]
async fn replace_over_a_read_only_target_gives_the_copy_read_only_last() {
    let (_temp, parent) = moves::protected_fixture();
    let source = parent.join("source");
    fs::create_dir(&source).unwrap();
    let file = source.join("a.txt");
    fs::write(&file, b"new content").unwrap();
    native::set(&file, FILE);
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let existing = destination.join("a.txt");
    fs::write(&existing, b"old content").unwrap();
    native::set(&existing, FILE);
    assert!(native::is_read_only(&existing));
    let mut harness = Harness::native();
    harness
        .controller
        .enqueue_metadata(preserving(
            TransferIntent::Copy,
            std::slice::from_ref(&file),
            &destination,
        ))
        .await;
    let conflict = harness.conflict(1).await;
    assert!(conflict.can_replace, "{conflict:?}");
    harness
        .controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::ReplaceRecoverable { all: false },
        })
        .await;
    let done = harness.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.metadata_not_preserved, 0, "{done:?}");
    // Verified content first; then times and attributes, Read-only among them.
    native::assert_preserved(&existing, FILE);
    assert!(native::is_read_only(&existing));
    assert_eq!(fs::read(&existing).unwrap(), b"new content");
    for recovered in &done.recovery_paths {
        release(recovered);
    }
    release(&destination);
    release(&source);
    harness.close().await;
}

#[tokio::test]
async fn cross_volume_move_preserves_times_and_attributes_and_removes_the_sources() {
    let (_temp, parent) = moves::protected_fixture();
    // Without Read-only sources: removing them is a Move's own matter.
    let root = parent.join("root");
    let sub = root.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(root.join("a.txt"), b"first file").unwrap();
    fs::write(sub.join("b.txt"), b"second file").unwrap();
    for path in [root.join("a.txt"), sub.join("b.txt")] {
        native::set(&path, OTHER_FILE);
    }
    native::set(&sub, FOLDER);
    native::set(&root, FOLDER);
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut harness = Harness::new(Some(Arc::new(moves::CrossDeviceService(
        midnight_platform::LocalFileSystem::new(None),
    ))));
    harness
        .controller
        .enqueue_metadata(preserving(
            TransferIntent::Move,
            std::slice::from_ref(&root),
            &destination,
        ))
        .await;
    let done = harness.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(
        done.native_entries_moved, 0,
        "copied across volumes: {done:?}"
    );
    assert_eq!(
        (done.failed, done.metadata_not_preserved),
        (0, 0),
        "{done:?}"
    );
    let copy = destination.join("root");
    native::assert_preserved(&copy.join("a.txt"), OTHER_FILE);
    native::assert_preserved(&copy.join("sub").join("b.txt"), OTHER_FILE);
    native::assert_preserved(&copy.join("sub"), FOLDER);
    native::assert_preserved(&copy, FOLDER);
    assert!(!root.exists(), "the moved sources are removed");
    release(&destination);
    harness.close().await;
}

/// More items than a job prepares before it starts stream one at a time (no 64-item
/// bound, no preflight); each still keeps its times and attributes, with one read of its
/// source and one change of its copy (`copy_preserve_worker.rs`).
#[tokio::test]
async fn a_streamed_selection_preserves_every_item_without_a_bound() {
    let (_temp, parent) = moves::protected_fixture();
    let source = parent.join("source");
    fs::create_dir(&source).unwrap();
    let count = 3 * 64 + 1;
    let files: Vec<_> = (0..count)
        .map(|index| {
            let file = source.join(format!("item-{index:03}.txt"));
            fs::write(&file, format!("item {index}")).unwrap();
            native::set(&file, OTHER_FILE);
            file
        })
        .collect();
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut harness = Harness::native();
    harness
        .controller
        .enqueue_metadata(preserving(TransferIntent::Copy, &files, &destination))
        .await;
    let done = harness.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, count as u64);
    assert_eq!(done.metadata_not_preserved, 0);
    for file in &files {
        native::assert_preserved(&destination.join(file.file_name().unwrap()), OTHER_FILE);
    }
    harness.close().await;
}

/// A destination whose folders cannot take times or attributes: each is a warning of
/// the job, asked about once; "Skip all this warning type" stops the questions, and the
/// copy completes without errors.
#[tokio::test]
async fn folder_metadata_a_destination_refuses_is_a_warning_skipped_by_type() {
    let (_temp, parent) = moves::protected_fixture();
    let root = tree(&parent);
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut harness = Harness::new(Some(Arc::new(RefusingFolders(
        midnight_platform::LocalFileSystem::new(None),
    ))));
    harness
        .controller
        .enqueue_metadata(preserving(
            TransferIntent::Copy,
            std::slice::from_ref(&root),
            &destination,
        ))
        .await;
    let mut questions = 0;
    let done = loop {
        match harness.next().await {
            CopyEvent::Failure(failure) => {
                questions += 1;
                assert_eq!(questions, 1, "asked again after Skip all: {failure:?}");
                assert_eq!(failure.stage, CopyErrorStage::PreservingMetadata);
                assert!(failure.can_skip && !failure.can_retry, "{failure:?}");
                harness
                    .controller
                    .decide(CopyDecision {
                        id: failure.id,
                        version: failure.version,
                        choice: CopyChoice::SkipAllErrors,
                    })
                    .await;
            }
            CopyEvent::Progress(progress) if progress.state.is_terminal() => break progress,
            _ => {}
        }
    };
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.failed, 0, "{done:?}");
    assert_eq!(done.metadata_not_preserved, 3, "{done:?}");
    assert_eq!(done.note_count, 3);
    assert!(
        done.message
            .contains("3 items were copied without all of their times or attributes"),
        "{}",
        done.message
    );
    // The files themselves keep theirs.
    let copy = destination.join("root");
    native::assert_preserved(&copy.join("a.txt"), FILE);
    native::assert_preserved(&copy.join("sub").join("b.txt"), OTHER_FILE);
    release(&destination);
    release(&root);
    harness.close().await;
}

/// The native filesystem, except that its folders refuse times and attributes.
struct RefusingFolders(midnight_platform::LocalFileSystem);
impl midnight_core::AuditedSynchronousTransferService for RefusingFolders {}
impl TransferService for RefusingFolders {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.0.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0
            .prepare_transfer_destination(path, cancel)
            .map(Refusing::wrap)
    }
}
struct Refusing(Arc<dyn TransferDirectory>);
impl Refusing {
    fn wrap(inner: Arc<dyn TransferDirectory>) -> Arc<dyn TransferDirectory> {
        Arc::new(Self(inner))
    }
}
impl TransferDirectory for Refusing {
    fn native_child_namespace_scope(
        &self,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferNamespacePin>>, FsError> {
        self.0.native_child_namespace_scope(name, kind, cancel)
    }
    fn stage_file_with_metadata(
        &self,
        policy: TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.0.stage_file_with_metadata(policy, cancel)
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.0.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.0.capabilities()
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.0.prepare_target(name, cancel)
    }
    fn prepare_target_with(
        &self,
        name: &OsStr,
        revisions: TransferRevisionPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.0.prepare_target_with(name, revisions, cancel)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn set_published_permissions(
        &self,
        unix_mode: u32,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.0.set_published_permissions(unix_mode, cancel)
    }
    fn apply_preserved_metadata(
        &self,
        _: &TransferPreservedMetadata,
        _: &Cancellation,
    ) -> Result<(), FsError> {
        Err(error(
            self.path(),
            io::ErrorKind::Unsupported,
            "This fixture's folders keep their own times and attributes.",
        ))
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.0.inspect(name, cancel)
    }
    fn ancestor_identities(
        &self,
        cancel: &Cancellation,
    ) -> Result<Option<Vec<TransferIdentity>>, FsError> {
        self.0.ancestor_identities(cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.0.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.create_directory(name, cancel).map(Self::wrap)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.0.stage_file(cancel)
    }
    fn stage_job_file(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.0.stage_job_file(cancel)
    }
    fn finish_staging(&self) -> Result<(), FsError> {
        self.0.finish_staging()
    }
}
