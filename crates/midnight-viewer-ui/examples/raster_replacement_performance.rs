//! Isolate raster replacement from an unchanged-image redraw, without a render notifier.
//! Run one mode per process: `fixed|precreated|fresh [--vulkan]`.
//! Precreated CPU Image handles do not retain two GPU textures in one Image item.
#[path = "performance_probe/native_parent.rs"]
mod native_parent;
#[path = "performance_probe/render_observer.rs"]
#[expect(
    dead_code,
    reason = "Raster timing uses only the existing target redraw observer."
)]
mod render_observer;

mod timeline {
    use std::{cell::RefCell, rc::Rc, time::Instant};

    const CAPACITY: usize = 32_768;

    #[derive(Clone)]
    pub struct Trace(Rc<RefCell<Rows>>);
    struct Rows {
        origin: Instant,
        events: Vec<(u128, &'static str, f64)>,
        dropped: usize,
    }
    impl Trace {
        pub fn new() -> Self {
            Self(Rc::new(RefCell::new(Rows {
                origin: Instant::now(),
                events: Vec::with_capacity(CAPACITY),
                dropped: 0,
            })))
        }
        pub fn enabled(&self) -> bool {
            true
        }
        pub fn record(&self, event: &'static str, extra: f64) {
            let mut rows = self.0.borrow_mut();
            if rows.events.len() == CAPACITY {
                rows.dropped += 1;
                return;
            }
            let elapsed = rows.origin.elapsed().as_micros();
            rows.events.push((elapsed, event, extra));
        }
        pub fn dump(&self) -> usize {
            let rows = self.0.borrow();
            for (elapsed, event, extra) in &rows.events {
                println!("raster_timeline elapsed_us={elapsed} event={event} extra={extra:.3}");
            }
            println!(
                "raster_timeline rows={} dropped={}",
                rows.events.len(),
                rows.dropped
            );
            rows.dropped
        }
    }
}

use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, winit_030::WinitWindowAccessor,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

const WARMUP: usize = 2;
const SAMPLES: usize = 12;
const QUIET: Duration = Duration::from_millis(300);
const COLORS: [[u8; 3]; 2] = [[18, 52, 86], [200, 40, 40]];

#[derive(Clone, Copy)]
enum Mode {
    Fixed,
    Precreated,
    Fresh,
}
impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Precreated => "precreated",
            Self::Fresh => "fresh",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    window: slint::PhysicalSize,
    canvas: [f32; 4],
    scale: f32,
    raster: [u32; 2],
}
impl Geometry {
    fn read(window: &ViewerWindow) -> Option<Self> {
        window.window().with_winit_window(|_| ())?;
        let scale = window.window().scale_factor();
        let canvas = [
            window.get_content_left(),
            window.get_content_top(),
            window.get_content_width(),
            window.get_content_height(),
        ];
        if !scale.is_finite()
            || scale <= 0.0
            || canvas.iter().any(|value| !value.is_finite())
            || canvas[2] <= 0.0
            || canvas[3] <= 0.0
        {
            return None;
        }
        Some(Self {
            window: window.window().size(),
            canvas,
            scale,
            raster: [
                (canvas[2] * scale).round() as u32,
                (canvas[3] * scale).round() as u32,
            ],
        })
    }
}

struct Pending {
    serial: usize,
    timer: Instant,
    source_ready: Instant,
    set_return: Instant,
    redraw_begin: Option<Instant>,
}
struct Sample {
    serial: usize,
    timer: Instant,
    source_ready: Instant,
    set_return: Instant,
    redraw_begin: Instant,
    observed_return: Instant,
}
struct State {
    origin: Instant,
    geometry: Option<Geometry>,
    images: Option<[Image; 2]>,
    pending: Option<Pending>,
    samples: Vec<Sample>,
    last_return: Option<Instant>,
    error: Option<&'static str>,
    complete: bool,
}
impl State {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            geometry: None,
            images: None,
            pending: None,
            samples: Vec::with_capacity(WARMUP + SAMPLES),
            last_return: None,
            error: None,
            complete: false,
        }
    }
    fn fail(&mut self, message: &'static str) {
        self.error = Some(message);
        let _ = slint::quit_event_loop();
    }
}

fn image(size: [u32; 2], color: [u8; 3]) -> Image {
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(size[0], size[1]);
    let [r, g, b] = color;
    pixels.make_mut_slice().fill(Rgba8Pixel { r, g, b, a: 255 });
    Image::from_rgba8_premultiplied(pixels)
}

