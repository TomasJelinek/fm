//! Scroll parity with the text viewer, measured on the real comparison window: its own
//! engine painting two text files, its native wheel and touchpad observer, its keys and its
//! position bar, with a controlled clock and the software renderer.
//!
//! The text viewer moves its text by exactly what the shared machinery delivers
//! (`midnight-viewer-ui` `raster_camera`): each glide frame's `WheelMotion` step, touchpad
//! finger and momentum deltas as they arrive, and a scroll key's whole distance at once
//! (`midnight-viewer-ui` `scrolling::keyboard`: 28 px a line, 40 px a column, the content
//! height a page). The expected offset below is that: the deltas this window's shared
//! `NativeScroll` delivered, plus the viewer's key distances. The shown offset is where the
//! comparison's text stands on screen: the position bar's fraction of the scrollable extent.
use super::identity_tests::TestRuntime;
use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
};
use std::time::Instant;

thread_local! {
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    static NATIVE: std::cell::OnceCell<Rc<MinimalSoftwareWindow>> = const {
        std::cell::OnceCell::new()
    };
}

/// A 125 Hz redraw, as `midnight_ui_input::WHEEL_FRAME` wakes a gliding window.
const FRAME: Duration = Duration::from_millis(8);
const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;
const LINES: usize = 3000;
const FONT: f32 = 14.0;
/// The painted row height in logical pixels at scale 1 (`text.rs`: font size × 1.6, up).
const ROW: f64 = 23.0;
/// The text viewer's key distances (`midnight-viewer-ui` `scrolling::keyboard`).
const KEY_LINE: f64 = 28.0;
const KEY_COLUMN: f64 = 40.0;

struct ClockPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for ClockPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        CLOCK.with(Cell::get)
    }
}

fn now() -> Duration {
    CLOCK.with(Cell::get)
}

fn advance(time: Duration) {
    CLOCK.with(|clock| clock.set(clock.get() + time));
}

/// One input of a schedule.
#[derive(Clone, Copy, Debug)]
pub(super) enum Step {
    /// Presented frames without input.
    Frames(u32),
    /// A wheel notch of these lines (x, y; negative scrolls down or right).
    Notch(f32, f32),
    /// A touchpad report 10 ms after the previous one: logical pixels and its phase.
    Finger(f32, TouchPhase),
    /// A keyboard action.
    Key(&'static str),
    /// The position bar moved to this fraction.
    Seek(f32),
}

/// What one presented frame showed.
#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    pub time: Duration,
    /// The viewer's offset: what the shared machinery delivered (logical px, down).
    pub expected: f64,
    /// The comparison's shown offset.
    pub shown: f64,
}

/// What the compared files hold.
#[derive(Clone, Copy, Debug)]
pub(super) enum Files {
    /// This many lines of text, the same but for one line near the end.
    Text(usize),
    /// Two binary files of this many bytes that differ in a few bytes (a hex view).
    Bytes(u64),
}

/// What presenting the worker's frames cost on the window's (UI) thread.
#[derive(Debug, Default)]
pub(super) struct Costs {
    /// Frames the worker painted and the window presented.
    pub paints: u32,
    /// Their presentation on the UI thread, in all and the longest.
    pub present: Duration,
    pub longest_present: Duration,
    /// From a frame's first input to its presentation (the worker's paint and delivery),
    /// in all and the longest.
    pub latency: Duration,
    pub longest_latency: Duration,
}

