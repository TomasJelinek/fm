//! Real submit-payload Drop panic stops the worker after publishing typed Unknown.
//! AUTHORED / NOT RUN pending the coordinated Build lease.
use super::{
    ComparatorWindow, ComparatorWindows, Input, OpenWindow, identity_tests::TestRuntime, raw_paths,
    support,
};
use midnight_compare_engine::{
    ExternalCompareChoiceOutcome as Outcome, ExternalCompareLauncher, FileComparatorPinError,
    FileComparatorPolicySource, Output, OwnedFileComparatorPolicy, Refused, Service, Session,
};
use midnight_core::*;
use slint::{
    ComponentHandle,
    platform::{
        PointerEventButton, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
    private_unstable_api::re_exports::{
        AccessibleRole, AccessibleStringProperty, ItemRc, WindowInner,
    },
};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

#[test]
fn dead_external_worker_refuses_manual_retry_and_retains_unknown_raw_drafts() {
    let native = platform();
    let runtime = runtime();
    let [a, b] = raw_paths::paths();
    // Unicode basenames qualify for the one '*' rule; raw parent components stay lossless.
    let paths = [a.join("before.c"), b.join("after.cpp")];
    assert!(paths.iter().all(|path| path.to_str().is_none()));
    let fixture = Fixture::new(&runtime, paths.clone(), Mode::DropPanic);
    let (manager, open) = open_manager(&fixture, &paths);

    // Consume actual Service output before allowing the real manager timer to resume.
    let output = choice_output(&open.session, Outcome::Unknown);
    let Output::ChooseFiles(choice) = &output else {
        unreachable!()
    };
    assert_eq!(choice.external_outcome, Some(Outcome::Unknown));
    assert_eq!(choice.paths, paths);
    open.present_output(&output);
    assert_eq!(open.external_outcome.get(), Some(Outcome::Unknown));
    assert!(!open.window.get_choice_message().is_empty());
    fixture
        .policy_dropped
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    wait_for(|| open.session.send(Input::Resize(geometry(), 77)) == Err(Refused::Stopped));
    assert_eq!(fixture.launcher.payload_drops.load(Ordering::Acquire), 1);
    assert!(open.session.send(Input::Compare(paths.clone())).is_err());
    for side in 0..2 {
        assert!(
            open.session
                .send(Input::Browse {
                    side,
                    paths: paths.clone(),
                    owner: PathPickerOwner::None,
                })
                .is_err()
        );
    }
    assert!(
        !open.closed.get(),
        "worker death must leave the uncertain chooser visible"
    );
    assert!(open.session.mailbox.take().is_none());

    open.poll.restart();
    assert!(open.poll.running());
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1100 * 740];
    tick(&native, &mut pixels);
    checkpoint(
        &native,
        &open.window,
        &mut pixels,
        "external-unknown-before-retry",
    );
    for (label, image) in [
        ("Compare", "external-unknown-after-refused-compare"),
        (
            "Browse for File A",
            "external-unknown-after-refused-browse-a",
        ),
        (
            "Browse for File B",
            "external-unknown-after-refused-browse-b",
        ),
    ] {
        click(&open.window, label);
        assert_refused_unknown(&open, &paths);
        // Pump the real window timers after each refused action.
        tick(&native, &mut pixels);
        tick(&native, &mut pixels);
        assert_refused_unknown(&open, &paths);
        assert_eq!(fixture.launcher.submits.load(Ordering::Acquire), 1);
        assert_eq!(fixture.picker.0.load(Ordering::Acquire), 0);
        assert_eq!(fixture.captures.load(Ordering::Acquire), 0);
        checkpoint(&native, &open.window, &mut pixels, image);
    }
    manager.close_all();
}

