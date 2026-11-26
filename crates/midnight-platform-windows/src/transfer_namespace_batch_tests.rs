//! Refresh batches (`TransferNamespaceRefreshBatch`): a batch changes no names, so a
//! folder's ancestry, root and lookup rules are checked once in it, and a change's binding
//! checks and announcement up to its native operation are one batch.
use super::*;
use midnight_core::TransferService;
use midnight_platform_common::transfer_namespace::with_transfer_namespace_changes;
use std::{fs, io::Write};

/// Native ancestor-binding opens that `run` made on this thread.
fn probes(run: impl FnOnce()) -> usize {
    let before = parent::EDGE_PROBES.with(std::cell::Cell::get);
    run();
    parent::EDGE_PROBES.with(std::cell::Cell::get) - before
}

fn folder(path: &Path) -> Arc<parent::Parent> {
    parent::capture(
        &native::open_parent(path).unwrap(),
        path,
        &Cancellation::default(),
    )
    .unwrap()
}

/// Reads of a volume's 8.3 state and root binding that `run` made on this thread.
fn volume_proofs(run: impl FnOnce()) -> usize {
    let before = parent::VOLUME_PROOFS.with(std::cell::Cell::get);
    run();
    parent::VOLUME_PROOFS.with(std::cell::Cell::get) - before
}

/// The volume's part of the lookup rules (its 8.3 state and root binding) is read once per
/// volume in a batch, whichever folders of it the batch checks; each folder's own rules
/// are still read for it. Outside a batch each check reads it again.
#[test]
fn a_batch_reads_the_volume_rules_once_for_all_its_folders() {
    let temp = crate::test_fixtures::build_temp("namespace-batch-");
    let cancel = Cancellation::default();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let first = folder(&first);
    let second = folder(&second);
    let qualified = matches!(first.mode, parent::LookupMode::ShortNamesDisabled { .. });
    let unbatched = volume_proofs(|| {
        first.validate(&cancel).unwrap();
        second.validate(&cancel).unwrap();
    });
    let batched = volume_proofs(|| {
        let _batch = TransferNamespaceRefreshBatch::begin();
        first.validate(&cancel).unwrap();
        second.validate(&cancel).unwrap();
        first.validate(&cancel).unwrap();
    });
    if qualified {
        assert_eq!(unbatched, 2);
        assert_eq!(batched, 1);
    } else {
        // An unqualified volume is never remembered: there is nothing to read again, or
        // its 8.3 state refuses each time.
        assert!(batched <= 2, "{batched}");
    }
}

#[test]
fn a_batch_checks_each_ancestor_binding_once_for_all_folders_that_share_it() {
    let temp = crate::test_fixtures::build_temp("namespace-batch-");
    let cancel = Cancellation::default();
    let outer = temp.path().join("outer 文档");
    let inner = outer.join("inner");
    fs::create_dir_all(&inner).unwrap();
    let below = folder(&inner);
    let above = folder(&outer);
    let edges = below.edges();
    assert_eq!(above.edges(), edges - 1);
    // Outside a batch every check opens every ancestor again.
    let unbatched = probes(|| {
        below.validate(&cancel).unwrap();
        below.validate(&cancel).unwrap();
        above.validate(&cancel).unwrap();
    });
    assert_eq!(unbatched, 3 * edges - 1);
    let batched = probes(|| {
        let _batch = TransferNamespaceRefreshBatch::begin();
        below.validate(&cancel).unwrap();
        below.validate(&cancel).unwrap();
        // The enclosing folder's ancestry is a part of the one already checked.
        above.validate(&cancel).unwrap();
    });
    assert_eq!(batched, edges);
    // An ended batch is forgotten, and a nested batch checks for itself.
    let nested = probes(|| {
        let _outer = TransferNamespaceRefreshBatch::begin();
        below.validate(&cancel).unwrap();
        {
            let _inner = TransferNamespaceRefreshBatch::begin();
            below.validate(&cancel).unwrap();
        }
    });
    assert_eq!(nested, 2 * edges);
}

