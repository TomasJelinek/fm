//! Actual queued Copy/Move cancellation with native writes and retained output pins.
use super::*;
use std::{
    ffi::OsStr,
    io::{Read, Seek, SeekFrom, Write},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Point {
    Write,
    Sync,
    Readback,
}
struct Control {
    point: Point,
    entered: Notify,
    released: AtomicBool,
    stages: AtomicUsize,
}
struct Release(Arc<Control>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.released.store(true, Ordering::Release);
    }
}
struct Service {
    inner: moves::CrossDeviceService,
    control: Arc<Control>,
}
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.inner.prepare_transfer_source(path, cancel)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        self.inner.prepare_move_source(path, cancel)
    }
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.0.prepare_checksum_directory(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            inner: self.inner.prepare_transfer_destination(path, cancel)?,
            control: self.control.clone(),
        }))
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            inner: self.inner.prepare_move_destination(path, cancel)?,
            control: self.control.clone(),
        }))
    }
}
struct Directory {
    inner: Arc<dyn TransferDirectory>,
    control: Arc<Control>,
}
impl Directory {
    fn wrap(&self, inner: Box<dyn StagedTransferFile>) -> Box<dyn StagedTransferFile> {
        let active = self.control.stages.fetch_add(1, Ordering::SeqCst) == 1;
        Box::new(Stage {
            inner,
            control: self.control.clone(),
            active,
            blocked: false,
        })
    }
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn capabilities(&self) -> TransferCapabilities {
        self.inner.capabilities()
    }
    fn supports_native_move_replacement(&self, source: TransferIdentity) -> bool {
        self.inner.supports_native_move_replacement(source)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        self.inner.prepare_target(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(source, cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.create_directory(name, cancel)
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner.stage_file(cancel).map(|stage| self.wrap(stage))
    }
    fn stage_file_for_target(
        &self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.inner
            .stage_file_for_target(name, target, policy, cancel)
            .map(|stage| self.wrap(stage))
    }
}
struct Stage {
    inner: Box<dyn StagedTransferFile>,
    control: Arc<Control>,
    active: bool,
    blocked: bool,
}
impl Stage {
    fn block(&mut self, point: Point) {
        if self.active && !self.blocked && self.control.point == point {
            self.blocked = true;
            self.control.entered.notify_one();
            while !self.control.released.load(Ordering::Acquire) {
                std::thread::park_timeout(Duration::from_millis(2));
            }
        }
    }
}
impl Read for Stage {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(data)?;
        self.block(Point::Readback);
        Ok(count)
    }
}
impl Write for Stage {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(data)?;
        self.block(Point::Write);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}
impl StagedTransferFile for Stage {
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.sync(cancel)?;
        self.block(Point::Sync);
        Ok(())
    }
    fn len(&self) -> Result<u64, FsError> {
        self.inner.len()
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferReadGuard>, FsError> {
        self.inner.rewind_for_validation(cancel)
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        self.inner.revalidate_publication(cancel)
    }
    fn prepare_replacement(
        &mut self,
        name: &OsStr,
        target: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.prepare_replacement(name, target, policy, cancel)
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_no_replace(name, cancel)
    }
    fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        self.inner.publish_replace(name, cancel)
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.inner.cleanup()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cancel_removes_only_unpublished_output_after_blocked_native_io() {
    for intent in [TransferIntent::Copy, TransferIntent::Move] {
        for verification in [
            TransferVerification::FullSha256,
            TransferVerification::SizeAndSamples,
        ] {
            for point in [Point::Write, Point::Sync, Point::Readback] {
                for replacement in [false, true] {
                    cancelled_case(intent, verification, point, replacement).await;
                }
            }
        }
    }
}

async fn cancelled_case(
    intent: TransferIntent,
    verification: TransferVerification,
    point: Point,
    replacement: bool,
) {
    let (_temp, parent) = moves::protected_fixture();
    let earlier = parent.join("earlier");
    let source = parent.join("current");
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&earlier, b"completed earlier").unwrap();
    let bytes = vec![0x5a; 300_017];
    fs::write(&source, &bytes).unwrap();
    if replacement {
        fs::write(destination.join("current"), b"old existing target").unwrap();
    }
    let control = Arc::new(Control {
        point,
        entered: Notify::new(),
        released: AtomicBool::new(false),
        stages: AtomicUsize::new(0),
    });
    let _release = Release(control.clone());
    let mut h = Harness::new(Some(Arc::new(Service {
        inner: moves::CrossDeviceService(midnight_platform::LocalFileSystem::new(None)),
        control: control.clone(),
    })));
    let mut request = request(1, &[earlier.clone(), source.clone()], &destination);
    request.verification = verification;
    if intent == TransferIntent::Move {
        h.controller.enqueue_move(request).await;
    } else {
        h.controller.enqueue(request).await;
    }
    if replacement {
        let conflict = h.conflict(1).await;
        assert!(conflict.can_replace);
        h.controller
            .decide(CopyDecision {
                id: conflict.id,
                version: conflict.version,
                choice: CopyChoice::ReplaceRecoverable { all: false },
            })
            .await;
    }
    tokio::time::timeout(Duration::from_secs(5), control.entered.notified())
        .await
        .unwrap();
    h.controller.cancel(OperationId(1));
    let premature = tokio::time::timeout(Duration::from_millis(80), async {
        loop {
            if let CopyEvent::Progress(progress) = h.next().await
                && progress.state.is_terminal()
            {
                break progress;
            }
        }
    })
    .await;
    let held = (h.io.available_permits(), h.mutation.available_permits());
    control.released.store(true, Ordering::Release);
    let premature_terminal = premature.is_ok();
    let done = match premature {
        Ok(progress) => progress,
        Err(_) => h.terminal(1).await,
    };
    h.close().await;
    assert!(
        !premature_terminal,
        "terminal cancellation before native {point:?} returned"
    );
    assert_eq!(held, (0, 0));
    assert_eq!(done.state, CopyOperationState::Cancelled, "{done:?}");
    assert!(done.errors.is_empty(), "{done:?}");
    assert_eq!(done.verification, verification);
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(
        fs::read(destination.join("earlier")).unwrap(),
        b"completed earlier"
    );
    assert_eq!(earlier.exists(), intent == TransferIntent::Copy);
    if replacement {
        assert_eq!(
            fs::read(destination.join("current")).unwrap(),
            b"old existing target"
        );
    } else {
        assert!(!destination.join("current").exists());
    }
    assert_eq!(
        fs::read_dir(&destination).unwrap().count(),
        if replacement { 2 } else { 1 },
        "owned partial output survived cancellation at {point:?}"
    );
}
