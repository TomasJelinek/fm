//! Engine integration with a synthetic original-link capability on a real filesystem.
//! Platform tests qualify the native link/reparse and mutation authority.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const DATA: &[u8] = b"verified target bytes";

struct Source {
    actual: Box<dyn TransferSource>,
    projected: TransferEntry,
    original: Option<TransferIdentity>,
    children: VecDeque<Box<dyn TransferSource>>,
}

impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.projected
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.actual.native_identity()
    }
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        self.original.or_else(|| self.actual.native_identity())
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.actual.revalidate(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        visit(&self.projected.item);
        for child in &self.children {
            child.discover(cancel, visit)?;
        }
        Ok(())
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

// Exercise two engine authorities for one physical identity without opening a
// second native directory read handle against the real root's Move guard.
struct RefusedEnumeration {
    projected: TransferEntry,
    original: TransferIdentity,
}

impl TransferSource for RefusedEnumeration {
    fn entry(&self) -> &TransferEntry {
        &self.projected
    }
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        Some(self.original)
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        panic!("the projected directory fixture has no file reads")
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Err(error(
            &self.projected.item.path,
            io::ErrorKind::PermissionDenied,
            "injected alias enumeration refusal",
        ))
    }
}

struct SelectedMoveDirectory {
    source: Box<dyn TransferSource>,
    link: Option<PreparedTransferMoveEntry>,
}

impl TransferSource for SelectedMoveDirectory {
    fn entry(&self) -> &TransferEntry {
        self.source.entry()
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        self.source.native_identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.source.revalidate(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.source.read_chunk(buffer, cancel)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        panic!("the ordinary Move root must enumerate through its typed Move plan")
    }
    fn next_move_child_with_links(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMoveEntry>, FsError> {
        assert_eq!(links, CopyLinkPolicy::FollowDirectoryContents);
        self.source.revalidate(cancel)?;
        Ok(self.link.take())
    }
}

struct Original {
    pin: Mutex<Option<Box<dyn TransferSource>>>,
    entry: TransferEntry,
    expected_outputs: Vec<std::path::PathBuf>,
    /// The removal leaves its private stage behind and reports that.
    uncertain: bool,
}

impl TransferDirectoryLinkMoveCapture for Original {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.pin
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .revalidate(cancel)
    }
    fn remove_original(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        self.revalidate(cancel)?;
        for output in &self.expected_outputs {
            assert_eq!(std::fs::read(output).unwrap(), DATA);
        }
        drop(self.pin.lock().unwrap().take());
        std::fs::remove_dir(&self.entry.item.path).unwrap();
        if self.uncertain {
            let stage = self.entry.item.path.with_file_name(".midnight-move-stage");
            return Ok(TransferDirectoryLinkRemoval::Removed {
                state: TransferSourceRemoval::Unlinked,
                durability_confirmed: false,
                recovery: vec![TransferRecovery {
                    path: stage.clone(),
                    identity: None,
                }],
                error: Some(error(
                    &stage,
                    io::ErrorKind::PermissionDenied,
                    "injected stage cleanup refusal",
                )),
            });
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
        std::fs::write(referent.join("contents"), DATA).unwrap();
        Self {
            folder,
            native: midnight_platform::LocalFileSystem::new(None),
            referent,
            destination,
        }
    }

    fn read(
        &self,
        physical: &Path,
        logical: &Path,
        original: Option<TransferIdentity>,
        children: VecDeque<Box<dyn TransferSource>>,
    ) -> Box<dyn TransferSource> {
        let actual = self
            .native
            .prepare_transfer_source(physical, &Cancellation::default())
            .unwrap();
        let mut projected = actual.entry().clone();
        projected.item.path = logical.to_owned();
        Box::new(Source {
            actual,
            projected,
            original,
            children,
        })
    }

    fn alias(&self, logical: &Path, original: Option<TransferIdentity>) -> Box<dyn TransferSource> {
        let child = self.read(
            &self.referent.join("contents"),
            &logical.join("contents"),
            None,
            VecDeque::new(),
        );
        self.read(&self.referent, logical, original, [child].into())
    }

    fn prepare_link(
        &self,
        logical: &Path,
        make_source: impl FnOnce(TransferIdentity) -> Box<dyn TransferSource>,
        expected_outputs: Vec<std::path::PathBuf>,
    ) -> PreparedTransferMoveEntry {
        self.prepare_link_removal(logical, make_source, expected_outputs, false)
    }

    /// `prepare_link`, whose original's removal is `uncertain` when asked.
    fn prepare_link_removal(
        &self,
        logical: &Path,
        make_source: impl FnOnce(TransferIdentity) -> Box<dyn TransferSource>,
        expected_outputs: Vec<std::path::PathBuf>,
        uncertain: bool,
    ) -> PreparedTransferMoveEntry {
        std::fs::create_dir(logical).unwrap();
        let original = self
            .native
            .prepare_transfer_source(logical, &Cancellation::default())
            .unwrap();
        let mut entry = original.entry().clone();
        entry.item.kind = EntryKind::DirectoryLink;
        let source = make_source(entry.identity);
        PreparedTransferMoveEntry::DirectoryLink {
            source,
            capture: Arc::new(Original {
                pin: Mutex::new(Some(original)),
                entry,
                expected_outputs,
                uncertain,
            }),
        }
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
            CopyLinkPolicy::FollowDirectoryContents,
        )
        .unwrap()
    }
}

fn run(mut job: Job, decide: impl Fn(&CopyConflict) -> CopyChoice) -> CopyProgress {
    for _ in 0..200 {
        match job.step() {
            Step::Continue => {}
            Step::Conflict(conflict) => {
                assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice: decide(&conflict),
                }));
            }
            Step::Failure(failure) => panic!("{failure:?}"),
            Step::Done => return job.finish(),
        }
    }
    panic!("test transfer did not terminate")
}