pub(super) struct Harness {
    native: Rc<MinimalSoftwareWindow>,
    /// The window's framebuffer: the renderer redraws only what changed.
    pixels: RefCell<Vec<slint::Rgb8Pixel>>,
    manager: ComparatorWindows,
    open: Rc<OpenWindow>,
    scroll: midnight_ui_input::NativeScroll,
    delivered: Rc<Cell<[f64; 2]>>,
    /// Whether the last frame moved anything (the next one waits for the worker too).
    moving: bool,
    /// Samples follow the horizontal axis.
    horizontal: bool,
    directory: std::path::PathBuf,
    lines: usize,
    pub(super) costs: Costs,
    _runtime: TestRuntime,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self::with(Files::Text(LINES))
    }

    pub(super) fn with(files: Files) -> Self {
        // One platform per test thread; a later harness on it reuses its window adapter.
        let native = NATIVE.with(|native| {
            native
                .get_or_init(|| {
                    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
                    native.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
                    slint::platform::set_platform(Box::new(ClockPlatform(native.clone()))).unwrap();
                    native
                })
                .clone()
        });
        let runtime = super::identity_tests::runtime();
        let service = Service::new(runtime.handle().clone(), runtime.spawner());
        let manager = ComparatorWindows::new_polled(service);
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("_build/qa")
            .join(format!(
                "compare-scroll-parity-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
        std::fs::create_dir_all(&directory).unwrap();
        let paths = [directory.join("left.txt"), directory.join("right.txt")];
        let lines = match files {
            Files::Text(lines) => {
                for (side, path) in paths.iter().enumerate() {
                    let text: String = (0..lines)
                        .map(|line| {
                            if line == lines - 10 {
                                format!("{} line {line}\n", ["old", "new"][side])
                            } else {
                                format!("shared line {line} of the text\n")
                            }
                        })
                        .collect();
                    std::fs::write(path, text).unwrap();
                }
                lines
            }
            Files::Bytes(size) => {
                let mut bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
                std::fs::write(&paths[0], &bytes).unwrap();
                for index in [1000, size / 2, size - 3] {
                    bytes[index as usize] ^= 0x5a;
                }
                std::fs::write(&paths[1], &bytes).unwrap();
                0
            }
        };
        let snapshot = midnight_core::FileComparatorSnapshot::default();
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
                ViewerAppearance {
                    family: "monospace".into(),
                    font_size: FONT,
                    foreground: [240, 240, 240, 255],
                    background: [20, 20, 20, 255],
                    selection: [60, 80, 120, 255],
                    selection_text: [0; 4],
                    accent: [100, 150, 220, 255],
                },
                slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
                Rc::new(|_, _| None),
            )
            .unwrap()
            .upgrade()
            .unwrap();
        let open = manager.windows.borrow()[0].clone();
        let delivered = Rc::new(Cell::new([0.0; 2]));
        let record = delivered.clone();
        let weak = Rc::downgrade(&open);
        let extent = (lines.max(1) - 1) as f64 * ROW;
        window.on_scroll(move |x, y| {
            // The text viewer stops at the text's edges.
            let [old_x, old_y] = record.get();
            record.set([
                (old_x - f64::from(x)).max(0.0),
                (old_y - f64::from(y)).clamp(0.0, extent),
            ]);
            if let Some(open) = weak.upgrade() {
                scroll_content(&open, x, y);
            }
        });
        let scroll = native_scroll(&window).synthesize_momentum(true);
        let mut harness = Self {
            native,
            pixels: RefCell::new(vec![slint::Rgb8Pixel::default(); (WIDTH * HEIGHT) as usize]),
            manager,
            open,
            scroll,
            delivered,
            moving: false,
            horizontal: false,
            directory,
            lines,
            costs: Costs::default(),
            _runtime: runtime,
        };
        harness.draw();
        let started = Instant::now();
        let ready = |status: &str| match files {
            Files::Text(lines) => status.contains(&format!("of {lines} ")),
            Files::Bytes(_) => status.contains("offset 0x"),
        };
        while !ready(&harness.open.window.get_status()) {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "no text frame: {:?}",
                harness.open.window.get_status()
            );
            std::thread::sleep(Duration::from_millis(5));
            slint::platform::update_timers_and_animations();
            harness.open.present(&harness.manager.service);
            harness.draw();
        }
        let window = &harness.open.window;
        let at = PhysicalPosition::new(
            f64::from(window.get_content_left() + window.get_content_width() / 2.0),
            f64::from(window.get_content_top() + window.get_content_height() / 2.0),
        );
        harness.native(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: at,
        });
        harness
    }

    fn draw(&self) {
        let mut pixels = self.pixels.borrow_mut();
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, WIDTH as usize);
        });
    }

    fn native(&mut self, event: WindowEvent) -> bool {
        self.scroll.observe(self.open.window.window(), &event, None)
    }

    /// The scrollable extent of the text: its last row at the top of the view.
    fn extent(&self) -> f64 {
        (self.lines.max(1) - 1) as f64 * ROW
    }

    /// Where the text stands on screen, logical pixels from its start.
    pub(super) fn shown(&self) -> f64 {
        f64::from(self.open.window.get_position()) * self.extent()
    }

    /// How far the text stands scrolled to the right on screen, logical pixels.
    fn shown_horizontal(&self) -> f64 {
        f64::from(self.open.window.get_shown_horizontal())
    }

    fn expected(&self) -> f64 {
        self.delivered.get()[1]
    }

    /// Present the worker's latest frame, if any, and count what that cost.
    fn present(&mut self, waited: Duration) {
        let before = self.open.window.get_surface();
        let started = Instant::now();
        self.open.present(&self.manager.service);
        let spent = started.elapsed();
        if self.open.window.get_surface() != before {
            let costs = &mut self.costs;
            costs.paints += 1;
            costs.present += spent;
            costs.longest_present = costs.longest_present.max(spent);
            costs.latency += waited;
            costs.longest_latency = costs.longest_latency.max(waited);
        }
    }

    /// Let the worker take what was sent and present what it paints: the comparison's best
    /// case, a worker that keeps up with every frame.
    fn settle(&mut self) {
        let started = Instant::now();
        let mut quiet = Instant::now();
        let mut last = (
            self.open.window.get_position(),
            self.open.window.get_status(),
        );
        while quiet.elapsed() < Duration::from_millis(30)
            && started.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(1));
            self.present(started.elapsed());
            let state = (
                self.open.window.get_position(),
                self.open.window.get_status(),
            );
            if state != last {
                last = state;
                quiet = Instant::now();
            }
        }
    }

    /// One presented frame: the native redraw, the window's timers, the worker's frame.
    fn frame(&mut self) -> Sample {
        let before = (self.expected(), self.shown());
        advance(FRAME);
        self.native(WindowEvent::RedrawRequested);
        slint::platform::update_timers_and_animations();
        if self.moving || self.expected() != before.0 {
            self.settle();
        } else {
            self.present(Duration::ZERO);
        }
        self.draw();
        let sample = self.sample();
        self.moving = sample.expected != before.0 || sample.shown != before.1;
        sample
    }

    fn sample(&self) -> Sample {
        if self.horizontal {
            return Sample {
                time: now(),
                expected: self.delivered.get()[0],
                shown: self.shown_horizontal(),
            };
        }
        Sample {
            time: now(),
            expected: self.expected(),
            shown: self.shown(),
        }
    }

    fn step(&mut self, step: Step, samples: &mut Vec<Sample>) {
        match step {
            Step::Frames(count) => {
                for _ in 0..count {
                    samples.push(self.frame());
                }
            }
            Step::Notch(x, y) => {
                self.native(WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::LineDelta(x, y),
                    phase: TouchPhase::Moved,
                });
            }
            Step::Finger(y, phase) => {
                advance(Duration::from_millis(10));
                let consumed = self.native(WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y.into())),
                    phase,
                });
                // Input the observer leaves alone reaches the content as Slint delivers it.
                if !consumed && y != 0.0 {
                    let window = &self.open.window;
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                            position: slint::LogicalPosition::new(
                                window.get_content_left() + window.get_content_width() / 2.0,
                                window.get_content_top() + window.get_content_height() / 2.0,
                            ),
                            delta_x: 0.0,
                            delta_y: y,
                        });
                }
            }
            Step::Key(action) => {
                // A key press ends wheel motion first, as the native observer does.
                self.scroll.cancel();
                let [x, y] = self.delivered.get();
                let page = f64::from(self.open.window.get_content_height());
                let (dx, dy) = match action {
                    "viewer.down" => (0.0, KEY_LINE),
                    "viewer.up" => (0.0, -KEY_LINE),
                    "viewer.right" => (KEY_COLUMN, 0.0),
                    "viewer.left" => (-KEY_COLUMN, 0.0),
                    "viewer.page-down" => (0.0, page),
                    "viewer.page-up" => (0.0, -page),
                    _ => (0.0, 0.0),
                };
                self.delivered
                    .set([(x + dx).max(0.0), (y + dy).clamp(0.0, self.extent())]);
                assert!(key(&self.open, action), "{action}");
                self.moving = true;
            }
            Step::Seek(fraction) => {
                let [x, _] = self.delivered.get();
                self.delivered.set([x, f64::from(fraction) * self.extent()]);
                self.open.window.invoke_seek(fraction);
                self.moving = true;
            }
        }
    }

    /// Run `steps` from the top of the text, then frames until everything has settled.
    pub(super) fn run(&mut self, steps: &[Step]) -> Vec<Sample> {
        self.horizontal = steps.iter().any(|step| {
            matches!(step, Step::Notch(x, _) if *x != 0.0)
                || matches!(step, Step::Key("viewer.left" | "viewer.right"))
        });
        self.reset();
        let mut samples = vec![self.sample()];
        for &step in steps {
            self.step(step, &mut samples);
        }
        for _ in 0..90 {
            samples.push(self.frame());
        }
        samples
    }

    /// Back to the top left, at rest.
    fn reset(&mut self) {
        self.scroll.cancel();
        let far = f32::from(i16::MAX);
        if self.shown_horizontal() > 0.0 {
            scroll_content(&self.open, far, 0.0);
        }
        self.open.window.invoke_seek(0.0);
        self.moving = true;
        for _ in 0..12 {
            self.frame();
        }
        self.settle();
        self.delivered.set([0.0; 2]);
        assert!(self.shown().abs() < 0.5, "not at the top: {}", self.shown());
        assert!(
            self.shown_horizontal().abs() < 0.5,
            "not at the left: {}",
            self.shown_horizontal()
        );
    }

    /// Whether both panes show the same pixels: the two files are the same near the top.
    fn panes_aligned(&self) -> Result<(), String> {
        self.open.window.window().request_redraw();
        self.draw();
        let pixels = self.pixels.borrow();
        let window = &self.open.window;
        let scale = window.window().scale_factor();
        let left = (window.get_content_left() * scale).round() as usize;
        let top = (window.get_content_top() * scale).round() as usize;
        let width = (window.get_content_width() * scale).round() as usize;
        let height = (window.get_content_height() * scale).round() as usize;
        let pane = width / 2;
        for y in top..top + height {
            let row = &pixels[y * WIDTH as usize..(y + 1) * WIDTH as usize];
            if let Some(x) = (0..pane).find(|&x| row[left + x] != row[left + pane + x]) {
                return Err(format!(
                    "pane rows differ at y {y}, x {x} of {pane}: {:?} and {:?}; top {top},                      height {height}",
                    row[left + x],
                    row[left + pane + x]
                ));
            }
        }
        Ok(())
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.manager.close_all();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// A notch every `frames` frames, `count` times.
fn notches(count: usize, frames: u32, lines: f32) -> Vec<Step> {
    (0..count)
        .flat_map(|index| {
            let mut steps = vec![Step::Notch(0.0, lines)];
            if index + 1 < count {
                steps.push(Step::Frames(frames));
            }
            steps
        })
        .collect()
}

fn flick() -> Vec<Step> {
    let mut steps = vec![Step::Finger(-20.0, TouchPhase::Started)];
    steps.extend(std::iter::repeat_n(
        Step::Finger(-20.0, TouchPhase::Moved),
        5,
    ));
    steps.push(Step::Finger(0.0, TouchPhase::Ended));
    steps
}

/// The measured schedules: name, glide scale, wheel speed scale, steps.
pub(super) fn schedules() -> Vec<(&'static str, f32, f32, Vec<Step>)> {
    let reverse = {
        let mut steps = notches(3, 3, -1.0);
        steps.push(Step::Frames(6));
        steps.push(Step::Notch(0.0, 1.0));
        steps
    };
    vec![
        ("one notch", 1.0, 1.0, notches(1, 0, -1.0)),
        ("five quick notches", 1.0, 1.0, notches(5, 3, -1.0)),
        ("reverse mid-glide", 1.0, 1.0, reverse),
        ("glide Off", 0.0, 1.0, notches(2, 10, -1.0)),
        ("glide Very long", 4.0, 1.0, notches(2, 10, -1.0)),
        ("speed Very slow", 1.0, 0.5, notches(2, 10, -1.0)),
        ("speed Very fast", 1.0, 2.0, notches(2, 10, -1.0)),
        ("touchpad flick", 1.0, 1.0, flick()),
        (
            "arrow down x3",
            1.0,
            1.0,
            vec![
                Step::Key("viewer.down"),
                Step::Frames(5),
                Step::Key("viewer.down"),
                Step::Frames(5),
                Step::Key("viewer.down"),
            ],
        ),
        (
            "PageDown x2",
            1.0,
            1.0,
            vec![
                Step::Key("viewer.page-down"),
                Step::Frames(10),
                Step::Key("viewer.page-down"),
            ],
        ),
        (
            "horizontal notch",
            1.0,
            1.0,
            notches(2, 10, -1.0)
                .into_iter()
                .map(|step| match step {
                    Step::Notch(_, y) => Step::Notch(y, 0.0),
                    step => step,
                })
                .collect(),
        ),
        (
            "arrow right x2",
            1.0,
            1.0,
            vec![
                Step::Key("viewer.right"),
                Step::Frames(5),
                Step::Key("viewer.right"),
            ],
        ),
        (
            "scrollbar drag",
            1.0,
            1.0,
            vec![
                Step::Seek(0.1),
                Step::Frames(2),
                Step::Seek(0.1003),
                Step::Frames(2),
                Step::Seek(0.25),
            ],
        ),
    ]
}

/// What one schedule's samples show.
#[derive(Debug)]
pub(super) struct Summary {
    pub max_error: f64,
    pub final_error: f64,
    pub travel: f64,
    pub moving_frames: usize,
    pub largest_step: f64,
    pub settled: Duration,
}

pub(super) fn summarize(samples: &[Sample]) -> Summary {
    let start = samples[0].time;
    let mut summary = Summary {
        max_error: 0.0,
        final_error: 0.0,
        travel: 0.0,
        moving_frames: 0,
        largest_step: 0.0,
        settled: Duration::ZERO,
    };
    for pair in samples.windows(2) {
        let step = (pair[1].shown - pair[0].shown).abs();
        if step > 1e-6 {
            summary.moving_frames += 1;
            summary.largest_step = summary.largest_step.max(step);
            summary.settled = pair[1].time - start;
        }
    }
    for sample in samples {
        summary.max_error = summary
            .max_error
            .max((sample.shown - sample.expected).abs());
    }
    let last = samples.last().unwrap();
    summary.final_error = (last.shown - last.expected).abs();
    summary.travel = last.shown - samples[0].shown;
    summary
}

fn with_settings<T>(glide: f32, speed: f32, run: impl FnOnce() -> T) -> T {
    midnight_ui_input::set_glide_scale(glide);
    midnight_ui_input::set_wheel_speed_scale(speed);
    let result = run();
    midnight_ui_input::set_glide_scale(1.0);
    midnight_ui_input::set_wheel_speed_scale(1.0);
    result
}

/// Prints every schedule's trace and summary (the measurement behind the parity tests).
/// `cargo test -p midnight-compare-ui --lib measure_scroll_parity -- --ignored --nocapture`
#[test]
#[ignore = "measurement: prints traces"]
fn measure_scroll_parity() {
    let mut harness = Harness::new();
    println!(
        "schedule | max error px | final error px | travel px | moving frames | largest step px | settled ms"
    );
    for (name, glide, speed, steps) in schedules() {
        let samples = with_settings(glide, speed, || harness.run(&steps));
        let summary = summarize(&samples);
        println!(
            "{name} | {:.1} | {:.1} | {:.1} | {} | {:.1} | {}",
            summary.max_error,
            summary.final_error,
            summary.travel,
            summary.moving_frames,
            summary.largest_step,
            summary.settled.as_millis()
        );
        let trace: Vec<String> = samples
            .iter()
            .take(45)
            .map(|sample| format!("{:.1}/{:.1}", sample.expected, sample.shown))
            .collect();
        println!("  expected/shown per frame: {}", trace.join(" "));
    }
}

/// Every frame within a pixel of the text viewer's offset, except `latency` frames after
/// a position bar step (the viewer's text, too, waits for its engine there).
fn assert_parity(name: &str, samples: &[Sample]) {
    let summary = summarize(samples);
    let trace = || {
        samples
            .iter()
            .map(|sample| format!("{:.1}/{:.1}", sample.expected, sample.shown))
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert!(
        summary.max_error <= 1.0,
        "{name}: {summary:?}; expected/shown per frame: {}",
        trace()
    );
}

fn check(names: &[&str]) {
    let mut harness = Harness::new();
    for (name, glide, speed, steps) in schedules() {
        if !names.contains(&name) {
            continue;
        }
        let samples = with_settings(glide, speed, || harness.run(&steps));
        assert_parity(name, &samples);
    }
}

#[test]
fn wheel_notches_move_the_text_as_far_and_as_smoothly_as_in_the_text_viewer() {
    check(&["one notch", "five quick notches", "reverse mid-glide"]);
}

#[test]
fn the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer() {
    check(&[
        "glide Off",
        "glide Very long",
        "speed Very slow",
        "speed Very fast",
    ]);
}

#[test]
fn touchpad_motion_and_its_momentum_move_the_text_as_in_the_text_viewer() {
    check(&["touchpad flick"]);
}

#[test]
fn scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer() {
    check(&[
        "arrow down x3",
        "PageDown x2",
        "horizontal notch",
        "arrow right x2",
    ]);
}

/// The position bar lands where the text viewer's does (to the pixel, not the row); like
/// the viewer's, the text follows once its engine has painted the new place.
#[test]
fn the_position_bar_moves_the_text_to_the_pixel() {
    let mut harness = Harness::new();
    let (_, _, _, steps) = schedules()
        .into_iter()
        .find(|(name, ..)| *name == "scrollbar drag")
        .unwrap();
    let samples = harness.run(&steps);
    let summary = summarize(&samples);
    assert!(summary.final_error <= 1.0, "{summary:?}");
    for pair in samples.windows(2) {
        let close = |expected: f64| (pair[1].shown - expected).abs() <= 1.0;
        assert!(
            close(pair[1].expected) || close(pair[0].expected) || close(pair[0].shown),
            "{pair:?}"
        );
    }
}

/// One view moves both panes: during a glide over a painted margin, across a new surface
/// and at rest, both show the same rows at the same place.
#[test]
fn both_panes_stay_aligned_during_and_after_a_glide() {
    let mut harness = Harness::new();
    harness.reset();
    let mut samples = Vec::new();
    for step in notches(5, 3, -1.0) {
        harness.step(step, &mut samples);
        harness.panes_aligned().unwrap();
    }
    for frame in 0..60 {
        samples.push(harness.frame());
        harness
            .panes_aligned()
            .unwrap_or_else(|error| panic!("frame {frame}: {error}"));
    }
    // Five notches 24 ms apart pace their glides to 50 ms: about 150 ms of motion.
    assert!(summarize(&samples).moving_frames > 12);
    // Five notches of the platform's distance: 100 px on Windows, 120 px on Linux.
    let notch = if cfg!(windows) { 100.0 } else { 120.0 };
    assert!(
        (harness.shown() - 5.0 * notch).abs() <= 1.0,
        "{}",
        harness.shown()
    );
}

/// The cost of scrolling large comparisons: a fast five-notch spin and a touchpad flick over
/// a million lines of text and over 100 MB compared as bytes. Prints how many frames the
/// worker painted for them and what presenting those cost on the window's thread.
/// `cargo test -p midnight-compare-ui --lib measure_large_scroll_costs -- --ignored --nocapture`
#[test]
#[ignore = "measurement: writes 60 MB of text and 200 MB of bytes under _build"]
fn measure_large_scroll_costs() {
    println!(
        "files | gesture | worker paints | present ms (longest) | paint latency ms \
         (longest) | frames | UI ms per frame (longest)"
    );
    for files in [Files::Text(1_000_000), Files::Bytes(100 << 20)] {
        let mut harness = Harness::with(files);
        for (name, steps) in [
            ("five quick notches", notches(5, 3, -1.0)),
            ("touchpad flick", flick()),
        ] {
            harness.reset();
            harness.costs = Costs::default();
            let mut samples = Vec::new();
            let mut frame_times = Vec::new();
            for step in steps {
                harness.step(step, &mut samples);
            }
            for _ in 0..90 {
                let started = Instant::now();
                advance(FRAME);
                harness.native(WindowEvent::RedrawRequested);
                slint::platform::update_timers_and_animations();
                harness.present(Duration::ZERO);
                harness.draw();
                frame_times.push(started.elapsed());
                // The worker paints at its own pace meanwhile.
                std::thread::sleep(FRAME);
                harness.present(FRAME);
            }
            let costs = &harness.costs;
            let ui: Duration = frame_times.iter().sum();
            println!(
                "{files:?} | {name} | {} | {:.1} ({:.1}) | {:.1} ({:.1}) | {} | {:.2} ({:.1})",
                costs.paints,
                costs.present.as_secs_f64() * 1000.0,
                costs.longest_present.as_secs_f64() * 1000.0,
                costs.latency.as_secs_f64() * 1000.0 / f64::from(costs.paints.max(1)),
                costs.longest_latency.as_secs_f64() * 1000.0,
                frame_times.len(),
                ui.as_secs_f64() * 1000.0 / frame_times.len() as f64,
                frame_times.iter().max().unwrap().as_secs_f64() * 1000.0,
            );
        }
    }
}
