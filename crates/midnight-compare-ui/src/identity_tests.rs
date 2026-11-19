//! Actual typed output is consumed by the production presentation path.
use super::support;
use super::*;
use midnight_compare_engine::{
    FileChoice, FileComparatorPinError, FileComparatorPolicySource, Frame, IdentityVerdict, Output,
    OwnedFileComparatorPolicy,
};
use midnight_core::{
    ExternalCompareSessionCredit, FileComparatorPolicyStore, FileComparatorPreferences,
    FileComparatorRule, FileComparatorSnapshot, FileComparatorTarget, FilterMode,
};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

#[path = "../tests/support/resident.rs"]
mod resident;
pub(super) use resident::TestRuntime;

pub(super) fn platform() -> Rc<MinimalSoftwareWindow> {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    native
}

pub(super) fn runtime() -> TestRuntime {
    TestRuntime::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(2)
            .build()
            .unwrap(),
    )
}

pub(super) type SelectedPairs = Arc<Mutex<Vec<[PathBuf; 2]>>>;

// Test-only raw metadata: select the explicit single rule before optional discovery.
// This fixture contains no compiled matcher or retirement-sensitive policy resources.
struct TextPolicySource {
    snapshot: FileComparatorSnapshot,
    selected: SelectedPairs,
}

struct TextPolicy {
    snapshot: FileComparatorSnapshot,
    selected: SelectedPairs,
    _session: Arc<dyn ExternalCompareSessionCredit>,
}

impl FileComparatorPolicySource for TextPolicySource {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.snapshot.clone()
    }

    fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<Box<dyn OwnedFileComparatorPolicy>, FileComparatorPinError> {
        if expected.id != self.snapshot.id
            || !Arc::ptr_eq(&expected.preferences, &self.snapshot.preferences)
        {
            return Err(FileComparatorPinError::PolicyChanged);
        }
        Ok(Box::new(TextPolicy {
            snapshot: self.snapshot.clone(),
            selected: self.selected.clone(),
            _session: session,
        }))
    }
}

impl OwnedFileComparatorPolicy for TextPolicy {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.snapshot.clone()
    }

    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.snapshot.preferences
    }

    fn select(&self, original_paths: &[PathBuf; 2]) -> Option<usize> {
        self.selected.lock().unwrap().push(original_paths.clone());
        Some(0)
    }
}

pub(super) fn text_service(runtime: &TestRuntime) -> (Service, SelectedPairs) {
    let snapshot = FileComparatorSnapshot {
        id: FileComparatorPolicyStore::default().prepare_id().unwrap(),
        preferences: Arc::new(FileComparatorPreferences {
            rules: vec![FileComparatorRule {
                enabled: true,
                mode: FilterMode::Masks,
                expression: "*".into(),
                case_sensitive: false,
                target: FileComparatorTarget::BuiltInText,
            }],
        }),
    };
    let selected = SelectedPairs::default();
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        runtime.spawner(),
        Arc::new(|_, _| Err("Rejected by UI test capture fixture.".into())),
    )
    .with_comparator_policies(Arc::new(TextPolicySource {
        snapshot,
        selected: selected.clone(),
    }));
    (service, selected)
}

pub(super) fn open(runtime: &TestRuntime) -> Rc<OpenWindow> {
    let (service, _) = text_service(runtime);
    open_service(&service)
}

