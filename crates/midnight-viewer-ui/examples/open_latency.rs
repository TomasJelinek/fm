//! Native opening latency: time from `ViewerWindows::open_with_ready` to the accepted
//! first content frame, for each file in turn (each opened `REPEAT` times in one process).
//! Deploy the executable beside a `viewers` directory. Set MIDNIGHT_VIEWER_OPEN_TRACE=1 in
//! debug builds for the per-phase opening timestamps. Results and method:
//! docs/verification/2026-11-20-viewer-speed.md.
//! With `--together FILE FILE [FILE...]`, leave every viewer surface open,
//! placing successive windows on successive monitors when more than one is available.
//! Set `MIDNIGHT_PROBE_PARENT=1` to keep a native parent visible as in the file manager;
//! each viewer then takes the parent's captured bounds, as the file manager's take the main
//! window's. `MIDNIGHT_PROBE_REPEAT=N` opens each file N times (default 3); the first
//! opening in a process is the cold one. `MIDNIGHT_PROBE_SETTLE_MS=N` waits N ms after
//! startup before the first opening (default: the first 250 ms tick), as a user who presses
//! F3 a moment after the window appeared. `MIDNIGHT_PROBE_PAUSE_MS=N` waits N ms after each
//! opening's first frame before the next opening. With `--features acceptance-seams`,
//! `MIDNIGHT_PROBE_SCROLL_MS=N` turns the wheel in each opened viewer every 16 ms for N ms
//! after its first frame and reports the longest gap between those turns (UI-thread stalls). Build with `--features open-trace` and
//! set MIDNIGHT_VIEWER_OPEN_TRACE=1 for per-phase timestamps in optimized builds. Startup
//! phases per viewer kind: docs/verification/2026-11-26-viewer-startup.md.
#[cfg(target_os = "linux")]
#[path = "support/native_html_linux.rs"]
mod native_linux;
#[path = "performance_probe/native_parent.rs"]
mod native_parent;
#[cfg(windows)]
#[path = "../../midnight-ui/src/window_gpu_windows.rs"]
mod native_windows;
#[path = "support/resident.rs"]
mod resident;

use midnight_core::*;
use midnight_viewer_ui::ViewerWindows;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const REPEAT: usize = 3;

fn environment_number(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
}

