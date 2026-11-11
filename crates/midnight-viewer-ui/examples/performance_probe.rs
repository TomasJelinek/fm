//! Native synthetic viewer latency/read probe; no preferences, clipboard or application launch.
#[path = "performance_probe/render_observer.rs"]
mod render_observer;
#[path = "performance_probe/scrollbar.rs"]
mod scrollbar;
#[path = "performance_probe/wheel.rs"]
mod wheel;
use midnight_core::*;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use midnight_viewer_ui::{ViewerWindow, ViewerWindows};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
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
        self.file.revalidate()
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.counts.reads.fetch_add(1, Ordering::Relaxed);
        self.counts
            .bytes
            .fetch_add(u64::from(length), Ordering::Relaxed);
        self.file.read_at(offset, length)
    }
}
struct Probe {
    scrollbar: Option<scrollbar::Probe>,
    wheel: Option<wheel::Probe>,
    stage: usize,
    round: usize,
    opened: Instant,
    first: bool,
    pending: Option<(Instant, f64)>,
    latencies: Vec<f64>,
    rows: Vec<String>,
    error: Option<String>,
}
fn marker(window: &ViewerWindow, stage: usize) -> f64 {
    if stage == 2 {
        f64::from(window.get_zoom_percent())
    } else {
        f64::from(window.get_position())
    }
}
fn active(window: &ViewerWindow, stage: usize) -> bool {
    let caption = window.get_caption();
    caption.contains(["plain.txt", "source.rs", "image.bmp"][stage])
        && !window.get_accessible_text().is_empty()
        && window.get_image_mode() == (stage == 2)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let directory = PathBuf::from(args.next().ok_or(
        "usage: performance_probe fixture-directory report-file [--load] [--count] [--scrollbar|--wheel] [--d3d|--software]",
    )?);
    let report = PathBuf::from(args.next().ok_or("report-file missing")?);
    let flags: Vec<_> = args.collect();
    let load = flags.iter().any(|flag| flag == "--load");
    let instrumented = flags.iter().any(|flag| flag == "--count");
    let scrollbar = flags.iter().any(|flag| flag == "--scrollbar");
    let wheel = flags.iter().any(|flag| flag == "--wheel");
    let d3d = flags.iter().any(|flag| flag == "--d3d");
    let software = flags.iter().any(|flag| flag == "--software");
    if flags.iter().any(|flag| {
        flag != "--load"
            && flag != "--count"
            && flag != "--scrollbar"
            && flag != "--wheel"
            && flag != "--d3d"
            && flag != "--software"
    }) {
        return Err("unknown probe argument".into());
    }
    if wheel && scrollbar {
        return Err("choose --wheel or --scrollbar".into());
    }
    if d3d && software {
        return Err("choose either --d3d or --software".into());
    }
    let after_render: render_observer::Callback = Rc::new(RefCell::new(None));
    let native_events = Rc::new(RefCell::new(Vec::new()));
    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    if d3d {
        // Match the Windows application's default renderer policy explicitly.
        let mut graphics = slint::wgpu_30::WGPUSettings::default();
        graphics.backends = slint::wgpu_30::wgpu::Backends::DX12;
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
        })
        .select()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let paths = ["plain.txt", "source.rs", "image.bmp"].map(|name| directory.join(name));
    let counts: Arc<HashMap<PathBuf, Arc<Counters>>> = Arc::new(
        paths
            .iter()
            .map(|path| (path.clone(), Arc::default()))
            .collect(),
    );
    let capture_counts = counts.clone();
    let capture: midnight_viewer_engine::NativeCapture = Arc::new(move |path, cancel| {
        if cancel.load(Ordering::Acquire) {
            return Err("closed".into());
        }
        let file = CapturedFile::capture(path).map_err(|e| e.to_string())?;
        if !instrumented {
            return Ok(file);
        }
        CapturedFile::from_source(Arc::new(Counted {
            file,
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
    let entries: Arc<[Entry]> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| Entry::new(EntryId(index as u64), path.clone(), EntryKind::File))
        .collect();
    let start = Instant::now();
    let window = manager.open(
        ViewerLaunch {
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::new(Default::default()),
            focused: paths[0].clone(),
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            archive: None,
            basic: false,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 15.0,
            foreground: [225, 229, 236, 255],
            background: [25, 29, 37, 255],
            selection: [55, 86, 124, 255],
            accent: [133, 185, 244, 255],
        },
        slint::LogicalSize::new(1000.0, 700.0),
        Rc::new(|chord, search| {
            (!search && chord.key == "Space").then(|| "viewer.next-file".to_owned())
        }),
        Rc::new(|_| {}),
    )?;
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
        scrollbar: scrollbar.then(scrollbar::Probe::default),
        wheel: wheel.then(wheel::Probe::default),
        stage: 0,
        round: 0,
        opened: start,
        first: false,
        pending: None,
        latencies: vec![],
        rows: vec![format!(
            "synthetic_cpu_load_jobs={} instrumented_source_reads={instrumented} requested_femtovg_d3d={d3d} actual_renderer={actual_renderer} timing=input_to_redraw_completed",
            if load { 2 } else { 0 }
        )],
        error: None,
    }));
    let notify = state.clone();
    let weak = window.clone();
    let native_counts = counts.clone();
    let native_paths = paths.clone();
    let presented = Rc::new(Cell::new(0_u64));
    let frames = presented.clone();
    window
        .upgrade()
        .ok_or("window disappeared")?
        .on_frame_presented(move || {
            frames.set(frames.get().saturating_add(1));
        });
    let rendered_frames = presented.clone();
    *after_render.borrow_mut() = Some(Box::new(move |redraw| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let Ok(mut state) = notify.try_borrow_mut() else {
            return;
        };
        if state.stage >= 3 || !active(&window, state.stage) {
            return;
        }
        if !state.first {
            let stage = state.stage;
            let counts = &native_counts[&native_paths[stage]];
            let elapsed = state.opened.elapsed().as_secs_f64() * 1000.0;
            state.rows.push(format!(
                "first stage={stage} ms={elapsed:.3} source_reads={} source_bytes={} source_validations={} physical_window={:?} scale={}",
                counts.reads.load(Ordering::Relaxed), counts.bytes.load(Ordering::Relaxed),
                counts.validations.load(Ordering::Relaxed), window.window().size(), window.window().scale_factor()
            ));
            state.first = true;
        }
        if let Some(gesture) = state.wheel.as_mut() {
            gesture.rendered(&window, redraw);
        } else if let Some(drag) = state.scrollbar.as_mut() {
            drag.rendered(&window, rendered_frames.get(), redraw);
        } else if let Some((sent, previous)) = state.pending
            && marker(&window, state.stage) != previous
        {
            state.latencies.push(sent.elapsed().as_secs_f64() * 1000.0);
            state.pending = None;
            state.round += 1;
        }
    }));
    let timer = slint::Timer::default();
    let tick = state.clone();
    let weak = window;
    let origin = start;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(4),
        move || {
            let Some(window) = weak.upgrade() else {
                let _ = slint::quit_event_loop();
                return;
            };
            let Ok(mut state) = tick.try_borrow_mut() else {
                return;
            };
            if origin.elapsed() > Duration::from_secs(150) {
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
            }
            if state.stage >= 3 || !state.first || state.pending.is_some() {
                return;
            }
            let stage = state.stage;
            if let Some(drag) = state.scrollbar.as_mut() {
                match drag.tick(&window, stage == 2, presented.get(), stage) {
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
                match gesture.tick(&window, stage == 2, stage) {
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
            } else if state.stage == 2 {
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
                state.stage += 1;
                state.round = 0;
                state.first = false;
                state.opened = Instant::now();
                state.scrollbar = scrollbar.then(scrollbar::Probe::default);
                state.wheel = wheel.then(wheel::Probe::default);
                if state.stage == 3 {
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
            state.pending = Some((Instant::now(), marker(&window, state.stage)));
            if state.stage == 2 {
                window.invoke_image_command(if state.round % 2 == 0 { 1 } else { 0 });
            } else {
                window.invoke_scroll(0.0, 60.0);
            }
        },
    );
    slint::run_event_loop()?;
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
    if let Some(error) = state.error.clone() {
        state.rows.push(format!("ERROR {error}"));
    }
    state.rows.extend(native_events.borrow_mut().drain(..));
    let output = state.rows.join("\n") + "\n";
    eprintln!("{output}");
    runtime.block_on(runtime.spawn_blocking(move || std::fs::write(report, output)))??;
    if let Some(error) = state.error.take() {
        return Err(error.into());
    }
    Ok(())
}