#[test]
fn selected_copy_aliases_share_physical_identity_without_collapsing_destinations() {
    let fixture = Fixture::new();
    let originals = [
        fixture.folder.path().join("first"),
        fixture.folder.path().join("second"),
    ];
    let cancel = Cancellation::default();
    let roots = originals
        .iter()
        .map(|logical| {
            std::fs::create_dir(logical).unwrap();
            let pin = fixture
                .native
                .prepare_transfer_source(logical, &cancel)
                .unwrap();
            fixture.alias(logical, pin.native_identity())
        })
        .collect::<VecDeque<_>>();
    let status = Arc::new(Mutex::new(CopyProgress::queued(
        OperationId(1),
        fixture.destination.clone(),
        roots.len() as u64,
        TransferIntent::Copy,
    )));
    let job = Job::prepare_sources_with_options(
        fixture.destination.clone(),
        roots,
        &fixture.native,
        cancel,
        status,
        TransferMetadataPolicy::ContentOnly,
        CopyLinkPolicy::FollowDirectoryContents,
    )
    .unwrap();
    let done = run(job, |_| panic!("unexpected conflict"));
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!(done.files_completed, 2);
    for original in originals {
        assert!(original.exists());
        let output = fixture
            .destination
            .join(original.file_name().unwrap())
            .join("contents");
        assert_eq!(std::fs::read(output).unwrap(), DATA);
    }
}

#[test]
fn selected_links_to_the_same_target_each_copy_and_remove_only_its_original() {
    let fixture = Fixture::new();
    let originals = [
        fixture.folder.path().join("first"),
        fixture.folder.path().join("second"),
    ];
    let entries = originals
        .iter()
        .map(|logical| {
            let output = fixture
                .destination
                .join(logical.file_name().unwrap())
                .join("contents");
            fixture.prepare_link(
                logical,
                |original| fixture.alias(logical, Some(original)),
                vec![output],
            )
        })
        .collect();
    let done = run(fixture.prepare(entries), |_| panic!("unexpected conflict"));
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!(done.files_completed, 2);
    assert!(originals.iter().all(|original| !original.exists()));
    assert_eq!(
        std::fs::read(fixture.referent.join("contents")).unwrap(),
        DATA
    );
}

