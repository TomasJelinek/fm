//! Granted peers refusing namespace changes after a Move capture, a native replacement
//! Move or a replacement publication has created its private directory: the
//! reservation made at that creation covers the later renames into and out of the
//! directory and its removal in the same turn, so peers can refuse such a turn only
//! before its first namespace change. Also the empty stage of a failed capture that
//! cannot be removed (another program created an entry in it): it is kept, then
//! reused (reserved again by the repeated capture's rename, or else by its removal
//! after another failure, which peers can refuse) or handed to the job's cleanup.
use super::*;
use midnight_core::{
    EntryKind, FsError, TransferNamespacePin, TransferNamespaceProbe, TransferNativeMoveOutcome,
    TransferPublication, TransferReplacementPolicy, TransferSource,
};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
};
use std::{
    ffi::{OsStr, OsString},
    io::{ErrorKind, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

type Capture = super::super::Capture;

/// The engine's namespace registry while granted peers run beside this job: it accepts
/// every reservation and keeps its pin, as the registry does, except one armed refusal.
/// As with `copy_namespace::take_peer_refusal`, every reservation first clears the
/// record of the previous one, so `refused` tells whether the refusal was the last.
#[derive(Default)]
struct Registry {
    pins: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    refuse_next: AtomicBool,
    refused: AtomicBool,
    /// Arms `refuse_next` when the registry accepts a pin it does not retain yet: the
    /// reservation a change makes when it creates its private directory. A refusal
    /// then shows that the change reserved again after that creation.
    refuse_after_fresh: AtomicBool,
}
impl Registry {
    fn retain(&self, pin: Arc<dyn TransferNamespacePin>) {
        self.pins.lock().unwrap().push(pin);
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
        let refuse = self.0.refuse_next.swap(false, Ordering::SeqCst);
        self.0.refused.store(refuse, Ordering::SeqCst);
        if refuse {
            return Ok(false);
        }
        if let Some(pin) = pin {
            pin.refresh(cancel)?;
            let mut pins = self.0.pins.lock().unwrap();
            let fresh = !pins
                .iter()
                .any(|retained| std::ptr::addr_eq(Arc::as_ptr(retained), Arc::as_ptr(&pin)));
            if fresh && self.0.refuse_after_fresh.swap(false, Ordering::SeqCst) {
                self.0.refuse_next.store(true, Ordering::SeqCst);
            }
            pins.push(pin);
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        Ok(())
    }
}

/// Runs `body` in a protected parent with this job's registry installed and the Move
/// source's original pin registered (the announcements' sentinel); `body` registers
/// any other original pin it needs. Announcements reserve pins, and so can be refused,
/// only where native lookups are qualified; elsewhere the test reruns itself in a child
/// process forced into that mode, as the namespace tests do for their other mode.
fn run(test: &str, body: impl FnOnce(&Registry, &Path, &Path, Arc<Capture>)) {
    let (_temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let registry = Arc::new(Registry::default());
    let changes: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(registry.clone()));
    let qualified = with_transfer_namespace_changes(changes, || {
        let cancel = Cancellation::default();
        let (reader, capture) = prepare_native(&source, &cancel).unwrap();
        let root = reader.native_namespace_scope(&cancel).unwrap().unwrap();
        if matches!(
            root.refresh(&cancel).unwrap(),
            TransferNamespaceProbe::Unknown
        ) {
            return false;
        }
        registry.retain(root);
        drop(reader);
        body(&registry, &parent, &source, capture);
        true
    });
    if !qualified {
        rerun_where_lookups_are_qualified(test);
    }
}

fn rerun_where_lookups_are_qualified(test: &str) {
    // `namespace::parent::TEST_LOOKUP_ENV`.
    const LOOKUP: &str = "MIDNIGHT_TEST_NAMESPACE_LOOKUP";
    assert_ne!(
        std::env::var(LOOKUP).as_deref(),
        Ok("short-names-disabled"),
        "the forced qualified lookup mode needs a local NTFS build volume"
    );
    let module = module_path!().split_once("::").unwrap().1;
    let stdout = crate::test_child::run_test_child(
        &[
            &format!("{module}::{test}"),
            "--exact",
            "--test-threads",
            "1",
        ],
        (LOOKUP, "short-names-disabled"),
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "{test} did not run in the qualified lookup mode:\n{stdout}"
    );
}

fn move_stages(parent: &Path) -> Vec<OsString> {
    let mut stages: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".midnight-move-"))
        .collect();
    stages.sort();
    stages
}

/// A native replacement Move into `parent/target/final`. Granted peers refuse the first
/// reservation that follows the creation of its recovery directory; there must be none.
/// With `cancel_after_capture`, the publication fails after the original was displaced
/// into that directory, so the original is restored and the empty directory removed.
fn native_replacement(test: &str, cancel_after_capture: bool) {
    run(test, |registry, parent, source, capture| {
        let target = parent.join("target");
        fs::create_dir(&target).unwrap();
        let final_path = target.join("final");
        fs::write(&final_path, b"old target").unwrap();
        let cancel = Cancellation::default();
        let destination = crate::WindowsFileSystem::new(None)
            .prepare_transfer_destination(&target, &cancel)
            .unwrap();
        registry.retain(
            destination
                .native_child_namespace_scope(OsStr::new("final"), EntryKind::File, &cancel)
                .unwrap()
                .unwrap(),
        );
        let expected = destination
            .prepare_target(OsStr::new("final"), &cancel)
            .unwrap()
            .unwrap();
        registry.refuse_after_fresh.store(true, Ordering::SeqCst);
        let outcome = super::super::native_move::attempt_with(
            &capture,
            destination.as_ref(),
            OsStr::new("final"),
            Some((expected, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
            || {
                if cancel_after_capture {
                    cancel.cancel();
                }
            },
        )
        .unwrap();
        assert!(
            registry.refuse_next.load(Ordering::SeqCst),
            "nothing is reserved after the recovery directory was created: {outcome:?}"
        );
        if cancel_after_capture {
            let TransferNativeMoveOutcome::RecoveryRequired {
                destination: None,
                recovery,
                ..
            } = outcome
            else {
                panic!("the cancelled publication restores the original: {outcome:?}");
            };
            assert!(
                recovery.is_empty(),
                "the empty recovery directory was removed: {recovery:?}"
            );
            drop(capture);
            assert_eq!(fs::read(&final_path).unwrap(), b"old target");
            assert_eq!(fs::read(source).unwrap(), b"original");
            assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        } else {
            let TransferNativeMoveOutcome::Moved { recovery, .. } = outcome else {
                panic!("the native replacement commits: {outcome:?}");
            };
            assert_eq!(recovery.len(), 1);
            drop(capture);
            assert_eq!(fs::read(&final_path).unwrap(), b"original");
            assert_eq!(fs::read(&recovery[0].path).unwrap(), b"old target");
            assert!(!source.exists());
        }
    });
}

#[test]
fn a_native_replacement_is_not_refused_after_creating_its_recovery_directory() {
    native_replacement(
        "a_native_replacement_is_not_refused_after_creating_its_recovery_directory",
        false,
    );
}

#[test]
fn a_failed_native_replacement_restores_and_removes_its_recovery_directory_unrefused() {
    native_replacement(
        "a_failed_native_replacement_restores_and_removes_its_recovery_directory_unrefused",
        true,
    );
}

#[test]
fn a_replacement_publication_is_not_refused_after_creating_its_recovery_directory() {
    run(
        "a_replacement_publication_is_not_refused_after_creating_its_recovery_directory",
        |registry, parent, _source, _capture| {
            let target = parent.join("target");
            fs::write(&target, b"old target").unwrap();
            let cancel = Cancellation::default();
            let destination = crate::WindowsFileSystem::new(None)
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
            registry.refuse_after_fresh.store(true, Ordering::SeqCst);
            let outcome = stage
                .publish_replace(OsStr::new("target"), &cancel)
                .unwrap();
            assert!(
                registry.refuse_next.load(Ordering::SeqCst),
                "nothing is reserved after the recovery directory was created: {outcome:?}"
            );
            let TransferPublication::Replaced { path, recovery } = outcome else {
                panic!("the publication replaces the target: {outcome:?}");
            };
            drop(stage);
            assert_eq!(path, target);
            assert_eq!(fs::read(&target).unwrap(), b"replacement");
            assert_eq!(fs::read(&recovery.path).unwrap(), b"old target");
        },
    );
}

#[test]
fn a_capture_is_not_refused_after_creating_its_stage() {
    run(
        "a_capture_is_not_refused_after_creating_its_stage",
        |registry, parent, source, capture| {
            registry.refuse_after_fresh.store(true, Ordering::SeqCst);
            let outcome = capture.capture(&Cancellation::default()).unwrap();
            assert!(
                registry.refuse_next.load(Ordering::SeqCst),
                "nothing is reserved after the stage was created: {outcome:?}"
            );
            let stages = move_stages(parent);
            assert_eq!(stages.len(), 1, "{stages:?}");
            let stage = parent.join(&stages[0]);
            let TransferMoveCaptureOutcome::Captured { recovery, .. } = outcome else {
                panic!("the capture moves the source: {outcome:?}");
            };
            assert_eq!(recovery.path, stage.join("source"));
            // The armed refusal was meant for the capture's own turn only.
            registry.refuse_next.store(false, Ordering::SeqCst);
            capture.restore().unwrap();
            let cleanup = capture.take_stage_cleanup().unwrap();
            assert_eq!(cleanup.path(), stage);
            drop(capture);
            cleanup.cleanup().unwrap();
            assert!(move_stages(parent).is_empty());
            assert_eq!(fs::read(source).unwrap(), b"original");
        },
    );
}

#[test]
fn a_capture_failing_after_staging_removes_its_stage_without_another_reservation() {
    run(
        "a_capture_failing_after_staging_removes_its_stage_without_another_reservation",
        |registry, parent, source, capture| {
            registry.refuse_after_fresh.store(true, Ordering::SeqCst);
            let cancel = Cancellation::default();
            let failure = capture
                .capture_with(&cancel, || cancel.cancel(), || {})
                .unwrap_err();
            assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
            assert!(
                registry.refuse_next.load(Ordering::SeqCst),
                "the removal of the stage this turn created reserves nothing"
            );
            assert!(!registry.refused.load(Ordering::SeqCst));
            assert!(move_stages(parent).is_empty(), "the stage was removed");
            assert!(capture.take_stage_cleanup().is_none());
            assert_eq!(fs::read(source).unwrap(), b"original");
        },
    );
}

/// Fails a capture after it created its stage, with an entry that another program
/// created in the stage, so that the kernel refuses to remove it. Returns the stage
/// that the capture kept, which still holds that entry, named `foreign`.
fn kept_stage(parent: &Path, source: &Path, capture: &Capture) -> PathBuf {
    let cancel = Cancellation::default();
    let failure = capture
        .capture_with(
            &cancel,
            || {
                let stages = move_stages(parent);
                fs::write(parent.join(&stages[0]).join("foreign"), b"foreign").unwrap();
                cancel.cancel();
            },
            || {},
        )
        .unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
    assert!(!capture.restoration_completed());
    assert_eq!(fs::read(source).unwrap(), b"original", "nothing moved");
    let stages = move_stages(parent);
    assert_eq!(stages.len(), 1, "{stages:?}");
    parent.join(&stages[0])
}

/// A Move source in a protected parent, without a namespace registry.
fn unregistered() -> (tempfile::TempDir, PathBuf, PathBuf, Arc<Capture>) {
    let (temp, parent) = fixture();
    let source = parent.join("source");
    fs::write(&source, b"original").unwrap();
    let (reader, capture) = prepare_native(&source, &Cancellation::default()).unwrap();
    drop(reader);
    (temp, parent, source, capture)
}

#[test]
fn a_capture_whose_stage_cannot_be_removed_reuses_it_when_repeated() {
    let (_temp, parent, source, capture) = unregistered();
    let stage = kept_stage(&parent, &source, &capture);
    let TransferMoveCaptureOutcome::Captured { recovery, .. } =
        capture.capture(&Cancellation::default()).unwrap()
    else {
        panic!("the repeated capture moves the source");
    };
    assert_eq!(recovery.path, stage.join("source"));
    assert_eq!(
        move_stages(&parent),
        vec![stage.file_name().unwrap().to_owned()],
        "no second stage"
    );
    capture.restore().unwrap();
    fs::remove_file(stage.join("foreign")).unwrap();
    let cleanup = capture.take_stage_cleanup().unwrap();
    assert_eq!(cleanup.path(), stage);
    drop(capture);
    cleanup.cleanup().unwrap();
    assert!(move_stages(&parent).is_empty());
    assert_eq!(fs::read(&source).unwrap(), b"original");
}

#[test]
fn a_stage_kept_after_a_failed_capture_is_handed_to_the_job_cleanup() {
    let (_temp, parent, source, capture) = unregistered();
    let stage = kept_stage(&parent, &source, &capture);
    // The job ends before another capture: its teardown restores (nothing moved),
    // then runs the cleanup ticket of every capture.
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
    assert!(move_stages(&parent).is_empty());
    assert_eq!(fs::read(&source).unwrap(), b"original");
}

#[test]
fn a_repeated_capture_that_fails_again_removes_the_stage_it_reused() {
    run(
        "a_repeated_capture_that_fails_again_removes_the_stage_it_reused",
        |registry, parent, source, capture| {
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
                "removed in the repeated capture's turn"
            );
            assert!(capture.take_stage_cleanup().is_none());
            assert!(!capture.restoration_completed());
            assert_eq!(fs::read(source).unwrap(), b"original");
        },
    );
}

#[test]
fn a_repeated_capture_whose_cleanup_is_refused_keeps_the_stage_it_reused() {
    run(
        "a_repeated_capture_whose_cleanup_is_refused_keeps_the_stage_it_reused",
        |registry, parent, source, capture| {
            let stage = kept_stage(parent, source, &capture);
            fs::remove_file(stage.join("foreign")).unwrap();
            let kept = vec![stage.file_name().unwrap().to_owned()];
            let cancel = Cancellation::default();
            let failure = capture
                .capture_with(
                    &cancel,
                    || {
                        // The repeated capture fails before its rename into the reused
                        // stage, which this turn has not reserved yet, so the stage's
                        // removal reserves it again. Peers refuse that.
                        registry.refuse_next.store(true, Ordering::SeqCst);
                        cancel.cancel();
                    },
                    || {},
                )
                .unwrap_err();
            assert_eq!(failure.kind, ErrorKind::Interrupted, "{failure:?}");
            assert!(
                !registry.refuse_next.load(Ordering::SeqCst),
                "the removal of the reused stage reserved it again"
            );
            assert!(
                registry.refused.load(Ordering::SeqCst),
                "no later reservation replaced the refusal that the engine retries on"
            );
            assert_eq!(move_stages(parent), kept, "the stage is kept, not leaked");
            assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
            assert_eq!(fs::read(source).unwrap(), b"original");
            let TransferMoveCaptureOutcome::Captured { recovery, .. } =
                capture.capture(&Cancellation::default()).unwrap()
            else {
                panic!("the repeated capture moves the source");
            };
            assert_eq!(recovery.path, stage.join("source"));
            assert_eq!(move_stages(parent), kept, "no second stage");
            capture.restore().unwrap();
            let cleanup = capture.take_stage_cleanup().unwrap();
            assert_eq!(cleanup.path(), stage);
            drop(capture);
            cleanup.cleanup().unwrap();
            assert!(move_stages(parent).is_empty());
            assert_eq!(fs::read(source).unwrap(), b"original");
        },
    );
}

#[test]
fn a_repeated_capture_refused_at_the_stage_it_reuses_keeps_that_stage() {
    run(
        "a_repeated_capture_refused_at_the_stage_it_reuses_keeps_that_stage",
        |registry, parent, source, capture| {
            let stage = kept_stage(parent, source, &capture);
            fs::remove_file(stage.join("foreign")).unwrap();
            let kept = vec![stage.file_name().unwrap().to_owned()];
            // The stage was reserved in an earlier turn, so the repeated capture's rename
            // announces it again; granted peers refuse that before anything moved.
            let refused = capture
                .capture_with(
                    &Cancellation::default(),
                    || registry.refuse_next.store(true, Ordering::SeqCst),
                    || {},
                )
                .unwrap_err();
            assert_eq!(refused.kind, ErrorKind::WouldBlock, "{refused:?}");
            assert!(
                registry.refused.load(Ordering::SeqCst),
                "no later reservation replaced the refusal that the engine retries on"
            );
            assert_eq!(move_stages(parent), kept, "the stage is kept, not leaked");
            assert_eq!(fs::read_dir(&stage).unwrap().count(), 0);
            assert_eq!(fs::read(source).unwrap(), b"original");
            // The engine repeats the capture once a refusing peer changes.
            let TransferMoveCaptureOutcome::Captured { recovery, .. } =
                capture.capture(&Cancellation::default()).unwrap()
            else {
                panic!("the repeated capture moves the source");
            };
            assert_eq!(recovery.path, stage.join("source"));
            assert_eq!(move_stages(parent), kept, "no second stage");
            capture.restore().unwrap();
            let cleanup = capture.take_stage_cleanup().unwrap();
            assert_eq!(cleanup.path(), stage);
            drop(capture);
            cleanup.cleanup().unwrap();
            assert!(move_stages(parent).is_empty());
            assert_eq!(fs::read(source).unwrap(), b"original");
        },
    );
}