fn install_observer(
    window: &ViewerWindow,
    state: &Rc<RefCell<State>>,
    callback: &render_observer::Callback,
) {
    let weak = window.as_weak();
    let before_state = state.clone();
    let after_state = state.clone();
    let staged = Rc::new(Cell::new(None));
    let before_staged = staged.clone();
    *callback.borrow_mut() = Some(render_observer::Callbacks {
        window: window.as_weak(),
        before: Box::new(move || {
            before_staged.set(None);
            let Some(window) = weak.upgrade() else {
                return;
            };
            let Some(id) = window.window().with_winit_window(|native| native.id()) else {
                return;
            };
            if let Some(pending) = &mut before_state.borrow_mut().pending {
                pending.redraw_begin = Some(Instant::now());
                before_staged.set(Some((id, pending.serial)));
            }
        }),
        after: Box::new(move |id, _| {
            let Some((started_id, serial)) = staged.take() else {
                return;
            };
            if started_id != id {
                return;
            }
            let now = Instant::now();
            let mut state = after_state.borrow_mut();
            if !state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.serial == serial)
            {
                return;
            }
            let Some(pending) = state.pending.take() else {
                return;
            };
            let Some(redraw_begin) = pending.redraw_begin else {
                state.fail("missing matching target redraw begin");
                return;
            };
            state.samples.push(Sample {
                serial,
                timer: pending.timer,
                source_ready: pending.source_ready,
                set_return: pending.set_return,
                redraw_begin,
                observed_return: now,
            });
            state.last_return = Some(now);
        }),
    });
}