pub(super) fn open_service(service: &Service) -> Rc<OpenWindow> {
    let comparator_snapshot = service.comparator_snapshot();
    let appearance = ViewerAppearance {
        family: "monospace".into(),
        font_size: 12.0,
        foreground: [240, 240, 240, 255],
        background: [24, 28, 36, 255],
        selection: [60, 80, 120, 255],
        accent: [100, 150, 220, 255],
    };
    let session = service
        .open(
            FileCompareLaunch {
                sources: Vec::new(),
                disabled_plugins: Arc::from([]),
                comparators: comparator_snapshot.preferences.clone(),
                comparator_policy: comparator_snapshot.id,
            },
            appearance.clone(),
            ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    let appearance_revision = session.palette_revision();
    assert_eq!(
        appearance_revision, 0,
        "The opening palette is revision zero in the approved engine contract"
    );
    OpenWindow::new(ComparatorWindow::new().unwrap(), session, &appearance)
}

pub(super) fn next_choice(open: &OpenWindow) -> FileChoice {
    let started = Instant::now();
    loop {
        if let Some(output) = open.session.mailbox.take() {
            match output {
                Output::ChooseFiles(choice) => return choice,
                Output::Status(_) => {}
                _ => panic!("text-only rejecting capture must return a chooser"),
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "worker chooser timed out"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn frame(generation: u64, identity: IdentityVerdict) -> Output {
    Output::Frame {
        // Initial opening colors are accepted at revision zero, not a fallback stamp.
        appearance_revision: 0,
        frame: Frame {
            find: Arc::default(),
            fonts: None,
            geometry_revision: 3,
            comparison_generation: generation,
            identity,
            mode: None,
            width: 2,
            height: 2,
            pixels: [12, 34, 56, 255].repeat(4),
            layers: None,
            accessible_text: "Same visible page".into(),
            status: "Misleading identical status is not proof".into(),
            image: false,
            position: 0.0,
        },
        names: [format!("left-{generation}"), format!("right-{generation}")],
    }
}

pub(super) fn paths() -> [std::path::PathBuf; 2] {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        root.join("unopened-left.txt"),
        root.join("unopened-right.txt"),
    ]
}

fn choice(message: &str) -> Output {
    Output::ChooseFiles(FileChoice {
        paths: paths(),
        message: message.into(),
        is_error: true,
        external_outcome: None,
    })
}

#[test]
fn typed_identity_controls_the_result_and_status_chooser_and_close_clear_it() {
    let _native = platform();
    let runtime = runtime();
    let open = open(&runtime);
    for (identity, expected) in [
        (IdentityVerdict::IdenticalFiles, 1),
        (IdentityVerdict::Pending, 0),
        (IdentityVerdict::IdenticalText, 2),
        (IdentityVerdict::Different, 3),
    ] {
        open.present_output(&frame(5, identity));
        assert_eq!(open.window.get_identity_state(), expected);
        assert_eq!(open.window.get_left_name(), "left-5");
    }
    open.present_output(&frame(5, IdentityVerdict::IdenticalFiles));
    open.present_output(&Output::Status("Revalidating source identities".into()));
    assert_eq!(open.window.get_identity_state(), -1);
    assert_eq!(open.window.get_status(), "Revalidating source identities");

    open.present_output(&frame(5, IdentityVerdict::IdenticalText));
    open.present_output(&choice("Source changed; choose current inputs"));
    assert_eq!(open.window.get_identity_state(), -1);
    assert_eq!(
        open.window.get_status(),
        "Source changed; choose current inputs"
    );
    assert!(open.window.get_choosing_files());
    assert_eq!(open.window.get_surface().size().width, 0);

    open.close();
    open.present_output(&frame(6, IdentityVerdict::IdenticalFiles));
    assert_eq!(open.window.get_identity_state(), -1);
    assert!(open.window.get_status().is_empty());
    assert_eq!(open.window.get_surface().size().width, 0);
}

#[test]
fn zero_and_known_old_generations_cannot_revive_a_result_or_dismiss_the_chooser() {
    let _native = platform();
    let runtime = runtime();
    let open = open(&runtime);
    open.present_output(&frame(0, IdentityVerdict::IdenticalFiles));
    assert_eq!(open.window.get_identity_state(), -1);
    assert!(open.window.get_left_name().is_empty());
    open.present_output(&frame(7, IdentityVerdict::IdenticalFiles));
    open.present_output(&choice("Retry current inputs"));
    open.window.set_mode(2);
    assert!(open.send(Input::Compare(paths())));
    assert_eq!(open.window.get_identity_state(), -1);

    assert!(open.pending.borrow().is_empty());
    // Session admission owns the producer fence; these are consumer generation checks.
    for generation in [0, 6, 7] {
        open.present_output(&frame(generation, IdentityVerdict::IdenticalFiles));
        assert!(open.window.get_choosing_files());
        assert_eq!(open.window.get_identity_state(), -1);
        assert_eq!(open.window.get_left_name(), "left-7");
        assert_eq!(open.window.get_surface().size().width, 0);
    }

    open.present_output(&frame(8, IdentityVerdict::IdenticalText));
    assert!(!open.window.get_choosing_files());
    assert_eq!(open.window.get_identity_state(), 2);
    assert_eq!(open.window.get_left_name(), "left-8");
    assert_eq!(open.comparison_generation.get(), 8);
    assert_eq!(
        open.window.get_mode(),
        2,
        "A verdict cannot change the requested image mode"
    );
    open.present_output(&frame(7, IdentityVerdict::IdenticalFiles));
    assert_eq!(open.window.get_identity_state(), 2);
    assert_eq!(open.window.get_left_name(), "left-8");
    open.close();
}

#[test]
fn session_admitted_compare_reaches_worker_without_a_local_pending_choice() {
    let _native = platform();
    let runtime = runtime();
    let (service, selected) = text_service(&runtime);
    let open = open_service(&service);
    open.present_output(&choice("Choose the current pair"));
    assert!(open.send(Input::Compare(paths())));
    assert!(open.pending.borrow().is_empty());
    assert!(open.comparison_requested.get());
    assert_eq!(open.window.get_identity_state(), -1);
    assert_eq!(open.window.get_status(), "Preparing comparison…");
    let result = next_choice(&open);
    assert_eq!(result.paths, paths());
    assert_eq!(selected.lock().unwrap().as_slice(), &[paths()]);
    assert!(result.is_error);
    let message = result.message.clone();
    open.present_output(&Output::ChooseFiles(result));
    assert!(!open.comparison_requested.get());
    assert_eq!(open.comparison_generation.get(), 0);
    assert_eq!(open.window.get_identity_state(), -1);
    assert_eq!(open.window.get_choice_message(), message);
    assert!(open.window.get_left_name().is_empty());
    assert!(open.window.get_choosing_files());
    open.close();
}

#[test]
fn session_compare_admission_ignores_full_local_queue_and_refusal_preserves_identity() {
    let _native = platform();
    let runtime = runtime();
    for closed in [false, true] {
        let open = open(&runtime);
        open.present_output(&frame(3, IdentityVerdict::IdenticalFiles));
        let status = open.window.get_status();
        open.flush();
        for _ in 0..MAX_PENDING {
            assert!(open.send(Input::Difference { backwards: false }));
        }
        if closed {
            open.session.close();
        }
        assert!(!open.closed.get());
        assert_eq!(open.send(Input::Compare(paths())), !closed);
        assert_eq!(open.pending.borrow().len(), MAX_PENDING);
        assert!(
            open.pending
                .borrow()
                .iter()
                .all(|input| matches!(input, Input::Difference { .. }))
        );
        if closed {
            assert_eq!(open.window.get_identity_state(), 1);
            assert_eq!(open.window.get_status(), status);
            assert!(!open.comparison_requested.get());
            assert_eq!(open.window.get_surface().size().width, 2);
        } else {
            assert_eq!(open.window.get_identity_state(), -1);
            assert!(open.comparison_requested.get());
            assert_eq!(next_choice(&open).paths, paths());
        }
        assert!(!open.window.get_choosing_files());
        open.close();
    }
}

#[test]
fn accepted_palettes_fence_frames_without_replaying_choices_or_clearing_unknown() {
    let _native = platform();
    let runtime = runtime();
    let open = open(&runtime);
    open.present_output(&frame(5, IdentityVerdict::IdenticalFiles));
    let initial_surface = open.window.get_surface();
    let initial_status = open.window.get_status();
    let a = ViewerAppearance {
        family: "Segoe UI".into(),
        font_size: 24.0,
        foreground: [240, 240, 240, 255],
        background: [24, 28, 36, 255],
        selection: [60, 80, 120, 255],
        accent: [100, 150, 220, 255],
    };
    let mut b = a.clone();
    b.foreground = [48, 48, 48, 255];
    b.background = [255, 255, 255, 255];
    let presentation = |value: &ViewerAppearance| {
        appearance::Presentation::new(
            value,
            midnight_core::PreferenceTheme::Light,
            midnight_core::UiDensity::Desktop,
            value.font_size as u16,
            28,
        )
    };
    // Palette updates use session configuration even when ordinary UI input is full.
    open.pending
        .borrow_mut()
        .extend((0..MAX_PENDING).map(|_| Input::Difference { backwards: false }));
    open.apply_appearance(&presentation(&b));
    let b_revision = open.appearance.revision();
    assert!(b_revision > 0);
    assert_eq!(open.window.get_status(), initial_status);
    assert_eq!(open.window.get_surface(), initial_surface);
    assert_eq!(open.comparison_generation.get(), 5);
    assert_eq!(open.pending.borrow().len(), MAX_PENDING);
    assert!(!open.comparison_requested.get());

    // A rollback must fence skipped B and return to A at a newer request revision.
    open.apply_appearance(&presentation(&a));
    let a_revision = open.appearance.revision();
    assert!(a_revision > b_revision);
    let publish = |revision| {
        let mut output = frame(5, IdentityVerdict::IdenticalText);
        let Output::Frame {
            appearance_revision,
            ..
        } = &mut output
        else {
            unreachable!()
        };
        *appearance_revision = revision;
        open.present_output(&output);
    };
    for old in [0, b_revision] {
        publish(old);
        assert_eq!(open.window.get_identity_state(), 1);
        assert_eq!(open.window.get_surface(), initial_surface);
    }
    publish(a_revision);
    assert_eq!(open.window.get_identity_state(), 2);
    assert_eq!(open.comparison_generation.get(), 5);
    let mut fonts_only = a.clone();
    fonts_only.family = "Arial".into();
    fonts_only.font_size = 32.0;
    open.apply_appearance(&presentation(&fonts_only));
    assert_eq!(open.appearance.revision(), a_revision);
    assert_eq!(open.window.get_ui_font_family(), "Arial");
    assert_eq!(open.window.get_ui_font_size(), 32.0);
    assert_eq!(open.pending.borrow().len(), MAX_PENDING);

    let mut warning = choice("External outcome is unknown; inspect the files before retrying.");
    let Output::ChooseFiles(choice) = &mut warning else {
        unreachable!()
    };
    choice.external_outcome = Some(ExternalCompareChoiceOutcome::Unknown);
    choice.is_error = true;
    open.present_output(&warning);
    let paths = open.choice_paths.borrow().clone();
    let message = open.window.get_choice_message();
    let status = open.window.get_status();
    open.session.close();
    open.apply_appearance(&presentation(&b));
    assert_eq!(open.appearance.revision(), a_revision);
    assert_eq!(
        open.external_outcome.get(),
        Some(ExternalCompareChoiceOutcome::Unknown)
    );
    assert_eq!(open.window.get_choice_message(), message);
    assert_eq!(open.window.get_status(), status);
    assert!(open.window.get_choice_error());
    assert!(!open.window.get_choice_busy());
    assert_eq!(*open.choice_paths.borrow(), paths);
    assert_eq!(open.pending.borrow().len(), MAX_PENDING);
    open.close();
}

#[test]
fn font_resize_survives_input_backpressure_without_another_window_resize() {
    let native = platform();
    let runtime = runtime();
    let (policy, selected) = text_service(&runtime);
    let snapshot = policy.comparator_snapshot();
    let service = Service::new(runtime.handle().clone(), runtime.spawner())
        .with_comparator_policies(Arc::new(TextPolicySource {
            snapshot: snapshot.clone(),
            selected,
        }));
    let manager = ComparatorWindows::new_polled(service);
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("_build/qa")
        .join(format!("compare-font-resize-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let paths = [directory.join("left.txt"), directory.join("right.txt")];
    std::fs::write(&paths[0], "Shared\nBefore\n").unwrap();
    std::fs::write(&paths[1], "Shared\nAfter\n").unwrap();
    let appearance = ViewerAppearance {
        family: "monospace".into(),
        font_size: 12.0,
        foreground: [240, 240, 240, 255],
        background: [24, 28, 36, 255],
        selection: [60, 80, 120, 255],
        accent: [100, 150, 220, 255],
    };
    let window = manager
        .open(
            FileCompareLaunch {
                sources: paths
                    .iter()
                    .cloned()
                    .map(midnight_core::FileCompareSource::Focused)
                    .collect(),
                disabled_plugins: Arc::from([]),
                comparators: snapshot.preferences.clone(),
                comparator_policy: snapshot.id,
            },
            appearance.clone(),
            slint::LogicalSize::new(1100.0, 740.0),
            Rc::new(|_, _| None),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    let open = manager.windows.borrow()[0].clone();
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1100 * 740];
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 1100);
        });
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        slint::platform::update_timers_and_animations();
        draw(&mut pixels);
        if open.geometry_revision.get() > 0
            && window.get_frame_revision() == open.geometry_revision.get() as i32
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Initial comparison geometry did not settle: geometry={:?}, revision={}, frame={}, \
             pending={}, choosing={}, choice_error={}, status={:?}, choice={:?}",
            open.geometry.get(),
            open.geometry_revision.get(),
            window.get_frame_revision(),
            open.pending.borrow().len(),
            window.get_choosing_files(),
            window.get_choice_error(),
            window.get_status(),
            window.get_choice_message(),
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let size = window.window().size();
    let geometry = open.geometry.get();
    let revision = open.geometry_revision.get();
    let generation = open.comparison_generation.get();
    open.flush();
    for _ in 0..MAX_PENDING {
        assert!(open.send(Input::Difference { backwards: false }));
    }
    let mut ui = appearance.clone();
    ui.family = "Segoe UI".into();
    ui.font_size = 32.0;
    manager.apply_appearance(
        &ui,
        midnight_core::PreferenceTheme::Dark,
        midnight_core::UiDensity::Desktop,
        32,
        36,
    );
    draw(&mut pixels);
    assert!(window.get_content_height() < geometry.1 as f32);
    // Geometry replaces navigation at capacity and remains queued for the next batch.
    open.sync_geometry();
    let latest = open.geometry_revision.get();
    assert!(latest > revision);
    assert_ne!(open.geometry.get(), geometry);
    assert_eq!(window.get_geometry_revision(), latest as i32);
    let pending = open.pending.borrow();
    assert_eq!(pending.len(), MAX_PENDING);
    assert!(matches!(pending.last(), Some(Input::Resize(_, revision)) if *revision == latest));
    drop(pending);
    loop {
        slint::platform::update_timers_and_animations();
        draw(&mut pixels);
        if open.geometry_revision.get() > revision
            && window.get_frame_revision() == open.geometry_revision.get() as i32
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Font resize did not reach the window after the queue drained: geometry={:?}, revision={}, \
             frame={}, pending={}, choosing={}, choice_error={}, status={:?}, choice={:?}",
            open.geometry.get(),
            open.geometry_revision.get(),
            window.get_frame_revision(),
            open.pending.borrow().len(),
            window.get_choosing_files(),
            window.get_choice_error(),
            window.get_status(),
            window.get_choice_message(),
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        window.window().size(),
        size,
        "No second native resize is needed"
    );
    let accepted = open.geometry.get();
    assert_eq!(open.geometry_revision.get(), revision + 1);
    assert_eq!(
        accepted.0,
        (window.get_content_width() * window.window().scale_factor()).max(2.0) as u32
    );
    assert_eq!(
        accepted.1,
        (window.get_content_height() * window.window().scale_factor()).max(1.0) as u32
    );
    let surface = window.get_surface().size();
    assert_eq!((surface.width, surface.height), (accepted.0, accepted.1));
    assert_eq!(open.comparison_generation.get(), generation);
    manager.close_all();
    drop(open);
    drop(window);
    drop(manager);
    drop(runtime);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn completed_external_outcomes_survive_the_real_opening_palette_read() {
    use midnight_compare_engine::ExternalCompareLauncher;
    use midnight_core::{
        Cancellation, ExternalCompareOutcome, ExternalCompareProcess, FileComparatorProgram,
        FsError,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::SyncSender,
    };
    struct SubmitPayload(Arc<AtomicUsize>);
    impl Drop for SubmitPayload {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
            panic!("controlled secondary submit-payload drop panic");
        }
    }
    struct Launcher {
        unknown: bool,
        submits: AtomicUsize,
        payload_drops: Arc<AtomicUsize>,
        expected: [PathBuf; 2],
    }
    impl ExternalCompareLauncher for Launcher {
        fn submit(
            &self,
            _: ExternalCompareProcess,
            original_paths: [PathBuf; 2],
            _: Arc<dyn ExternalCompareSessionCredit>,
            _: Cancellation,
            reply: SyncSender<ExternalCompareOutcome>,
        ) -> Result<(), FsError> {
            assert_eq!(self.submits.fetch_add(1, Ordering::AcqRel), 0);
            assert_eq!(original_paths, self.expected);
            if self.unknown {
                std::panic::panic_any(SubmitPayload(self.payload_drops.clone()));
            }
            reply
                .try_send(ExternalCompareOutcome::Started {
                    pid: 4321,
                    tracking_warning: None,
                })
                .unwrap();
            Ok(())
        }
    }
    let _native = platform();
    for unknown in [false, true] {
        let runtime = runtime();
        let paths = paths();
        let snapshot = FileComparatorSnapshot {
            id: FileComparatorPolicyStore::default().prepare_id().unwrap(),
            preferences: Arc::new(FileComparatorPreferences {
                rules: vec![FileComparatorRule {
                    enabled: true,
                    mode: FilterMode::Masks,
                    expression: "*".into(),
                    case_sensitive: false,
                    target: FileComparatorTarget::External(FileComparatorProgram {
                        executable: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("never-executed.exe"),
                        arguments: vec!["{left}".into(), "{right}".into()],
                        working_directory: None,
                    }),
                }],
            }),
        };
        snapshot.preferences.validate().unwrap();
        let selected = SelectedPairs::default();
        let launcher = Arc::new(Launcher {
            unknown,
            submits: AtomicUsize::new(0),
            payload_drops: Arc::new(AtomicUsize::new(0)),
            expected: paths.clone(),
        });
        let captures = Arc::new(AtomicUsize::new(0));
        let observed = captures.clone();
        let service = Service::with_native_capture(
            runtime.handle().clone(),
            runtime.spawner(),
            Arc::new(move |_, _| {
                observed.fetch_add(1, Ordering::AcqRel);
                Err("configured External must not capture file contents".into())
            }),
        )
        .with_comparator_policies(Arc::new(TextPolicySource {
            snapshot: snapshot.clone(),
            selected: selected.clone(),
        }))
        .with_external_launcher(launcher.clone());
        let manager = ComparatorWindows::new_polled(service.clone());
        let mut app = ViewerAppearance {
            family: "Segoe UI".into(),
            font_size: 18.0,
            foreground: [48, 48, 48, 255],
            background: [255, 255, 255, 255],
            selection: [204, 204, 204, 255],
            accent: [69, 69, 69, 255],
        };
        manager.apply_appearance(
            &app,
            midnight_core::PreferenceTheme::Light,
            midnight_core::UiDensity::Desktop,
            18,
            24,
        );
        let mut document = app.clone();
        document.family = "monospace".into();
        document.font_size = 16.0;
        let outputs = RefCell::new(Vec::new());
        let before_getter = Cell::new(None);
        let window = manager
            .open_with_observer(
                FileCompareLaunch {
                    sources: paths
                        .iter()
                        .cloned()
                        .map(midnight_core::FileCompareSource::Focused)
                        .collect(),
                    disabled_plugins: Arc::from([]),
                    comparators: snapshot.preferences.clone(),
                    comparator_policy: snapshot.id,
                },
                document,
                slint::LogicalSize::new(1100.0, 740.0),
                Rc::new(|_, _| None),
                |session| {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    let mut terminal = false;
                    loop {
                        if let Some(output) = session.mailbox.take() {
                            terminal |= match &output {
                                Output::ExternalStarted { .. } => {
                                    assert!(!unknown);
                                    true
                                }
                                Output::ChooseFiles(choice) => {
                                    assert!(unknown);
                                    assert_eq!(
                                        choice.external_outcome,
                                        Some(ExternalCompareChoiceOutcome::Unknown)
                                    );
                                    true
                                }
                                Output::Status(_) => false,
                                Output::Frame { .. } => {
                                    panic!("External must not fall back to a built-in frame")
                                }
                            };
                            outputs.borrow_mut().push(output);
                        }
                        // Same opening colors never change the revision. Rejection here
                        // proves worker disconnect, unlike a queue-full send(false).
                        if terminal && session.restore_palette(&app).is_err() {
                            before_getter.set(Some(session.palette_revision()));
                            break;
                        }
                        assert!(
                            Instant::now() < deadline,
                            "External worker did not finish before the frontend getter"
                        );
                        std::thread::yield_now();
                    }
                },
            )
            .expect("A completed typed external outcome must survive opening construction")
            .upgrade()
            .unwrap();
        let open = manager.windows.borrow()[0].clone();
        let revision = before_getter.get().unwrap();
        assert_eq!(open.appearance.revision(), revision);
        assert_eq!(open.session.palette_revision(), revision);
        assert!(open.session.restore_palette(&app).is_err());
        assert_eq!(window.get_ui_font_family(), "Segoe UI");
        // The normal timer consumer receives genuine worker output AFTER the getter.
        for output in outputs.into_inner() {
            open.present_output(&output);
            service.retire(output);
        }
        if unknown {
            assert!(!open.closed.get());
            assert_eq!(
                open.external_outcome.get(),
                Some(ExternalCompareChoiceOutcome::Unknown)
            );
            assert_eq!(*open.choice_paths.borrow(), paths);
            let message = window.get_choice_message();
            let status = window.get_status();
            assert!(window.get_choice_error());
            app.family = "Arial".into();
            app.font_size = 32.0;
            app.foreground = [255; 4];
            app.background = [0, 0, 0, 255];
            app.selection = [255; 4];
            app.accent = [191, 255, 255, 255];
            manager.apply_appearance(
                &app,
                midnight_core::PreferenceTheme::HighContrastDark,
                midnight_core::UiDensity::Desktop,
                32,
                36,
            );
            assert_eq!(window.get_ui_font_family(), "Arial");
            assert_eq!(window.get_ui_font_size(), 32.0);
            assert_eq!(window.get_choice_message(), message);
            assert_eq!(window.get_status(), status);
            assert_eq!(
                open.external_outcome.get(),
                Some(ExternalCompareChoiceOutcome::Unknown)
            );
            assert_eq!(*open.choice_paths.borrow(), paths);
            assert_eq!(open.appearance.revision(), revision);
            assert_eq!(open.session.palette_revision(), revision);
            assert!(open.session.restore_palette(&app).is_err());
        } else {
            assert!(open.closed.get());
            assert!(!window.window().is_visible());
        }
        assert!(open.pending.borrow().is_empty());
        assert_eq!(launcher.submits.load(Ordering::Acquire), 1);
        assert_eq!(
            launcher.payload_drops.load(Ordering::Acquire),
            usize::from(unknown)
        );
        assert_eq!(captures.load(Ordering::Acquire), 0);
        assert_eq!(selected.lock().unwrap().as_slice(), &[paths]);
        manager.close_all();
    }
}