#[test]
fn a_rename_during_a_batch_is_seen_by_the_next_batch_and_a_failure_is_never_kept() {
    let temp = crate::test_fixtures::build_temp("namespace-batch-");
    let cancel = Cancellation::default();
    let original = temp.path().join("folder");
    fs::create_dir(&original).unwrap();
    let held = folder(&original);
    {
        let _batch = TransferNamespaceRefreshBatch::begin();
        held.validate(&cancel).unwrap();
        held.check_path(&original).unwrap();
        fs::rename(&original, temp.path().join("renamed")).unwrap();
        // As a check that ran just before the rename.
        held.validate(&cancel).unwrap();
        held.check_path(&original).unwrap();
    }
    let _batch = TransferNamespaceRefreshBatch::begin();
    assert!(held.validate(&cancel).is_err());
    assert!(
        held.validate(&cancel).is_err(),
        "a failed check is checked again"
    );
    assert!(held.check_path(&original).is_err());
}

/// A host that refreshes each pin it reserves, as the engine does, and refreshes the pins
/// of a finished change in one batch (`copy_namespace::ProbeBatch`).
#[derive(Default)]
struct Host {
    pins: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    turns: AtomicUsize,
}
struct HostContext(Arc<Host>);
struct HostTurn(Arc<Host>);
impl TransferNamespaceChanges for HostContext {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        self.0
            .turns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::new(HostTurn(self.0.clone())))
    }
}
impl TransferNamespaceChange for HostTurn {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        // An unqualified binding (8.3 names, or the forced `unqualified` test mode)
        // reserves no pin; the host then refreshes every pin at the end of the turn.
        if let Some(pin) = pin {
            pin.refresh(cancel)?;
            let mut pins = self.0.pins.lock().unwrap();
            if !pins.iter().any(|old| Arc::ptr_eq(old, &pin)) {
                pins.push(pin);
            }
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        let _batch = TransferNamespaceRefreshBatch::begin();
        let pins = self.0.pins.lock().unwrap().clone();
        for pin in pins {
            pin.refresh(&Cancellation::default())?;
        }
        Ok(())
    }
}

#[test]
fn a_stage_and_its_publication_each_check_their_folder_once_before_the_native_change() {
    let temp = crate::test_fixtures::build_temp("namespace-batch-");
    let host = Arc::new(Host::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(HostContext(host.clone()));
    with_transfer_namespace_changes(context, || {
        let cancel = Cancellation::default();
        let directory = crate::WindowsFileSystem::new(None)
            .prepare_transfer_destination(temp.path(), &cancel)
            .unwrap();
        // Retained, as a job's pins retain it: each turn reuses the pooled folder.
        let held = folder(temp.path());
        let edges = held.edges();
        for index in 0..3 {
            let turns = host.turns.load(std::sync::atomic::Ordering::SeqCst);
            let mut stage = None;
            // One check before the native creation, one in the host's refresh batch.
            let created = probes(|| stage = Some(directory.stage_file(&cancel).unwrap()));
            assert_eq!(created, 2 * edges, "stage {index}");
            let mut stage = stage.unwrap();
            stage.write_all(b"published bytes").unwrap();
            stage.sync(&cancel).unwrap();
            let name = format!("output-{index}");
            let published = probes(|| {
                stage
                    .publish_no_replace(OsStr::new(&name), &cancel)
                    .unwrap();
            });
            assert_eq!(published, 2 * edges, "publication {index}");
            assert_eq!(
                host.turns.load(std::sync::atomic::Ordering::SeqCst),
                turns + 2
            );
            stage.cleanup().unwrap();
            drop(stage);
            assert_eq!(
                fs::read(temp.path().join(&name)).unwrap(),
                b"published bytes"
            );
        }
    });
}
