//! A source removal that the platform completes only when the job's own handles on the
//! removed entry close (a Windows delete disposition) is confirmed, and counted, only after
//! the job has dropped its source, capture and finalizer; deferred rename confirmations
//! run together (`TransferDurabilityConfirmation`).
use super::*;
use std::sync::{
    Weak,
    atomic::{AtomicUsize, Ordering},
};

/// The confirmation of each removal the wrapped provider reports as pending deletion:
/// records whether every wrapper of the removed entry was dropped when the job ran it, and
/// then answers `confirms`.
struct Probe {
    owners: Mutex<Weak<()>>,
    confirms: bool,
    ran: Mutex<Vec<bool>>,
}
impl Probe {
    /// A new token for the wrappers of one prepared entry.
    fn owner(&self) -> Arc<()> {
        let owner = Arc::new(());
        *self.owners.lock().unwrap() = Arc::downgrade(&owner);
        owner
    }
}
struct Confirmation(Arc<Probe>);
impl TransferDurabilityConfirmation for Confirmation {
    fn confirm(self: Box<Self>, _: &mut TransferDurabilityRound) -> bool {
        let dropped = self.0.owners.lock().unwrap().strong_count() == 0;
        self.0.ran.lock().unwrap().push(dropped);
        self.0.confirms
    }
}

/// The verified (cross-device) Move of the native provider, whose removals are reported
/// as pending deletion with a `Probe` confirmation, as Windows reports them.
struct Deferring {
    inner: CrossDeviceService,
    probe: Arc<Probe>,
}
impl TransferService for Deferring {
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let prepared = self.inner.prepare_move_source(path, cancel)?;
        let owner = self.probe.owner();
        Ok(PreparedTransferMove {
            source: Box::new(Source {
                inner: prepared.source,
                _owner: owner.clone(),
            }),
            capture: Arc::new(Capture {
                inner: prepared.capture,
                owner,
                probe: self.probe.clone(),
            }),
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

struct Source {
    inner: Box<dyn TransferSource>,
    _owner: Arc<()>,
}
impl TransferSource for Source {
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
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.inner.next_move_child(cancel)
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.inner.seek_for_validation(offset, cancel)
    }
}

struct Capture {
    inner: Arc<dyn TransferMoveCapture>,
    owner: Arc<()>,
    probe: Arc<Probe>,
}
impl TransferMoveCapture for Capture {
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
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        self.inner.take_stage_cleanup()
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        Ok(Box::new(Finalizer {
            inner: self.inner.prepare_finalization(cancel)?,
            _owner: self.owner.clone(),
            probe: self.probe.clone(),
        }))
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        self.inner.remove_empty_directory(cancel)?;
        Ok(TransferDirectoryRemoval {
            state: TransferSourceRemoval::DeletePending,
            durability_confirmed: false,
        })
    }
    fn take_removal_confirmation(&self) -> Option<Box<dyn TransferDurabilityConfirmation>> {
        Some(Box::new(Confirmation(self.probe.clone())))
    }
}

struct Finalizer {
    inner: Box<dyn TransferMoveFinalizer>,
    _owner: Arc<()>,
    probe: Arc<Probe>,
}
impl TransferMoveFinalizer for Finalizer {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        match self.inner.finalize(cancel) {
            TransferMoveFinalizeOutcome::Removed {
                source, recovery, ..
            } => TransferMoveFinalizeOutcome::Removed {
                source,
                recovery,
                state: TransferSourceRemoval::DeletePending,
                durability_confirmed: false,
            },
            retained => retained,
        }
    }
    fn take_removal_confirmation(&mut self) -> Option<Box<dyn TransferDurabilityConfirmation>> {
        Some(Box::new(Confirmation(self.probe.clone())))
    }
}

#[test]
fn a_pending_removal_is_confirmed_and_counted_only_after_the_job_drops_its_handles() {
    for confirms in [true, false] {
        let (_temp, parent) = super::directories::tree_fixture();
        let file = parent.join("file");
        let folder = parent.join("folder");
        let destination = parent.join("destination");
        fs::write(&file, b"verified bytes").unwrap();
        fs::create_dir(&folder).unwrap();
        fs::create_dir(&destination).unwrap();
        // A file (removed by its finalizer) and an emptied folder (removed by its capture
        // while its traversal frame still holds its source), each in a Move of its own.
        for source in [&file, &folder] {
            let probe = Arc::new(Probe {
                owners: Mutex::new(Weak::new()),
                confirms,
                ran: Mutex::new(Vec::new()),
            });
            let request = request(1, std::slice::from_ref(source), &destination);
            let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
            let service = Deferring {
                inner: CrossDeviceService(midnight_platform::LocalFileSystem::new(None)),
                probe: probe.clone(),
            };
            let mut job =
                worker::Job::prepare_move(request, &service, Cancellation::default(), status)
                    .unwrap();
            for _ in 0..1000 {
                match job.step() {
                    worker::Step::Continue => {}
                    worker::Step::Done => break,
                    worker::Step::Failure(failure) => panic!("{source:?}: {failure:?}"),
                    worker::Step::Conflict(conflict) => panic!("{source:?}: {conflict:?}"),
                }
            }
            let done = job.finish();
            assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
            // Run once, with every source, capture and finalizer of the entry dropped.
            assert_eq!(*probe.ran.lock().unwrap(), [true], "{source:?}");
            let counted = u64::from(!confirms);
            assert_eq!(
                (
                    done.source_delete_pending,
                    done.source_removal_durability_unconfirmed
                ),
                (counted, counted),
                "{source:?}: {done:?}"
            );
            assert!(!source.exists(), "{source:?}");
        }
        assert_eq!(
            fs::read(destination.join("file")).unwrap(),
            b"verified bytes"
        );
        assert!(destination.join("folder").is_dir());
    }
}

