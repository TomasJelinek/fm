//! Granted peers refusing namespace changes after a Move capture, a native Move or a
//! replacement publication has created its private directory: the reservation made at
//! that creation covers the later changes beneath the directory and its removal in the
//! same change, so a failure after the first namespace change removes the empty
//! directory without another reservation. Also the empty stage of a failed capture that
//! cannot be removed: it is kept, then reused (announced again, as is its removal after
//! another failure, which peers can refuse) or handed to the job's cleanup.
use super::*;
use midnight_core::{
    TransferNamespacePin, TransferNamespaceProbe, TransferNativeMoveOutcome, TransferPublication,
    TransferReplacementPolicy, TransferSource,
};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
};
use std::{
    fs,
    io::{ErrorKind, Write},
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
    /// Refuses the next reservation of a pin bound at this name, wherever it falls.
    refuse_name: Mutex<Option<OsString>>,
    refused: AtomicBool,
}
impl Registry {
    fn retain(&self, pin: Arc<dyn TransferNamespacePin>) {
        self.pins.lock().unwrap().push(pin);
    }
    fn refuse_name(&self, name: &OsStr) {
        *self.refuse_name.lock().unwrap() = Some(name.to_owned());
    }
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
        let named = {
            let mut name = self.0.refuse_name.lock().unwrap();
            let bound = match (&pin, name.as_deref()) {
                (Some(pin), Some(refused)) => binds(pin.as_ref(), refused, cancel)?,
                _ => false,
            };
            if bound {
                name.take();
            }
            bound
        };
        let refuse = named || self.0.refuse_next.swap(false, Ordering::SeqCst);
        self.0.refused.store(refuse, Ordering::SeqCst);
        if refuse {
            return Ok(false);
        }
        if let Some(pin) = pin {
            pin.refresh(cancel)?;
            self.0.retain(pin);
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        Ok(())
    }
}

/// Whether `pin` witnesses a binding named `name`.
fn binds(
    pin: &dyn TransferNamespacePin,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<bool, FsError> {
    Ok(match pin.refresh(cancel)? {
        TransferNamespaceProbe::Known(scopes) => scopes
            .iter()
            .any(|scope| *scope.lookup_key == *name.as_bytes()),
        TransferNamespaceProbe::Unknown => false,
    })
}

/// Runs `body` with this job's registry installed and the Move source's original pin
/// registered (the announcements' sentinel); `body` registers any other original pin
/// it needs. Announcements reserve pins, and so can be refused, only where native
/// lookups are qualified, as the namespace tests require.
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
        registry.retain(root);
        drop(reader);
        body(&registry, &parent, &source, capture);
    });
}

fn private_directories(parent: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut directories: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(prefix)
        })
        .collect();
    directories.sort();
    directories
}

fn move_stages(parent: &Path) -> Vec<PathBuf> {
    private_directories(parent, ".midnight-move-")
}

#[test]
fn a_capture_failing_after_staging_removes_its_stage_without_another_reservation() {
    run(|registry, parent, source, capture| {
        let cancel = Cancellation::default();
        let failure = capture
            .capture_with(
                &cancel,
                || {
                    assert_eq!(move_stages(parent).len(), 1, "the stage was created");
                    // A cancellation stands in for any failure that is not a refusal
                    // (a changed source, say); granted peers would refuse whatever this
                    // change reserves next.
                    cancel.cancel();
                    registry.refuse_next.store(true, Ordering::SeqCst);
                },
                || {},
            )
            .unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
        assert!(
            registry.refuse_next.load(Ordering::SeqCst),
            "the removal of the stage this change created reserves nothing"
        );
        assert!(!registry.refused.load(Ordering::SeqCst));
        assert!(move_stages(parent).is_empty(), "the stage was removed");
        assert!(capture.take_stage_cleanup().is_none());
        assert!(!capture.restoration_completed());
        assert_eq!(fs::read(source).unwrap(), b"original");
    });
}

/// Fails a capture after it created its stage, with an entry that another program
/// created in the stage, so that rmdir refuses to remove it. Returns the stage that the
/// capture kept, which still holds that entry, named `foreign`.
fn kept_stage(parent: &Path, source: &Path, capture: &Capture) -> PathBuf {
    let cancel = Cancellation::default();
    let failure = capture
        .capture_with(
            &cancel,
            || {
                fs::write(move_stages(parent)[0].join("foreign"), b"foreign").unwrap();
                cancel.cancel();
            },
            || {},
        )
        .unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
    assert!(
        !failure.message.contains("temporary directory"),
        "the kept stage is no cleanup failure: {failure:?}"
    );
    assert!(!capture.restoration_completed());
    assert_eq!(fs::read(source).unwrap(), b"original", "nothing moved");
    let stages = move_stages(parent);
    assert_eq!(stages.len(), 1, "{stages:?}");
    stages[0].clone()
}

