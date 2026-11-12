//! Namespace restoration and its durability error are independent facts.
use super::*;

struct SyncFailureCapture(Arc<dyn TransferMoveCapture>);
impl TransferMoveCapture for SyncFailureCapture {
    fn entry(&self) -> &TransferEntry {
        self.0.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.0.revalidate(cancel)
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
    fn capture(&self, cancel: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        self.0.capture(cancel)
    }
    fn restore(&self) -> Result<(), FsError> {
        self.0.restore()?;
        Err(error(
            &self.entry().item.path,
            io::ErrorKind::Other,
            "Injected directory sync failure after successful source restoration",
        ))
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn midnight_core::TransferMoveStageCleanup>> {
        self.0.take_stage_cleanup()
    }
    fn restoration_completed(&self) -> bool {
        self.0.restoration_completed()
    }
}

struct SyncFailureService(midnight_platform::LocalFileSystem);
impl TransferService for SyncFailureService {
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let mut prepared = self.0.prepare_move_source(path, cancel)?;
        prepared.capture = Arc::new(SyncFailureCapture(prepared.capture));
        Ok(prepared)
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepare_move_destination(path, cancel)
    }
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
}

#[test]
fn successful_restore_with_sync_error_reports_error_without_fictitious_staged_source() {
    let (_temp, parent) = protected_fixture();
    let source = parent.join("source 文档");
    let destination = parent.join("destination");
    fs::write(&source, b"original").unwrap();
    fs::create_dir(&destination).unwrap();
    let cancel = Cancellation::default();
    let request = request(1, std::slice::from_ref(&source), &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let service = SyncFailureService(midnight_platform::LocalFileSystem::new(None));
    let mut job =
        worker::Job::prepare_move(request, &service, cancel.clone(), status.clone()).unwrap();
    for _ in 0..1000 {
        assert!(matches!(job.step(), worker::Step::Continue));
        if snapshot(&status).source_names_staged == 1 {
            break;
        }
    }
    assert_eq!(snapshot(&status).source_names_staged, 1);
    cancel.cancel();
    assert!(matches!(job.step(), worker::Step::Done));
    let done = job.finish();
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(done.source_files_removed, 0);
    assert_eq!(done.failed, 1);
    assert!(done.message.contains("sync failure"));
    assert_eq!(done.source_residual_count, 0, "{done:?}");
    assert!(done.source_residuals.is_empty());
    assert_eq!(fs::read_dir(parent).unwrap().count(), 3);
}
