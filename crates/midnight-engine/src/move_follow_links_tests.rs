//! A Move that copies what links point to (FollowLinks): a link's target is copied and
//! verified like any file or folder, and then only the link is removed; a link is never
//! renamed and a folder never renamed whole, so that the links in it are followed too.
//! The native link and its protected removal are synthetic here; platform tests qualify
//! them.
use super::*;
use std::{
    ffi::OsStr,
    sync::atomic::{AtomicUsize, Ordering},
};

const DATA: &[u8] = b"the file a link points to";

/// A projection that reads `actual` (a link's target) at the link's own path.
struct Projection {
    actual: Box<dyn TransferSource>,
    projected: TransferEntry,
    original: TransferIdentity,
    children: VecDeque<Box<dyn TransferSource>>,
}

impl TransferSource for Projection {
    fn entry(&self) -> &TransferEntry {
        &self.projected
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.actual.native_identity()
    }
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        Some(self.original)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.actual.revalidate(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.actual.read_chunk(buffer, cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.actual.rewind_for_validation(cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.actual.seek_for_validation(offset, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.revalidate(cancel)?;
        Ok(self.children.pop_front())
    }
}

/// The original link: a placeholder entry whose removal is recorded. Its rename must
/// never be asked for.
struct Link {
    pin: Mutex<Option<Box<dyn TransferSource>>>,
    entry: TransferEntry,
    renames: AtomicUsize,
    removals: AtomicUsize,
    /// What its removal requires to exist first: the verified copy of its target.
    output: std::path::PathBuf,
    /// The removal is refused, as a write-protected folder refuses it.
    refuse: bool,
}

impl TransferDirectoryLinkMoveCapture for Link {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        match self.pin.lock().unwrap().as_ref() {
            Some(pin) => pin.revalidate(cancel),
            None => Err(error(
                &self.entry.item.path,
                io::ErrorKind::NotFound,
                "the link was removed",
            )),
        }
    }
    fn try_native_move(
        &self,
        _: &dyn TransferDirectory,
        _: &OsStr,
        _: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        _: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        self.renames.fetch_add(1, Ordering::SeqCst);
        Ok(TransferNativeMoveOutcome::Unsupported)
    }
    fn remove_original(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        self.revalidate(cancel)?;
        self.removals.fetch_add(1, Ordering::SeqCst);
        assert!(
            self.output.exists(),
            "the copy is published before the link goes"
        );
        if self.refuse {
            return Err(error(
                &self.entry.item.path,
                io::ErrorKind::PermissionDenied,
                "injected removal refusal",
            ));
        }
        drop(self.pin.lock().unwrap().take());
        let path = &self.entry.item.path;
        if path.is_dir() {
            std::fs::remove_dir(path).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
        Ok(TransferDirectoryLinkRemoval::Removed {
            state: TransferSourceRemoval::Unlinked,
            durability_confirmed: true,
            recovery: Vec::new(),
            error: None,
        })
    }
}

struct Fixture {
    folder: tempfile::TempDir,
    native: midnight_platform::LocalFileSystem,
    referent: std::path::PathBuf,
    destination: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let referent = folder.path().join("referent");
        let destination = folder.path().join("destination");
        std::fs::create_dir(&referent).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(referent.join("file.txt"), DATA).unwrap();
        Self {
            folder,
            native: midnight_platform::LocalFileSystem::new(None),
            referent,
            destination,
        }
    }

    fn source(&self, path: &Path) -> Box<dyn TransferSource> {
        self.native
            .prepare_transfer_source(path, &Cancellation::default())
            .unwrap()
    }

    /// A link at `logical` (stood for by a placeholder of the target's kind) whose target
    /// is `target`, with its projection and capture.
    fn link(
        &self,
        logical: &Path,
        target: &Path,
        refuse: bool,
    ) -> (PreparedTransferMoveEntry, Arc<Link>) {
        let folder = target.is_dir();
        if folder {
            std::fs::create_dir(logical).unwrap();
        } else {
            std::fs::write(logical, b"placeholder").unwrap();
        }
        let pin = self.source(logical);
        let mut entry = pin.entry().clone();
        entry.item.kind = if folder {
            EntryKind::DirectoryLink
        } else {
            EntryKind::Symlink
        };
        let actual = self.source(target);
        let mut projected = actual.entry().clone();
        projected.item.path = logical.to_owned();
        let children = if folder {
            let child = self.source(&target.join("file.txt"));
            let mut entry = child.entry().clone();
            entry.item.path = logical.join("file.txt");
            let original = entry.identity;
            [Box::new(Projection {
                actual: child,
                projected: entry,
                original,
                children: VecDeque::new(),
            }) as Box<dyn TransferSource>]
            .into()
        } else {
            VecDeque::new()
        };
        let output = self.destination.join(logical.file_name().unwrap());
        let capture = Arc::new(Link {
            pin: Mutex::new(Some(pin)),
            entry: entry.clone(),
            renames: AtomicUsize::new(0),
            removals: AtomicUsize::new(0),
            output: if folder {
                output.join("file.txt")
            } else {
                output
            },
            refuse,
        });
        let prepared = PreparedTransferMoveEntry::DirectoryLink {
            source: Box::new(Projection {
                actual,
                projected,
                original: entry.identity,
                children,
            }),
            capture: capture.clone(),
        };
        (prepared, capture)
    }

    fn prepare(&self, roots: VecDeque<PreparedTransferMoveEntry>) -> Job {
        let status = Arc::new(Mutex::new(CopyProgress::queued(
            OperationId(1),
            self.destination.clone(),
            roots.len() as u64,
            TransferIntent::Move,
        )));
        move_job::prepare_entries(
            self.destination.clone(),
            roots,
            &self.native,
            Cancellation::default(),
            status,
            TransferMetadataPolicy::ContentOnly,
            CopyLinkPolicy::FollowLinks,
        )
        .unwrap()
    }
}

fn run(mut job: Job) -> CopyProgress {
    for _ in 0..400 {
        match job.step() {
            Step::Continue => {}
            Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
            Step::Failure(failure) => panic!("unexpected failure: {failure:?}"),
            Step::Done => return job.finish(),
        }
    }
    panic!("test transfer did not terminate")
}

/// The referent's identity, which the Move must keep: what a link points to is never
/// moved, removed or rewritten.
fn identity_of(fixture: &Fixture, path: &Path) -> TransferIdentity {
    fixture.source(path).entry().identity
}

#[test]
fn a_linked_file_is_copied_and_verified_and_only_the_link_is_removed() {
    let fixture = Fixture::new();
    let target = fixture.referent.join("file.txt");
    let before = identity_of(&fixture, &target);
    let link = fixture.folder.path().join("to-file");
    let (entry, capture) = fixture.link(&link, &target, false);
    let done = run(fixture.prepare([entry].into()));
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 1);
    assert_eq!(
        done.native_entries_moved, 0,
        "a followed link is never renamed"
    );
    assert_eq!(capture.renames.load(Ordering::SeqCst), 0);
    assert_eq!(capture.removals.load(Ordering::SeqCst), 1);
    assert!(!link.exists(), "the link itself is removed");
    let copy = fixture.destination.join("to-file");
    assert!(std::fs::symlink_metadata(&copy).unwrap().is_file());
    assert_eq!(std::fs::read(&copy).unwrap(), DATA);
    // The file the link pointed to is untouched: the same object with the same bytes.
    assert_eq!(std::fs::read(&target).unwrap(), DATA);
    assert_eq!(identity_of(&fixture, &target), before);
}

#[test]
fn a_linked_folder_is_copied_and_only_the_link_is_removed_never_renamed() {
    let fixture = Fixture::new();
    let before = identity_of(&fixture, &fixture.referent.join("file.txt"));
    let link = fixture.folder.path().join("to-folder");
    let (entry, capture) = fixture.link(&link, &fixture.referent, false);
    let done = run(fixture.prepare([entry].into()));
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(capture.renames.load(Ordering::SeqCst), 0);
    assert_eq!(capture.removals.load(Ordering::SeqCst), 1);
    assert!(!link.exists());
    assert_eq!(
        std::fs::read(fixture.destination.join("to-folder/file.txt")).unwrap(),
        DATA
    );
    assert_eq!(
        std::fs::read(fixture.referent.join("file.txt")).unwrap(),
        DATA
    );
    assert_eq!(
        identity_of(&fixture, &fixture.referent.join("file.txt")),
        before
    );
}

#[test]
fn a_link_whose_removal_fails_after_its_copy_asks_skip_and_keeps_both() {
    let fixture = Fixture::new();
    let target = fixture.referent.join("file.txt");
    let link = fixture.folder.path().join("to-file");
    let (entry, capture) = fixture.link(&link, &target, true);
    let mut job = fixture.prepare([entry].into());
    let mut asked = Vec::new();
    let mut done = None;
    for _ in 0..400 {
        match job.step() {
            Step::Continue => {}
            Step::Failure(failure) => {
                assert!(job.accept_decision(CopyDecision {
                    id: failure.id,
                    version: failure.version,
                    choice: CopyChoice::Skip,
                }));
                asked.push(failure);
            }
            Step::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
            Step::Done => {
                done = Some(job.finish());
                break;
            }
        }
    }
    let done = done.expect("the Move ends within bounded steps");
    let [failure] = asked.as_slice() else {
        panic!("one question: {asked:?}");
    };
    // The copy is published: there is nothing to retry, only whether to go on.
    assert_eq!(failure.stage, CopyErrorStage::RemovingSource, "{failure:?}");
    assert!(!failure.can_retry && failure.can_skip, "{failure:?}");
    assert_eq!(failure.source.as_deref(), Some(link.as_path()));
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!(capture.removals.load(Ordering::SeqCst), 1);
    assert!(link.exists(), "the link stays where it is");
    assert_eq!(
        std::fs::read(fixture.destination.join("to-file")).unwrap(),
        DATA
    );
    assert_eq!(std::fs::read(&target).unwrap(), DATA);
}

#[test]
fn a_folder_on_the_same_volume_is_traversed_not_renamed_whole() {
    // Renamed whole, its links would move as links: the folder is created at the
    // destination, its file moved by its own rename, and the emptied folder removed.
    let fixture = Fixture::new();
    let folder = fixture.folder.path().join("moved");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("plain.txt"), DATA).unwrap();
    let prepared = fixture
        .native
        .prepare_move_source_with_policy(
            &folder,
            CopyLinkPolicy::FollowLinks,
            TRANSFER_REVISIONS,
            &Cancellation::default(),
        )
        .unwrap();
    let done = run(fixture.prepare([prepared].into()));
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.directories_created, 1, "{done:?}");
    assert_eq!(
        done.native_entries_moved, 1,
        "only the file is renamed: {done:?}"
    );
    assert!(!folder.exists(), "the emptied folder is removed");
    assert_eq!(
        std::fs::read(fixture.destination.join("moved/plain.txt")).unwrap(),
        DATA
    );
}