/// What deferred rename confirmations did: the folders they saved, and with the folder
/// removals, the order things ran in.
#[derive(Default)]
struct Saves {
    confirms: bool,
    folders: AtomicUsize,
    log: Mutex<Vec<&'static str>>,
}
/// Each confirmation saves the rename's destination folder through its round.
struct Saving {
    folder: TransferIdentity,
    saves: Arc<Saves>,
}
impl TransferDurabilityConfirmation for Saving {
    fn confirm(self: Box<Self>, round: &mut TransferDurabilityRound) -> bool {
        self.saves.log.lock().unwrap().push("confirm");
        let saves = &self.saves;
        round.folder(self.folder, || {
            saves.folders.fetch_add(1, Ordering::SeqCst);
            saves.confirms
        })
    }
}

/// The native provider, whose file renames leave saving their folder to a `Saving`
/// confirmation (as Windows does in a Move) and whose folders go the verified way.
struct Renaming {
    inner: midnight_platform::LocalFileSystem,
    saves: Arc<Saves>,
}
impl TransferService for Renaming {
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let prepared = self.inner.prepare_move_source(path, cancel)?;
        Ok(PreparedTransferMove {
            source: prepared.source,
            capture: Arc::new(Renamed {
                inner: prepared.capture,
                saves: self.saves.clone(),
            }),
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
struct Renamed {
    inner: Arc<dyn TransferMoveCapture>,
    saves: Arc<Saves>,
}
impl TransferMoveCapture for Renamed {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn try_native_move_deferred(
        &self,
        destination: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<DeferredNativeMove, FsError> {
        if self.entry().item.kind == EntryKind::Directory {
            return Ok((TransferNativeMoveOutcome::CrossDevice, None));
        }
        let outcome = self
            .inner
            .try_native_move(destination, name, replace, cancel)?;
        let saving = Box::new(Saving {
            folder: destination.identity(),
            saves: self.saves.clone(),
        });
        Ok((outcome, Some(saving)))
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
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        self.inner.take_stage_cleanup()
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        self.inner.prepare_finalization(cancel)
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        self.saves.log.lock().unwrap().push("remove");
        self.inner.remove_empty_directory(cancel)
    }
    fn take_removal_confirmation(&self) -> Option<Box<dyn TransferDurabilityConfirmation>> {
        self.inner.take_removal_confirmation()
    }
}

#[test]
fn deferred_rename_confirmations_save_each_folder_once_before_a_folder_is_removed() {
    for confirms in [true, false] {
        let (_temp, parent) = super::directories::tree_fixture();
        let [first, second, folder] = ["first", "second", "folder"].map(|name| parent.join(name));
        let destination = parent.join("destination");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        fs::create_dir(&folder).unwrap();
        fs::create_dir(&destination).unwrap();
        let saves = Arc::new(Saves {
            confirms,
            ..Saves::default()
        });
        let request = request(
            1,
            &[first.clone(), second.clone(), folder.clone()],
            &destination,
        );
        let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
        let service = Renaming {
            inner: midnight_platform::LocalFileSystem::new(None),
            saves: saves.clone(),
        };
        let mut job =
            worker::Job::prepare_move(request, &service, Cancellation::default(), status).unwrap();
        for _ in 0..1000 {
            match job.step() {
                worker::Step::Continue => {}
                worker::Step::Done => break,
                worker::Step::Failure(failure) => panic!("{failure:?}"),
                worker::Step::Conflict(conflict) => panic!("{conflict:?}"),
            }
        }
        let done = job.finish();
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert_eq!(done.native_entries_moved, 2, "{done:?}");
        // Both renames are confirmed in one round, before the folder's removal, which saves
        // the destination folder they share once.
        assert_eq!(*saves.log.lock().unwrap(), ["confirm", "confirm", "remove"]);
        assert_eq!(saves.folders.load(Ordering::SeqCst), 1);
        assert_eq!(
            done.source_removal_durability_unconfirmed,
            if confirms { 0 } else { 2 },
            "{done:?}"
        );
        assert_eq!(fs::read(destination.join("second")).unwrap(), b"second");
        assert!(!first.exists() && !second.exists() && !folder.exists());
    }
}
