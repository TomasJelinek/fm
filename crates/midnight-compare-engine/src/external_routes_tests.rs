//! Real Service routing with provider-free policy and launcher test doubles.
use super::*;
use crate::policy_test_support::FakePolicySource;
use midnight_core::{
    ExternalCompareOutcome, ExternalCompareProcess, ExternalCompareSessionCredit,
    FileComparatorProgram, FileComparatorSnapshot, FileComparatorTarget, FileCompareSource,
    FsError, UserCommandProcessInvocation,
};
use std::{
    ffi::OsString,
    io,
    path::Path,
    sync::{Weak, mpsc::Receiver},
    thread::ThreadId,
    time::{Duration, Instant},
};

fn paths() -> [PathBuf; 2] {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        base.join("before 文档 & $x.cpp"),
        base.join("after e\u{301} & %PATH%.c"),
    ]
}
fn program() -> FileComparatorProgram {
    FileComparatorProgram {
        executable: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("owned-compare-probe"),
        arguments: vec![
            "{right}".into(),
            "--literal={left}".into(),
            "{left}".into(),
            String::new(),
            " & $x %PATH% ; ".into(),
            "{left}".into(),
        ],
        working_directory: None,
    }
}
fn launch(paths: &[PathBuf; 2], snapshot: &FileComparatorSnapshot) -> FileCompareLaunch {
    FileCompareLaunch {
        sources: paths
            .iter()
            .cloned()
            .map(FileCompareSource::Focused)
            .collect(),
        disabled_plugins: Arc::from(["disabled-image-decoder".to_owned()]),
        comparators: snapshot.preferences.clone(),
        comparator_policy: snapshot.id,
    }
}
fn unresolved(snapshot: &FileComparatorSnapshot) -> FileCompareLaunch {
    FileCompareLaunch {
        sources: Vec::new(),
        disabled_plugins: Arc::from([]),
        comparators: snapshot.preferences.clone(),
        comparator_policy: snapshot.id,
    }
}
fn open(service: &Service, launch: FileCompareLaunch) -> Result<Session, OpenError> {
    service.open(
        launch,
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [30, 30, 30, 255],
            selection: [50, 80, 120, 255],
            selection_text: [0; 4],
            accent: [90, 150, 220, 255],
        },
        ViewerGeometry {
            width: 320,
            height: 160,
            scale: 1.0,
        },
    )
}
fn captures() -> (NativeCapture, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    (
        Arc::new(move |_, _| {
            count.fetch_add(1, Ordering::AcqRel);
            Err("Built-in NativeCapture must not run for this external attempt.".into())
        }),
        calls,
    )
}

fn forbid_image_discovery() -> Result<Vec<midnight_image_viewer_host::Provider>, String> {
    panic!("Forced Text or External invoked image discovery")
}

fn fail_image_discovery() -> Result<Vec<midnight_image_viewer_host::Provider>, String> {
    Err("controlled installed image discovery failure".into())
}

fn empty_image_discovery() -> Result<Vec<midnight_image_viewer_host::Provider>, String> {
    Ok(Vec::new())
}
fn wait_for<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "bounded comparison test timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn terminal(session: &Session) -> Output {
    wait_for(|| match session.mailbox.take() {
        Some(Output::Status(_)) | None => None,
        Some(output) => Some(output),
    })
}
fn choice(session: &Session, expected: ExternalCompareChoiceOutcome) -> FileChoice {
    match terminal(session) {
        Output::ChooseFiles(choice) => {
            assert_eq!(choice.external_outcome, Some(expected));
            choice
        }
        Output::ExternalStarted { .. } => panic!("unexpected external child start"),
        Output::Frame { .. } => panic!("external failure fell back to a built-in frame"),
        Output::Status(_) => unreachable!(),
    }
}
fn failure(message: &str) -> FsError {
    FsError::from_io(
        "external route test",
        Path::new(""),
        io::Error::other(message),
    )
}

#[derive(Clone, Copy)]
enum Mode {
    Started,
    NotStarted,
    Unknown,
    Disconnected,
    Refused,
    Unwind,
    UnwindDrop,
}