fn launch(path: &std::path::Path) -> ViewerLaunch {
    let entries: Arc<[Entry]> = [Entry::new(EntryId(0), path.to_owned(), EntryKind::File)].into();
    ViewerLaunch {
        find: None,
        associations: Default::default(),
        tree: None,
        generated: None,
        preferences: Arc::new(Default::default()),
        archive: None,
        focused: path.to_owned(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        basic: false,
        disabled_plugins: Arc::from([]),
    }
}

/// The application's Linux renderer: winit with femtovg-wgpu. With
/// MIDNIGHT_PROBE_SHARED_WGPU=1, every window reuses one wgpu device.
fn select_renderer() -> Result<(), Box<dyn std::error::Error>> {
    // MIDNIGHT_PROBE_RENDERER=software measures without GPU presentation, which
    // blocks on vertical blanking (and waits about a second while the monitor is off).
    let renderer =
        std::env::var("MIDNIGHT_PROBE_RENDERER").unwrap_or_else(|_| "femtovg-wgpu".into());
    let mut selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.clone());
    #[cfg(windows)]
    if renderer == "femtovg-wgpu" {
        let share = std::env::var_os("MIDNIGHT_SHARED_WGPU").is_none_or(|value| value != "0");
        let started = Instant::now();
        let config = native_windows::windows_gpu_configuration(share);
        let mode = if matches!(config, slint::wgpu_30::WGPUConfiguration::Manual { .. }) {
            "shared"
        } else {
            "automatic"
        };
        eprintln!(
            "Windows wgpu configuration: {mode} ({:?})",
            started.elapsed()
        );
        selector = selector.require_wgpu_30(config);
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("MIDNIGHT_PROBE_SHARED_WGPU").is_some_and(|value| value == "1") {
        let started = Instant::now();
        let configuration = shared_wgpu()?;
        eprintln!("shared wgpu device: {:?}", started.elapsed());
        selector = selector.require_wgpu_30(configuration);
    }
    selector.select()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn shared_wgpu() -> Result<slint::wgpu_30::WGPUConfiguration, Box<dyn std::error::Error>> {
    use slint::wgpu_30::wgpu;
    fn ready<F: std::future::Future>(future: F) -> F::Output {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(future).poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("native wgpu setup completes synchronously"),
        }
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = ready(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    // Slint's own default device: every non-experimental adapter feature and limit.
    let (device, queue) = ready(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: adapter.features() - wgpu::Features::all_experimental_mask(),
        required_limits: adapter.limits(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::default(),
    }))?;
    // Slint drops its copy in a thread-local destructor at exit; keep the last
    // reference here so that drop never tears the device down there.
    std::mem::forget((
        instance.clone(),
        adapter.clone(),
        device.clone(),
        queue.clone(),
    ));
    Ok(slint::wgpu_30::WGPUConfiguration::Manual {
        instance,
        adapter,
        device,
        queue,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let process = Instant::now();
    let mut args = std::env::args_os().skip(1).peekable();
    let together = args.next_if(|arg| arg == "--together").is_some();
    let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if paths.is_empty() || together && paths.len() < 2 {
        return Err("usage: open_latency FILE... | --together FILE FILE [FILE...]".into());
    }
    let repeat = if together {
        1
    } else {
        environment_number("MIDNIGHT_PROBE_REPEAT").map_or(REPEAT, |value| value.max(1) as usize)
    };
    let settle = Duration::from_millis(environment_number("MIDNIGHT_PROBE_SETTLE_MS").unwrap_or(0));
    let pause = Duration::from_millis(environment_number("MIDNIGHT_PROBE_PAUSE_MS").unwrap_or(0));
    let finished = Rc::new(Cell::new(None::<Instant>));
    select_renderer()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let runtime = resident::ResidentRuntime::new(runtime)?;
    // MIDNIGHT_PROBE_NATIVE_CAPTURE=1 captures through the installed native filesystem
    // provider with directory resources, as the application does; the default is the
    // engine's built-in capture.
    #[cfg(target_os = "linux")]
    let service =
        if std::env::var_os("MIDNIGHT_PROBE_NATIVE_CAPTURE").is_some_and(|value| value == "1") {
            native_linux::service(runtime.handle().clone(), runtime.spawner())
        } else {
            midnight_viewer_engine::Service::new(runtime.handle().clone(), runtime.spawner())
        };
    #[cfg(not(target_os = "linux"))]
    let service = midnight_viewer_engine::Service::new(runtime.handle().clone(), runtime.spawner());
    runtime.track_spare(&service);
    // Like the application once its settings are read: prepare an image worker spare.
    service.prepare_image_workers(Some(Arc::from([])), Arc::default());
    let manager = Rc::new(ViewerWindows::new(service));
    // Like the application: the first viewer component is built while idle.
    manager.prepare_spare_window();
    let shortcuts = ShortcutMap::new(&[])?;
    let keys: midnight_viewer_ui::KeyResolver =
        Rc::new(move |chord: &ShortcutChord, _search: bool| {
            shortcuts
                .resolve(chord, ShortcutContext::Viewer)
                .map(|action| action.id.clone())
        });
    let failure = Rc::new(RefCell::new(None::<String>));
    // (index of the next opening, opened-at, completed-at)
    let next = Rc::new(Cell::new(0usize));
    let started = Rc::new(Cell::new(None::<Instant>));
    let done = Rc::new(Cell::new(None::<Result<Duration, String>>));
    let windows = RefCell::new(Vec::<slint::Weak<midnight_viewer_ui::ViewerWindow>>::new());
    // With a native parent, each viewer takes the parent's captured bounds, monitor scale
    // and position, as the file manager's viewers take the main window's.
    let placement = Rc::new(RefCell::new(None::<midnight_ui_input::WindowPlacement>));
    let captured = placement.clone();
    // Wheel turns after the first frame, while the scrolled viewer stays open.
    let scroll_for = Duration::from_millis(
        environment_number("MIDNIGHT_PROBE_SCROLL_MS")
            .filter(|_| cfg!(feature = "acceptance-seams"))
            .unwrap_or(0),
    );
    let scrolling = Rc::new(Cell::new(false));
    let after_scroll = Cell::new(false);
    let scroll_timer = RefCell::new(None::<slint::Timer>);
    let timer = slint::Timer::default();
    let tick = {
        let manager = manager.clone();
        let failure = failure.clone();
        let done = done.clone();
        move || {
            if let Some(start) = started.get() {
                if scrolling.get() {
                    return;
                }
                if after_scroll.take() {
                    // Dropped outside its own callback.
                    scroll_timer.borrow_mut().take();
                    manager.close_all();
                    windows.borrow_mut().clear();
                    started.set(None);
                    finished.set(Some(Instant::now()));
                    return;
                }
                let Some(result) = done.take() else {
                    if start.elapsed() > Duration::from_secs(60) {
                        *failure.borrow_mut() = Some("opening timed out".into());
                        let _ = slint::quit_event_loop();
                    }
                    return;
                };
                let index = next.get() - 1;
                let path = &paths[index / repeat];
                // The status line shows errors that follow the first frame, such as a
                // failed resource release; ticks are 250 ms apart, so this is after one.
                let status = windows
                    .borrow()
                    .last()
                    .and_then(|window| window.upgrade())
                    .map(|window| window.get_status().to_string())
                    .unwrap_or_default();
                match result {
                    Ok(elapsed) => println!(
                        "open file={} attempt={} first_content_ms={:.1} process_ms={} status={status:?}",
                        path.display(),
                        index % repeat,
                        elapsed.as_secs_f64() * 1000.0,
                        process.elapsed().as_millis()
                    ),
                    Err(error) => {
                        *failure.borrow_mut() = Some(format!("{}: {error}", path.display()));
                        let _ = slint::quit_event_loop();
                        return;
                    }
                }
                if !scroll_for.is_zero()
                    && let Some(window) = windows.borrow().last().and_then(|w| w.upgrade())
                {
                    scrolling.set(true);
                    after_scroll.set(true);
                    *scroll_timer.borrow_mut() =
                        scroll(&manager, window, scroll_for, scrolling.clone());
                    return;
                }
                if !together {
                    manager.close_all();
                    windows.borrow_mut().clear();
                }
                started.set(None);
                finished.set(Some(Instant::now()));
                return;
            }
            let index = next.get();
            if index == 0 && process.elapsed() < settle
                || finished.get().is_some_and(|at| at.elapsed() < pause)
            {
                return;
            }
            if index >= paths.len() * repeat {
                if together {
                    let visible = windows
                        .borrow()
                        .iter()
                        .filter_map(slint::Weak::upgrade)
                        .filter(|window| window.window().is_visible())
                        .count();
                    if visible != paths.len() {
                        *failure.borrow_mut() =
                            Some(format!("only {visible} windows still visible"));
                    } else {
                        println!("concurrent native windows with first content: {visible}");
                    }
                }
                let _ = slint::quit_event_loop();
                return;
            }
            next.set(index + 1);
            let start = Instant::now();
            if let Some(origin) = midnight_viewer_engine::open_trace::origin() {
                eprintln!(
                    "viewer_phase at_us={} phase=probe_open_request attempt={}",
                    origin.elapsed().as_micros(),
                    index % repeat
                );
            }
            started.set(Some(start));
            let finished = done.clone();
            let ready = Box::new(move |result: Result<(), String>| {
                finished.set(Some(result.map(|()| start.elapsed())));
            });
            let placement = captured.borrow().clone().unwrap_or_else(|| {
                midnight_ui_input::WindowPlacement::from(slint::LogicalSize::new(1000.0, 700.0))
            });
            let opened = manager.open_with_ready(
                launch(&paths[index / repeat]),
                ViewerAppearance {
                    family: "sans-serif".into(),
                    font_size: 13.0,
                    foreground: [225, 229, 236, 255],
                    background: [25, 29, 37, 255],
                    selection: [55, 86, 124, 255],
                    selection_text: [0; 4],
                    accent: [133, 185, 244, 255],
                },
                placement,
                keys.clone(),
                Rc::new(|_| {}),
                ready,
            );
            match opened {
                Ok(window) => {
                    if together && let Some(component) = window.upgrade() {
                        component.window().with_winit_window(|native| {
                            let monitors: Vec<_> = native.available_monitors().collect();
                            if !monitors.is_empty() {
                                let monitor = &monitors[index % monitors.len()];
                                native.set_outer_position(monitor.position());
                            }
                        });
                    }
                    windows.borrow_mut().push(window);
                }
                Err(error) => done.set(Some(Err(error))),
            }
        }
    };
    // Settle between openings like a user would; timing excludes this pause.
    let timer = if std::env::var_os("MIDNIGHT_PROBE_PARENT").is_some_and(|value| value == "1") {
        native_parent::run(true, move |parent| {
            *placement.borrow_mut() = Some(parent);
            timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), tick);
            Ok(timer)
        })?
    } else {
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), tick);
        // Closing each viewer must not end the loop between openings.
        slint::run_event_loop_until_quit()?;
        timer
    };
    timer.stop();
    manager.close_all();
    if let Some(message) = failure.borrow_mut().take() {
        return Err(message.into());
    }
    Ok(())
}

