//! Native synthetic viewer latency/read probe; no preferences, clipboard or application launch.
#[path = "performance_probe/cases.rs"]
mod cases;
#[path = "performance_probe/gpu_proof.rs"]
mod gpu_proof;
#[path = "performance_probe/image_proof.rs"]
mod image_proof;
#[path = "performance_probe/native_parent.rs"]
mod native_parent;
#[path = "performance_probe/opening_proof.rs"]
mod opening_proof;
#[path = "performance_probe/render_observer.rs"]
mod render_observer;
#[path = "performance_probe/scrollbar.rs"]
mod scrollbar;
#[path = "performance_probe/timeline.rs"]
mod timeline;
#[path = "performance_probe/wheel.rs"]
mod wheel;
use midnight_core::*;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use midnight_viewer_ui::{ViewerWindow, ViewerWindows};
use slint::{ComponentHandle, Model, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default, Debug)]
struct Counters {
    reads: AtomicU64,
    bytes: AtomicU64,
    validations: AtomicU64,
}
#[derive(Debug)]
struct Counted {
    file: CapturedFile,
    origin: Instant,
    counts: Arc<Counters>,
}
impl ContentSource for Counted {
    fn length(&self) -> u64 {
        self.file.length()
    }
    fn display_name(&self) -> String {
        self.file.display_name()
    }
    fn version(&self) -> Vec<u8> {
        self.file.stable_version()
    }
    fn validate(&self) -> Result<(), Error> {
        self.counts.validations.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let result = self.file.revalidate();
        eprintln!(
            "source_validate_ms={:.3} success={} at_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0,
            result.is_ok(),
            self.origin.elapsed().as_secs_f64() * 1000.0
        );
        result
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.counts.reads.fetch_add(1, Ordering::Relaxed);
        self.counts
            .bytes
            .fetch_add(u64::from(length), Ordering::Relaxed);
        let started = Instant::now();
        let result = self.file.read_at(offset, length);
        eprintln!(
            "source_read_ms={:.3} offset={offset} length={length} success={} at_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0,
            result.is_ok(),
            self.origin.elapsed().as_secs_f64() * 1000.0
        );
        result
    }
}
struct Probe {
    opening: Option<opening_proof::Probe>,
    scrollbar: Option<scrollbar::Probe>,
    wheel: Option<wheel::Probe>,
    stage: usize,
    round: usize,
    opened: Instant,
    first: bool,
    image_ready: Option<Instant>,
    image_proofs: Vec<image_proof::Capture>,
    image_settled: bool,
    pending: Option<(Instant, f64)>,
    latencies: Vec<f64>,
    rows: Vec<String>,
    error: Option<String>,
    last_status: String,
    proofs: Vec<gpu_proof::Capture>,
}
fn marker(window: &ViewerWindow) -> f64 {
    if window.get_image_mode() {
        f64::from(window.get_zoom_percent())
    } else {
        f64::from(window.get_position())
    }
}
fn active(window: &ViewerWindow, path: &std::path::Path) -> bool {
    window
        .get_caption()
        .contains(path.file_name().unwrap().to_string_lossy().as_ref())
        && !window.get_accessible_text().is_empty()
        && window.get_image_mode()
            == (path.extension().is_some_and(|extension| {
                matches!(
                    extension.to_str(),
                    Some("bmp" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "tif" | "tiff")
                )
            }))
}
fn presentation(window: &ViewerWindow) -> &'static str {
    if window.get_retained_raster() {
        "retained-raster"
    } else if window.get_tiled() {
        "tiled-image"
    } else {
        "legacy-viewport"
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let directory = PathBuf::from(args.next().ok_or(
        "usage: performance_probe fixture-directory report-file [--load] [--count] [--scrollbar|--wheel] [--d3d|--software] [--wheel-trace] [--extended-wheel|--real-wheel|--wrapped-tabs|--file=path] [--sustained] [--tail] [--gpu-proof] [--image-proof] [--opening-proof] [--native-parent]",
    )?);
    let report = PathBuf::from(args.next().ok_or("report-file missing")?);
    let flags: Vec<_> = args.collect();
    let load = flags.iter().any(|flag| flag == "--load");
    let instrumented = flags.iter().any(|flag| flag == "--count");
    let scrollbar = flags.iter().any(|flag| flag == "--scrollbar");
    let wheel = flags.iter().any(|flag| flag == "--wheel");
    let wheel_trace = flags.iter().any(|flag| flag == "--wheel-trace");
    let extended = flags.iter().any(|flag| flag == "--extended-wheel");
    let real_only = flags.iter().any(|flag| flag == "--real-wheel");
    let wrapped_tabs = flags.iter().any(|flag| flag == "--wrapped-tabs");
    let selected_files: Vec<_> = flags
        .iter()
        .filter_map(|flag| {
            flag.to_str()
                .and_then(|value| value.strip_prefix("--file="))
                .map(PathBuf::from)
        })
        .collect();
    if selected_files.len() > 1 {
        return Err("choose a single --file path per diagnostic run".into());
    }
    let selected_file = selected_files.first();
    let sustained = wrapped_tabs || flags.iter().any(|flag| flag == "--sustained");
    let tail = flags.iter().any(|flag| flag == "--tail");
    let gpu_proof = flags.iter().any(|flag| flag == "--gpu-proof");
    let image_proof = flags.iter().any(|flag| flag == "--image-proof");
    let opening_proof = flags.iter().any(|flag| flag == "--opening-proof");
    let native_parent = flags.iter().any(|flag| flag == "--native-parent");
    let d3d = flags.iter().any(|flag| flag == "--d3d");
    let software = flags.iter().any(|flag| flag == "--software");
    if flags.iter().any(|flag| {
        flag != "--load"
            && flag != "--count"
            && flag != "--scrollbar"
            && flag != "--wheel"
            && flag != "--wheel-trace"
            && flag != "--extended-wheel"
            && flag != "--real-wheel"
            && flag != "--wrapped-tabs"
            && flag != "--sustained"
            && flag != "--tail"
            && flag != "--gpu-proof"
            && flag != "--image-proof"
            && flag != "--opening-proof"
            && flag != "--native-parent"
            && flag != "--d3d"
            && flag != "--software"
            && !flag
                .to_str()
                .is_some_and(|value| value.starts_with("--file="))
    }) {
        return Err("unknown probe argument".into());
    }
    if opening_proof && (selected_file.is_none() || wheel || scrollbar || image_proof || gpu_proof)
    {
        return Err("--opening-proof requires one --file and no gesture/snapshot options".into());
    }
    if wheel_trace && !wheel && !scrollbar {
        return Err("input tracing requires --wheel or --scrollbar".into());
    }
    if (extended || real_only || sustained || tail || gpu_proof) && !wheel {
        return Err("wheel diagnostic options require --wheel".into());
    }
    if usize::from(extended)
        + usize::from(real_only)
        + usize::from(wrapped_tabs)
        + usize::from(selected_file.is_some())
        > 1
    {
        return Err(
            "choose one of --extended-wheel, --real-wheel, --wrapped-tabs, or --file=path".into(),
        );
    }
    let wheel_options = wheel::Options {
        sustained,
        tail,
        require_wrapped_handoff: wrapped_tabs,
    };
    if wheel && scrollbar {
        return Err("choose --wheel or --scrollbar".into());
    }
    if d3d && software {
        return Err("choose either --d3d or --software".into());
    }
    let timeline = timeline::Trace::new(wheel_trace);
    let after_render: render_observer::Callback = Rc::new(RefCell::new(None));
    let native_events = Rc::new(RefCell::new(Vec::new()));
    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    if d3d {
        // Match the Windows application's default renderer policy explicitly.
        let mut graphics = slint::wgpu_30::WGPUSettings::default();
        graphics.backends = slint::wgpu_30::wgpu::Backends::from_env()
            .unwrap_or(slint::wgpu_30::wgpu::Backends::DX12);
        eprintln!("app_wgpu_backends={:?}", graphics.backends);
        let mut flags = slint::wgpu_30::wgpu::InstanceFlags::from_build_config();
        flags.remove(
            slint::wgpu_30::wgpu::InstanceFlags::DEBUG
                | slint::wgpu_30::wgpu::InstanceFlags::VALIDATION,
        );
        flags = flags.with_env();
        flags.insert(slint::wgpu_30::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
        graphics.instance_flags = flags;
        selector = selector
            .renderer_name("femtovg-wgpu".into())
            .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(graphics));
    } else if software {
        selector = selector.renderer_name("software".into());
    }
    selector
        .with_winit_custom_application_handler(render_observer::Observer {
            callback: after_render.clone(),
            drawing: None,
            injected_wheel: wheel,
            native_events: native_events.clone(),
            timeline: timeline.clone(),
        })
        .select()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let paths = if let Some(path) = selected_file {
        if !std::fs::metadata(path)?.is_file() {
            return Err("--file must name an existing regular file".into());
        }
        vec![path.clone()]
    } else {
        cases::paths(&directory, extended, real_only, wrapped_tabs)?
    };
    let stage_count = paths.len();
    let counts: Arc<HashMap<PathBuf, Arc<Counters>>> = Arc::new(
        paths
            .iter()
            .map(|path| (path.clone(), Arc::default()))
            .collect(),
    );
    let capture_counts = counts.clone();
    let capture_started = Arc::new(OnceLock::new());
    let source_started = capture_started.clone();
    let capture: midnight_viewer_engine::NativeCapture = Arc::new(move |path, cancel| {
        if cancel.load(Ordering::Acquire) {
            return Err("closed".into());
        }
        let started = Instant::now();
        let origin: Instant = source_started.get().copied().unwrap_or(started);
        if instrumented {
            eprintln!(
                "source_capture_begin_ms={:.3}",
                origin.elapsed().as_secs_f64() * 1000.0
            );
        }
        let captured = CapturedFile::capture(path);
        if instrumented {
            eprintln!(
                "source_capture_ms={:.3} success={}",
                started.elapsed().as_secs_f64() * 1000.0,
                captured.is_ok()
            );
        }
        let file = captured.map_err(|e| e.to_string())?;
        if !instrumented {
            return Ok(file);
        }
        CapturedFile::from_source(Arc::new(Counted {
            file,
            origin,
            counts: capture_counts[path].clone(),
        }))
        .map_err(|e| e.to_string())
    });
    let service =
        midnight_viewer_engine::Service::with_native_capture(runtime.handle().clone(), capture);
    let manager = Rc::new(ViewerWindows::new(service));
    manager.set_preferences_listener(Rc::new(|change| {
        eprintln!("probe accepted preference: {change:?}");
    }));
    let stop = Arc::new(AtomicBool::new(false));
    struct Stop(Arc<AtomicBool>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _stop_guard = Stop(stop.clone());
    let mut jobs = vec![];
    if load {
        for seed in [7u64, 19] {
            let stop = stop.clone();
            jobs.push(runtime.spawn_blocking(move || {
                let mut x = seed;
                while !stop.load(Ordering::Acquire) {
                    for _ in 0..100000 {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        std::hint::black_box(x);
                    }
                }
            }));
        }
    }
    let setup_manager = manager.clone();
    let setup_paths = paths.clone();
    let setup_counts = counts.clone();
    let setup_timeline = timeline.clone();
    let exact_file = selected_file.is_some();
    let (timer, state) = native_parent::run(native_parent, move |placement| {
        let manager = setup_manager;
        let paths = setup_paths;
        let counts = setup_counts;
        let timeline = setup_timeline;
        let entries: Arc<[Entry]> = paths
            .iter()
            .enumerate()
            .map(|(index, path)| Entry::new(EntryId(index as u64), path.clone(), EntryKind::File))
            .collect();
        let start = Instant::now();
        let _ = capture_started.set(start);
        let window = manager.open_with_ready(
            ViewerLaunch {
                find: None,
                associations: Default::default(),
                tree: None,
                generated: None,
                preferences: Arc::new(ViewerPreferences {
                    wrap: wrapped_tabs,
                    ..Default::default()
                }),
                focused: paths[0].clone(),
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                archive: None,
                basic: false,
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: std::env::var("MIDNIGHT_VIEWER_PROBE_UI_FAMILY")
                    .unwrap_or_else(|_| "monospace".into()),
                font_size: std::env::var("MIDNIGHT_VIEWER_PROBE_UI_SIZE")
                    .ok()
                    .and_then(|value| value.parse::<f32>().ok())
                    .filter(|size| (8.0..=48.0).contains(size))
                    .unwrap_or(15.0),
                foreground: [225, 229, 236, 255],
                background: [25, 29, 37, 255],
                selection: [55, 86, 124, 255],
                accent: [133, 185, 244, 255],
            },
            placement,
            {
                let shortcuts = ShortcutMap::new(&[])?;
                Rc::new(move |chord, search| {
                    shortcuts
                        .resolve(
                            chord,
                            if search {
                                ShortcutContext::ViewerSearch
                            } else {
                                ShortcutContext::Viewer
                            },
                        )
                        .map(|action| action.id.clone())
                })
            },
            Rc::new(|_| {}),
            Box::new(|result| eprintln!("opening_result={result:?}")),
        )?;
        eprintln!(
            "viewer_construct_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        timeline.bind(window.clone());
        // This build enables FemtoVG/WGPU and software. Software explicitly lacks
        // transformations; inspect the initialized renderer rather than counting
        // a requested graphics API as evidence that it was selected successfully.
        let actual_gpu = i_slint_core::window::WindowInner::from_pub(
            window.upgrade().ok_or("window disappeared")?.window(),
        )
        .window_adapter()
        .renderer()
        .supports_transformations();
        let actual_renderer = if actual_gpu {
            "femtovg-wgpu"
        } else {
            "software"
        };
        eprintln!("requested_d3d={d3d} actual_renderer={actual_renderer}");
        if d3d && !actual_gpu {
            manager.close_all();
            return Err("requested D3D renderer fell back to software".into());
        }
        let state = Rc::new(RefCell::new(Probe {
            opening: opening_proof.then(opening_proof::Probe::default),
            scrollbar: scrollbar.then(scrollbar::Probe::default),
            wheel: wheel.then(|| wheel::Probe::new(timeline.clone(), wheel_options)),
            stage: 0,
            round: 0,
            opened: start,
            first: false,
            image_ready: None,
            image_proofs: Vec::new(),
            image_settled: false,
            pending: None,
            latencies: vec![],
            rows: vec![format!(
                "synthetic_cpu_load_jobs={} instrumented_source_reads={instrumented} requested_femtovg_d3d={d3d} actual_renderer={actual_renderer} native_parent={native_parent} timing=input_to_redraw_completed exact_file={} pointer_dispatch=slint-events extended={extended} real_only={real_only} wrapped_tabs={wrapped_tabs} sustained={sustained} tail={tail} gpu_proof={gpu_proof} image_proof={image_proof} snapshot_run_excluded_from_timing_comparison={snapshot_run}",
                if load { 2 } else { 0 },
                exact_file,
                snapshot_run = gpu_proof
                    || image_proof
                    || opening_proof
                    || std::env::var_os("MIDNIGHT_VIEWER_PROBE_SPATIAL_PROOF").is_some()
            )],
            error: None,
            last_status: "viewer constructed; no timer status yet".into(),
            proofs: Vec::new(),
        }));
        let notify = state.clone();
        let weak = window.clone();
        let native_counts = counts.clone();
        let native_paths = paths.clone();
        let presented = Rc::new(Cell::new(0_u64));
        let frames = presented.clone();
        let publication_timeline = timeline.clone();
        window
            .upgrade()
            .ok_or("window disappeared")?
            .on_frame_presented(move || {
                frames.set(frames.get().saturating_add(1));
                if frames.get() == 1 {
                    eprintln!(
                        "first_publication_ms={:.3}",
                        start.elapsed().as_secs_f64() * 1000.0
                    );
                }
                publication_timeline.record("ui_published", frames.get() as f64);
            });
        let rendered_frames = presented.clone();
        let before_state = state.clone();
        let before_window = window.clone();
        let before_frames = presented.clone();
        let before_paths = paths.clone();
        *after_render.borrow_mut() = Some(render_observer::Callbacks {
            window: window.clone(),
            before: Box::new(move || {
                let Some(window) = before_window.upgrade() else {
                    return;
                };
                let Ok(mut state) = before_state.try_borrow_mut() else {
                    return;
                };
                let active =
                    state.stage < stage_count && active(&window, &before_paths[state.stage]);
                if let Some(opening) = state.opening.as_mut() {
                    opening.beginning(&window, before_frames.get(), active);
                }
            }),
            after: Box::new(move |window_id, redraw| {
                let Some(window) = weak.upgrade() else {
                    return;
                };
                if window.window().with_winit_window(|native| native.id()) != Some(window_id) {
                    return;
                }
                let Ok(mut state) = notify.try_borrow_mut() else {
                    return;
                };
                if state.stage >= stage_count || !active(&window, &native_paths[state.stage]) {
                    return;
                }
                let opened_ms = state.opened.elapsed().as_secs_f64() * 1000.0;
                if let Some(opening) = state.opening.as_mut() {
                    match opening.rendered(&window, opened_ms, rendered_frames.get()) {
                        Ok(true) => {}
                        Ok(false) => return,
                        Err(error) => {
                            state.error = Some(error);
                            let _ = slint::quit_event_loop();
                            return;
                        }
                    }
                }
                if !state.first {
                    if wrapped_tabs && (!window.get_wrap_available() || !window.get_wrap_enabled())
                    {
                        state.error = Some("wrapped-tab case did not activate wrapping".into());
                        let _ = slint::quit_event_loop();
                        return;
                    }
                    let stage = state.stage;
                    let counts = &native_counts[&native_paths[stage]];
                    let elapsed = state.opened.elapsed().as_secs_f64() * 1000.0;
                    state.rows.push(format!(
                "first stage={stage} ms={elapsed:.3} source_reads={} source_bytes={} source_validations={} physical_window={:?} scale={} file={:?} source_mode={} markdown_mode={} retained={} tiled={} wrap_enabled={} wrap_available={} presentation={}",
                counts.reads.load(Ordering::Relaxed), counts.bytes.load(Ordering::Relaxed),
                counts.validations.load(Ordering::Relaxed), window.window().size(), window.window().scale_factor(), native_paths[stage], window.get_source_mode(), window.get_markdown_mode(), window.get_retained_raster(), window.get_tiled(), window.get_wrap_enabled(), window.get_wrap_available(), presentation(&window)
            ));
                    state.first = true;
                    if window.get_image_mode() {
                        state.image_ready = Some(Instant::now());
                        if image_proof {
                            match image_proof::capture(&window, format!("stage-{stage}-first")) {
                                Ok((capture, row)) => {
                                    state.image_proofs.push(capture);
                                    state.rows.push(row);
                                }
                                Err(error) => {
                                    state.error = Some(error);
                                    let _ = slint::quit_event_loop();
                                    return;
                                }
                            }
                        }
                    }
                }
                if window.get_image_mode() {
                    let canvas = [window.get_content_width(), window.get_content_height()];
                    let viewport = [
                        window.get_image_viewport_width() * window.get_image_pixel_scale(),
                        window.get_image_viewport_height() * window.get_image_pixel_scale(),
                    ];
                    let tiles: Vec<_> = window
                        .get_image_tiles()
                        .iter()
                        .map(|tile| {
                            [
                                (canvas[0] - viewport[0]) / 2.0
                                    + (tile.x - window.get_tile_offset_x()) * viewport[0],
                                (canvas[1] - viewport[1]) / 2.0
                                    + (tile.y - window.get_tile_offset_y()) * viewport[1],
                                tile.width * viewport[0],
                                tile.height * viewport[1],
                            ]
                        })
                        .collect();
                    let stage = state.stage;
                    let round = state.round;
                    let elapsed = state.opened.elapsed().as_secs_f64() * 1000.0;
                    state.rows.push(format!(
                "image_redraw stage={stage} round={round} ms={elapsed:.3} zoom={} canvas={canvas:?} viewport={viewport:?} content_origin={:?} tiles={tiles:?} uploads={} status={}",
                window.get_zoom_percent(),
                [window.get_content_left(), window.get_content_top()],
                window.get_tile_upload_count(), window.get_status(),
            ));
                }
                if let Some(gesture) = state.wheel.as_mut() {
                    gesture.rendered(&window, redraw);
                } else if let Some(drag) = state.scrollbar.as_mut() {
                    drag.rendered(&window, rendered_frames.get(), redraw);
                } else if let Some((sent, previous)) = state.pending
                    && marker(&window) != previous
                {
                    let latency = sent.elapsed().as_secs_f64() * 1000.0;
                    state.latencies.push(latency);
                    if window.get_image_mode() {
                        let round = state.round;
                        state.rows.push(format!(
                        "image_zoom round={round} previous_zoom={previous} zoom={} ms={latency:.3}",
                        window.get_zoom_percent(),
                    ));
                    }
                    state.pending = None;
                    state.round += 1;
                }
            }),
        });
        let timer = slint::Timer::default();
        let tick = state.clone();
        let weak = window;
        let origin = start;
        let timer_timeline = timeline.clone();
        let last_tick = Cell::new(Instant::now());
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(4),
            move || {
                let gap = last_tick.replace(Instant::now()).elapsed().as_secs_f64() * 1000.0;
                if gap > 30.0 {
                    timer_timeline.record("timer_gap", gap);
                }
                let Some(window) = weak.upgrade() else {
                    eprintln!("probe_exit viewer_window_disappeared=true");
                    let _ = slint::quit_event_loop();
                    return;
                };
                let Ok(mut state) = tick.try_borrow_mut() else {
                    return;
                };
                state.last_status = window.get_status().to_string();
                if origin.elapsed() > Duration::from_secs(300) {
                    state.error = Some(format!(
                        "timeout stage={} round={} status={}",
                        state.stage,
                        state.round,
                        window.get_status()
                    ));
                    let _ = slint::quit_event_loop();
                    return;
                }
                // Reaching a clamped page edge can legitimately paint the same
                // scrollbar position. Do not report that as a measured frame or
                // wait indefinitely for an impossible position acknowledgement.
                if let Some((sent, previous)) = state.pending
                    && sent.elapsed() > Duration::from_secs(2)
                {
                    let stage = state.stage;
                    let round = state.round;
                    state.rows.push(format!(
                    "unchanged_position stage={stage} round={round} position={previous} status={}",
                    window.get_status()
                ));
                    state.pending = None;
                    if window.get_image_mode() {
                        state.error = Some(format!("image zoom timed out at round {round}"));
                        let _ = slint::quit_event_loop();
                        return;
                    }
                }
                if state.stage >= stage_count || !state.first || state.pending.is_some() {
                    return;
                }
                if let Some(opening) = &state.opening {
                    if opening.settled() {
                        let _ = slint::quit_event_loop();
                    }
                    return;
                }
                if window.get_image_mode()
                    && state
                        .image_ready
                        .is_some_and(|ready| ready.elapsed() < Duration::from_millis(600))
                {
                    return;
                }
                let stage = state.stage;
                if image_proof && window.get_image_mode() && !state.image_settled {
                    match image_proof::capture(&window, format!("stage-{stage}-settled")) {
                        Ok((capture, row)) => {
                            state.image_proofs.push(capture);
                            state.rows.push(row);
                            state.image_settled = true;
                        }
                        Err(error) => {
                            state.error = Some(error);
                            let _ = slint::quit_event_loop();
                            return;
                        }
                    }
                }
                if let Some(drag) = state.scrollbar.as_mut() {
                    match drag.tick(&window, window.get_image_mode(), presented.get(), stage) {
                        Ok(Some(rows)) => {
                            state.rows.extend(rows);
                            state.round = 1;
                        }
                        Ok(None) => return,
                        Err(error) => {
                            state.error = Some(error);
                            let _ = slint::quit_event_loop();
                            return;
                        }
                    }
                }
                if let Some(gesture) = state.wheel.as_mut() {
                    match gesture.tick(&window, window.get_image_mode(), stage) {
                        Ok(Some(rows)) => {
                            state.rows.extend(rows);
                            state.round = 1;
                        }
                        Ok(None) => return,
                        Err(error) => {
                            let trace = gesture.trace(stage);
                            state.rows.extend(trace);
                            state
                                .rows
                                .push(format!("wheel_failure stage={stage}: {error}"));
                            state.error.get_or_insert(error);
                            // Keep collecting later modes while retaining a failing exit status.
                            state.round = 1;
                        }
                    }
                }
                let rounds = if scrollbar || wheel {
                    1
                } else if window.get_image_mode() {
                    12
                } else {
                    60
                };
                if state.round >= rounds {
                    let stage = state.stage;
                    if !scrollbar && !wheel {
                        let mut times = std::mem::take(&mut state.latencies);
                        times.sort_by(f64::total_cmp);
                        state.rows.push(format!(
                            "frames stage={stage} n={} median_ms={:.3} p95_ms={:.3} max_ms={:.3}",
                            times.len(),
                            times[times.len() / 2],
                            times[(times.len() * 95 / 100).min(times.len() - 1)],
                            times[times.len() - 1]
                        ));
                    }
                    if image_proof && window.get_image_mode() {
                        match image_proof::capture(&window, format!("stage-{stage}-final")) {
                            Ok((capture, row)) => {
                                state.image_proofs.push(capture);
                                state.rows.push(row);
                            }
                            Err(error) => {
                                state.error = Some(error);
                                let _ = slint::quit_event_loop();
                                return;
                            }
                        }
                    }
                    if gpu_proof {
                        match gpu_proof::capture(&window, stage) {
                            Ok(proof) => {
                                state.rows.extend(proof.rows);
                                state.proofs.extend(proof.captures);
                                if let Some(error) = proof.error {
                                    state.error.get_or_insert(error);
                                }
                            }
                            Err(error) => {
                                state
                                    .rows
                                    .push(format!("gpu_proof_failure stage={stage}: {error}"));
                                state.error.get_or_insert(error);
                            }
                        }
                    }
                    state.stage += 1;
                    timer_timeline.stage(state.stage);
                    state.round = 0;
                    state.first = false;
                    state.image_ready = None;
                    state.image_settled = false;
                    state.opened = Instant::now();
                    state.scrollbar = scrollbar.then(scrollbar::Probe::default);
                    state.wheel =
                        wheel.then(|| wheel::Probe::new(timer_timeline.clone(), wheel_options));
                    if state.stage == stage_count {
                        let _ = slint::quit_event_loop();
                    } else {
                        window.invoke_focus_content();
                        window
                            .window()
                            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                                text: " ".into(),
                            });
                        window
                            .window()
                            .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                                text: " ".into(),
                            });
                    }
                    return;
                }
                state.pending = Some((Instant::now(), marker(&window)));
                if window.get_image_mode() {
                    window.invoke_image_command(if state.round < 6 { 2 } else { 3 });
                } else {
                    window.invoke_scroll(0.0, 60.0);
                }
            },
        );
        Ok((timer, state))
    })?;
    timer.stop();
    manager.close_all();
    stop.store(true, Ordering::Release);
    for job in jobs {
        runtime.block_on(job)?;
    }
    let mut state = state.borrow_mut();
    for (index, path) in paths.iter().enumerate() {
        let counts = &counts[path];
        state.rows.push(format!(
            "final stage={index} reads={} bytes={} validations={}",
            counts.reads.load(Ordering::Relaxed),
            counts.bytes.load(Ordering::Relaxed),
            counts.validations.load(Ordering::Relaxed)
        ));
    }
    let completed_openings = state
        .rows
        .iter()
        .filter(|row| row.starts_with("first "))
        .count();
    if completed_openings < paths.len() && state.error.is_none() {
        state.error = Some(format!(
            "event loop ended before image/content publication: completed={completed_openings}/{} \
             stage={} round={} status={}",
            paths.len(),
            state.stage,
            state.round,
            state.last_status,
        ));
    }
    if let Some(error) = state.error.clone() {
        state.rows.push(format!("ERROR {error}"));
    }
    if let Some(opening) = state.opening.take() {
        state.rows.extend(opening.rows());
        opening.write(&report.with_extension("opening"))?;
    }
    state.rows.extend(native_events.borrow_mut().drain(..));
    state.rows.extend(timeline.rows());
    gpu_proof::write(
        &report.with_extension("pixels"),
        std::mem::take(&mut state.proofs),
    )?;
    image_proof::write(
        &report.with_extension("images"),
        std::mem::take(&mut state.image_proofs),
    )?;
    let output = state.rows.join("\n") + "\n";
    eprintln!("{output}");
    runtime.block_on(runtime.spawn_blocking(move || std::fs::write(report, output)))??;
    if let Some(error) = state.error.take() {
        return Err(error.into());
    }
    Ok(())
}