#[test]
fn a_capture_whose_stage_cannot_be_removed_reuses_it_when_repeated() {
    run(|_, parent, source, capture| {
        let stage = kept_stage(parent, source, &capture);
        let TransferMoveCaptureOutcome::Captured { recovery, .. } =
            capture.capture(&Cancellation::default()).unwrap()
        else {
            panic!("the repeated capture moves the source");
        };
        assert_eq!(recovery.path, stage.join("source"));
        assert_eq!(move_stages(parent), vec![stage.clone()], "no second stage");
        capture.restore().unwrap();
        fs::remove_file(stage.join("foreign")).unwrap();
        let cleanup = capture.take_stage_cleanup().unwrap();
        assert_eq!(cleanup.path(), stage);
        drop(capture);
        cleanup.cleanup().unwrap();
        assert!(move_stages(parent).is_empty());
        assert_eq!(fs::read(source).unwrap(), b"original");
    });
}

#[test]
fn a_stage_kept_after_a_failed_capture_is_handed_to_the_job_cleanup() {
    run(|_, parent, source, capture| {
        let stage = kept_stage(parent, source, &capture);
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
        fs::remove_file(stage.join("foreign")).unwrap();
        cleanup.cleanup().unwrap();
        assert!(move_stages(parent).is_empty());
        assert_eq!(fs::read(source).unwrap(), b"original");
    });
}