struct LaunchPanic {
    drops: Arc<AtomicUsize>,
    panic_on_drop: bool,
}
impl Drop for LaunchPanic {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        assert!(
            !self.panic_on_drop,
            "controlled secondary panic while dropping submit payload"
        );
    }
}
struct Submission {
    paths: [PathBuf; 2],
    executable: PathBuf,
    arguments: Vec<OsString>,
    cwd: PathBuf,
    credit: Weak<dyn ExternalCompareSessionCredit>,
    thread: ThreadId,
}
struct Launcher {
    mode: Mode,
    seen: Mutex<Vec<Submission>>,
    panic_payload_drops: Arc<AtomicUsize>,
}
impl Launcher {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            seen: Mutex::default(),
            panic_payload_drops: Arc::new(AtomicUsize::new(0)),
        })
    }
}
impl ExternalCompareLauncher for Launcher {
    fn submit(
        &self,
        process: ExternalCompareProcess,
        original_paths: [PathBuf; 2],
        session: Arc<dyn ExternalCompareSessionCredit>,
        _cancel: Cancellation,
        reply: SyncSender<ExternalCompareOutcome>,
    ) -> Result<(), FsError> {
        let plan = process.as_process();
        let UserCommandProcessInvocation::Program {
            executable,
            arguments,
        } = &plan.invocation
        else {
            panic!("comparator invoked a shell")
        };
        self.seen.lock().unwrap().push(Submission {
            paths: original_paths,
            executable: executable.clone(),
            arguments: arguments.clone(),
            cwd: plan.working_directory.clone(),
            credit: Arc::downgrade(&session),
            thread: std::thread::current().id(),
        });
        let outcome = match self.mode {
            Mode::Started => ExternalCompareOutcome::Started {
                pid: 4242,
                tracking_warning: Some("tracked by Engine".into()),
            },
            Mode::NotStarted => ExternalCompareOutcome::NotStarted(failure("known not started")),
            Mode::Unknown => ExternalCompareOutcome::Unknown {
                message: "native outcome uncertain".into(),
            },
            Mode::Disconnected => return Ok(()),
            Mode::Refused => return Err(failure("command admission refused")),
            Mode::Unwind | Mode::UnwindDrop => std::panic::panic_any(LaunchPanic {
                drops: self.panic_payload_drops.clone(),
                panic_on_drop: matches!(self.mode, Mode::UnwindDrop),
            }),
        };
        reply.try_send(outcome).unwrap();
        Ok(())
    }
}

#[test]
fn external_success_uses_original_paths_literal_argv_and_no_builtin_capture_or_frame() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let configured = program();
    let (snapshot, policy) = source.publish(FileComparatorTarget::External(configured.clone()));
    let (capture, calls) = captures();
    let launcher = Launcher::new(Mode::Started);
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source)
        .with_external_launcher(launcher.clone());
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let expected = paths();
    let session = open(&service, launch(&expected, &snapshot)).unwrap();
    wait_for(|| (service.active.load(Ordering::Acquire) == 0).then_some(()));
    assert!(
        session
            .restore_palette(&crate::appearance_tests::changed())
            .is_err()
    );
    assert!(matches!(
        terminal(&session),
        Output::ExternalStarted {
            pid: 4242,
            tracking_warning: Some(_)
        }
    ));
    assert_eq!(calls.load(Ordering::Acquire), 0);
    let seen = launcher.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].paths, expected);
    assert_eq!(
        seen[0].arguments,
        configured.arguments_for(&expected).unwrap()
    );
    assert_eq!(seen[0].executable, configured.executable);
    assert_eq!(seen[0].cwd, configured.executable.parent().unwrap());
    assert_ne!(seen[0].thread, std::thread::current().id());
    assert_eq!(*policy.seen.lock().unwrap(), vec![expected.clone()]);
    assert!(session.send(Input::Compare(expected)).is_err());
    wait_for(|| seen[0].credit.upgrade().is_none().then_some(()));
    drop(seen);
    session.close();
}

