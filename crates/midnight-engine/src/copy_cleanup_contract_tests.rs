//! Cancellation must keep native cleanup diagnostics, even after earlier failures.
use super::*;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

fn refusal(path: &Path) -> FsError {
    FsError::from_io(
        "remove owned temporary output",
        path,
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "cleanup refused; original output retained",
        ),
    )
}
struct Stage {
    file: fs::File,
    path: PathBuf,
}
impl Read for Stage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes)
    }
}
impl Write for Stage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}
impl StagedTransferFile for Stage {
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
        self.file
            .sync_all()
            .map_err(|e| FsError::from_io("sync fixture", &self.path, e))
    }
    fn len(&self) -> Result<u64, FsError> {
        Ok(self.file.metadata().unwrap().len())
    }
    fn modified_time(&self, cancel: &Cancellation) -> Result<std::time::SystemTime, FsError> {
        cancel.cancel();
        Err(FsError::from_io(
            "probe metadata",
            &self.path,
            io::ErrorKind::Interrupted.into(),
        ))
    }
    fn publish_no_replace(
        &mut self,
        _: &std::ffi::OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        panic!("cancelled stage must not publish")
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        Err(refusal(&self.path))
    }
}
struct Directory {
    inner: Arc<dyn TransferDirectory>,
    stage: PathBuf,
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn identity(&self) -> TransferIdentity {
        self.inner.identity()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn inspect(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.inner.inspect(name, cancel)
    }
    fn reject_source_ancestor(
        &self,
        id: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.inner.reject_source_ancestor(id, cancel)
    }
    fn create_directory(
        &self,
        name: &std::ffi::OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.inner.create_directory(name, cancel)
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&self.stage)
            .unwrap();
        file.write_all(b"owned partial").unwrap();
        Ok(Box::new(Stage {
            file,
            path: self.stage.clone(),
        }))
    }
    fn stage_file_with_metadata(
        &self,
        _: TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.stage_file(cancel)
    }
}
fn fixture() -> (
    tempfile::TempDir,
    midnight_platform::LocalFileSystem,
    Arc<dyn TransferDirectory>,
    PathBuf,
    PathBuf,
    Status,
) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"original source").unwrap();
    fs::create_dir(&destination).unwrap();
    let stage = destination.join(".me-refused");
    let service = midnight_platform::LocalFileSystem::new(None);
    let directory = service
        .prepare_transfer_destination(&destination, &Cancellation::default())
        .unwrap();
    let status = Arc::new(Mutex::new(CopyProgress::queued(
        OperationId(1),
        destination,
        1,
        TransferIntent::Copy,
    )));
    (temp, service, directory, source, stage, status)
}

#[test]
fn cancelled_native_error_keeps_exact_owned_partial_receipt() {
    let (_temp, _service, directory, _source, stage, status) = fixture();
    let cancel = Cancellation::default();
    let mut job = Job::from_sources(directory, VecDeque::new(), cancel.clone(), status.clone());
    cancel.cancel();
    assert!(matches!(
        job.wait_for_failure(refusal(&stage), failures::Context::Entry),
        Step::Done
    ));
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Cancelled);
    assert!(
        done.errors
            .iter()
            .any(|e| e.path == stage && e.kind == io::ErrorKind::PermissionDenied),
        "cancel erased a real cleanup refusal"
    );
}

#[test]
fn cancelled_preparation_keeps_exact_owned_partial_receipt() {
    let (_temp, _service, _directory, _source, stage, status) = fixture();
    let cancel = Cancellation::default();
    cancel.cancel();
    let preparation = failed(&status, refusal(&stage), &cancel);
    assert!(
        preparation.errors.iter().any(|e| e.path == stage),
        "cancelled preparation erased its owned partial receipt"
    );
}

#[test]
fn cancelled_cleanup_receipt_survives_sixteen_earlier_errors() {
    cleanup_receipt_survives(0);
}

#[test]
fn active_partial_receipt_survives_seventeen_later_metadata_cleanup_failures() {
    cleanup_receipt_survives(17);
}

fn cleanup_receipt_survives(pending_stages: u8) {
    let (_temp, service, directory, source, stage, status) = fixture();
    let cancel = Cancellation::default();
    let source_pin = service.prepare_transfer_source(&source, &cancel).unwrap();
    let entry = source_pin.entry().clone();
    let staged = Directory {
        inner: directory.clone(),
        stage: stage.clone(),
    }
    .stage_file(&cancel)
    .unwrap();
    let mut job = Job::from_sources(directory.clone(), VecDeque::new(), cancel.clone(), status);
    for number in 0..16 {
        job.record_decided_error(refusal(Path::new(&format!("earlier-{number}"))));
    }
    let mut stages = std::collections::HashMap::new();
    for number in 0..pending_stages {
        let pending = stage.with_file_name(format!(".me-unused-{number}"));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&pending)
            .unwrap();
        let stage: Box<dyn StagedTransferFile> = Box::new(Stage {
            file,
            path: pending,
        });
        stages.insert(
            TransferIdentity {
                device: 100,
                object: [number; 16],
            },
            stage,
        );
    }
    job.metadata = Some(metadata_job::MetadataWork {
        pins: Default::default(),
        stages,
    });
    job.current = Some(EntryWork {
        entry,
        native_identity: source_pin.native_identity(),
        source: None,
        copy: Some(VerifiedFileCopy::new(source_pin, staged).unwrap()),
        link: None,
        destination: directory,
        name: "final".into(),
        target: None,
        consent: None,
        replacement_prepared: false,
        native_fallback: false,
        move_preflight_done: false,
        force_prompt: false,
    });
    cancel.cancel();
    let done = job.finish();
    assert_eq!(done.failed, 17 + u64::from(pending_stages));
    assert_eq!(done.errors.len(), 16);
    assert!(
        done.errors.iter().any(|e| e.path == stage),
        "bounded history hid the remaining partial output"
    );
    assert!(done.message.contains("omitted"));
    assert_eq!(fs::read(&source).unwrap(), b"original source");
    assert_eq!(fs::read(&stage).unwrap(), b"owned partial");
}

#[test]
fn cancelled_metadata_probe_promotes_cleanup_failure_over_interrupted() {
    let (_temp, service, directory, source, stage, _status) = fixture();
    let cancel = Cancellation::default();
    let roots = VecDeque::from([service.prepare_transfer_source(&source, &cancel).unwrap()]);
    let destination: Arc<dyn TransferDirectory> = Arc::new(Directory {
        inner: directory,
        stage: stage.clone(),
    });
    let Err(error) = metadata_job::stages(
        &roots,
        &destination,
        TransferMetadataPolicy::RequireModifiedTime,
        false,
        &cancel,
    ) else {
        panic!("cancelled metadata probe succeeded")
    };
    let error = error.error;
    assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
    assert_eq!(error.path, stage);
    assert!(error.message.contains("probe metadata"));
}