#[test]
fn an_uncertain_original_link_removal_asks_skip_only_and_skip_moves_the_next_link() {
    let fixture = Fixture::new();
    let originals = [
        fixture.folder.path().join("first"),
        fixture.folder.path().join("second"),
    ];
    let entries = originals
        .iter()
        .enumerate()
        .map(|(index, logical)| {
            let output = fixture
                .destination
                .join(logical.file_name().unwrap())
                .join("contents");
            fixture.prepare_link_removal(
                logical,
                |original| fixture.alias(logical, Some(original)),
                vec![output],
                index == 0,
            )
        })
        .collect();
    let mut job = fixture.prepare(entries);
    let mut asked = Vec::new();
    let mut done = None;
    for _ in 0..400 {
        match job.step() {
            Step::Continue => {}
            Step::Failure(failure) => {
                for (choice, accepted) in [(CopyChoice::Retry, false), (CopyChoice::Skip, true)] {
                    assert_eq!(
                        job.accept_decision(CopyDecision {
                            id: failure.id,
                            version: failure.version,
                            choice,
                        }),
                        accepted
                    );
                }
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
    assert_eq!(failure.stage, CopyErrorStage::RemovingSource, "{failure:?}");
    assert!(!failure.can_retry && failure.can_skip, "{failure:?}");
    assert_eq!(failure.source.as_deref(), Some(originals[0].as_path()));
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.failed, done.skipped), (1, 0), "{done:?}");
    assert_eq!(done.files_completed, 2);
    assert_eq!(done.source_residual_count, 1, "{done:?}");
    assert!(originals.iter().all(|original| !original.exists()));
}

#[test]
fn sibling_aliases_inside_followed_move_remain_read_only() {
    let fixture = Fixture::new();
    let original = fixture.folder.path().join("outer");
    let output = fixture.destination.join("outer");
    let outer_target = fixture.folder.path().join("outer-target");
    std::fs::create_dir(&outer_target).unwrap();
    let entry = fixture.prepare_link(
        &original,
        |identity| {
            let children = [
                fixture.alias(&original.join("first"), None),
                fixture.alias(&original.join("second"), None),
            ];
            fixture.read(&outer_target, &original, Some(identity), children.into())
        },
        vec![
            output.join("first/contents"),
            output.join("second/contents"),
        ],
    );
    let done = run(fixture.prepare([entry].into()), |_| {
        panic!("unexpected conflict")
    });
    assert_eq!(done.state, CopyOperationState::Completed);
    assert_eq!(done.files_completed, 2);
    assert!(!original.exists());
    assert_eq!(
        std::fs::read(fixture.referent.join("contents")).unwrap(),
        DATA
    );
}

#[test]
fn skipped_followed_child_retains_the_original_link() {
    let fixture = Fixture::new();
    let original = fixture.folder.path().join("outer");
    let output = fixture.destination.join("outer");
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("contents"), b"existing destination").unwrap();
    let entry = fixture.prepare_link(
        &original,
        |identity| fixture.alias(&original, Some(identity)),
        vec![output.join("contents")],
    );
    let done = run(fixture.prepare([entry].into()), |conflict| {
        if conflict.can_merge {
            CopyChoice::Merge { all: false }
        } else {
            CopyChoice::Skip
        }
    });
    assert_eq!(done.skipped, 1);
    assert!(original.exists());
    assert_eq!(
        std::fs::read(output.join("contents")).unwrap(),
        b"existing destination"
    );
    assert_eq!(
        std::fs::read(fixture.referent.join("contents")).unwrap(),
        DATA
    );
}