fn tick(window: &ViewerWindow, state: &mut State, mode: Mode, trace: &timeline::Trace) {
    let now = Instant::now();
    if state.origin.elapsed() > Duration::from_secs(40) {
        state.fail("probe duration exceeded 40 seconds");
        return;
    }
    let Some(geometry) = Geometry::read(window) else {
        return;
    };
    if state.geometry.is_some_and(|initial| initial != geometry) {
        state.fail("window, canvas or DPI changed during probe");
        return;
    }
    if state.images.is_none() {
        if geometry
            .raster
            .iter()
            .any(|extent| *extent == 0 || *extent > 4096)
        {
            state.fail("raster extent must be between 1 and 4096 physical pixels");
            return;
        }
        state.geometry = Some(geometry);
        // Setup is common to all modes and excluded from serial measurements.
        state.images = Some(COLORS.map(|color| image(geometry.raster, color)));
        window.set_raster_viewport_width(geometry.raster[0] as f32 / geometry.scale);
        window.set_raster_viewport_height(geometry.raster[1] as f32 / geometry.scale);
        if let Some(images) = &state.images {
            window.set_surface(images[0].clone());
        }
        state.last_return = Some(Instant::now());
        return;
    }
    if let Some(pending) = &state.pending {
        if now.duration_since(pending.timer) > Duration::from_secs(5) {
            state.fail("serial redraw did not return within five seconds");
        }
        return;
    }
    if state.samples.len() == WARMUP + SAMPLES {
        state.complete = true;
        let _ = slint::quit_event_loop();
        return;
    }
    if state
        .last_return
        .is_none_or(|last| now.duration_since(last) < QUIET)
    {
        return;
    }
    let serial = state.samples.len();
    trace.record("serial_timer", serial as f64);
    let source = match mode {
        Mode::Fixed => None,
        Mode::Precreated => state
            .images
            .as_ref()
            .map(|images| images[serial % 2].clone()),
        // The old current source remains owned by the item while this allocates.
        Mode::Fresh => Some(image(geometry.raster, COLORS[serial % 2])),
    };
    let source_ready = Instant::now();
    trace.record("serial_source_ready", serial as f64);
    if let Some(source) = source {
        window.set_surface(source);
    }
    // Explicitly request even fixed/identical-source updates; no input is injected.
    window.window().request_redraw();
    let set_return = Instant::now();
    trace.record("serial_set_return", serial as f64);
    state.pending = Some(Pending {
        serial,
        timer: now,
        source_ready,
        set_return,
        redraw_begin: None,
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let mode = match arguments.next().as_deref().and_then(|value| value.to_str()) {
        Some("fixed") => Mode::Fixed,
        Some("precreated") => Mode::Precreated,
        Some("fresh") => Mode::Fresh,
        _ => {
            return Err(
                "usage: raster_replacement_performance fixed|precreated|fresh [--vulkan]".into(),
            );
        }
    };
    let vulkan = match arguments.next().as_deref() {
        None => false,
        Some(value) if value == "--vulkan" => true,
        _ => return Err("only optional --vulkan is accepted".into()),
    };
    if arguments.next().is_some() {
        return Err("unexpected extra argument".into());
    }
    let backends = if vulkan {
        slint::wgpu_30::wgpu::Backends::VULKAN
    } else {
        slint::wgpu_30::wgpu::Backends::from_env().unwrap_or(if cfg!(windows) {
            slint::wgpu_30::wgpu::Backends::VULKAN | slint::wgpu_30::wgpu::Backends::DX12
        } else {
            slint::wgpu_30::wgpu::Backends::PRIMARY
        })
    };
    let mut graphics = slint::wgpu_30::WGPUSettings::default();
    graphics.backends = backends;
    let mut flags = slint::wgpu_30::wgpu::InstanceFlags::from_build_config();
    flags.remove(
        slint::wgpu_30::wgpu::InstanceFlags::DEBUG
            | slint::wgpu_30::wgpu::InstanceFlags::VALIDATION,
    );
    flags = flags.with_env();
    flags.insert(slint::wgpu_30::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
    graphics.instance_flags = flags;
    let callback: render_observer::Callback = Rc::new(RefCell::new(None));
    let trace = timeline::Trace::new();
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(graphics))
        .with_winit_custom_application_handler(render_observer::Observer {
            callback: callback.clone(),
            drawing: None,
            injected_wheel: false,
            native_events: Rc::new(RefCell::new(Vec::new())),
            timeline: trace.clone(),
            native_proof: None,
        })
        .select()?;
    println!(
        "raster_probe mode={} requested_backends={backends:?} warmup={WARMUP} measured={SAMPLES} notifier=false input=false",
        mode.name()
    );
    let state = Rc::new(RefCell::new(State::new()));
    let setup_state = state.clone();
    let setup_trace = trace.clone();
    let (window, timer) = native_parent::run(true, move |placement| {
        let window = ViewerWindow::new()?;
        window.set_caption("Raster replacement diagnostic".into());
        window.set_status("Raster replacement diagnostic".into());
        window.set_ui_font_family("Segoe UI".into());
        window.set_ui_font_size(13.0);
        window.set_compact(placement.logical_size().width < 500.0);
        window.set_web_mode(true);
        window.set_font_available(false);
        placement.apply(window.window());
        install_observer(&window, &setup_state, &callback);
        let closing = setup_state.clone();
        window.window().on_close_requested(move || {
            closing
                .borrow_mut()
                .fail("window closed before bounded sample pass completed");
            slint::CloseRequestResponse::HideWindow
        });
        window.show()?;
        let actual_gpu = i_slint_core::window::WindowInner::from_pub(window.window())
            .window_adapter()
            .renderer()
            .supports_transformations();
        if !actual_gpu {
            window.hide()?;
            return Err("renderer class mismatch: expected femtovg-wgpu".into());
        }
        println!("raster_renderer class=femtovg-wgpu actual_backend=unobserved");
        let weak = window.as_weak();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(4),
            move || {
                let Some(window) = weak.upgrade() else {
                    setup_state
                        .borrow_mut()
                        .fail("diagnostic window disappeared");
                    return;
                };
                tick(&window, &mut setup_state.borrow_mut(), mode, &setup_trace);
            },
        );
        Ok((window, timer))
    })?;
    timer.stop();
    window.hide()?;
    let dropped = trace.dump();
    let state = state.borrow();
    println!("raster_geometry {:?}", state.geometry);
    for sample in &state.samples {
        let ms = |end: Instant, begin: Instant| end.duration_since(begin).as_secs_f64() * 1000.0;
        println!(
            "raster_sample serial={} warmup={} source_ms={:.3} set_ms={:.3} wait_ms={:.3} redraw_remainder_ms={:.3} total_ms={:.3}",
            sample.serial,
            sample.serial < WARMUP,
            ms(sample.source_ready, sample.timer),
            ms(sample.set_return, sample.source_ready),
            ms(sample.redraw_begin, sample.set_return),
            ms(sample.observed_return, sample.redraw_begin),
            ms(sample.observed_return, sample.timer)
        );
        println!(
            "raster_sample_times serial={} timer_us={} source_ready_us={} set_return_us={} redraw_begin_us={} observed_return_us={}",
            sample.serial,
            sample.timer.duration_since(state.origin).as_micros(),
            sample.source_ready.duration_since(state.origin).as_micros(),
            sample.set_return.duration_since(state.origin).as_micros(),
            sample.redraw_begin.duration_since(state.origin).as_micros(),
            sample
                .observed_return
                .duration_since(state.origin)
                .as_micros(),
        );
    }
    if let Some(error) = state.error {
        return Err(error.into());
    }
    if !state.complete || state.samples.len() != WARMUP + SAMPLES || dropped != 0 {
        return Err("incomplete sample pass or exhausted trace capacity".into());
    }
    println!("raster_probe complete=true measured={SAMPLES} display_ack=false");
    Ok(())
}