#[test]
fn configured_external_not_started_keeps_raw_drafts_and_visible_error() {
    let native = platform();
    let runtime = runtime();
    let [a, b] = raw_paths::paths();
    let paths = [a.join("before.c"), b.join("after.cpp")];
    let fixture = Fixture::new(&runtime, paths.clone(), Mode::NotStarted);
    let (manager, open) = open_manager(&fixture, &paths);
    let output = choice_output(&open.session, Outcome::NotStarted);
    open.present_output(&output);
    open.poll.restart();
    assert!(open.poll.running());
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1100 * 740];
    tick(&native, &mut pixels);
    assert_eq!(open.external_outcome.get(), Some(Outcome::NotStarted));
    assert_eq!(*open.choice_paths.borrow(), paths);
    assert_eq!(
        open.window.get_file_a(),
        paths[0].to_string_lossy().as_ref()
    );
    assert_eq!(
        open.window.get_file_b(),
        paths[1].to_string_lossy().as_ref()
    );
    assert!(open.window.get_choosing_files());
    assert!(open.window.get_choice_error());
    assert!(!open.window.get_choice_busy());
    assert!(!open.comparison_requested.get());
    assert_eq!(open.window.get_status(), open.window.get_choice_message());
    let message = open.window.get_choice_message().to_ascii_lowercase();
    assert!(message.contains("did not start"), "{message}");
    assert!(message.contains("controlled notstarted reply"), "{message}");
    assert_eq!(fixture.launcher.submits.load(Ordering::Acquire), 1);
    assert_eq!(fixture.launcher.payload_drops.load(Ordering::Acquire), 0);
    assert_eq!(fixture.captures.load(Ordering::Acquire), 0);
    assert_eq!(fixture.picker.0.load(Ordering::Acquire), 0);
    checkpoint(&native, &open.window, &mut pixels, "external-not-started");
    manager.close_all();
}

// A selector spy for the single '*' rule; no matching implementation is copied.
struct Policy(Arc<FileComparatorPreferences>);
impl CompiledFileComparatorPolicy for Policy {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.0
    }
    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        Some(0)
    }
    fn matcher_budget_bytes(&self) -> usize {
        0
    }
}
struct Source {
    store: FileComparatorPolicyStore,
    dropped: SyncSender<()>,
}
impl FileComparatorPolicySource for Source {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.store.snapshot()
    }
    fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<Box<dyn OwnedFileComparatorPolicy>, FileComparatorPinError> {
        let policy = self
            .store
            .pin(expected)
            .map_err(|_| FileComparatorPinError::PolicyChanged)?;
        Ok(Box::new(Owned {
            policy,
            dropped: self.dropped.clone(),
            _credit: session,
        }))
    }
}
struct Owned {
    policy: AcceptedFileComparatorPolicy,
    dropped: SyncSender<()>,
    _credit: Arc<dyn ExternalCompareSessionCredit>,
}
impl OwnedFileComparatorPolicy for Owned {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.policy.snapshot()
    }
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        self.policy.preferences()
    }
    fn select(&self, paths: &[PathBuf; 2]) -> Option<usize> {
        self.policy.select(paths)
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.dropped.try_send(());
    }
}

