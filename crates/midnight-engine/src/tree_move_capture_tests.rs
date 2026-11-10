//! Provider preflight and independent authority lease lifetimes.
use super::*;
use crate::{
    providers::{
        ArchiveTarget, MountedFilesystem, OpenedLocation, ProviderEntry, ProviderRegistry,
    },
    tree_actor::WorkerBudget,
    tree_projection::ProjectionOptions,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Archive;
impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        77
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("Move must never enumerate archives")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("Move must never extract archives")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}
struct Mount {
    root: PathBuf,
    mixed: bool,
}
fn token(n: u8) -> NodeToken {
    NodeToken::new(&[n]).unwrap()
}
impl MountedFilesystem for Mount {
    fn list(
        &self,
        _: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        for n in 1..=2 {
            emit(ProviderEntry {
                token: token(n),
                raw_name: Arc::from(format!("file-{n}").as_bytes()),
                display_name: format!("file-{n}"),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            });
        }
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
    fn resolve_target(
        &self,
        node: &NodeToken,
        _: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        let n = node.as_bytes()[0];
        if !self.mixed || n != 2 {
            return Ok(ProviderTarget::Native(self.root.join(format!("file-{n}"))));
        }
        let node = ArchiveNode {
            modified: None,
            id: VirtualEntryId {
                mount: 77,
                entry: 2,
            },
            parent: 1,
            raw_name: Arc::from(&b"file-2"[..]),
            display_name: "native-looking file".into(),
            kind: EntryKind::File,
            size: Some(1),
            readable: true,
        };
        Ok(ProviderTarget::Archive(Box::new(ArchiveTarget {
            location: ArchiveLocation {
                mount: Some(Arc::new(Archive)),
                address: ArchiveAddress {
                    provider: "fixture".into(),
                    format: None,
                    container: self.root.join("archive"),
                    source_version: vec![],
                    components: vec![],
                },
                directory: 1,
                trail: vec![(1, "archive".into())],
            },
            member: Some(node.clone()),
            original: Some(node),
        })))
    }
}
fn fixture(
    mixed: bool,
) -> (
    tempfile::TempDir,
    Session,
    std::sync::Weak<ProviderRegistry>,
) {
    let temp = tempfile::Builder::new()
        .prefix("tree-move-authority-")
        .tempdir_in(std::env::temp_dir())
        .unwrap();
    let entries: Arc<[Entry]> = (1..=2)
        .map(|n| {
            Entry::new(
                EntryId(n),
                temp.path().join(format!("file-{n}")),
                EntryKind::File,
            )
        })
        .collect::<Vec<_>>()
        .into();
    let registry = Arc::new(ProviderRegistry::default());
    let weak = Arc::downgrade(&registry);
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let mut session = Session::new(
        root,
        OpenedLocation {
            mount: Arc::new(Mount {
                root: temp.path().to_owned(),
                mixed,
            }),
            root: token(0),
        },
        registry,
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let p = session.publication().unwrap();
    session.commit(&p.view);
    session
        .select(&TreeSelectionRequest {
            view: p.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Bulk(SelectionOperation::All),
        })
        .unwrap();
    (temp, session, weak)
}
struct NoPreparation;
impl TransferService for NoPreparation {
    fn prepare_transfer_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("mixed Move must preflight before Copy source preparation")
    }
    fn prepare_move_source(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        panic!("mixed Move must preflight before native Move preparation")
    }
    fn prepare_transfer_destination(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("mixed Move must never prepare a destination")
    }
}
#[test]
fn mixed_move_rejects_at_capture_and_rechecks_all_targets_before_any_source_preparation() {
    let (_temp, mut session, _) = fixture(true);
    let p = session.publication().unwrap();
    session.commit(&p.view);
    let capacity = Arc::new(Semaphore::new(4));
    let request = TreeTransferRequest {
        intent: TransferIntent::Move,
        view: p.view.clone(),
        request: TreeRequestId(1),
        selection_revision: p.selection_revision,
        selection: p.selection.clone(),
        fallback: None,
        cancel: Cancellation::default(),
    };
    assert!(matches!(
        session.capture_transfer(&request, &capacity, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
    assert_eq!(capacity.available_permits(), 4);
    let capture = session
        .capture_selection(
            &p.view,
            p.selection_revision,
            &p.selection,
            None,
            &Cancellation::default(),
        )
        .unwrap();
    let sources = Sources {
        lease: Arc::new(Lease {
            capture,
            _permit: capacity.clone().try_acquire_owned().unwrap(),
        }),
        metadata: false,
        intent: TransferIntent::Move,
    };
    let error = match sources.prepare(&NoPreparation, &Cancellation::default()) {
        Err(error) => error,
        Ok(_) => panic!("mixed Move admitted"),
    };
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    drop(sources);
    assert_eq!(capacity.available_permits(), 4);
}

struct Capture {
    entry: TransferEntry,
    calls: Arc<AtomicUsize>,
}
impl TransferMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn preflight_finalization(&self, _: &Cancellation) -> Result<(), FsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn capture(&self, _: &Cancellation) -> Result<TransferMoveCaptureOutcome, FsError> {
        panic!("no mutation in lifetime fixture")
    }
    fn prepare_finalization(
        &self,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
        Ok(Box::new(Finalizer(self.calls.clone())))
    }
    fn take_stage_cleanup(&self) -> Option<Box<dyn TransferMoveStageCleanup>> {
        Some(Box::new(Cleanup(self.calls.clone())))
    }
}
struct Finalizer(Arc<AtomicUsize>);
impl TransferMoveFinalizer for Finalizer {
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn finalize(&mut self, _: &Cancellation) -> TransferMoveFinalizeOutcome {
        panic!("no mutation in lifetime fixture")
    }
}
struct Cleanup(Arc<AtomicUsize>);
impl TransferMoveStageCleanup for Cleanup {
    fn path(&self) -> &Path {
        Path::new("fixture")
    }
    fn cleanup(self: Box<Self>) -> Result<(), FsError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn finalizer_and_cleanup_ticket_independently_keep_provider_and_capture_permit() {
    for keep_finalizer in [true, false] {
        let (_temp, mut session, registration) = fixture(false);
        let p = session.publication().unwrap();
        session.commit(&p.view);
        let capacity = Arc::new(Semaphore::new(4));
        let capture = session
            .capture_selection(
                &p.view,
                p.selection_revision,
                &p.selection,
                None,
                &Cancellation::default(),
            )
            .unwrap();
        let lease = Arc::new(Lease {
            capture,
            _permit: capacity.clone().try_acquire_owned().unwrap(),
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let authority = leased_capture(
            Arc::new(Capture {
                entry: TransferEntry {
                    item: MutationItem {
                        path: PathBuf::from("fixture"),
                        kind: EntryKind::File,
                        metadata: EntryMetadata::default(),
                    },
                    identity: TransferIdentity {
                        device: 1,
                        object: [1; 16],
                    },
                },
                calls: calls.clone(),
            }),
            &lease,
        );
        let finalizer = authority
            .prepare_finalization(&Cancellation::default())
            .unwrap();
        let cleanup = authority.take_stage_cleanup().unwrap();
        drop(authority);
        drop(lease);
        drop(session);
        if keep_finalizer {
            drop(cleanup);
            assert!(registration.upgrade().is_some());
            assert_eq!(capacity.available_permits(), 3);
            finalizer.revalidate(&Cancellation::default()).unwrap();
            drop(finalizer);
        } else {
            drop(finalizer);
            assert!(registration.upgrade().is_some());
            assert_eq!(capacity.available_permits(), 3);
            cleanup.cleanup().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(registration.upgrade().is_none());
        assert_eq!(capacity.available_permits(), 4);
    }
}
