//! Granted peers refusing the removal of the empty private stage of a capture that
//! failed before moving its source: the stage is neither leaked nor reported as moved.
use super::*;
use midnight_core::{TransferNamespacePin, TransferNamespaceProbe, TransferSource};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// The engine's namespace registry while granted peers run beside this job: it accepts
/// every reservation and keeps its pin, as the registry does, except one armed refusal.
/// As with `copy_namespace::take_peer_refusal`, every reservation first clears the
/// record of the previous one, so `refused` tells whether the refusal was the last.
#[derive(Default)]
struct Registry {
    pins: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    refuse_next: AtomicBool,
    refused: AtomicBool,
}
struct Changes(Arc<Registry>);
struct Turn(Arc<Registry>);
impl TransferNamespaceChanges for Changes {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        Ok(Box::new(Turn(self.0.clone())))
    }
}
impl TransferNamespaceChange for Turn {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        let refuse = self.0.refuse_next.swap(false, Ordering::SeqCst);
        self.0.refused.store(refuse, Ordering::SeqCst);
        if refuse {
            return Ok(false);
        }
        if let Some(pin) = pin {
            pin.refresh(cancel)?;
            self.0.pins.lock().unwrap().push(pin);
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        Ok(())
    }
}

/// Runs `body` with this job's registry installed and the Move source's original pin
/// registered (the announcements' sentinel). Announcements reserve pins, and so can be
/// refused, only where native lookups are qualified, as the namespace tests require.
fn run(body: impl FnOnce(&Registry, &Path, &Path, Arc<Capture>)) {
    let temp = crate::test_support::temporary("move-refusal-");
    let parent = temp.path().to_owned();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let registry = Arc::new(Registry::default());
    let changes: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(registry.clone()));
    with_transfer_namespace_changes(changes, || {
        let cancel = Cancellation::default();
        let (reader, capture) = prepare_native(&source, &cancel).unwrap();
        let root = reader.native_namespace_scope(&cancel).unwrap().unwrap();
        assert!(
            matches!(
                root.refresh(&cancel).unwrap(),
                TransferNamespaceProbe::Known(_)
            ),
            "fixture requires qualified local byte lookup"
        );
        registry.pins.lock().unwrap().push(root);
        drop(reader);
        body(&registry, &parent, &source, capture);
    });
}

fn move_stages(parent: &Path) -> Vec<PathBuf> {
    let mut stages: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".midnight-move-")
        })
        .collect();
    stages.sort();
    stages
}

/// Fails a capture after it has staged, with a cancellation standing in for any
/// failure that is not a refusal (a changed source, say), while granted peers refuse
/// the removal of its empty stage: the first reservation after staging. Returns the
/// stage that the capture kept.
fn refused_cleanup(
    registry: &Registry,
    parent: &Path,
    source: &Path,
    capture: &Capture,
) -> PathBuf {
    let cancel = Cancellation::default();
    let failure = capture
        .capture_with(
            &cancel,
            || {
                cancel.cancel();
                registry.refuse_next.store(true, Ordering::SeqCst);
            },
            || {},
        )
        .unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
    assert!(
        registry.refused.load(Ordering::SeqCst),
        "no later reservation replaced the refusal that the engine retries on"
    );
    assert!(
        !failure.message.contains("temporary directory"),
        "the kept stage is no cleanup failure: {failure:?}"
    );
    assert!(!capture.restoration_completed());
    assert_eq!(
        fs::read(source).unwrap(),
        b"original",
        "the refusal moved nothing"
    );
    let stages = move_stages(parent);
    assert_eq!(stages.len(), 1, "{stages:?}");
    assert_eq!(
        fs::read_dir(&stages[0]).unwrap().count(),
        0,
        "the kept stage is empty"
    );
    stages[0].clone()
}

#[test]
fn a_stage_whose_removal_was_refused_is_reused_by_the_repeated_capture() {
    run(|registry, parent, source, capture| {
        let stage = refused_cleanup(registry, parent, source, &capture);
        // The engine repeats the capture once a refusing peer changes.
        let TransferMoveCaptureOutcome::Captured { recovery, .. } =
            capture.capture(&Cancellation::default()).unwrap()
        else {
            panic!("the repeated capture moves the source");
        };
        assert_eq!(recovery.path, stage.join("source"));
        assert_eq!(fs::read(&recovery.path).unwrap(), b"original");
        assert_eq!(move_stages(parent), vec![stage.clone()], "no second stage");
        capture.restore().unwrap();
        let cleanup = capture.take_stage_cleanup().unwrap();
        assert_eq!(cleanup.path(), stage);
        drop(capture);
        cleanup.cleanup().unwrap();
        assert!(move_stages(parent).is_empty());
        assert_eq!(fs::read(source).unwrap(), b"original");
    });
}

#[test]
fn a_stage_whose_removal_was_refused_is_handed_to_the_job_cleanup() {
    run(|registry, parent, source, capture| {
        let stage = refused_cleanup(registry, parent, source, &capture);
        // The job ends before another capture: its teardown restores (nothing
        // moved), then runs the cleanup ticket of every capture.
        capture.restore().unwrap();
        assert!(!capture.restoration_completed());
        let cleanup = capture
            .take_stage_cleanup()
            .expect("the kept stage is handed over");
        assert_eq!(cleanup.path(), stage);
        assert!(capture.take_stage_cleanup().is_none(), "handed over once");
        drop(capture);
        cleanup.cleanup().unwrap();
        assert!(move_stages(parent).is_empty());
        assert_eq!(fs::read(source).unwrap(), b"original");
    });
}

#[test]
fn a_repeated_capture_that_fails_again_removes_the_kept_stage() {
    run(|registry, parent, source, capture| {
        refused_cleanup(registry, parent, source, &capture);
        let cancel = Cancellation::default();
        let failure = capture
            .capture_with(&cancel, || cancel.cancel(), || {})
            .unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
        assert!(
            move_stages(parent).is_empty(),
            "removed in the granted turn"
        );
        assert!(capture.take_stage_cleanup().is_none());
        assert_eq!(fs::read(source).unwrap(), b"original");
    });
}