#[test]
fn explicit_not_started_and_sender_refusal_keep_paths_without_any_builtin_fallback() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for mode in [Mode::NotStarted, Mode::Refused] {
        let source = FakePolicySource::new();
        let (snapshot, _policy) = source.publish(FileComparatorTarget::External(program()));
        let launcher = Launcher::new(mode);
        let (capture, calls) = captures();
        let (_owner, spawner) = crate::resident_test_support::workers();
        let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
            .with_comparator_policies(source)
            .with_external_launcher(launcher.clone());
        let service = service.with_image_viewer_discovery(forbid_image_discovery);
        let expected = paths();
        let session = open(&service, launch(&expected, &snapshot)).unwrap();
        let result = choice(&session, ExternalCompareChoiceOutcome::NotStarted);
        assert_eq!(result.paths, expected);
        assert!(result.is_error);
        assert_eq!(calls.load(Ordering::Acquire), 0);
        assert_eq!(launcher.seen.lock().unwrap().len(), 1);
        assert!(session.mailbox.take().is_none());
        session.close();
    }
}

#[test]
fn unknown_and_disconnection_remain_typed_without_automatic_resubmission() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for mode in [Mode::Unknown, Mode::Disconnected] {
        let source = FakePolicySource::new();
        let (snapshot, _policy) = source.publish(FileComparatorTarget::External(program()));
        let launcher = Launcher::new(mode);
        let (capture, calls) = captures();
        let (_owner, spawner) = crate::resident_test_support::workers();
        let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
            .with_comparator_policies(source)
            .with_external_launcher(launcher.clone());
        let service = service.with_image_viewer_discovery(forbid_image_discovery);
        let expected = paths();
        let session = open(&service, launch(&expected, &snapshot)).unwrap();
        assert_eq!(
            choice(&session, ExternalCompareChoiceOutcome::Unknown).paths,
            expected
        );
        assert_eq!(
            session
                .restore_palette(&crate::appearance_tests::changed())
                .unwrap(),
            1
        );
        assert!(
            session
                .send(Input::Resize(
                    ViewerGeometry {
                        width: 800,
                        height: 600,
                        scale: 1.0
                    },
                    2,
                ))
                .is_ok()
        );
        assert!(session.mailbox.take().is_none());
        assert_eq!(launcher.seen.lock().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::Acquire), 0);
        session.close();
    }
}

#[test]
fn submit_unwind_publishes_unknown_before_payload_release_and_secondary_drop_panic() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for mode in [Mode::Unwind, Mode::UnwindDrop] {
        let source = FakePolicySource::new();
        let (snapshot, _policy) = source.publish(FileComparatorTarget::External(program()));
        let launcher = Launcher::new(mode);
        let (capture, calls) = captures();
        let (_owner, spawner) = crate::resident_test_support::workers();
        let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
            .with_comparator_policies(source.clone())
            .with_external_launcher(launcher.clone());
        let service = service.with_image_viewer_discovery(forbid_image_discovery);
        let expected = paths();
        let session = open(&service, launch(&expected, &snapshot)).unwrap();
        assert_eq!(
            choice(&session, ExternalCompareChoiceOutcome::Unknown).paths,
            expected
        );
        wait_for(|| (launcher.panic_payload_drops.load(Ordering::Acquire) == 1).then_some(()));
        assert_eq!(launcher.seen.lock().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::Acquire), 0);

        if matches!(mode, Mode::UnwindDrop) {
            // Actual outer Service catch/finish has run once its last credit dies.
            wait_for(|| {
                launcher.seen.lock().unwrap()[0]
                    .credit
                    .upgrade()
                    .is_none()
                    .then_some(())
            });
            assert_eq!(source.live_pins.load(Ordering::Acquire), 0);
            assert!(
                session.mailbox.take().is_none(),
                "worker completion erased Unknown"
            );
            assert!(session.send(Input::Compare(paths())).is_err());
        } else {
            // Normal destruction retains a live chooser. Explicit retry uses its
            // SAME policy and each new panic payload is actually destroyed once.
            assert!(session.send(Input::Compare(paths())).is_ok());
            let _ = choice(&session, ExternalCompareChoiceOutcome::Unknown);
            wait_for(|| (launcher.panic_payload_drops.load(Ordering::Acquire) == 2).then_some(()));
            assert_eq!(launcher.seen.lock().unwrap().len(), 2);
        }
        session.close();
    }
}

