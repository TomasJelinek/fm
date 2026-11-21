//! Native HTML timing against a marked diagnostic page or an explicit local document.
//! Set MIDNIGHT_HTML_PROBE_RENDER_MARKERS=0 with timeline enabled to omit the render notifier.
#[path = "support/resident.rs"]
mod resident;

#[path = "performance_probe/native_parent.rs"]
mod native_parent;
#[path = "performance_probe/render_observer.rs"]
#[expect(
    dead_code,
    reason = "HTML uses redraw timing without the inactive Source drag proof."
)]
mod render_observer;
#[path = "support/html_wheel.rs"]
mod wheel;
mod timeline {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    const CAPACITY: usize = 65_536;

    #[derive(Clone)]
    pub struct Trace(Rc<State>);
    struct State {
        enabled: bool,
        render_markers: bool,
        origin: Instant,
        utc_us: u128,
        open: Cell<u8>,
        native_wheels: Cell<u64>,
        rows: RefCell<Vec<Row>>,
        dropped: Cell<u64>,
    }
    struct Row {
        elapsed_us: u128,
        kind: &'static str,
        open: u8,
        extra: f64,
    }
    pub struct Span<'a> {
        trace: &'a Trace,
        end: &'static str,
    }
    impl Drop for Span<'_> {
        fn drop(&mut self) {
            self.trace.record(self.end, 0.0);
        }
    }
    impl Trace {
        pub fn new(enabled: bool) -> Self {
            Self(Rc::new(State {
                enabled,
                render_markers: enabled
                    && std::env::var_os("MIDNIGHT_HTML_PROBE_RENDER_MARKERS")
                        .is_none_or(|value| value != "0"),
                origin: Instant::now(),
                utc_us: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros(),
                open: Cell::new(0),
                native_wheels: Cell::new(0),
                rows: RefCell::new(Vec::with_capacity(if enabled { CAPACITY } else { 0 })),
                dropped: Cell::new(0),
            }))
        }
        pub fn enabled(&self) -> bool {
            self.0.enabled
        }
        pub fn render_markers(&self) -> bool {
            self.0.render_markers
        }
        pub fn open(&self, reopened: bool) {
            self.0.open.set(u8::from(reopened));
            self.record("open", 0.0);
        }
        pub fn record(&self, kind: &'static str, extra: f64) {
            if kind == "native_wheel" {
                self.0
                    .native_wheels
                    .set(self.0.native_wheels.get().saturating_add(1));
            }
            if !self.enabled() {
                return;
            }
            let elapsed_us = self.0.origin.elapsed().as_micros();
            let mut rows = self.0.rows.borrow_mut();
            if rows.len() == CAPACITY {
                self.0.dropped.set(self.0.dropped.get().saturating_add(1));
                return;
            }
            rows.push(Row {
                elapsed_us,
                kind,
                open: self.0.open.get(),
                extra,
            });
        }
        pub fn span(&self, begin: &'static str, end: &'static str) -> Option<Span<'_>> {
            self.enabled().then(|| {
                self.record(begin, 0.0);
                Span { trace: self, end }
            })
        }
        pub fn native_wheels(&self) -> u64 {
            self.0.native_wheels.get()
        }
        pub fn dump(&self) {
            if !self.enabled() {
                return;
            }
            eprintln!(
                "html_timeline schema=1 utc_origin_us={} capacity={CAPACITY} render_markers={}",
                self.0.utc_us,
                self.render_markers()
            );
            for row in self.0.rows.borrow().iter() {
                eprintln!(
                    "html_timeline elapsed_us={} kind={} open={} extra={:.3}",
                    row.elapsed_us, row.kind, row.open, row.extra
                );
            }
            eprintln!(
                "html_timeline end=true rows={} dropped={}",
                self.0.rows.borrow().len(),
                self.0.dropped.get()
            );
        }
    }
    impl Default for Trace {
        fn default() -> Self {
            Self::new(false)
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn disabled_trace_keeps_delivery_count_without_retaining_events() {
            let trace = Trace::default();
            trace.record("native_wheel", 0.0);
            trace.record("timer_entry", 0.0);
            assert_eq!(trace.native_wheels(), 1);
            assert!(trace.0.rows.borrow().is_empty());
            assert_eq!(trace.0.rows.borrow().capacity(), 0);
            assert_eq!(trace.0.dropped.get(), 0);
        }

        #[test]
        fn exhausted_trace_preserves_rows_and_counts_later_native_delivery() {
            let trace = Trace::new(true);
            for _ in 0..CAPACITY {
                trace.record("timer_entry", 0.0);
            }
            trace.open(true);
            trace.record("native_wheel", 0.0);
            assert_eq!(trace.0.rows.borrow().len(), CAPACITY);
            assert_eq!(trace.0.dropped.get(), 2);
            assert_eq!(trace.native_wheels(), 1);
            assert!(trace.0.rows.borrow().iter().all(|row| row.open == 0));
        }
    }
}
use midnight_core::*;
use midnight_ui_input::WindowPlacement;
use midnight_viewer_ui::{ViewerWindow, ViewerWindows};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const SCROLL_ROUNDS: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fixture {
    Synthetic,
    Document,
}

