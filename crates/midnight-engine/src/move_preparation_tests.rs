//! Move root ancestry checks open each distinct selected-source parent once.
use super::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Parents {
    prepared: Mutex<Vec<PathBuf>>,
    checks: AtomicUsize,
}
struct Service(Arc<Parents>);
impl TransferService for Service {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Err(unsupported(path, "Move preparation reads no content."))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(unsupported(path, "Move preparation writes nothing."))
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.0.prepared.lock().unwrap().push(path.to_owned());
        Ok(Arc::new(Parent {
            path: path.to_owned(),
            parents: self.0.clone(),
        }))
    }
}
struct Parent {
    path: PathBuf,
    parents: Arc<Parents>,
}
impl TransferDirectory for Parent {
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        TransferIdentity {
            device: 1,
            object: [9; 16],
        }
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        Ok(None)
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        self.parents.checks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Err(unsupported(&self.path, "Move preparation writes nothing."))
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        Err(unsupported(&self.path, "Move preparation writes nothing."))
    }
}
struct Root(TransferEntry);
impl TransferSource for Root {
    fn entry(&self) -> &TransferEntry {
        &self.0
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Err(unsupported(
            &self.0.item.path,
            "Move preparation reads no content.",
        ))
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Err(unsupported(
            &self.0.item.path,
            "Move preparation lists nothing.",
        ))
    }
}
fn root(path: PathBuf, kind: EntryKind, object: u8) -> Box<dyn TransferSource> {
    Box::new(Root(TransferEntry {
        item: MutationItem {
            path,
            kind,
            metadata: EntryMetadata::default(),
        },
        identity: TransferIdentity {
            device: 1,
            object: [object; 16],
        },
    }))
}

#[test]
fn shared_source_parents_are_prepared_and_checked_once() {
    let base = std::env::current_dir()
        .unwrap()
        .join("_build/virtual-move-roots");
    let parents = Arc::new(Parents::default());
    let roots = VecDeque::from([
        root(base.join("folder"), EntryKind::Directory, 1),
        root(base.join("a.txt"), EntryKind::File, 2),
        root(base.join("b.txt"), EntryKind::File, 3),
        root(base.join("other/c.txt"), EntryKind::File, 4),
    ]);
    reject_nested_roots(&roots, &Service(parents.clone()), &Cancellation::default()).unwrap();
    assert_eq!(
        *parents.prepared.lock().unwrap(),
        [base.clone(), base.join("other")]
    );
    // Every distinct parent is checked against the one selected directory.
    assert_eq!(parents.checks.load(Ordering::SeqCst), 2);
}

#[test]
fn file_only_roots_need_no_parent_ancestry_checks() {
    let base = std::env::current_dir()
        .unwrap()
        .join("_build/virtual-move-roots");
    let parents = Arc::new(Parents::default());
    let roots = VecDeque::from([
        root(base.join("a.txt"), EntryKind::File, 2),
        root(base.join("b.txt"), EntryKind::File, 3),
    ]);
    reject_nested_roots(&roots, &Service(parents.clone()), &Cancellation::default()).unwrap();
    assert!(parents.prepared.lock().unwrap().is_empty());
}