#[test]
fn manual_retry_uses_the_original_session_policy_after_live_policy_changes() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let (snapshot, old) = source.publish(FileComparatorTarget::External(program()));
    let launcher = Launcher::new(Mode::NotStarted);
    let (capture, calls) = captures();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source.clone())
        .with_external_launcher(launcher.clone());
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let initial = paths();
    let session = open(&service, launch(&initial, &snapshot)).unwrap();
    let _ = choice(&session, ExternalCompareChoiceOutcome::NotStarted);
    let (_next_snapshot, next) = source.publish(FileComparatorTarget::BuiltInText);
    let replacement = [initial[1].clone(), initial[0].clone()];
    assert!(session.send(Input::Compare(replacement.clone())).is_ok());
    assert_eq!(
        choice(&session, ExternalCompareChoiceOutcome::NotStarted).paths,
        replacement
    );
    assert_eq!(launcher.seen.lock().unwrap().len(), 2);
    assert_eq!(*old.seen.lock().unwrap(), vec![initial, replacement]);
    assert!(next.seen.lock().unwrap().is_empty());
    assert_eq!(calls.load(Ordering::Acquire), 0);
    session.close();
}

#[test]
fn a_real_worker_superseded_before_the_external_fence_submits_only_the_accepted_replacement() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let (snapshot, policy) = source.publish(FileComparatorTarget::External(program()));
    let (selected, release) = policy.gate_next_selection();
    let launcher = Launcher::new(Mode::Started);
    let (capture, calls) = captures();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source)
        .with_external_launcher(launcher.clone());
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let initial = paths();
    let session = open(&service, launch(&initial, &snapshot)).unwrap();
    selected.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut appearance = crate::appearance_tests::changed();
    assert_eq!(session.restore_palette(&appearance).unwrap(), 1);
    let replacement = [initial[1].clone(), initial[0].clone()];
    assert!(session.send(Input::Compare(replacement.clone())).is_ok());
    release.send(()).unwrap();
    wait_for(|| (service.active.load(Ordering::Acquire) == 0).then_some(()));
    assert_eq!(session.palette_revision(), 1);
    appearance.background = [1, 2, 3, 255];
    assert!(session.restore_palette(&appearance).is_err());
    assert_eq!(session.palette_revision(), 1);
    assert!(matches!(
        terminal(&session),
        Output::ExternalStarted { pid: 4242, .. }
    ));
    let submitted = launcher.seen.lock().unwrap();
    assert_eq!(
        submitted.len(),
        1,
        "stale generation launched an extra child"
    );
    assert_eq!(submitted[0].paths, replacement);
    assert_eq!(*policy.seen.lock().unwrap(), vec![initial, replacement]);
    assert_eq!(calls.load(Ordering::Acquire), 0);
    session.close();
}

#[test]
fn changed_or_forged_raw_snapshot_rejects_before_worker_capture_selection_or_submit() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let (old_snapshot, old) = source.publish(FileComparatorTarget::External(program()));
    let (current, next) = source.publish(FileComparatorTarget::BuiltInText);
    let launcher = Launcher::new(Mode::Started);
    let (capture, calls) = captures();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source)
        .with_external_launcher(launcher.clone());
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let stale = launch(&paths(), &old_snapshot);
    let mut forged_arc = launch(&paths(), &current);
    forged_arc.comparators = Arc::new(current.preferences.as_ref().clone());
    let mut mixed_revision = launch(&paths(), &current);
    mixed_revision.comparator_policy = old_snapshot.id;
    for draft in [stale, forged_arc, mixed_revision] {
        assert!(matches!(
            open(&service, draft),
            Err(OpenError::PolicyChanged)
        ));
    }
    assert_eq!(service.active.load(Ordering::Acquire), 0);
    assert_eq!(calls.load(Ordering::Acquire), 0);
    assert!(old.seen.lock().unwrap().is_empty());
    assert!(next.seen.lock().unwrap().is_empty());
    assert!(launcher.seen.lock().unwrap().is_empty());
}