#[test]
fn skipping_failed_alias_enumeration_retains_separately_selected_real_directory_capture() {
    let fixture = Fixture::new();
    let alias = fixture.folder.path().join("alias");
    let selected = fixture
        .native
        .prepare_move_source(&fixture.referent, &Cancellation::default())
        .unwrap();
    let mut projected = selected.source.entry().clone();
    projected.item.path = alias.clone();
    let alias_entry = fixture.prepare_link(
        &alias,
        |original| {
            Box::new(RefusedEnumeration {
                projected,
                original,
            })
        },
        Vec::new(),
    );
    let PreparedTransferMoveEntry::DirectoryLink { capture, .. } = &alias_entry else {
        panic!("fixture must retain the original link capture");
    };
    let stub_capture = Arc::downgrade(capture);
    let mut job =
        fixture.prepare([alias_entry, PreparedTransferMoveEntry::Ordinary(selected)].into());
    let refusal = (0..30)
        .find_map(|_| match job.step() {
            Step::Continue => None,
            Step::Failure(failure) => Some(failure),
            _ => panic!("expected a paused alias enumeration failure"),
        })
        .expect("alias enumeration must fail");
    assert_eq!(refusal.stage, CopyErrorStage::Enumerating);
    assert_eq!(refusal.source.as_deref(), Some(alias.as_path()));
    assert!(job.accept_decision(CopyDecision {
        id: refusal.id,
        version: refusal.version,
        choice: CopyChoice::Skip,
    }));
    assert!(matches!(job.step(), Step::Continue));
    assert!(stub_capture.upgrade().is_none());

    let done = run(job, |_| {
        panic!("unexpected conflict for the real directory")
    });
    assert_eq!(done.skipped, 1);
    assert!(alias.exists());
    assert!(!fixture.referent.exists());
    assert_eq!(
        std::fs::read(fixture.destination.join("referent/contents")).unwrap(),
        DATA
    );
}

#[test]
fn copy_cannot_merge_into_an_active_followed_directory() {
    assert_refuses_merge_into_followed_directory(TransferIntent::Copy);
}

#[test]
fn projected_move_cannot_merge_into_an_active_followed_directory() {
    assert_refuses_merge_into_followed_directory(TransferIntent::Move);
}

fn assert_refuses_merge_into_followed_directory(intent: TransferIntent) {
    let fixture = Fixture::new();
    let container = fixture.folder.path().join("source/container");
    std::fs::create_dir_all(&container).unwrap();
    let alias = container.join("L");
    let destination_container = fixture.destination.join("container");
    let output = destination_container.join("L");
    let followed = output.join("sub");
    let child = followed.join("sub");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(child.join("file"), DATA).unwrap();
    let entry = fixture.prepare_link(
        &alias,
        |identity| {
            let file = fixture.read(
                &child.join("file"),
                &alias.join("sub/file"),
                None,
                VecDeque::new(),
            );
            let sub = fixture.read(&child, &alias.join("sub"), None, [file].into());
            fixture.read(&followed, &alias, Some(identity), [sub].into())
        },
        Vec::new(),
    );
    let followed_identity = entry.source().entry().identity;
    let mut job = match intent {
        TransferIntent::Copy => {
            let PreparedTransferMoveEntry::DirectoryLink { source, .. } = entry else {
                panic!("fixture must project a directory link");
            };
            let root = fixture.read(&container, &container, None, [source].into());
            let status = Arc::new(Mutex::new(CopyProgress::queued(
                OperationId(1),
                fixture.destination.clone(),
                1,
                intent,
            )));
            Job::prepare_sources_with_options(
                fixture.destination.clone(),
                [root].into(),
                &fixture.native,
                Cancellation::default(),
                status,
                TransferMetadataPolicy::ContentOnly,
                CopyLinkPolicy::FollowDirectoryContents,
            )
            .unwrap()
        }
        TransferIntent::Move => {
            let midnight_core::PreparedTransferMove { source, capture } = fixture
                .native
                .prepare_move_source(&container, &Cancellation::default())
                .unwrap();
            let root = PreparedTransferMoveEntry::Ordinary(midnight_core::PreparedTransferMove {
                source: Box::new(SelectedMoveDirectory {
                    source,
                    link: Some(entry),
                }),
                capture,
            });
            fixture.prepare([root].into())
        }
    };
    assert!(!job.protected_roots.contains(&followed_identity));
    let expected_conflicts = [&destination_container, &output, &followed];
    let mut merges = 0;
    for _ in 0..100 {
        match job.step() {
            Step::Continue => {}
            Step::Conflict(conflict) => {
                assert_eq!(&conflict.destination, expected_conflicts[merges]);
                merges += 1;
                assert!(conflict.can_merge);
                assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice: CopyChoice::Merge { all: false },
                }));
            }
            Step::Failure(failure) => {
                assert_eq!(merges, expected_conflicts.len());
                assert_eq!(job.stack.len(), 2);
                assert_eq!(
                    job.stack.last().unwrap().source.native_identity(),
                    Some(followed_identity)
                );
                assert_eq!(failure.stage, CopyErrorStage::CreatingDirectory);
                assert_eq!(failure.error.kind, io::ErrorKind::InvalidInput);
                assert_eq!(failure.error.path, followed);
                assert!(
                    failure
                        .error
                        .message
                        .contains("inside the source directory")
                );
                assert!(job.accept_decision(CopyDecision {
                    id: failure.id,
                    version: failure.version,
                    choice: CopyChoice::Skip,
                }));
                let done = run(job, |_| panic!("unexpected conflict after Skip"));
                assert_eq!(done.skipped, 1);
                assert!(alias.exists());
                assert!(!followed.join("file").exists());
                assert_eq!(std::fs::read(child.join("file")).unwrap(), DATA);
                return;
            }
            Step::Done => panic!("Merge must refuse writes into the active referent"),
        }
    }
    panic!("overlapping Merge did not reach the refusal");
}

