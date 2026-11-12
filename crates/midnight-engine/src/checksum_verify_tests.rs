use super::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Fixture {
    path: PathBuf,
    manifest: Vec<u8>,
    change_manifest: bool,
    change_data: bool,
    fail_data: bool,
    cancel_data: bool,
    data_opens: AtomicUsize,
    data_reads: AtomicUsize,
}
impl Fixture {
    fn new() -> Self {
        Self {
            path: std::env::current_dir()
                .unwrap()
                .join("_build/virtual-verification"),
            manifest: b"900150983cd24fb0d6963f7d28e17f72 *data\n".to_vec(),
            change_manifest: false,
            change_data: false,
            fail_data: false,
            cancel_data: false,
            data_opens: AtomicUsize::new(0),
            data_reads: AtomicUsize::new(0),
        }
    }
}
struct Provider(Arc<Fixture>);
impl TransferService for Provider {
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        assert_eq!(path, self.0.path);
        Ok(Arc::new(Directory(self.0.clone())))
    }
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("verification must use the pinned manifest directory")
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("verification must never request a write destination")
    }
}
struct Directory(Arc<Fixture>);
fn identity() -> TransferIdentity {
    TransferIdentity {
        device: 1,
        object: [1; 16],
    }
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.0.path
    }
    fn identity(&self) -> TransferIdentity {
        identity()
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn open_read_child(
        &self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let manifest = name == "files.md5";
        if !manifest {
            self.0.data_opens.fetch_add(1, Ordering::Relaxed);
        }
        let data = if manifest {
            self.0.manifest.clone()
        } else {
            b"abc".to_vec()
        };
        Ok(Box::new(Source {
            fixture: self.0.clone(),
            manifest,
            offset: 0,
            entry: TransferEntry {
                item: MutationItem {
                    path: self.path().join(name),
                    kind: EntryKind::File,
                    metadata: EntryMetadata {
                        size: Some(data.len() as u64),
                        ..Default::default()
                    },
                },
                identity: identity(),
            },
            data,
            changed: AtomicBool::new(false),
        }))
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        panic!("write-only capability")
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        panic!("write-only capability")
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("write-only capability")
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("write-only capability")
    }
}
struct Source {
    fixture: Arc<Fixture>,
    manifest: bool,
    offset: usize,
    entry: TransferEntry,
    data: Vec<u8>,
    changed: AtomicBool,
}
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self.changed.load(Ordering::Relaxed) {
            Err(worker::failure(
                &self.entry.item.path,
                io::ErrorKind::InvalidData,
                "source changed",
            ))
        } else {
            Ok(())
        }
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        if !self.manifest {
            self.fixture.data_reads.fetch_add(1, Ordering::Relaxed);
            if self.fixture.fail_data {
                return Err(worker::failure(
                    &self.entry.item.path,
                    io::ErrorKind::PermissionDenied,
                    "unreadable",
                ));
            }
        }
        let count = buffer.len().min(self.data.len() - self.offset);
        buffer[..count].copy_from_slice(&self.data[self.offset..self.offset + count]);
        self.offset += count;
        if (self.manifest && self.fixture.change_manifest)
            || (!self.manifest && self.fixture.change_data)
        {
            self.changed.store(true, Ordering::Relaxed);
        }
        if !self.manifest && self.fixture.cancel_data {
            cancel.cancel();
        }
        Ok(count)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        panic!("verification must not enumerate unlisted files")
    }
}
fn run(fixture: Arc<Fixture>) -> Arc<ChecksumRun> {
    verify(
        ChecksumVerifyRequest {
            job: ChecksumJobId(41),
            generation: ChecksumGeneration(7),
            manifest: fixture.path.join("files.md5"),
            limits: ChecksumLimits::default(),
        },
        &Provider(fixture),
        &Cancellation::default(),
        ChecksumRetention::new(()),
        |_| true,
    )
}

#[test]
fn changed_manifest_is_rejected_before_any_listed_source_opens() {
    let fixture = Arc::new(Fixture {
        change_manifest: true,
        ..Fixture::new()
    });
    let result = run(fixture.clone());
    assert_eq!(result.summary.completion, ChecksumCompletion::Failed);
    assert!(result.rows.is_empty());
    assert_eq!(fixture.data_opens.load(Ordering::Relaxed), 0);
}

#[test]
fn malformed_manifest_preflights_every_entry_before_any_listed_source_opens() {
    let fixture = Arc::new(Fixture {
        manifest: b"900150983cd24fb0d6963f7d28e17f72 *data\ninvalid *other\n".to_vec(),
        ..Fixture::new()
    });
    let result = run(fixture.clone());
    assert_eq!(result.summary.completion, ChecksumCompletion::Failed);
    assert_eq!(fixture.data_opens.load(Ordering::Relaxed), 0);
}

#[test]
fn changed_and_unreadable_listed_files_never_expose_success_or_a_partial_digest() {
    for (change_data, fail_data, expected) in [
        (true, false, ChecksumRowState::SourceChanged),
        (false, true, ChecksumRowState::ReadError),
    ] {
        let result = run(Arc::new(Fixture {
            change_data,
            fail_data,
            ..Fixture::new()
        }));
        assert_eq!(
            result.summary.completion,
            ChecksumCompletion::CompletedWithErrors
        );
        assert_eq!(result.rows[0].state, expected);
        assert!(result.rows[0].digests.is_empty());
        assert_eq!(result.summary.progress.verified, 0);
    }
}

#[test]
fn cancellation_during_a_native_data_read_keeps_the_row_unverified() {
    let result = run(Arc::new(Fixture {
        cancel_data: true,
        ..Fixture::new()
    }));
    assert_eq!(result.summary.completion, ChecksumCompletion::Cancelled);
    assert_eq!(result.rows[0].state, ChecksumRowState::Cancelled);
    assert!(result.rows[0].digests.is_empty());
    assert_eq!(result.summary.progress.verified, 0);
}

#[test]
fn actual_reads_are_bound_to_the_captured_manifest_and_generation() {
    let fixture = Arc::new(Fixture::new());
    let result = run(fixture.clone());
    assert_eq!(result.job, ChecksumJobId(41));
    assert_eq!(result.generation, ChecksumGeneration(7));
    assert_eq!(result.rows[0].path, fixture.path.join("data"));
    assert_eq!(result.rows[0].state, ChecksumRowState::Verified);
    assert_eq!(result.summary.progress.verified, 1);
    assert_eq!(fixture.data_reads.load(Ordering::Relaxed), 2);
}