#[derive(Clone, Copy)]
struct ProbeOptions {
    fixture: Fixture,
    software: bool,
    backends: slint::wgpu_30::wgpu::Backends,
}

struct State {
    start: Instant,
    reopened: bool,
    ready: bool,
    first: bool,
    signature: u64,
    published: u64,
    drawn: u64,
    changed: Instant,
    changed_draw_return: Instant,
    pending_native_wheel: u64,
    pending: Option<Instant>,
    wheel_after_publication: u64,
    publication_ms: Option<f64>,
    draw_ms: Option<f64>,
    round: usize,
    burst: Option<wheel::Burst>,
    error: Option<String>,
}
impl State {
    fn new(reopened: bool) -> Self {
        Self {
            start: Instant::now(),
            reopened,
            ready: false,
            first: false,
            signature: 0,
            published: 0,
            drawn: 0,
            changed: Instant::now(),
            changed_draw_return: Instant::now(),
            pending_native_wheel: 0,
            pending: None,
            wheel_after_publication: 0,
            publication_ms: None,
            draw_ms: None,
            round: 0,
            burst: None,
            error: None,
        }
    }

    fn begin_wheel(&mut self, native_wheels: u64, now: Instant) {
        self.pending = Some(now);
        self.pending_native_wheel = native_wheels.saturating_add(1);
        self.wheel_after_publication = self.published;
        self.publication_ms = None;
        self.draw_ms = None;
    }

    fn observe_publication(&mut self, native_wheels: u64, now: Instant) {
        if let Some(sent) = self.pending {
            // Exclude pixels published before this wheel reached the current viewer.
            // Later pixels are still not an acknowledgement of this input.
            if native_wheels < self.pending_native_wheel {
                self.wheel_after_publication = self.published;
                return;
            }
            self.publication_ms
                .get_or_insert(now.duration_since(sent).as_secs_f64() * 1000.0);
        }
    }

    fn observe_draw(&mut self, publication: u64, native_wheels: u64, now: Instant) {
        if publication <= self.drawn {
            return;
        }
        self.drawn = publication;
        self.changed_draw_return = now;
        if let Some(sent) = self.pending
            && native_wheels >= self.pending_native_wheel
            && publication > self.wheel_after_publication
            && self.publication_ms.is_some()
        {
            self.draw_ms
                .get_or_insert(now.duration_since(sent).as_secs_f64() * 1000.0);
        }
    }

    fn quiet_enough(&self, now: Instant) -> bool {
        let settle = Duration::from_millis(300);
        self.drawn >= self.published
            && now.duration_since(self.changed) >= settle
            && now.duration_since(self.changed_draw_return) >= settle
    }

    fn prefix(&self, fixture: Fixture) -> &'static str {
        match (fixture, self.reopened) {
            (Fixture::Synthetic, _) => "",
            (Fixture::Document, false) => "open=first ",
            (Fixture::Document, true) => "open=reopen ",
        }
    }
}