struct Held {
    _credit: Arc<dyn ExternalCompareSessionCredit>,
    cancel: Cancellation,
    _reply: SyncSender<ExternalCompareOutcome>,
}
struct HoldingLauncher {
    held: Mutex<Option<Held>>,
    arrived: SyncSender<()>,
}
impl ExternalCompareLauncher for HoldingLauncher {
    fn submit(
        &self,
        _process: ExternalCompareProcess,
        _original_paths: [PathBuf; 2],
        session: Arc<dyn ExternalCompareSessionCredit>,
        cancel: Cancellation,
        reply: SyncSender<ExternalCompareOutcome>,
    ) -> Result<(), FsError> {
        assert!(
            self.held
                .lock()
                .unwrap()
                .replace(Held {
                    _credit: session,
                    cancel,
                    _reply: reply,
                })
                .is_none()
        );
        self.arrived.try_send(()).unwrap();
        Ok(())
    }
}
fn holding() -> (Arc<HoldingLauncher>, Receiver<()>) {
    let (arrived, receiver) = mpsc::sync_channel(1);
    (
        Arc::new(HoldingLauncher {
            held: Mutex::new(None),
            arrived,
        }),
        receiver,
    )
}

#[test]
fn pending_external_refuses_real_input_and_retains_only_counter_credit_after_close() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let (snapshot, policy) = source.publish(FileComparatorTarget::External(program()));
    let weak = Arc::downgrade(&policy);
    let (launcher, arrived) = holding();
    let (capture, calls) = captures();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source.clone())
        .with_external_launcher(launcher.clone());
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let first = open(&service, launch(&paths(), &snapshot)).unwrap();
    arrived.recv_timeout(Duration::from_secs(5)).unwrap();
    let second = open(&service, unresolved(&snapshot)).unwrap();
    assert!(first.send(Input::Compare(paths())).is_err());
    assert!(
        first
            .send(Input::Browse {
                side: 1,
                paths: paths(),
                owner: PathPickerOwner::None
            })
            .is_err()
    );
    for revision in 0..32 {
        assert!(
            first
                .send(Input::Resize(
                    ViewerGeometry {
                        width: 320,
                        height: 160,
                        scale: 1.0
                    },
                    revision,
                ))
                .is_ok()
        );
    }
    assert!(
        first
            .send(Input::Resize(
                ViewerGeometry {
                    width: 320,
                    height: 160,
                    scale: 1.0
                },
                33,
            ))
            .is_err()
    );
    // Sessions are not counted: a third opens beside the two.
    let third = open(&service, unresolved(&snapshot)).unwrap();
    third.close();
    drop(third);

    let (current, _next) = source.publish(FileComparatorTarget::Automatic);
    drop(policy);
    assert!(
        first.send(Input::Close).is_ok(),
        "Close must work even when the input FIFO is full"
    );
    second.close();
    wait_for(|| {
        let held_credit_only = launcher
            .held
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|held| Arc::strong_count(&held._credit) == 1);
        (source.live_pins.load(Ordering::Acquire) == 0
            && weak.upgrade().is_none()
            && service.active.load(Ordering::Acquire) == 1
            && held_credit_only)
            .then_some(())
    });
    assert!(
        weak.upgrade().is_none(),
        "admission credit retained the fake owned policy probe"
    );
    // This is a provider-free owner graph test. It does not establish production
    // compiled-policy or fm-retirement behavior; Backend owns those real gates.
    assert!(
        launcher
            .held
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .cancel
            .is_cancelled()
    );

    // Only the external owner's erased credit remains from the closed Session; it
    // refuses no opening.
    let fresh = open(&service, unresolved(&current)).unwrap();
    let other = open(&service, unresolved(&current)).unwrap();
    drop(launcher.held.lock().unwrap().take());
    wait_for(|| (service.active.load(Ordering::Acquire) == 2).then_some(()));
    assert_eq!(calls.load(Ordering::Acquire), 0);
    fresh.close();
    other.close();
}

#[test]
fn forced_external_without_a_host_launcher_is_not_started_and_never_falls_back() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let (snapshot, _policy) = source.publish(FileComparatorTarget::External(program()));
    let (capture, calls) = captures();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source);
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let session = open(&service, launch(&paths(), &snapshot)).unwrap();
    assert_eq!(
        choice(&session, ExternalCompareChoiceOutcome::NotStarted).paths,
        paths()
    );
    assert_eq!(calls.load(Ordering::Acquire), 0);
    session.close();
}

#[derive(Debug)]
struct TextBytes(Arc<AtomicUsize>);