#[test]
fn a_repeated_capture_refused_at_the_stage_it_reuses_keeps_that_stage() {
    run(|registry, parent, source, capture| {
        let stage = kept_stage(parent, source, &capture);
        fs::remove_file(stage.join("foreign")).unwrap();
        // The stage was reserved in an earlier change, so the repeated capture
        // announces it again before anything is renamed into it; peers refuse that.
        registry.refuse_name(stage.file_name().unwrap());
        let refused = capture.capture(&Cancellation::default()).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::WouldBlock, "{refused:?}");
        assert!(
            registry.refuse_name.lock().unwrap().is_none(),
            "the reused stage's announcement was refused"
        );
        assert!(
            registry.refused.load(Ordering::SeqCst),
            "no later reservation replaced the refusal that the engine retries on"
        );
        assert_eq!(
            move_stages(parent),
            vec![stage.clone()],
            "the stage is kept"
        );
        assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
        assert_eq!(fs::read(source).unwrap(), b"original");
        // The engine repeats the capture once a refusing peer changes.
        let TransferMoveCaptureOutcome::Captured { recovery, .. } =
            capture.capture(&Cancellation::default()).unwrap()
        else {
            panic!("the repeated capture moves the source");
        };
        assert_eq!(recovery.path, stage.join("source"));
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
fn a_repeated_capture_that_fails_again_removes_the_stage_it_reused() {
    run(|registry, parent, source, capture| {
        let stage = kept_stage(parent, source, &capture);
        fs::remove_file(stage.join("foreign")).unwrap();
        let cancel = Cancellation::default();
        let failure = capture
            .capture_with(&cancel, || cancel.cancel(), || {})
            .unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
        assert!(!registry.refused.load(Ordering::SeqCst));
        assert!(
            move_stages(parent).is_empty(),
            "removed in the repeated capture's change"
        );
        assert!(capture.take_stage_cleanup().is_none());
        assert!(!capture.restoration_completed());
        assert_eq!(fs::read(source).unwrap(), b"original");
    });
}

#[test]
fn a_repeated_capture_whose_cleanup_is_refused_keeps_the_stage_it_reused() {
    run(|registry, parent, source, capture| {
        let stage = kept_stage(parent, source, &capture);
        fs::remove_file(stage.join("foreign")).unwrap();
        let cancel = Cancellation::default();
        let failure = capture
            .capture_with(
                &cancel,
                || {
                    // The repeated capture has announced the reused stage; it fails before
                    // moving anything, and the stage was reserved in an earlier change, so
                    // its removal announces it again. Peers refuse that.
                    registry.refuse_name(stage.file_name().unwrap());
                    cancel.cancel();
                },
                || {},
            )
            .unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
        assert!(
            registry.refuse_name.lock().unwrap().is_none(),
            "the removal of the reused stage was refused"
        );
        assert!(
            registry.refused.load(Ordering::SeqCst),
            "no later reservation replaced the refusal that the engine retries on"
        );
        assert_eq!(
            move_stages(parent),
            vec![stage.clone()],
            "the stage is kept"
        );
        assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
        assert_eq!(fs::read(source).unwrap(), b"original");
        let TransferMoveCaptureOutcome::Captured { recovery, .. } =
            capture.capture(&Cancellation::default()).unwrap()
        else {
            panic!("the repeated capture moves the source");
        };
        assert_eq!(recovery.path, stage.join("source"));
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

/// A native Move of the source into `parent/target/moved`; granted peers refuse the
/// next reservation after the source was captured into the stage, and with
/// `cancel_before_publish` the Move is also cancelled there, so the source is restored.
fn native_move(cancel_before_publish: bool) {
    run(|registry, parent, source, capture| {
        let target = parent.join("target");
        fs::create_dir(&target).unwrap();
        let cancel = Cancellation::default();
        let destination = crate::LinuxFileSystem::new(None)
            .prepare_transfer_destination(&target, &cancel)
            .unwrap();
        let outcome = native::attempt_with(
            &capture,
            destination.as_ref(),
            OsStr::new("moved"),
            None,
            &cancel,
            || {},
            || {
                assert_eq!(move_stages(parent).len(), 1, "the source was staged");
                registry.refuse_next.store(true, Ordering::SeqCst);
                if cancel_before_publish {
                    cancel.cancel();
                }
            },
        )
        .unwrap();
        assert!(
            registry.refuse_next.load(Ordering::SeqCst),
            "nothing is reserved after the source was captured: {outcome:?}"
        );
        assert!(move_stages(parent).is_empty(), "{outcome:?}");
        if cancel_before_publish {
            let TransferNativeMoveOutcome::RecoveryRequired {
                destination: None,
                recovery,
                error,
            } = outcome
            else {
                panic!("the cancelled Move restores the source: {outcome:?}");
            };
            assert!(recovery.is_empty(), "{recovery:?}");
            assert!(
                !error.message.contains("empty staging cleanup"),
                "{error:?}"
            );
            assert_eq!(fs::read(source).unwrap(), b"original");
            assert!(!target.join("moved").exists());
        } else {
            assert!(
                matches!(
                    outcome,
                    TransferNativeMoveOutcome::Moved {
                        durability_confirmed: true,
                        ..
                    }
                ),
                "{outcome:?}"
            );
            assert_eq!(fs::read(target.join("moved")).unwrap(), b"original");
            assert!(!source.exists());
        }
    });
}

#[test]
fn a_native_move_removes_its_empty_stage_without_another_reservation() {
    native_move(false);
}

#[test]
fn a_restored_native_move_removes_its_empty_stage_without_another_reservation() {
    native_move(true);
}

#[test]
fn a_refused_recovery_witness_removes_the_new_recovery_directory_and_is_retried() {
    run(|registry, parent, _source, _capture| {
        if rustix::fs::statfs(parent).is_ok_and(|stat| stat.f_type != 0xef53) {
            eprintln!(
                "SKIP recovery witness refusal: the fixture is not on a local ext filesystem"
            );
            return;
        }
        let target = parent.join("target");
        fs::write(&target, b"old target").unwrap();
        let cancel = Cancellation::default();
        let destination = crate::LinuxFileSystem::new(None)
            .prepare_transfer_destination(parent, &cancel)
            .unwrap();
        registry.retain(
            destination
                .native_child_namespace_scope(OsStr::new("target"), EntryKind::File, &cancel)
                .unwrap()
                .unwrap(),
        );
        let expected = destination
            .prepare_target(OsStr::new("target"), &cancel)
            .unwrap()
            .unwrap();
        let mut stage = destination
            .stage_file_for_target(
                OsStr::new("target"),
                expected,
                TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel,
            )
            .unwrap();
        stage.write_all(b"replacement").unwrap();
        // The witness of the replacement inside the recovery directory is reserved
        // after that directory was created.
        registry.refuse_name(OsStr::new("item"));
        let refused = stage
            .publish_replace(OsStr::new("target"), &cancel)
            .unwrap_err();
        assert_eq!(refused.kind, ErrorKind::WouldBlock, "{refused:?}");
        assert!(registry.refuse_name.lock().unwrap().is_none());
        assert!(
            registry.refused.load(Ordering::SeqCst),
            "no later reservation replaced the refusal that the engine retries on"
        );
        assert!(
            private_directories(parent, ".midnight-recovery-").is_empty(),
            "the empty recovery directory was removed"
        );
        assert_eq!(fs::read(&target).unwrap(), b"old target");
        // The engine repeats the publication once the refusing peer changes.
        let TransferPublication::Replaced { path, recovery } = stage
            .publish_replace(OsStr::new("target"), &cancel)
            .unwrap()
        else {
            panic!("the repeated publication replaces the target");
        };
        drop(stage);
        assert_eq!(path, target);
        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        assert_eq!(fs::read(&recovery.path).unwrap(), b"old target");
    });
}
