//! Exercise verified cross-volume phases using owned fixtures on one test volume.
//! Only the native eligibility answer is injected; real pinned reads, staging,
//! publication, proof, restoration and finalization remain the platform's code.
use super::*;

pub(in super::super) struct CrossDeviceService(
    pub(in super::super) midnight_platform::LocalFileSystem,
);
impl TransferService for CrossDeviceService {
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
        self.0.prepare_transfer_destination(path, cancel)
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_move_destination(path, cancel)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        self.0.prepare_move_source(path, cancel).map(wrap)
    }
}
fn wrap(prepared: PreparedTransferMove) -> PreparedTransferMove {
    PreparedTransferMove {
        source: Box::new(Source(prepared.source)),
        capture: Arc::new(Capture(prepared.capture)),
    }
}
struct Source(Box<dyn TransferSource>);
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        self.0.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn read_chunk(&mut self, bytes: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.0.read_chunk(bytes, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.0.next_child(cancel)
    }
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.0.next_move_child(cancel).map(|child| child.map(wrap))
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.0.rewind_for_validation(cancel)
    }
    fn seek_for_validation(&mut self, offset: u64, cancel: &Cancellation) -> Result<u64, FsError> {
        self.0.seek_for_validation(offset, cancel)
    }
    fn prepare_modified_time(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferModifiedTimePin>, FsError> {
        self.0.prepare_modified_time(cancel)
    }
    fn supports_read_retry(&self) -> bool {
        self.0.supports_read_retry()
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.retry_read_from(offset, cancel)
    }
}
struct Capture(Arc<dyn TransferMoveCapture>);
impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        self.0.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
    }
    fn try_native_move(
        &self,
        _: &dyn TransferDirectory,
        _: &OsStr,
        _: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        _: &Cancellation,
    ) -> Result<TransferNativeMoveOutcome, FsError> {
        Ok(TransferNativeMoveOutcome::CrossDevice)
    }
    fn preflight_finalization(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.preflight_finalization(cancel)
    }
    fn prepare_finalization(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        self.0.prepare_finalization(cancel)
    }
    fn remove_empty_directory(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryRemoval, FsError> {
        self.0.remove_empty_directory(cancel)
    }
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.0.capture(cancel)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.0.restore()
    }
    fn restoration_completed(&self) -> bool {
        self.0.restoration_completed()
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        self.0.take_stage_cleanup()
    }
}