impl midnight_plugin_host::ContentSource for TextBytes {
    fn length(&self) -> u64 {
        b"forced text comparison\n".len() as u64
    }

    fn display_name(&self) -> String {
        "provider-synthetic-name.bin".into()
    }

    fn version(&self) -> Vec<u8> {
        vec![1]
    }

    fn validate(&self) -> Result<(), midnight_plugin_host::Error> {
        Ok(())
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, midnight_plugin_host::Error> {
        self.0.fetch_add(1, Ordering::AcqRel);
        let bytes = b"forced text comparison\n";
        let start = offset.min(bytes.len() as u64) as usize;
        let end = start.saturating_add(length as usize).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }
}

#[test]
fn forced_text_worker_bypasses_discovery_and_matches_original_names_before_capturing() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let (snapshot, policy) = source.publish(FileComparatorTarget::BuiltInText);
    let reads = Arc::new(AtomicUsize::new(0));
    let captures = Arc::new(AtomicUsize::new(0));
    let counted = captures.clone();
    let content_reads = reads.clone();
    let capture: NativeCapture = Arc::new(move |_, _| {
        counted.fetch_add(1, Ordering::AcqRel);
        midnight_plugin_host::CapturedFile::from_source(Arc::new(TextBytes(content_reads.clone())))
            .map_err(|error| error.to_string())
    });
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source);
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let expected = paths().map(|mut path| {
        path.set_extension("png");
        path
    });
    let session = open(&service, launch(&expected, &snapshot)).unwrap();
    match terminal(&session) {
        Output::Frame {
            frame,
            names,
            appearance_revision,
        } => {
            assert_eq!(appearance_revision, 0);
            assert!(!frame.image);
            assert!(frame.accessible_text.contains("forced text comparison"));
            assert_eq!(
                names,
                expected.each_ref().map(|path| path.display().to_string())
            );
        }
        _ => panic!("forced text did not produce a text frame"),
    }
    assert_eq!(captures.load(Ordering::Acquire), 2);
    assert!(reads.load(Ordering::Acquire) >= 2);
    assert_eq!(*policy.seen.lock().unwrap(), vec![expected]);
    assert!(
        policy
            .selection_threads
            .lock()
            .unwrap()
            .iter()
            .all(|thread| { *thread != std::thread::current().id() })
    );
    session.close();
}

#[test]
fn forced_images_worker_reports_unavailable_coverage_without_reading_text() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for discover in [
        fail_image_discovery as fn() -> Result<Vec<midnight_image_viewer_host::Provider>, String>,
        empty_image_discovery,
    ] {
        let source = FakePolicySource::new();
        let (snapshot, _policy) = source.publish(FileComparatorTarget::BuiltInImages);
        let (capture, calls) = captures();
        let (_owner, spawner) = crate::resident_test_support::workers();
        let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
            .with_comparator_policies(source);
        let service = service.with_image_viewer_discovery(discover);
        let expected = paths();
        let session = open(&service, launch(&expected, &snapshot)).unwrap();
        match terminal(&session) {
            Output::ChooseFiles(choice) => {
                assert_eq!(choice.paths, expected);
                assert!(choice.is_error);
                assert_eq!(choice.external_outcome, None);
            }
            _ => panic!("forced images silently fell back to a text or external comparison"),
        }
        assert_eq!(calls.load(Ordering::Acquire), 0);
        session.close();
    }
}

#[test]
fn forced_text_capture_failure_stays_a_chooser_and_never_discovers_images() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let source = FakePolicySource::new();
    let (snapshot, _policy) = source.publish(FileComparatorTarget::BuiltInText);
    let (capture, calls) = captures();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::with_native_capture(runtime.handle().clone(), spawner, capture)
        .with_comparator_policies(source);
    let service = service.with_image_viewer_discovery(forbid_image_discovery);
    let session = open(&service, launch(&paths(), &snapshot)).unwrap();
    match terminal(&session) {
        Output::ChooseFiles(choice) => {
            assert!(choice.is_error);
            assert_eq!(choice.external_outcome, None);
        }
        _ => panic!("forced text capture failure selected another comparator"),
    }
    assert_eq!(calls.load(Ordering::Acquire), 1);
    session.close();
}
