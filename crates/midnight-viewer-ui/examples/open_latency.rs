//! Native opening latency: time from `ViewerWindows::open_with_ready` to the accepted
//! first content frame, for each file in turn (each opened `REPEAT` times in one process).
//! Deploy the executable beside a `viewers` directory. Set MIDNIGHT_VIEWER_OPEN_TRACE=1 in
//! debug builds for the per-phase opening timestamps. Results and method:
//! docs/verification/2026-11-20-viewer-speed.md.
#[cfg(target_os = "linux")]
#[path = "support/native_html_linux.rs"]
mod native_linux;
#[path = "support/resident.rs"]
mod resident;

use midnight_core::*;
use midnight_viewer_ui::ViewerWindows;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const REPEAT: usize = 3;

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
        .renderer_name(renderer);
    if std::env::var_os("MIDNIGHT_PROBE_SHARED_WGPU").is_some_and(|value| value == "1") {
        let started = Instant::now();
        let configuration = shared_wgpu()?;
        eprintln!("shared wgpu device: {:?}", started.elapsed());
        selector = selector.require_wgpu_30(configuration);
    }
    selector.select()?;
    Ok(())
}

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
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        return Err("usage: open_latency FILE...".into());
    }
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
    let current = Rc::new(RefCell::new(
        None::<slint::Weak<midnight_viewer_ui::ViewerWindow>>,
    ));
    let timer = slint::Timer::default();
    let tick = {
        let manager = manager.clone();
        let failure = failure.clone();
        let done = done.clone();
        move || {
            if let Some(start) = started.get() {
                let Some(result) = done.take() else {
                    if start.elapsed() > Duration::from_secs(60) {
                        *failure.borrow_mut() = Some("opening timed out".into());
                        let _ = slint::quit_event_loop();
                    }
                    return;
                };
                let index = next.get() - 1;
                let path = &paths[index / REPEAT];
                // The status line shows errors that follow the first frame, such as a
                // failed resource release; ticks are 250 ms apart, so this is after one.
                let status = current
                    .borrow()
                    .as_ref()
                    .and_then(|window| window.upgrade())
                    .map(|window| window.get_status().to_string())
                    .unwrap_or_default();
                match result {
                    Ok(elapsed) => println!(
                        "open file={} attempt={} first_content_ms={:.1} process_ms={} status={status:?}",
                        path.display(),
                        index % REPEAT,
                        elapsed.as_secs_f64() * 1000.0,
                        process.elapsed().as_millis()
                    ),
                    Err(error) => println!("open file={} error={error}", path.display()),
                }
                manager.close_all();
                started.set(None);
                return;
            }
            let index = next.get();
            if index >= paths.len() * REPEAT {
                let _ = slint::quit_event_loop();
                return;
            }
            next.set(index + 1);
            let start = Instant::now();
            started.set(Some(start));
            let finished = done.clone();
            let ready = Box::new(move |result: Result<(), String>| {
                finished.set(Some(result.map(|()| start.elapsed())));
            });
            let opened = manager.open_with_ready(
                launch(&paths[index / REPEAT]),
                ViewerAppearance {
                    family: "sans-serif".into(),
                    font_size: 13.0,
                    foreground: [225, 229, 236, 255],
                    background: [25, 29, 37, 255],
                    selection: [55, 86, 124, 255],
                    accent: [133, 185, 244, 255],
                },
                slint::LogicalSize::new(1000.0, 700.0),
                keys.clone(),
                Rc::new(|_| {}),
                ready,
            );
            match opened {
                Ok(window) => *current.borrow_mut() = Some(window),
                Err(error) => done.set(Some(Err(error))),
            }
        }
    };
    // Settle between openings like a user would; timing excludes this pause.
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), tick);
    // Closing each viewer must not end the loop between openings.
    slint::run_event_loop_until_quit()?;
    timer.stop();
    manager.close_all();
    if let Some(message) = failure.borrow_mut().take() {
        return Err(message.into());
    }
    Ok(())
}