fn sample_pixels(pixels: &[slint::Rgba8Pixel], fixture: Fixture) -> Option<(u64, bool)> {
    let first = pixels.first()?;
    let stride = match fixture {
        Fixture::Synthetic => 997,
        Fixture::Document => (pixels.len() / 4_096).max(1),
    };
    let (signature, varied) =
        pixels
            .iter()
            .step_by(stride)
            .fold((0u64, false), |(signature, varied), pixel| {
                (
                    signature.wrapping_mul(16_777_619)
                        ^ u64::from(pixel.r)
                        ^ (u64::from(pixel.g) << 8)
                        ^ (u64::from(pixel.b) << 16),
                    varied || [pixel.r, pixel.g, pixel.b] != [first.r, first.g, first.b],
                )
            });
    let visible = match fixture {
        Fixture::Synthetic => [first.r, first.g, first.b] == [0x12, 0x34, 0x56],
        Fixture::Document => varied,
    };
    Some((signature, visible))
}

fn stamp(window: &ViewerWindow, fixture: Fixture) -> Option<(u64, bool)> {
    if fixture == Fixture::Document && !window.get_web_mode() {
        return None;
    }
    // The presentation already owns a premultiplied RGBA shared buffer.
    let pixels = window.get_surface().to_rgba8_premultiplied()?;
    sample_pixels(pixels.as_slice(), fixture)
}

fn progress_error(window: &ViewerWindow, fixture: Fixture, message: String) -> String {
    if fixture == Fixture::Synthetic {
        format!("{message}: {}", window.get_status())
    } else {
        message
    }
}