/// Turn the wheel down in `window` every 16 ms for `duration`, then report the longest
/// gap between turns: a UI-thread stall delays the timer that turns it. The returned timer
/// turns until it is dropped; it stops turning after `duration`.
#[cfg(feature = "acceptance-seams")]
fn scroll(
    manager: &Rc<ViewerWindows>,
    window: midnight_viewer_ui::ViewerWindow,
    duration: Duration,
    scrolling: Rc<Cell<bool>>,
) -> Option<slint::Timer> {
    use midnight_viewer_ui::acceptance_seams::{NativeWheel, wheel};
    let manager = Rc::downgrade(manager);
    // Weak: Slint drops a timer's callback inside its timer list when the timer is dropped
    // while it waits in the same expiry batch (here, by the tick that closes the viewer).
    // A callback owning the window's last handle would drop the component there, and its
    // accessibility teardown starts a timer while the list is borrowed: a panic.
    let window = window.as_weak();
    let begun = Instant::now();
    let last = Cell::new(begun);
    let longest = Cell::new(Duration::ZERO);
    let turns = Cell::new(0u32);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            if !scrolling.get() {
                return;
            }
            let now = Instant::now();
            longest.set(longest.get().max(now - last.replace(now)));
            if let Some(manager) = manager.upgrade()
                && let Some(window) = window.upgrade()
            {
                let _ = wheel(
                    &manager,
                    &window,
                    slint::LogicalPosition::new(300.0, 300.0),
                    NativeWheel::Lines { x: 0.0, y: -1.0 },
                );
            }
            turns.set(turns.get() + 1);
            if now - begun >= duration {
                println!(
                    "scroll turns={} max_gap_ms={:.1}",
                    turns.get(),
                    longest.get().as_secs_f64() * 1000.0
                );
                scrolling.set(false);
            }
        },
    );
    Some(timer)
}

#[cfg(not(feature = "acceptance-seams"))]
fn scroll(
    _: &Rc<ViewerWindows>,
    _: midnight_viewer_ui::ViewerWindow,
    _: Duration,
    scrolling: Rc<Cell<bool>>,
) -> Option<slint::Timer> {
    scrolling.set(false);
    None
}
