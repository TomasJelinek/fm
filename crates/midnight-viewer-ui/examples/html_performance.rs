//! Native HTML wheel measurement against an explicit, script-free diagnostic document.
#[path = "performance_probe/native_parent.rs"]
mod native_parent;
#[path = "performance_probe/render_observer.rs"]
mod render_observer;
#[path = "support/html_wheel.rs"]
mod wheel;
mod timeline {
    #[derive(Clone, Default)]
    pub struct Trace;
    impl Trace {
        pub fn record(&self, _: &'static str, _: f64) {}
    }
}
use midnight_core::*;
use midnight_viewer_ui::{ViewerWindow, ViewerWindows};
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

struct State {
    start: Instant,
    first: bool,
    signature: u64,
    published: u64,
    drawn: u64,
    changed: Instant,
    pending: Option<Instant>,
    publication_ms: Option<f64>,
    draw_ms: Option<f64>,
    round: usize,
    error: Option<String>,
}
fn stamp(window: &ViewerWindow) -> Option<(u64, bool)> {
    // The presentation already owns a premultiplied RGBA shared buffer; no full image copy.
    let pixels = window.get_surface().to_rgba8_premultiplied()?;
    let first = pixels.as_slice().first()?;
    let colored = first.r == 0x12 && first.g == 0x34 && first.b == 0x56;
    let signature = pixels
        .as_slice()
        .iter()
        .step_by(997)
        .fold(0u64, |hash, pixel| {
            hash.wrapping_mul(16777619)
                ^ u64::from(pixel.r)
                ^ (u64::from(pixel.g) << 8)
                ^ (u64::from(pixel.b) << 16)
        });
    Some((signature, colored))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: html_performance fixture.html [--software]")?,
    );
    let software = args.next().is_some_and(|value| value == "--software");
    let observer: render_observer::Callback = Rc::new(RefCell::new(None));
    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    if software {
        selector = selector.renderer_name("software".into());
    } else {
        let mut graphics = slint::wgpu_30::WGPUSettings::default();
        graphics.backends = slint::wgpu_30::wgpu::Backends::from_env()
            .unwrap_or(slint::wgpu_30::wgpu::Backends::DX12);
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
    }
    selector
        .with_winit_custom_application_handler(render_observer::Observer {
            callback: observer.clone(),
            drawing: None,
            injected_wheel: true,
            native_events: Rc::new(RefCell::new(Vec::new())),
            timeline: timeline::Trace,
        })
        .select()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let service = midnight_viewer_engine::Service::new(runtime.handle().clone());
    let manager = Rc::new(ViewerWindows::new(service));
    let state = Rc::new(RefCell::new(State {
        start: Instant::now(),
        first: false,
        signature: 0,
        published: 0,
        drawn: 0,
        changed: Instant::now(),
        pending: None,
        publication_ms: None,
        draw_ms: None,
        round: 0,
        error: None,
    }));
    let result = state.clone();
    let manager_for_open = manager.clone();
    let timer = native_parent::run(true, move |placement| {
        let entries: Arc<[Entry]> =
            vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
        state.borrow_mut().start = Instant::now();
        let window = manager_for_open.open_with_ready(
            ViewerLaunch {
                find: None,
                associations: Default::default(),
                tree: None,
                generated: None,
                preferences: Arc::new(Default::default()),
                archive: None,
                focused: path,
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                basic: false,
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "Segoe UI".into(),
                font_size: 13.0,
                foreground: [225, 229, 236, 255],
                background: [25, 29, 37, 255],
                selection: [55, 86, 124, 255],
                accent: [133, 185, 244, 255],
            },
            placement,
            Rc::new(|_, _| None),
            Rc::new(|_| {}),
            Box::new(|ready| eprintln!("opening_result={ready:?}")),
        )?;
        eprintln!(
            "construct_ms={:.3}",
            state.borrow().start.elapsed().as_secs_f64() * 1000.0
        );
        let weak = window.clone();
        let publish = state.clone();
        window
            .upgrade()
            .ok_or("window disappeared")?
            .on_frame_presented(move || {
                let Some(window) = weak.upgrade() else { return };
                let Some((signature, colored)) = stamp(&window) else {
                    return;
                };
                let mut state = publish.borrow_mut();
                if !state.first && !colored {
                    return;
                }
                if !state.first {
                    state.first = true;
                    eprintln!(
                        "first_publication_ms={:.3} physical={}x{} scale={}",
                        state.start.elapsed().as_secs_f64() * 1000.0,
                        window.get_surface().size().width,
                        window.get_surface().size().height,
                        window.window().scale_factor()
                    );
                }
                if signature != state.signature {
                    state.signature = signature;
                    state.published += 1;
                    state.changed = Instant::now();
                    if let Some(sent) = state.pending {
                        state
                            .publication_ms
                            .get_or_insert(sent.elapsed().as_secs_f64() * 1000.0);
                    }
                }
            });
        let draw = state.clone();
        *observer.borrow_mut() = Some(render_observer::Callbacks {
            window: window.clone(),
            before: Box::new(|| {}),
            after: Box::new(move |_, _| {
                let mut state = draw.borrow_mut();
                if state.published == state.drawn {
                    return;
                }
                if state.drawn == 0 {
                    eprintln!(
                        "first_draw_ms={:.3}",
                        state.start.elapsed().as_secs_f64() * 1000.0
                    );
                }
                state.drawn = state.published;
                if let Some(sent) = state.pending {
                    state
                        .draw_ms
                        .get_or_insert(sent.elapsed().as_secs_f64() * 1000.0);
                }
            }),
        });
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(4), move || {
            let Some(window) = window.upgrade() else {
                state.borrow_mut().error = Some("viewer disappeared".into());
                let _ = slint::quit_event_loop(); return;
            };
            let mut state = state.borrow_mut();
            if state.start.elapsed() > Duration::from_secs(40) {
                state.error = Some(format!("stage {} timeout: {}", state.round, window.get_status()));
                let _ = slint::quit_event_loop(); return;
            }
            if !state.first || state.changed.elapsed() < Duration::from_millis(300) { return; }
            if let Some(sent) = state.pending {
                if let (Some(publication), Some(draw)) = (state.publication_ms, state.draw_ms) {
                    eprintln!("wheel round={} first_publication_ms={publication:.3} first_draw_ms={draw:.3}", state.round);
                    state.round += 1;
                    state.pending = None;
                } else if sent.elapsed() > Duration::from_secs(5) {
                    state.error = Some(format!("wheel {} produced no changed frame: {}", state.round, window.get_status()));
                    let _ = slint::quit_event_loop(); return;
                } else { return; }
            }
            if state.round == 12 { let _ = slint::quit_event_loop(); return; }
            state.pending = Some(Instant::now());
            state.publication_ms = None;
            state.draw_ms = None;
            if let Err(error) = wheel::send(&window) {
                state.error = Some(error); let _ = slint::quit_event_loop();
            }
        });
        Ok(timer)
    })?;
    timer.stop();
    manager.close_all();
    let state = result.borrow();
    if let Some(error) = &state.error {
        return Err(error.clone().into());
    }
    if state.round != 12 {
        return Err(format!("event loop exited early at round {}", state.round).into());
    }
    Ok(())
}
