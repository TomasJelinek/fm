//! Skip of a Move entry's uncertain end keeps the source folders above the entry only when
//! the entry left something in them (`failures::Committed::leaves_source`).
use super::*;

/// What the children's removals of the wrapped provider end with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Removal {
    /// Removed and unstaged, but the folder's sync failed: its durability alone is uncertain.
    Unconfirmed,
    /// Removed, but the private stage cannot be removed: it stays in the source folder.
    StageKept,
}

/// The verified (cross-device) Move of the native provider, whose child removals end as
/// `removal` says.
struct Children {
    inner: CrossDeviceService,
    removal: Removal,
}
impl TransferService for Children {
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let prepared = self.inner.prepare_move_source(path, cancel)?;
        Ok(PreparedTransferMove {
            source: Box::new(Folder {
                inner: prepared.source,
                removal: self.removal,
            }),
            capture: prepared.capture,
        })
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_move_destination(path, cancel)
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.prepare_transfer_destination(path, cancel)
    }
}

/// The selected folder, whose children's captures end their removals as `removal` says.
struct Folder {
    inner: Box<dyn TransferSource>,
    removal: Removal,
}
impl Folder {
    fn wrap(&self, entry: PreparedTransferMoveEntry) -> PreparedTransferMoveEntry {
        match entry {
            PreparedTransferMoveEntry::Ordinary(prepared) => {
                PreparedTransferMoveEntry::Ordinary(PreparedTransferMove {
                    source: prepared.source,
                    capture: Arc::new(Child {
                        inner: prepared.capture,
                        removal: self.removal,
                    }),
                })
            }
            link => link,
        }
    }
}
impl TransferSource for Folder {
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.inner.discover(cancel, visit)
    }
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn open_for_reading(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.open_for_reading(cancel)
    }
    fn read_chunk(&mut self, bytes: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.inner.read_chunk(bytes, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner.next_child(cancel)
    }
    fn next_move_child_or_failure(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<TransferMoveChild>, FsError> {
        let child = self.inner.next_move_child_or_failure(links, cancel)?;
        Ok(child.map(|child| child.map(|entry| self.wrap(entry))))
    }
    fn prepare_move_child(
        &self,
        name: &OsStr,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMoveEntry, FsError> {
        let entry = self.inner.prepare_move_child(name, links, cancel)?;
        Ok(self.wrap(entry))
    }
}

struct Child {
    inner: Arc<dyn TransferMoveCapture>,
    removal: Removal,
}
impl TransferMoveCapture for Child {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn try_native_move(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        self.inner
            .try_native_move(destination, name, replace, cancel)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.preflight_finalization(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.inner.capture(cancel)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.inner.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.inner.restoration_completed()
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        let inner = self.inner.prepare_finalization(cancel)?;
        Ok(match self.removal {
            Removal::Unconfirmed => Box::new(Unconfirmed(inner)),
            Removal::StageKept => inner,
        })
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        let cleanup = self.inner.take_stage_cleanup()?;
        Some(match self.removal {
            Removal::Unconfirmed => cleanup,
            Removal::StageKept => Box::new(Kept(cleanup)),
        })
    }
}

/// Reports a removal as unlinked without a confirmed sync of its folder.
struct Unconfirmed(Box<dyn TransferMoveFinalizer>);
impl TransferMoveFinalizer for Unconfirmed {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        match self.0.finalize(cancel) {
            TransferMoveFinalizeOutcome::Removed {
                source, recovery, ..
            } => TransferMoveFinalizeOutcome::Removed {
                source,
                recovery,
                state: TransferSourceRemoval::Unlinked,
                durability_confirmed: false,
            },
            retained => retained,
        }
    }
}

/// Refuses to remove the private stage, which stays (empty) in the source folder.
struct Kept(Box<dyn TransferMoveStageCleanup>);
impl TransferMoveStageCleanup for Kept {
    fn path(&self) -> &Path {
        self.0.path()
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        Err(error(
            self.0.path(),
            io::ErrorKind::PermissionDenied,
            "injected stage cleanup refusal",
        ))
    }
}

#[test]
fn skip_keeps_the_moved_folder_only_when_the_entry_left_something_in_it() {
    for removal in [Removal::Unconfirmed, Removal::StageKept] {
        let (_temp, parent) = super::directories::tree_fixture();
        let folder = parent.join("folder 文档");
        let destination = parent.join("destination");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("child"), b"verified bytes").unwrap();
        fs::create_dir(&destination).unwrap();
        let request = request(1, std::slice::from_ref(&folder), &destination);
        let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
        let service = Children {
            inner: CrossDeviceService(midnight_platform::LocalFileSystem::new(None)),
            removal,
        };
        let mut job =
            worker::Job::prepare_move(request, &service, Cancellation::default(), status).unwrap();
        let mut asked = Vec::new();
        for _ in 0..1000 {
            match job.step() {
                worker::Step::Continue => {}
                worker::Step::Done => break,
                worker::Step::Failure(failure) => {
                    assert!(failure.can_skip && !failure.can_retry, "{failure:?}");
                    asked.push(failure.stage);
                    assert!(job.accept_decision(CopyDecision {
                        id: failure.id,
                        version: failure.version,
                        choice: CopyChoice::Skip,
                    }));
                }
                worker::Step::Conflict(conflict) => panic!("{removal:?}: {conflict:?}"),
            }
        }
        let done = job.finish();
        assert_eq!(
            done.state,
            CopyOperationState::CompletedWithErrors,
            "{removal:?}: {done:?}"
        );
        // The child was moved and its copy verified either way.
        assert_eq!(
            fs::read(destination.join("folder 文档/child")).unwrap(),
            b"verified bytes"
        );
        assert!(!folder.join("child").exists());
        match removal {
            // Nothing of the child is left: the emptied folder goes, as without the
            // question, and no other question follows.
            Removal::Unconfirmed => {
                assert_eq!(asked, [CopyErrorStage::RemovingSource], "{done:?}");
                assert!(!folder.exists(), "{done:?}");
                assert_eq!(done.source_residual_count, 0, "{done:?}");
            }
            // The child's stage stays in the folder, so the folder stays, unasked.
            Removal::StageKept => {
                assert_eq!(asked, [CopyErrorStage::Cleanup], "{done:?}");
                assert_eq!(fs::read_dir(&folder).unwrap().count(), 1, "{done:?}");
                assert_eq!(done.source_residual_count, 1, "{done:?}");
            }
        }
    }
}