struct SubmitPayload(Arc<AtomicUsize>);
impl Drop for SubmitPayload {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
        panic!("controlled secondary panic while dropping submit payload");
    }
}
#[derive(Clone, Copy)]
enum Mode {
    DropPanic,
    NotStarted,
}
struct Launcher {
    mode: Mode,
    submits: AtomicUsize,
    payload_drops: Arc<AtomicUsize>,
    release: Mutex<Receiver<()>>,
    expected: [PathBuf; 2],
}
impl ExternalCompareLauncher for Launcher {
    fn submit(
        &self,
        _process: ExternalCompareProcess,
        original_paths: [PathBuf; 2],
        _session: Arc<dyn ExternalCompareSessionCredit>,
        _cancel: Cancellation,
        reply: SyncSender<ExternalCompareOutcome>,
    ) -> Result<(), FsError> {
        assert_eq!(self.submits.fetch_add(1, Ordering::AcqRel), 0);
        assert_eq!(original_paths, self.expected);
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        match self.mode {
            Mode::DropPanic => std::panic::panic_any(SubmitPayload(self.payload_drops.clone())),
            Mode::NotStarted => {
                reply
                    .try_send(ExternalCompareOutcome::NotStarted(FsError::from_io(
                        "launch external comparison",
                        Path::new(""),
                        std::io::Error::other("controlled NotStarted reply"),
                    )))
                    .unwrap();
                Ok(())
            }
        }
    }
}
struct Picker(AtomicUsize);
impl PathPickerService for Picker {
    fn pick_path(
        &self,
        _: &PathPickerRequest,
        _: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        self.0.fetch_add(1, Ordering::AcqRel);
        panic!("a dead comparison worker must never invoke the picker");
    }
}
struct Fixture {
    service: Service,
    snapshot: FileComparatorSnapshot,
    launcher: Arc<Launcher>,
    picker: Arc<Picker>,
    captures: Arc<AtomicUsize>,
    release: SyncSender<()>,
    policy_dropped: Receiver<()>,
}
impl Fixture {
    fn new(runtime: &TestRuntime, paths: [PathBuf; 2], mode: Mode) -> Self {
        let (dropped, policy_dropped) = mpsc::sync_channel(1);
        let store = FileComparatorPolicyStore::default();
        let raw = Arc::new(FileComparatorPreferences {
            rules: vec![FileComparatorRule {
                enabled: true,
                mode: FilterMode::Masks,
                expression: "*".into(),
                case_sensitive: false,
                target: FileComparatorTarget::External(FileComparatorProgram {
                    executable: Path::new(env!("CARGO_MANIFEST_DIR")).join("never-executed.exe"),
                    arguments: vec!["{left}".into(), "{right}".into()],
                    working_directory: None,
                }),
            }],
        });
        let prepared = PreparedFileComparatorPolicy::new(Arc::new(Policy(raw)))
            .with_id(store.prepare_id().unwrap());
        let snapshot = store.publish(prepared);
        let source = Arc::new(Source { store, dropped });
        let (release, blocked) = mpsc::sync_channel(1);
        let launcher = Arc::new(Launcher {
            mode,
            submits: AtomicUsize::new(0),
            payload_drops: Arc::new(AtomicUsize::new(0)),
            release: Mutex::new(blocked),
            expected: paths,
        });
        let picker = Arc::new(Picker(AtomicUsize::new(0)));
        let captures = Arc::new(AtomicUsize::new(0));
        let calls = captures.clone();
        let service = Service::with_native_capture(
            runtime.handle().clone(),
            runtime.spawner(),
            Arc::new(move |_, _| {
                calls.fetch_add(1, Ordering::AcqRel);
                Err("forced External must not capture file contents".into())
            }),
        )
        .with_comparator_policies(source)
        .with_external_launcher(launcher.clone())
        .with_path_picker(picker.clone());
        Self {
            service,
            snapshot,
            launcher,
            picker,
            captures,
            release,
            policy_dropped,
        }
    }
    fn launch(&self, paths: &[PathBuf; 2]) -> FileCompareLaunch {
        FileCompareLaunch {
            sources: paths
                .iter()
                .cloned()
                .map(FileCompareSource::Focused)
                .collect(),
            disabled_plugins: Arc::from(["disabled.image.fixture".into()]),
            comparators: self.snapshot.preferences.clone(),
            comparator_policy: self.snapshot.id,
        }
    }
}
fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 16.0,
        foreground: [255; 4],
        background: [24, 28, 36, 255],
        selection: [60, 80, 120, 255],
        selection_text: [0; 4],
        accent: [100, 150, 220, 255],
    }
}
fn geometry() -> ViewerGeometry {
    ViewerGeometry {
        width: 1100,
        height: 640,
        scale: 1.0,
    }
}
fn platform() -> Rc<MinimalSoftwareWindow> {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    native
}
fn runtime() -> TestRuntime {
    TestRuntime::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(2)
            .build()
            .unwrap(),
    )
}
fn open_manager(fixture: &Fixture, paths: &[PathBuf; 2]) -> (ComparatorWindows, Rc<OpenWindow>) {
    let manager = ComparatorWindows::new_polled(fixture.service.clone());
    manager
        .open(
            fixture.launch(paths),
            appearance(),
            slint::LogicalSize::new(1100.0, 740.0),
            Rc::new(|_, _| None),
        )
        .unwrap();
    let open = manager.windows.borrow().last().unwrap().clone();
    open.poll.stop();
    open.window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    fixture.release.try_send(()).unwrap();
    (manager, open)
}
fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "comparison worker did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn choice_output(session: &Session, expected: Outcome) -> Output {
    let mut output = None;
    wait_for(|| match session.mailbox.take() {
        Some(Output::Status(_)) | None => false,
        Some(actual) => {
            output = Some(actual);
            true
        }
    });
    let output = output.unwrap();
    assert!(
        matches!(&output, Output::ChooseFiles(choice) if choice.external_outcome == Some(expected))
    );
    output
}
fn assert_refused_unknown(open: &OpenWindow, paths: &[PathBuf; 2]) {
    assert_eq!(open.external_outcome.get(), Some(Outcome::Unknown));
    assert_eq!(&*open.choice_paths.borrow(), paths);
    assert_eq!(
        open.window.get_file_a(),
        paths[0].to_string_lossy().as_ref()
    );
    assert_eq!(
        open.window.get_file_b(),
        paths[1].to_string_lossy().as_ref()
    );
    assert!(open.window.get_choosing_files());
    assert!(open.window.get_choice_error());
    assert!(!open.window.get_choice_busy());
    assert!(!open.comparison_requested.get());
    assert_eq!(open.window.get_status(), open.window.get_choice_message());
    let message = open.window.get_choice_message().to_ascii_lowercase();
    assert!(message.contains("may have started"), "{message}");
    assert!(message.contains("check the other application"), "{message}");
    assert!(
        message.contains("could not accept the request"),
        "{message}"
    );
    assert!(message.contains("busy or stopped"), "{message}");
    assert!(
        open.pending
            .borrow()
            .iter()
            .all(|input| !matches!(input, Input::Compare(_) | Input::Browse { .. })),
        "a refused request remained queued for replay"
    );
}
fn tick(native: &MinimalSoftwareWindow, pixels: &mut [slint::Rgb8Pixel]) {
    std::thread::sleep(Duration::from_millis(20));
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(pixels, 1100);
    });
}
fn checkpoint(
    native: &MinimalSoftwareWindow,
    window: &ComparatorWindow,
    pixels: &mut [slint::Rgb8Pixel],
    name: &str,
) {
    assert_eq!(window.window().size(), slint::PhysicalSize::new(1100, 740));
    window.window().request_redraw();
    native.draw_if_needed(|renderer| {
        renderer.render(pixels, 1100);
    });
    if let Some(directory) = std::env::var_os("ME_COMPARE_POLISH_SCREENSHOTS") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut ppm = b"P6\n1100 740\n255\n".to_vec();
        for pixel in pixels {
            ppm.extend([pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(directory.join(format!("{name}-1100x740.ppm")), ppm).unwrap();
    }
}
fn find_button(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == AccessibleRole::Button
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|text| text == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_button(item, label) {
            return Some(found);
        }
    }
    None
}
fn click(window: &ComparatorWindow, label: &str) {
    let root = ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    let button =
        find_button(root, label).unwrap_or_else(|| panic!("missing chooser button: {label}"));
    assert!(button.is_visible(), "{label} is not visible");
    assert_eq!(
        button
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .as_deref(),
        Some("true")
    );
    let rect = button.geometry();
    let origin = button.map_to_native_window(rect.origin);
    let size = window
        .window()
        .size()
        .to_logical(window.window().scale_factor());
    assert!(rect.width() > 0.0 && rect.height() > 0.0);
    assert!(
        origin.x >= 0.0
            && origin.y >= 0.0
            && origin.x + rect.width() <= size.width + 0.5
            && origin.y + rect.height() <= size.height + 0.5,
        "{label} outside chooser window"
    );
    let position = slint::LogicalPosition::new(
        origin.x + rect.width() / 2.0,
        origin.y + rect.height() / 2.0,
    );
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        window.window().dispatch_event(event);
    }
}