fn open_probe(
    manager: &ViewerWindows,
    path: &Path,
    placement: WindowPlacement,
    observer: &render_observer::Callback,
    state: &Rc<RefCell<State>>,
    options: ProbeOptions,
    timeline: &timeline::Trace,
) -> Result<slint::Weak<ViewerWindow>, Box<dyn std::error::Error>> {
    let fixture = options.fixture;
    timeline.open(state.borrow().reopened);
    let prefix = state.borrow().prefix(fixture);
    let entries: Arc<[Entry]> =
        vec![Entry::new(EntryId(1), path.to_path_buf(), EntryKind::File)].into();
    state.borrow_mut().start = Instant::now();
    let ready_state = state.clone();
    let window = manager.open_with_ready(
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::new(Default::default()),
            archive: None,
            focused: path.to_path_buf(),
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
            selection_text: [0; 4],
            accent: [133, 185, 244, 255],
        },
        placement,
        Rc::new(|_, _| None),
        Rc::new(|_| {}),
        Box::new(move |ready| {
            if fixture == Fixture::Synthetic {
                eprintln!("opening_result={ready:?}");
                return;
            }
            let ok = ready.is_ok();
            eprintln!("{prefix}opening_ok={ok}");
            let mut state = ready_state.borrow_mut();
            state.ready = ok;
            if let Err(error) = ready {
                eprintln!("{prefix}opening_error={error:?}");
                state.error = Some(format!("{prefix}viewer open failed: {error}"));
            }
        }),
    )?;
    eprintln!(
        "{prefix}construct_ms={:.3}",
        state.borrow().start.elapsed().as_secs_f64() * 1000.0
    );
    // Only FemtoVG-WGPU and software are enabled in this pinned probe build.
    // Query the actual renderer without installing a rendering notifier.
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
    let expected_renderer = if options.software {
        "software"
    } else {
        "femtovg-wgpu"
    };
    eprintln!(
        "{prefix}renderer expected={expected_renderer} requested_backends={:?} actual={actual_renderer}",
        options.backends,
    );
    if actual_renderer != expected_renderer {
        manager.close_all();
        return Err(format!(
            "HTML probe renderer mismatch: expected {expected_renderer}, got {actual_renderer}"
        )
        .into());
    }
    if timeline.render_markers() {
        let render_trace = timeline.clone();
        window
            .upgrade()
            .ok_or("window disappeared")?
            .window()
            .set_rendering_notifier(move |phase, api| {
                if matches!(phase, slint::RenderingState::RenderingSetup)
                    && let slint::GraphicsAPI::WGPU30 { device, .. } = api
                {
                    eprintln!(
                        "html_renderer_identity schema=1 source=render_setup timing_eligible=false backend={:?}",
                        device.adapter_info().backend,
                    );
                }
                let kind = match phase {
                    slint::RenderingState::RenderingSetup => "render_setup",
                    slint::RenderingState::BeforeRendering => "render_before",
                    slint::RenderingState::AfterRendering => "render_after",
                    slint::RenderingState::RenderingTeardown => "render_teardown",
                    _ => return,
                };
                render_trace.record(kind, 0.0);
            })?;
    }
    let weak = window.clone();
    let publication_trace = timeline.clone();
    let publish = state.clone();
    window
        .upgrade()
        .ok_or("window disappeared")?
        .on_frame_presented(move || {
            let Some(window) = weak.upgrade() else { return };
            let sampled = {
                let _span = publication_trace.span("stamp_begin", "stamp_end");
                stamp(&window, fixture)
            };
            let Some((signature, colored)) = sampled else {
                return;
            };
            let mut state = publish.borrow_mut();
            if !state.first && !colored {
                return;
            }
            let first = !state.first;
            if first {
                state.first = true;
                eprintln!(
                    "{prefix}first_publication_ms={:.3} physical={}x{} scale={}",
                    state.start.elapsed().as_secs_f64() * 1000.0,
                    window.get_surface().size().width,
                    window.get_surface().size().height,
                    window.window().scale_factor()
                );
            }
            if first || signature != state.signature {
                state.signature = signature;
                state.published += 1;
                publication_trace.record("publication", state.published as f64);
                let now = Instant::now();
                state.changed = now;
                let native_wheels = publication_trace.native_wheels();
                state.observe_publication(native_wheels, now);
                let publication = state.published;
                if let Some(burst) = &mut state.burst {
                    burst.observe_publication(publication, native_wheels);
                }
            }
        });
    let draw = state.clone();
    let draw_trace = timeline.clone();
    let viewer_at_start = window.clone();
    let state_at_start = state.clone();
    let started = Rc::new(Cell::new(None));
    let finished = started.clone();
    *observer.borrow_mut() = Some(render_observer::Callbacks {
        window: window.clone(),
        before: Box::new(move || {
            started.set(viewer_at_start.upgrade().and_then(|viewer| {
                viewer
                    .window()
                    .with_winit_window(|native| (native.id(), state_at_start.borrow().published))
            }));
        }),
        after: Box::new(move |window_id, _| {
            let Some((started_window, publication)) = finished.take() else {
                return;
            };
            if started_window != window_id {
                return;
            }
            let mut state = draw.borrow_mut();
            if publication <= state.drawn {
                return;
            }
            if state.drawn == 0 {
                eprintln!(
                    "{prefix}first_draw_ms={:.3}",
                    state.start.elapsed().as_secs_f64() * 1000.0
                );
            }
            let native_wheels = draw_trace.native_wheels();
            state.observe_draw(publication, native_wheels, Instant::now());
            if let Some(burst) = &mut state.burst {
                burst.observe_draw(publication, native_wheels);
            }
        }),
    });
    Ok(window)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: html_performance fixture.html [--software] [--document]")?,
    );
    let mut software = false;
    let mut fixture = Fixture::Synthetic;
    for arg in args {
        if arg == "--software" {
            software = true;
        } else if arg == "--document" {
            fixture = Fixture::Document;
        } else {
            return Err("unknown HTML performance probe option".into());
        }
    }
    let observer: render_observer::Callback = Rc::new(RefCell::new(None));
    let timeline = timeline::Trace::new(
        std::env::var_os("MIDNIGHT_HTML_PROBE_TIMELINE").is_some_and(|value| value == "1"),
    );
    let diagnostic = timeline.clone();
    let backends = if software {
        slint::wgpu_30::wgpu::Backends::empty()
    } else {
        slint::wgpu_30::wgpu::Backends::from_env().unwrap_or(
            slint::wgpu_30::wgpu::Backends::VULKAN | slint::wgpu_30::wgpu::Backends::DX12,
        )
    };
    let options = ProbeOptions {
        fixture,
        software,
        backends,
    };
    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    if software {
        selector = selector.renderer_name("software".into());
    } else {
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
            timeline: timeline.clone(),
            // HTML wheel timing does not collect the inactive Source drag proof.
            native_proof: None,
        })
        .select()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let runtime = resident::ResidentRuntime::new(runtime)?;
    let service = midnight_viewer_engine::Service::new(runtime.handle().clone(), runtime.spawner());
    runtime.track_spare(&service);
    let manager = Rc::new(ViewerWindows::new(service));
    let state = Rc::new(RefCell::new(State::new(false)));
    let result = state.clone();
    let manager_for_open = manager.clone();
    let timer = native_parent::run(true, move |placement| {
        let mut viewer = open_probe(
            &manager_for_open,
            &path,
            placement.clone(),
            &observer,
            &state,
            options,
            &timeline,
        )?;
        let timer = slint::Timer::default();
        let mut last_tick = None::<Instant>;
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(4), move || {
            let _timer_span = timeline.span("timer_entry", "timer_return");
            if timeline.enabled() {
                let now = Instant::now();
                if let Some(previous) = last_tick.replace(now) {
                    timeline.record("timer_gap_ms", now.duration_since(previous).as_secs_f64() * 1000.0);
                }
            }
            let Some(window) = viewer.upgrade() else {
                state.borrow_mut().error.get_or_insert_with(|| "viewer disappeared".into());
                let _ = slint::quit_event_loop();
                return;
            };
            let mut progress = state.borrow_mut();
            if progress.error.is_some() {
                let _ = slint::quit_event_loop();
                return;
            }
            let prefix = progress.prefix(fixture);
            if progress.start.elapsed() > Duration::from_secs(40) {
                let message = format!("{prefix}stage {} timeout", progress.round);
                progress.error = Some(progress_error(&window, fixture, message));
                let _ = slint::quit_event_loop();
                return;
            }
            let last_publication = progress.changed;
            if let Some(burst) = &mut progress.burst {
                if let Err(error) = burst.advance(&window, last_publication, &timeline) {
                    progress.error = Some(error);
                    let _ = slint::quit_event_loop();
                } else if burst.finished {
                    let _ = slint::quit_event_loop();
                }
                return;
            }
            if !progress.first
                || !progress.quiet_enough(Instant::now())
                || (fixture == Fixture::Document && (!progress.ready || progress.drawn == 0))
            {
                return;
            }
            if fixture == Fixture::Document && !progress.reopened {
                drop(progress);
                drop(window);
                let close = Instant::now();
                manager_for_open.close_all();
                eprintln!("open=first close_request_ms={:.3}", close.elapsed().as_secs_f64() * 1000.0);
                *state.borrow_mut() = State::new(true);
                match open_probe(
                    &manager_for_open,
                    &path,
                    placement.clone(),
                    &observer,
                    &state,
                    options,
                    &timeline,
                ) {
                    Ok(reopened) => viewer = reopened,
                    Err(_) => {
                        state.borrow_mut().error = Some("open=reopen construction failed".into());
                        let _ = slint::quit_event_loop();
                    }
                }
                return;
            }
            if let Some(sent) = progress.pending {
                if let (Some(publication), Some(draw)) =
                    (progress.publication_ms, progress.draw_ms)
                {
                    eprintln!(
                        "{prefix}wheel round={} first_publication_ms={publication:.3} first_draw_ms={draw:.3}",
                        progress.round
                    );
                    progress.round += 1;
                    progress.pending = None;
                } else if sent.elapsed() > Duration::from_secs(5) {
                    let message = format!("{prefix}wheel {} produced no changed frame", progress.round);
                    progress.error = Some(progress_error(&window, fixture, message));
                    let _ = slint::quit_event_loop();
                    return;
                } else {
                    return;
                }
            }
            if progress.round == SCROLL_ROUNDS {
                if fixture == Fixture::Document {
                    progress.burst = Some(wheel::Burst::new(
                        progress.published,
                        timeline.native_wheels(),
                    ));
                } else {
                    let _ = slint::quit_event_loop();
                }
                return;
            }
            progress.begin_wheel(timeline.native_wheels(), Instant::now());
            // A small report can reach the end after one forward gesture.
            let delta = if fixture == Fixture::Document && !progress.round.is_multiple_of(2) {
                120
            } else {
                -120
            };
            if let Err(error) = wheel::send(&window, delta, &timeline) {
                progress.error = Some(error);
                let _ = slint::quit_event_loop();
            }
        });
        Ok(timer)
    });
    diagnostic.dump();
    let timer = timer?;
    timer.stop();
    manager.close_all();
    let state = result.borrow();
    if let Some(error) = &state.error {
        return Err(error.clone().into());
    }
    if state.round != SCROLL_ROUNDS
        || (fixture == Fixture::Document
            && !(state.reopened && state.burst.as_ref().is_some_and(|burst| burst.finished)))
    {
        return Err(format!("event loop exited early at round {}", state.round).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_gate_rejects_publication_before_current_native_wheel() {
        let now = Instant::now();
        let mut state = State::new(true);
        state.published = 10;
        state.drawn = 10;
        state.begin_wheel(5, now);
        state.published = 11;
        state.observe_publication(5, now + Duration::from_millis(10));
        assert!(state.publication_ms.is_none());
        assert_eq!(state.wheel_after_publication, 11);
    }

    #[test]
    fn delivery_gate_rejects_later_redraw_of_pre_delivery_publication() {
        let now = Instant::now();
        let mut state = State::new(true);
        state.published = 10;
        state.drawn = 10;
        state.begin_wheel(5, now);
        state.published = 11;
        state.observe_publication(5, now + Duration::from_millis(10));
        state.observe_draw(11, 6, now + Duration::from_millis(30));
        assert!(state.draw_ms.is_none());
    }

    #[test]
    fn delivery_gate_credits_post_native_changed_publication_and_redraw() {
        let now = Instant::now();
        let mut state = State::new(true);
        state.published = 10;
        state.drawn = 10;
        state.begin_wheel(5, now);
        state.published = 11;
        state.observe_publication(6, now + Duration::from_millis(20));
        state.observe_draw(11, 6, now + Duration::from_millis(30));
        assert_eq!(state.publication_ms, Some(20.0));
        assert_eq!(state.draw_ms, Some(30.0));
    }

    #[test]
    fn delivery_gate_quiet_waits_since_blocked_redraw_return_and_pending_publication() {
        let now = Instant::now();
        let mut state = State::new(true);
        state.changed = now;
        state.published = 2;
        state.drawn = 1;
        state.observe_draw(2, 0, now + Duration::from_millis(500));
        assert!(!state.quiet_enough(now + Duration::from_millis(500)));
        assert!(!state.quiet_enough(now + Duration::from_millis(799)));
        assert!(state.quiet_enough(now + Duration::from_millis(800)));
        state.published = 3;
        assert!(!state.quiet_enough(now + Duration::from_millis(1000)));
    }

    #[test]
    fn real_document_paint_requires_visible_content_and_preserves_synthetic_marker() {
        let marker = slint::Rgba8Pixel {
            r: 0x12,
            g: 0x34,
            b: 0x56,
            a: 0xff,
        };
        let white = slint::Rgba8Pixel {
            r: 0xff,
            g: 0xff,
            b: 0xff,
            a: 0xff,
        };
        assert!(sample_pixels(&[marker; 8], Fixture::Synthetic).unwrap().1);
        assert!(!sample_pixels(&[marker; 8], Fixture::Document).unwrap().1);
        assert!(!sample_pixels(&[white; 8], Fixture::Synthetic).unwrap().1);
        assert!(!sample_pixels(&[white; 8], Fixture::Document).unwrap().1);
        assert!(
            sample_pixels(&[white, marker, white], Fixture::Document)
                .unwrap()
                .1
        );
    }
}