#[test]
fn copy_cannot_merge_into_a_pending_followed_root() {
    assert_refuses_merge_into_pending_root(TransferIntent::Copy);
}

#[test]
fn projected_move_cannot_merge_into_a_pending_followed_root() {
    assert_refuses_merge_into_pending_root(TransferIntent::Move);
}

struct DeferredRead {
    entry: TransferEntry,
    original: Arc<dyn TransferDirectoryLinkMoveCapture>,
}

impl TransferSource for DeferredRead {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }
    fn native_original_identity(&self) -> Option<TransferIdentity> {
        Some(self.entry.identity)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.original.revalidate(cancel)
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        panic!("read through an unresolved directory link")
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        panic!("enumeration through an unresolved directory link")
    }
}

struct DeferredCapture {
    original: Arc<dyn TransferDirectoryLinkMoveCapture>,
    projection: Mutex<Option<Box<dyn TransferSource>>>,
    preparations: Arc<AtomicUsize>,
}

impl TransferDirectoryLinkMoveCapture for DeferredCapture {
    fn entry(&self) -> &TransferEntry {
        self.original.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.original.revalidate(cancel)
    }
    fn prepare_copy_projection(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.original.revalidate(cancel)?;
        self.preparations.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .projection
            .lock()
            .unwrap()
            .take()
            .expect("qualify the same pending link once"))
    }
    fn remove_original(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        self.original.remove_original(cancel)
    }
}

fn defer_link(
    prepared: PreparedTransferMoveEntry,
    preparations: Arc<AtomicUsize>,
) -> PreparedTransferMoveEntry {
    let PreparedTransferMoveEntry::DirectoryLink { source, capture } = prepared else {
        panic!("expected a directory link");
    };
    let mut entry = capture.entry().clone();
    entry.item.kind = EntryKind::Directory;
    PreparedTransferMoveEntry::DirectoryLink {
        source: Box::new(DeferredRead {
            entry,
            original: capture.clone(),
        }),
        capture: Arc::new(DeferredCapture {
            original: capture,
            projection: Mutex::new(Some(source)),
            preparations,
        }),
    }
}

