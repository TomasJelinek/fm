use super::*;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Provider {
    child_kind: EntryKind,
    reads: Arc<AtomicUsize>,
    stages: Arc<AtomicUsize>,
}
struct Node {
    entry: TransferEntry,
    child: Option<Box<Node>>,
    reads: Arc<AtomicUsize>,
}
fn entry(path: PathBuf, kind: EntryKind, device: u64) -> TransferEntry {
    TransferEntry {
        item: MutationItem {
            path,
            kind,
            metadata: EntryMetadata {
                size: (kind == EntryKind::File).then_some(0),
                ..Default::default()
            },
        },
        identity: TransferIdentity {
            device,
            object: [device as u8; 16],
        },
    }
}
impl TransferService for Provider {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(Node {
            entry: entry(path.to_owned(), EntryKind::Directory, 1),
            child: Some(Box::new(Node {
                entry: entry(path.join("mounted"), self.child_kind, 2),
                child: None,
                reads: self.reads.clone(),
            })),
            reads: self.reads.clone(),
        }))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            stages: self.stages.clone(),
        }))
    }
}
impl TransferSource for Node {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(0)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Ok(self
            .child
            .take()
            .map(|child| child as Box<dyn TransferSource>))
    }
}
struct Directory {
    path: PathBuf,
    stages: Arc<AtomicUsize>,
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        TransferIdentity {
            device: 2,
            object: [2; 16],
        }
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        Ok(None)
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(worker::failure(
            &self.path,
            io::ErrorKind::Unsupported,
            "Unexpected directory creation.",
        ))
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.stages.fetch_add(1, Ordering::Relaxed);
        Err(worker::failure(
            &self.path,
            io::ErrorKind::Other,
            "A different-volume save reached staging.",
        ))
    }
}
fn provider(kind: EntryKind) -> Provider {
    Provider {
        child_kind: kind,
        reads: Arc::default(),
        stages: Arc::default(),
    }
}

#[test]
fn recursive_mount_boundaries_are_skipped_before_content_reads_or_subtree_traversal() {
    let root = std::env::current_dir()
        .unwrap()
        .join("_build/virtual-mount");
    for kind in [EntryKind::File, EntryKind::Directory] {
        let service = provider(kind);
        let mut request =
            ChecksumRequest::new(ChecksumJobId(1), ChecksumGeneration(1), vec![root.clone()]);
        request.recursive = true;
        let run = worker::calculate(
            request,
            &service,
            &worker::UNCLASSIFIED,
            &Cancellation::default(),
            ChecksumRetention::new(()),
            |_| true,
        );
        assert_eq!(service.reads.load(Ordering::Relaxed), 0);
        assert_eq!(run.rows.len(), 1);
        assert_eq!(run.rows[0].state, ChecksumRowState::Skipped);
        assert!(run.rows[0].digests.is_empty());
        assert_eq!(
            run.summary.completion,
            if kind == EntryKind::Directory {
                ChecksumCompletion::Incomplete
            } else {
                ChecksumCompletion::CompletedWithErrors
            }
        );
    }
}

#[test]
fn different_native_volume_is_refused_before_manifest_staging_even_with_a_common_lexical_base() {
    let directory = std::env::current_dir()
        .unwrap()
        .join("_build/virtual-volume");
    let retention = ChecksumRetention::new(());
    let rows = Arc::new(ChecksumChunk {
        base: directory.as_path().into(),
        rows: vec![ChecksumRow {
            id: ChecksumRowId(1),
            name: Path::new("mounted/file").into(),
            identity: Some(TransferIdentity {
                device: 1,
                object: [1; 16],
            }),
            kind: Some(EntryKind::File),
            state: ChecksumRowState::Calculated,
            bytes_read: 0,
            digests: ChecksumDigests::new(&[(ChecksumAlgorithm::Sha256, &[0; 32])]).unwrap(),
            error: None,
        }]
        .into(),
        retention: retention.clone(),
    })
    .into();
    let run = ChecksumRun {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(1),
        algorithms: ChecksumAlgorithms::default(),
        captured_roots: vec![directory.join("mounted/file")].into(),
        rows,
        retention,
        summary: ChecksumSummary {
            completion: ChecksumCompletion::Completed,
            progress: ChecksumProgress::default(),
            diagnostics: vec![],
        },
    };
    let request = ChecksumManifestRequest {
        algorithm: ChecksumAlgorithm::Sha256,
        destination: directory.join("out.sha256"),
        acknowledge_partial: None,
        overwrite: false,
    };
    let service = provider(EntryKind::File);
    let error = save_manifest(&run, &request, &service, &Cancellation::default()).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidInput);
    assert_eq!(service.stages.load(Ordering::Relaxed), 0);
}
