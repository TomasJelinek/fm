//! Granted peers refusing a capture's rename into the private stage that the same
//! namespace turn has just created: the stage is neither leaked nor reported as moved.
use super::*;
use midnight_core::{FsError, TransferNamespacePin, TransferNamespaceProbe, TransferSource};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
};
use std::{
    ffi::OsString,
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

/// Runs `body` in a protected parent with this job's registry installed and the Move
/// source's original pin registered (the announcements' sentinel). Announcements reserve
/// pins, and so can be refused, only where native lookups are qualified; elsewhere the
/// test reruns itself in a child process forced into that mode, as the namespace tests
/// do for their other mode.
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
        registry.pins.lock().unwrap().push(root);
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
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg(format!("{module}::{test}"))
        .args(["--exact", "--test-threads", "1"])
        .env(LOOKUP, "short-names-disabled")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed"),
        "{test} failed in the qualified lookup mode:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
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

/// Captures while granted peers refuse the first reservation after staging: the
/// rename's announcement of the stage that the same turn has just created. Returns the
/// stage that the capture kept.
fn refused_capture(
    registry: &Registry,
    parent: &Path,
    source: &Path,
    capture: &Capture,
) -> PathBuf {
    let refused = capture
        .capture_with(
            &Cancellation::default(),
            || registry.refuse_next.store(true, Ordering::SeqCst),
            || {},
        )
        .unwrap_err();
    assert_eq!(refused.kind, std::io::ErrorKind::WouldBlock, "{refused:?}");
    assert!(
        registry.refused.load(Ordering::SeqCst),
        "no later reservation replaced the refusal that the engine retries on"
    );
    assert!(!capture.restoration_completed());
    assert_eq!(
        fs::read(source).unwrap(),
        b"original",
        "the refusal moved nothing"
    );
    let stages = move_stages(parent);
    assert_eq!(stages.len(), 1, "{stages:?}");
    let stage = parent.join(&stages[0]);
    assert_eq!(
        fs::read_dir(&stage).unwrap().count(),
        0,
        "the kept stage is empty"
    );
    stage
}

#[test]
fn a_capture_refused_after_staging_reuses_its_stage_when_repeated() {
    run(
        "a_capture_refused_after_staging_reuses_its_stage_when_repeated",
        |registry, parent, source, capture| {
            let stage = refused_capture(registry, parent, source, &capture);
            // The engine repeats the capture once a refusing peer changes.
            let TransferMoveCaptureOutcome::Captured { recovery, .. } =
                capture.capture(&Cancellation::default()).unwrap()
            else {
                panic!("the repeated capture moves the source");
            };
            assert_eq!(recovery.path, stage.join("source"));
            assert_eq!(fs::read(&recovery.path).unwrap(), b"original");
            assert_eq!(
                move_stages(parent),
                vec![stage.file_name().unwrap().to_owned()],
                "no second stage"
            );
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
fn a_stage_kept_by_a_refused_capture_is_handed_to_the_job_cleanup() {
    run(
        "a_stage_kept_by_a_refused_capture_is_handed_to_the_job_cleanup",
        |registry, parent, source, capture| {
            let stage = refused_capture(registry, parent, source, &capture);
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
        },
    );
}