fn assert_refuses_merge_into_pending_root(intent: TransferIntent) {
    let fixture = Fixture::new();
    let source_parent = fixture.folder.path().join("source");
    let a = source_parent.join("A");
    let b = source_parent.join("B");
    let output = fixture.destination.join("A");
    let followed = output.join("sub");
    std::fs::create_dir_all(a.join("sub")).unwrap();
    std::fs::create_dir_all(&followed).unwrap();
    std::fs::write(a.join("sub/new"), DATA).unwrap();
    let b_entry = fixture.prepare_link(
        &b,
        |identity| fixture.read(&followed, &b, Some(identity), VecDeque::new()),
        Vec::new(),
    );
    let followed_id = b_entry.source().native_identity().unwrap();
    let preparations = Arc::new(AtomicUsize::new(0));
    let cancel = Cancellation::default();
    let links = CopyLinkPolicy::FollowDirectoryContents;
    let mut job = match intent {
        TransferIntent::Copy => {
            let a = fixture
                .native
                .prepare_transfer_source_with_links(&a, links, &cancel)
                .unwrap();
            let PreparedTransferMoveEntry::DirectoryLink { source, .. } = b_entry else {
                panic!("expected read-only directory link projection");
            };
            let status = Arc::new(Mutex::new(CopyProgress::queued(
                OperationId(1),
                fixture.destination.clone(),
                2,
                intent,
            )));
            Job::prepare_sources_with_options(
                fixture.destination.clone(),
                [a, source].into(),
                &fixture.native,
                cancel,
                status,
                TransferMetadataPolicy::ContentOnly,
                links,
            )
            .unwrap()
        }
        TransferIntent::Move => {
            let a = fixture
                .native
                .prepare_move_source_with_links(&a, links, &cancel)
                .unwrap();
            fixture.prepare([a, defer_link(b_entry, preparations.clone())].into())
        }
    };
    let check_refusal = |failure: &FsError| {
        assert_eq!(failure.kind, io::ErrorKind::InvalidInput);
        assert_eq!(failure.path, followed);
        assert!(b.exists());
        assert!(!fixture.destination.join("B").exists());
        assert!(!followed.join("new").exists());
        assert_eq!(std::fs::read(a.join("sub/new")).unwrap(), DATA);
    };
    assert_eq!(preparations.load(Ordering::Relaxed), 0);
    let expected = [&output, &followed];
    let mut merges = 0;
    for _ in 0..100 {
        match job.step() {
            Step::Continue => {}
            Step::Conflict(conflict) => {
                assert_eq!(&conflict.destination, expected[merges]);
                if intent == TransferIntent::Copy && merges == 1 {
                    // Current prompts already restrict an exact selected-root ID.
                    // Exercise the independent write-time check with retained consent.
                    assert!(!conflict.can_merge);
                    assert_eq!(
                        job.roots.front().unwrap().native_identity(),
                        Some(followed_id)
                    );
                    let mut work = job.current.take().unwrap();
                    work.consent = Some(Consent::Merge);
                    let failure = job
                        .prepare_entry(&mut work)
                        .err()
                        .expect("pinned merge must refuse pending source");
                    check_refusal(&failure);
                    job.current = Some(work);
                    let _ = job.finish();
                    return;
                }
                assert!(conflict.can_merge);
                merges += 1;
                assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice: CopyChoice::Merge { all: false },
                }));
            }
            Step::Failure(failure) => {
                assert_eq!(merges, expected.len());
                assert_eq!(failure.stage, CopyErrorStage::CreatingDirectory);
                assert_eq!(job.roots.front().unwrap().native_identity(), None);
                assert_eq!(preparations.load(Ordering::Relaxed), 1);
                check_refusal(&failure.error);
                let _ = job.finish();
                return;
            }
            Step::Done => panic!("Merge must refuse to write into the pending source referent"),
        }
    }
    panic!("the nested Merge did not reach its pending-source refusal");
}

#[test]
fn active_physical_ancestor_is_not_copied_through_a_link_backedge() {
    let fixture = Fixture::new();
    let original = fixture.folder.path().join("outer");
    let entry = fixture.prepare_link(
        &original,
        |identity| {
            let child = fixture.alias(&original.join("back"), None);
            fixture.read(&fixture.referent, &original, Some(identity), [child].into())
        },
        Vec::new(),
    );
    let mut job = fixture.prepare([entry].into());
    for _ in 0..100 {
        match job.step() {
            Step::Continue => {}
            Step::Failure(_) => {
                assert!(!fixture.destination.join("outer/back").exists());
                job.finish();
                assert!(original.exists());
                return;
            }
            Step::Conflict(_) | Step::Done => panic!("a physical ancestor must be refused"),
        }
    }
    panic!("cycle test did not reach the refusal")
}
