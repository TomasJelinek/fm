//! The prepared text window under scripted scrolling: a timed co-simulation of the real
//! worker, a simulated plugin and the viewer window's retained camera at 60 Hz.
//!
//! It runs the frontier co-simulation (`worker_frontier_tests.rs`) with time: the window
//! presents every 16.7 ms frame and moves its camera within the pixels it has, while the
//! worker takes a turn only when its previous one has finished. A turn's real cost (the
//! worker's own wall time: inputs, plugin events, layout, rasterization and preparation;
//! not the simulations) keeps the worker busy for as many frames, and its outputs reach
//! the window only then, through a mailbox that keeps the newest frame. The plugin
//! answers a request a frame later. Wheel notches glide as `midnight_ui_input::WheelMotion`
//! does (an ease-out on Chromium's timing, paced by the input's interval, only undelivered
//! distance adding up); a key moves its whole distance at once; a scrollbar drag asks the
//! worker for each position.
//!
//! A frame stalls when the camera has travel left but cannot move: the pixels it needs
//! are not prepared yet. Counters of the worker's layouts and rasterized pixels
//! (`prepare_counters.rs`) show the work a gesture caused.
use super::super::prepare_counters;
use super::frontier::{Engine, Window, rows, rust, settle};
use super::highlight::{Lexer, cpp, highlight_cpp, shows_plain_rows};
use super::*;
use std::fmt::Write as _;
use std::time::Duration;

const FRAME: Duration = Duration::from_nanos(16_666_667);
/// `midnight_ui_input::WheelMotion`: the longest frame time a glide advances by.
const MAX_GLIDE_FRAME: Duration = Duration::from_millis(50);

/// The wheel glides the co-simulation can run.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum GlideModel {
    /// `midnight_ui_input::WheelMotion` (`wheel_motion.rs`, `glide.rs`): a
    /// cubic-bezier(0.42, 1, 0.58, 1) ease-out over Chromium's 200 ms up to 120 px, falling
    /// to 100 ms from 480 px; a notch glides the rest from where the glide is, never slower
    /// than it moves, and over no more than twice the interval since the previous notch (at
    /// least 50 ms).
    #[default]
    Wheel,
    /// Scrolling glide Off: each notch moves whole on its frame.
    Off,
    /// The glide before Chromium's: 200 ms of quadratic ease-out per notch, restarted for
    /// the remaining distance, each frame counted up to 16 ms.
    Quadratic,
    /// Chromium's ease-in-out, cubic-bezier(0.42, 0, 0.58, 1), a notch during a glide
    /// keeping its speed within 2.5 times the remaining distance at that speed: the wheel
    /// glide until 24 November 2026.
    Chromium,
}

impl GlideModel {
    pub(super) const ALL: [Self; 4] = [Self::Wheel, Self::Off, Self::Quadratic, Self::Chromium];

    /// `PREPARED_GLIDE=off|quadratic|chromium` measures with another glide.
    fn from_env() -> Self {
        match std::env::var("PREPARED_GLIDE").as_deref() {
            Ok("off") => Self::Off,
            Ok("quadratic") => Self::Quadratic,
            Ok("chromium") => Self::Chromium,
            _ => Self::Wheel,
        }
    }
}

/// A wheel glide of the co-simulation. Notches arrive at the start of a frame, as the
/// frame's input.
#[derive(Default)]
struct Glide {
    model: GlideModel,
    /// Seconds since the gesture began, at the end of the last presented frame.
    now: f64,
    /// When the previous notch arrived.
    input: Option<f64>,
    elapsed: f64,
    duration: f64,
    /// The Chromium curve's first control point's y (0 from rest).
    rise: f64,
    distance: f64,
    delivered: f64,
    active: bool,
}

fn bezier(t: f64, a: f64, b: f64) -> f64 {
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

fn bezier_slope(t: f64, a: f64, b: f64) -> f64 {
    let u = 1.0 - t;
    3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
}

/// The curve parameter whose x is `x`, by bisection.
fn parameter(x: f64) -> f64 {
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..50 {
        let middle = (low + high) / 2.0;
        if bezier(middle, 0.42, 0.58) < x {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

/// Chromium's inverse-delta duration for `distance` pixels, seconds.
fn chromium_duration(distance: f64) -> f64 {
    (12.0 - (distance.abs() - 120.0) / 60.0).clamp(6.0, 12.0) / 60.0
}

impl Glide {
    fn new(model: GlideModel) -> Self {
        Self {
            model,
            ..Self::default()
        }
    }

    /// The glide's progress at time fraction `fraction`.
    fn eased(&self, fraction: f64) -> f64 {
        if fraction >= 1.0 {
            return 1.0;
        }
        match self.model {
            GlideModel::Quadratic => 1.0 - (1.0 - fraction) * (1.0 - fraction),
            GlideModel::Chromium => bezier(parameter(fraction), self.rise, 1.0),
            GlideModel::Wheel | GlideModel::Off => {
                let rest = 1.0 - parameter(fraction);
                1.0 - rest * rest * rest
            }
        }
    }

    /// The current speed, pixels a second.
    fn speed(&self, fraction: f64) -> f64 {
        if !self.active || fraction >= 1.0 || self.duration <= 0.0 {
            return 0.0;
        }
        let t = parameter(fraction);
        let slope = match self.model {
            GlideModel::Chromium => bezier_slope(t, self.rise, 1.0),
            _ => 3.0 * (1.0 - t) * (1.0 - t),
        };
        self.distance * slope / bezier_slope(t, 0.42, 0.58) / self.duration
    }

    fn push(&mut self, pixels: f64) {
        let fraction = if self.active && self.duration > 0.0 {
            (self.elapsed / self.duration).min(1.0)
        } else {
            1.0
        };
        match self.model {
            GlideModel::Off => {
                let mut remaining = self.distance - self.delivered;
                if remaining.signum() != pixels.signum() {
                    remaining = 0.0;
                }
                self.distance = remaining + pixels;
                self.delivered = 0.0;
                self.elapsed = 0.0;
                self.duration = 0.0;
            }
            GlideModel::Quadratic | GlideModel::Chromium => {
                let speed = self.speed(fraction);
                let mut remaining = self.distance - self.delivered;
                if remaining.signum() != pixels.signum() {
                    remaining = 0.0;
                }
                self.distance = (remaining + pixels).clamp(-2400.0, 2400.0);
                self.delivered = 0.0;
                self.elapsed = 0.0;
                if self.model == GlideModel::Quadratic {
                    self.duration = 0.2;
                } else {
                    self.duration = chromium_duration(self.distance);
                    self.rise = 0.0;
                    if speed != 0.0 && speed.signum() == self.distance.signum() {
                        self.duration = self.duration.min(2.5 * self.distance / speed);
                        self.rise = (0.42 * speed * self.duration / self.distance).clamp(0.0, 1.0);
                    }
                }
            }
            GlideModel::Wheel => {
                let speed = self.speed(fraction);
                let position = if self.active {
                    self.distance * self.eased(fraction)
                } else {
                    self.delivered
                };
                let mut remaining = self.distance - position;
                let mut owed = position - self.delivered;
                if (remaining + owed).signum() != pixels.signum() {
                    remaining = 0.0;
                    owed = 0.0;
                }
                self.distance = remaining + pixels;
                self.delivered = -owed;
                self.elapsed = 0.0;
                self.duration = chromium_duration(self.distance);
                if let Some(previous) = self.input.replace(self.now) {
                    self.duration = self.duration.min((2.0 * (self.now - previous)).max(0.05));
                }
                if speed != 0.0 && speed.signum() == self.distance.signum() {
                    self.duration = self.duration.min(self.distance / 0.42 / speed);
                }
            }
        }
        self.active = true;
    }

    /// The distance of one presented frame.
    fn tick(&mut self) -> f64 {
        self.now += FRAME.as_secs_f64();
        if !self.active {
            return 0.0;
        }
        let cap = if self.model == GlideModel::Quadratic {
            Duration::from_millis(16)
        } else {
            MAX_GLIDE_FRAME
        };
        self.elapsed += FRAME.min(cap).as_secs_f64();
        let fraction = if self.duration > 0.0 {
            (self.elapsed / self.duration).min(1.0)
        } else {
            1.0
        };
        let position = self.distance * self.eased(fraction);
        let delta = position - self.delivered;
        self.delivered = position;
        if fraction >= 1.0 {
            self.active = false;
        }
        delta
    }
}

/// One frame's input.
#[derive(Clone, Copy, Debug)]
pub(super) enum Input {
    /// Wheel notches (or a touchpad's fraction of one), in physical pixels: they glide.
    Wheel(f64),
    /// A key's whole distance, in physical pixels.
    Key(f64),
    /// A scrollbar position.
    Position(f64),
}

/// A gesture: the inputs of each frame, frame by frame.
pub(super) struct Script {
    pub(super) name: &'static str,
    pub(super) frames: Vec<Vec<Input>>,
}

pub(super) fn every(frames: usize, interval: usize, input: Input) -> Vec<Vec<Input>> {
    (0..frames * interval)
        .map(|frame| {
            if frame % interval == 0 {
                vec![input]
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// The measured gestures for a window `height` physical pixels tall, with wheel notches of
/// `notch` physical pixels.
pub(super) fn scripts(notch: f64, height: f64) -> Vec<Script> {
    // A Windows precision touchpad flick arrives as wheel events whose rate decays after
    // the lift: here 30 notches per second at first, decaying with 0.5 s, for 1.5 s.
    let fling = (0..90)
        .map(|frame| {
            let seconds = frame as f64 / 60.0;
            vec![Input::Wheel(notch * 30.0 * (-seconds / 0.5).exp() / 60.0)]
        })
        .collect();
    vec![
        Script {
            name: "wheel 12 notches, 50 ms apart",
            frames: every(12, 3, Input::Wheel(notch)),
        },
        Script {
            name: "wheel 40 notches, 25 ms apart",
            // Notch k arrives at k * 25 ms, in frame k * 25 / 16.7.
            frames: (0..60)
                .map(|frame| {
                    let notches = (0..40).filter(|k| k * 25 * 60 / 1000 == frame).count();
                    vec![Input::Wheel(notch * notches as f64)]
                })
                .collect(),
        },
        Script {
            name: "touchpad flick",
            frames: fling,
        },
        Script {
            name: "PageDown x8, 120 ms apart",
            frames: every(8, 7, Input::Key(height)),
        },
        Script {
            name: "wheel back up 12 notches",
            frames: every(12, 3, Input::Wheel(-notch)),
        },
        Script {
            name: "scrollbar drag 0.30 to 0.36",
            frames: (0..60)
                .map(|frame| vec![Input::Position(0.30 + 0.001 * frame as f64)])
                .collect(),
        },
    ]
}

/// What one gesture cost and how it looked.
#[derive(Default)]
pub(super) struct Report {
    frames: usize,
    /// Frames on which the camera had travel left and did not move.
    pub(super) stalls: usize,
    /// How far the shown camera trailed the input, physical pixels, per frame.
    lag: Vec<f64>,
    /// The worker's turns, milliseconds.
    pub(super) turns: Vec<f64>,
    counts: prepare_counters::Counts,
    /// Pixel bytes the window received that it had not shown before.
    uploads: u64,
    /// The input's distance and how far the content moved.
    travel: f64,
    pub(super) moved: f64,
    reads: usize,
    /// The most bytes the viewer's prepared strips held at once.
    peak_bytes: usize,
    /// Strips the window was given again, with a new ID, for rows it already had.
    repainted: usize,
    /// The work of each frame.
    frame_counts: Vec<prepare_counters::Counts>,
    /// `repainted` after each frame.
    repainted_by_frame: Vec<usize>,
    /// Source highlighting: preparations and refreshes, the times the notice began to show,
    /// and frames that showed a row with a token uncoloured (`worker_highlight_tests.rs`).
    preparations: usize,
    refreshes: usize,
    notices: usize,
    plain_frames: usize,
}

pub(super) fn percentile(values: &[f64], percent: usize) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

impl Report {
    pub(super) fn line(&self, name: &str) -> String {
        format!(
            "{name:<30} frames {:>4}, stalls {:>3}, lag p50/p95/max {:>6.1}/{:>6.1}/{:>6.1} px, \
             turns {:>3} p50/p95/max {:>5.2}/{:>6.2}/{:>6.2} ms, total {:>7.1} ms, \
             layouts {:>3} ({:>4} KiB), rasters {:>3} ({:>6.2} Mpx), uploaded {:>6.1} MiB, \
             reads {:>3}, moved {:.0} of {:.0} px, strips peak {:.1} MiB, \
             preparations {}, refreshes {}, notices {}, plain frames {}",
            self.frames,
            self.stalls,
            percentile(&self.lag, 50),
            percentile(&self.lag, 95),
            percentile(&self.lag, 100),
            self.turns.len(),
            percentile(&self.turns, 50),
            percentile(&self.turns, 95),
            percentile(&self.turns, 100),
            self.turns.iter().sum::<f64>(),
            self.counts.layouts,
            self.counts.layout_bytes / 1024,
            self.counts.rasters,
            self.counts.raster_pixels as f64 / 1e6,
            self.uploads as f64 / (1024.0 * 1024.0),
            self.reads,
            self.moved,
            self.travel,
            self.peak_bytes as f64 / (1024.0 * 1024.0),
            self.preparations,
            self.refreshes,
            self.notices,
            self.plain_frames,
        )
    }
}

/// The worker's outputs on their way to the window, and the pixels it showed.
#[derive(Default)]
struct Timeline {
    busy_until: u32,
    held: Vec<(u32, ViewerOutput, Option<f64>)>,
    shown: Option<(u64, u64)>,
    /// Prepared strips the window has images of (`raster_presentation::receive_strips`).
    strips: std::collections::HashSet<u64>,
    /// The newest strip painted before the gesture: older ones are already shown.
    painted_before: u64,
    /// The ID of the strip the window was last given at each world row.
    tops: std::collections::HashMap<i64, u64>,
    repainted: usize,
}

impl Timeline {
    /// Pixel bytes `frame` brings that the window did not have.
    fn uploads(&mut self, frame: &ViewerFrame) -> u64 {
        let Some(info) = &frame.raster else {
            return frame.pixels.len() as u64;
        };
        let key = (info.surface.0, info.pixel_id);
        if self.shown == Some(key) {
            return 0;
        }
        self.shown = Some(key);
        if let Some(bands) = &info.bands {
            let mut bytes = 0;
            let mut shown = std::collections::HashSet::new();
            for band in bands.list.iter() {
                if let Some(old) = self.tops.insert(band.top as i64, band.id)
                    && old != band.id
                {
                    self.repainted += 1;
                }
                if band.id > self.painted_before && !self.strips.contains(&band.id) {
                    bytes += band.pixels.len() as u64
                        + band.gutter.as_ref().map_or(0, |gutter| gutter.len() as u64);
                }
                shown.insert(band.id);
            }
            self.strips = shown;
            return bytes;
        }
        frame.pixels.len() as u64 + info.gutter.as_ref().map_or(0, |g| g.pixels.len() as u64)
    }
}

/// One timed frame: due outputs reach the window, the window presents, and a worker that
/// is not busy takes a turn, every `period` frames, whose cost decides when its outputs
/// arrive.
fn timed_frame(engine: &mut Engine, window: &mut Window, line: &mut Timeline, report: &mut Report) {
    engine.advance_clock();
    let tick = engine.tick;
    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut line.held)
        .into_iter()
        .partition(|(at, _, _)| *at <= tick);
    line.held = later;
    for (_, output, offset) in due {
        if let ViewerOutput::Frame(frame) = &output {
            report.uploads += line.uploads(frame);
        }
        window.receive(output, offset);
    }
    window.tick(engine);
    if line.busy_until <= tick && tick.is_multiple_of(engine.period) {
        let before = engine.cost;
        engine.apply_inputs();
        engine.answer();
        let outputs = engine.paint();
        engine.prepare_source();
        engine.observe();
        let cost = engine.cost - before;
        let frames = (cost.as_secs_f64() / FRAME.as_secs_f64()).ceil().max(1.0) as u32;
        if !cost.is_zero() {
            report.turns.push(cost.as_secs_f64() * 1000.0);
        }
        line.busy_until = tick + frames;
        for (output, offset) in outputs {
            // The mailbox keeps the newest frame and the newest raster control output.
            let raster = |output: &ViewerOutput| {
                matches!(
                    output,
                    ViewerOutput::RasterRebase { .. } | ViewerOutput::RasterUnavailable { .. }
                )
            };
            if matches!(output, ViewerOutput::Frame(_)) {
                line.held
                    .retain(|(_, old, _)| !matches!(old, ViewerOutput::Frame(_)));
            } else if raster(&output) {
                line.held.retain(|(_, old, _)| !raster(old));
            }
            line.held.push((tick + frames, output, offset));
        }
    }
    engine.tick += 1;
}

/// Run `script` from rest and report it; the content then comes to rest again.
pub(super) fn run(engine: &mut Engine, window: &mut Window, script: &Script) -> Report {
    run_with(engine, window, script, GlideModel::from_env())
}

/// [`run`] with the wheel glide `model`.
pub(super) fn run_with(
    engine: &mut Engine,
    window: &mut Window,
    script: &Script,
    model: GlideModel,
) -> Report {
    assert!(
        settle(engine, window),
        "the view did not come to rest before {:?}: window active {}, worker dirty {}, \
         strips pending {}, busy {}, log {:?}, dropped {:?}",
        script.name,
        window.active(),
        engine.worker.dirty,
        engine.worker.raster.strips_pending(),
        engine.busy(),
        window.log.iter().rev().take(6).collect::<Vec<_>>(),
        window.dropped,
    );
    let mut report = Report::default();
    let highlighting = (engine.preparations, engine.refreshes, engine.notices);
    // Strips painted before the gesture reached the window while it came to rest.
    let mut line = Timeline {
        painted_before: engine.worker.raster.last_strip_id(),
        ..Timeline::default()
    };
    let mut glide = Glide::new(model);
    let start = window.shown();
    let reads = engine.reads;
    let mut ideal = start;
    let mut previous = start;
    let positional = script
        .frames
        .iter()
        .flatten()
        .any(|input| matches!(input, Input::Position(_)));
    prepare_counters::take();
    let mut frame = 0;
    loop {
        let inputs = script.frames.get(frame).cloned().unwrap_or_default();
        for input in &inputs {
            match *input {
                Input::Wheel(pixels) => {
                    glide.push(pixels);
                    report.travel += pixels;
                }
                Input::Key(pixels) => {
                    window.push(engine, pixels, true);
                    ideal += pixels;
                    report.travel += pixels;
                }
                Input::Position(position) => window.seek(engine, position),
            }
        }
        let step = glide.tick();
        if step != 0.0 {
            window.push(engine, step, true);
            ideal += step;
        }
        window.gliding = glide.active;
        timed_frame(engine, window, &mut line, &mut report);
        report.plain_frames += usize::from(engine.lexer.is_some() && shows_plain_rows(engine));
        report.frame_counts.push(prepare_counters::take());
        report.repainted_by_frame.push(line.repainted);
        report.peak_bytes = report.peak_bytes.max(
            engine
                .worker
                .raster_budget
                .strip_bytes(engine.worker.raster_owner),
        );
        let shown = window.shown();
        if window.active() && (shown - previous).abs() < 1e-6 {
            report.stalls += 1;
        }
        if !positional {
            report.lag.push((ideal - shown).abs());
        }
        previous = shown;
        frame += 1;
        let done = frame >= script.frames.len()
            && !glide.active
            && !window.active()
            && line.held.is_empty()
            && !engine.busy()
            && !engine.worker.dirty;
        if done || frame > 3000 {
            break;
        }
    }
    report.frames = frame;
    report.repainted = line.repainted;
    for counts in &report.frame_counts {
        report.counts.layouts += counts.layouts;
        report.counts.layout_bytes += counts.layout_bytes;
        report.counts.rasters += counts.rasters;
        report.counts.raster_pixels += counts.raster_pixels;
    }
    report.moved = window.shown() - start;
    report.reads = engine.reads - reads;
    report.preparations = engine.preparations - highlighting.0;
    report.refreshes = engine.refreshes - highlighting.1;
    report.notices = engine.notices - highlighting.2;
    report
}

pub(super) fn geometry(width: u32, height: u32, scale: f32) -> ViewerGeometry {
    ViewerGeometry {
        width,
        height,
        scale,
    }
}

/// Colour spans as the Source viewer sends them for the generated Rust: keywords,
/// strings, numbers and comments, at most `viewer::MAX_SPANS`.
fn highlight(page: &mut viewer::TextPage) {
    let text = page.text.clone();
    let mut spans = Vec::new();
    let mut at = 0;
    let bytes = text.as_bytes();
    while at < bytes.len() && spans.len() < viewer::MAX_SPANS {
        let rest = &text[at..];
        let (length, role) = if rest.starts_with("//") {
            (rest.find('\n').unwrap_or(rest.len()), viewer::STYLE_COMMENT)
        } else if let Some(quoted) = rest.strip_prefix('"') {
            (
                quoted.find('"').map_or(rest.len(), |end| end + 2),
                viewer::STYLE_STRING,
            )
        } else if bytes[at].is_ascii_digit() {
            (
                rest.find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len()),
                viewer::STYLE_NUMBER,
            )
        } else if ["let ", "fn ", "->"]
            .iter()
            .any(|word| rest.starts_with(word))
        {
            (
                rest.find([' ', '>']).map_or(rest.len(), |end| end + 1),
                viewer::STYLE_KEYWORD,
            )
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        spans.push(viewer::Span {
            start: at as u32,
            end: (at + length) as u32,
            role,
            flags: 0,
        });
        at += length.max(1);
    }
    page.spans = spans;
}

/// Measurement: stalls, lag, worker turns and preparation work of wheel, touchpad, key
/// and scrollbar gestures over large files, in Text and Source, at 1x and 2x.
#[test]
#[ignore = "measurement; prints the cost and smoothness of scripted gestures"]
fn measure_prepared_text_scrolling() {
    let notch: f64 = std::env::var("PREPARED_NOTCH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(100.0);
    let mut report = String::new();
    for (scale, size) in [
        (1.0, geometry(1312, 720, 1.0)),
        (2.0, geometry(2624, 1440, 2.0)),
    ] {
        for (name, mode, source, wrap) in [
            (
                "Text, 60-byte rows, 100 MB",
                viewer::MODE_TEXT,
                rows(1_700_000, 60),
                false,
            ),
            (
                "Text, 2400-byte rows, wrapped",
                viewer::MODE_TEXT,
                rows(4000, 2400),
                true,
            ),
            (
                "Text, 2400-byte rows",
                viewer::MODE_TEXT,
                rows(4000, 2400),
                false,
            ),
            ("Source, Rust", viewer::MODE_SOURCE, rust(200_000), false),
        ] {
            let mut engine = Engine::open(source, mode, size, 1);
            if wrap {
                engine = engine.wrapped();
            }
            if mode == viewer::MODE_SOURCE {
                engine.highlight_replies = Some(highlight);
            }
            let mut window = Window::default();
            assert!(settle(&mut engine, &mut window));
            window.seek(&mut engine, 0.3);
            assert!(settle(&mut engine, &mut window));
            let _ = writeln!(report, "-- {name} at {scale}x, notch {notch} px");
            for script in scripts(notch * scale, f64::from(size.height)) {
                let measured = run(&mut engine, &mut window, &script);
                let _ = writeln!(report, "{}", measured.line(script.name));
            }
        }
    }
    println!("{report}");
}

/// A Text view of short rows, opened at the file's start and at rest: its prepared window
/// is painted.
fn prepared_text(geometry: ViewerGeometry) -> (Engine, Window) {
    let mut engine = Engine::open(rows(20_000, 60), viewer::MODE_TEXT, geometry, 1);
    let mut window = Window::default();
    assert!(settle(&mut engine, &mut window));
    (engine, window)
}

/// Scroll by `pixels` in wheel notches of `notch` and come to rest; the work it caused.
fn wheel(
    engine: &mut Engine,
    window: &mut Window,
    pixels: f64,
    notch: f64,
) -> (Report, prepare_counters::Counts) {
    wheel_with(engine, window, pixels, notch, GlideModel::from_env())
}

/// [`wheel`] with the wheel glide `model`.
fn wheel_with(
    engine: &mut Engine,
    window: &mut Window,
    pixels: f64,
    notch: f64,
    model: GlideModel,
) -> (Report, prepare_counters::Counts) {
    let count = (pixels.abs() / notch).round() as usize;
    let script = Script {
        name: "wheel",
        frames: every(count, 3, Input::Wheel(notch.copysign(pixels))),
    };
    let report = run_with(engine, window, &script, model);
    let counts = report.counts;
    (report, counts)
}

/// Turning back and forth inside the prepared window shows prepared strips only: no page is
/// read or laid out and no strip is painted again. Before the window was prepared, the
/// retained raster was painted again whenever the camera neared its 256-pixel guards. With
/// every wheel glide (the current one, Off and the two before it) and at every scale.
#[test]
fn scrolling_inside_the_prepared_window_prepares_nothing_again() {
    let mut failures = Vec::new();
    for model in GlideModel::ALL {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let size = geometry(
                (1000.0 * scale) as u32,
                (720.0 * scale) as u32,
                scale as f32,
            );
            let (mut engine, mut window) = prepared_text(size);
            let screen = f64::from(size.height);
            let notch = 100.0 * scale;
            // The first turn down paints the strips its window newly reaches.
            let (first, _) = wheel_with(&mut engine, &mut window, 0.75 * screen, notch, model);
            assert!(
                (first.moved - first.travel).abs() < 1.0,
                "{model:?} at {scale}x: moved {} of {}",
                first.moved,
                first.travel
            );
            for turn in 0..3 {
                let reads = engine.reads;
                let (up, up_counts) =
                    wheel_with(&mut engine, &mut window, -0.75 * screen, notch, model);
                let (down, down_counts) =
                    wheel_with(&mut engine, &mut window, 0.75 * screen, notch, model);
                assert!(
                    (up.moved - up.travel).abs() < 1.0 && (down.moved - down.travel).abs() < 1.0,
                    "{model:?} at {scale}x, turn {turn}: moved {} and {}",
                    up.moved,
                    down.moved
                );
                let prepared = (up_counts, down_counts, engine.reads - reads);
                if prepared
                    != (
                        prepare_counters::Counts::default(),
                        prepare_counters::Counts::default(),
                        0,
                    )
                {
                    failures.push(format!("{model:?} at {scale}x, turn {turn}: {prepared:?}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "scrolling inside the prepared window prepared content again:\n{}",
        failures.join("\n")
    );
}

/// The window moves over prepared strips on every frame although the worker answers only
/// every twelfth frame: no frame waits for preparation that was done before the gesture.
#[test]
fn a_slow_worker_never_stalls_motion_inside_the_prepared_window() {
    let size = geometry(1000, 720, 1.0);
    let (mut engine, mut window) = prepared_text(size);
    engine.period = 12;
    // Ten notches of 100 px, 50 ms apart: 1000 px, inside the two screens prepared below.
    let (report, counts) = wheel(&mut engine, &mut window, 1000.0, 100.0);
    assert_eq!(
        report.stalls,
        0,
        "frames waited for pixels: lag p95 {:.1} px, max {:.1} px",
        percentile(&report.lag, 95),
        percentile(&report.lag, 100)
    );
    assert!(
        (report.moved - 1000.0).abs() < 1.0,
        "moved {}",
        report.moved
    );
    assert!(
        percentile(&report.lag, 100) < 1.0,
        "the content trailed the wheel by {:.1} px",
        percentile(&report.lag, 100)
    );
    // Moving down a screen and a half extends the window: new strips only, no layout yet.
    assert_eq!(counts.layouts, 0, "{counts:?}");
}

/// A refill keeps every strip whose rows its page holds the same way: over six screens
/// in one gesture the window is given a strip again only at a page's edges, where a page
/// held some of the rows whose ink reaches the strip and not others (at most two strips at
/// the old page's end and two at the new page's start), and the worker paints the new
/// strips and those few. Which frames reach the window depends on the worker's timing,
/// so the count is bounded, not exact; without reuse a refill gives the window six strips
/// again, above the bound.
#[test]
fn a_refill_keeps_the_strips_of_rows_it_shows_unchanged() {
    let size = geometry(1000, 720, 1.0);
    let (mut engine, mut window) = prepared_text(size);
    let screen = f64::from(size.height);
    let strips_per_screen = (screen / f64::from(super::super::raster::BAND_ROWS)).ceil() as u64;
    let (report, counts) = wheel(&mut engine, &mut window, 6.0 * screen, 100.0);
    assert!(
        (report.moved - report.travel).abs() < 1.0,
        "moved {} of {}",
        report.moved,
        report.travel
    );
    let refills = counts.layouts as usize;
    assert!(refills > 0, "the travel crossed no page");
    assert!(
        report.repainted <= 4 * refills,
        "the window was given {} strips again for rows it had, over {refills} refills",
        report.repainted
    );
    // Six screens of travel need six screens of new strips ahead; painting the shown
    // window again at each refill would add four screens per refill.
    assert!(
        counts.rasters <= 6 * strips_per_screen + 3 * refills as u64 + 3,
        "{} strips painted over six screens with {refills} refills",
        counts.rasters
    );
}

/// The largest window (the 4-megapixel geometry cap at 200 %) prepares only what the
/// viewer's strip budget holds, and travel over many screens stays within it.
#[test]
fn prepared_strips_stay_within_the_viewer_budget() {
    let size = geometry(2048, 2048, 2.0);
    let (mut engine, mut window) = prepared_text(size);
    let owner = engine.worker.raster_owner;
    let budget = engine.worker.raster_budget.clone();
    for _ in 0..4 {
        let (report, _) = wheel(&mut engine, &mut window, 2048.0, 200.0);
        assert!((report.moved - report.travel).abs() < 1.0);
        let bytes = budget.strip_bytes(owner);
        assert!(
            bytes <= crate::raster_budget::MAX_STRIP_BYTES,
            "{} MiB of strips",
            bytes / (1024 * 1024)
        );
    }
}

/// What one viewer mode costs on the worker for a page of several screens, and for a
/// glide over it in 24-pixel camera steps (a wheel glide's frames).
fn measure_mode(name: &str, mode: u16, page: Page, size: ViewerGeometry, report: &mut String) {
    let (mut worker, output) = super::worker();
    worker.mode = mode;
    worker.binary = mode == viewer::MODE_BINARY;
    worker.geometry = size;
    worker.image_size = [size.width, size.height];
    worker.length = page.end;
    worker.input(ViewerInput::RetainedRaster(true), &output);
    prepare_counters::take();
    let started = std::time::Instant::now();
    assert!(worker.set_page(page, &output), "{name}: page laid out");
    let layout = started.elapsed();
    let started = std::time::Instant::now();
    worker.paint(&output).unwrap();
    let first = started.elapsed();
    let opened = prepare_counters::take();
    let started = std::time::Instant::now();
    prepare_all(&mut worker, &output);
    let prepared = started.elapsed();
    let window = prepare_counters::take();
    let surface = output
        .take()
        .into_iter()
        .rev()
        .find_map(|message| match message {
            ViewerOutput::Frame(frame) => frame.raster.map(|info| info.surface),
            _ => None,
        })
        .expect("retained frame");
    let mut steps = Vec::new();
    for step in 1..=30 {
        let started = std::time::Instant::now();
        worker.input(
            ViewerInput::Raster {
                surface,
                input: ViewerRasterInput::PanTo {
                    sequence: step,
                    origin: [0.0, 24.0 * step as f64],
                    settled: false,
                },
            },
            &output,
        );
        if worker.dirty {
            worker.dirty = false;
            worker.paint(&output).unwrap();
        }
        prepare_all(&mut worker, &output);
        steps.push(started.elapsed().as_secs_f64() * 1000.0);
        output.take();
    }
    let glide = prepare_counters::take();
    let _ = writeln!(
        report,
        "{name:<28} layout {:>6.2} ms ({:>3} KiB), first frame {:>6.2} ms ({:>5.2} Mpx), \
         window {:>6.2} ms ({:>5.2} Mpx), 30 glide steps p50/p95/max {:>5.2}/{:>5.2}/{:>5.2} ms, \
         {} rasters ({:.2} Mpx), {} layouts",
        layout.as_secs_f64() * 1000.0,
        opened.layout_bytes / 1024,
        first.as_secs_f64() * 1000.0,
        opened.raster_pixels as f64 / 1e6,
        prepared.as_secs_f64() * 1000.0,
        window.raster_pixels as f64 / 1e6,
        percentile(&steps, 50),
        percentile(&steps, 95),
        percentile(&steps, 100),
        glide.rasters,
        glide.raster_pixels as f64 / 1e6,
        glide.layouts,
    );
}

/// Run strip preparation to its end, publishing what it paints (the worker loop).
fn prepare_all(worker: &mut Worker, output: &Mailbox) {
    for _ in 0..200 {
        let pending = worker.prepare_strips();
        if worker.dirty {
            worker.dirty = false;
            worker.paint(output).unwrap();
        }
        if !pending && !worker.dirty {
            return;
        }
    }
}

/// A whole text as one page, larger than a plugin reply may be (the worker's page joins
/// several): one anchor per byte of ASCII text.
fn text_page(text: String, spans: bool, appearance: &ViewerAppearance) -> Page {
    let mut page = viewer::TextPage {
        revision: 1,
        source: viewer::SourceRange {
            offset: 0,
            length: text.len() as u64,
        },
        anchors: (0..=text.len())
            .map(|at| viewer::Anchor {
                text_offset: at as u32,
                source_offset: at as u64,
            })
            .collect(),
        text,
        spans: vec![],
        has_before: false,
        has_after: false,
        partial_line: false,
    };
    if spans {
        highlight(&mut page);
    }
    Page::text(page, appearance)
}

fn markdown_page(appearance: &ViewerAppearance) -> Page {
    use midnight_plugin_host::api::document;
    let mut blocks = Vec::new();
    let mut offset = 0_u64;
    for index in 0..120 {
        let (kind, level, text) = match index % 4 {
            0 => (document::BLOCK_HEADING, 2, format!("Section {index}")),
            1 => (
                document::BLOCK_PARAGRAPH,
                0,
                format!(
                    "Paragraph {index}: {}",
                    "words of prose that wrap ".repeat(8)
                ),
            ),
            2 => (
                document::BLOCK_CODE,
                0,
                format!("fn example_{index}() -> u64 {{\n    {index}\n}}"),
            ),
            _ => (
                document::BLOCK_QUOTE,
                0,
                format!("Quoted {index}: {}", "a remark ".repeat(6)),
            ),
        };
        let length = text.len() as u64;
        blocks.push(document::Block {
            kind,
            heading_level: level,
            depth: 0,
            links: vec![],
            table: None,
            strikethrough: None,
            text: viewer::TextPage {
                revision: 1,
                source: viewer::SourceRange { offset, length },
                anchors: text
                    .char_indices()
                    .map(|(at, _)| at)
                    .chain(std::iter::once(text.len()))
                    .map(|at| viewer::Anchor {
                        text_offset: at as u32,
                        source_offset: offset + at as u64,
                    })
                    .collect(),
                text,
                spans: vec![],
                has_before: false,
                has_after: false,
                partial_line: false,
            },
        });
        offset += length + 2;
    }
    Page::document(
        document::DocumentPage {
            revision: 1,
            source: viewer::SourceRange {
                offset: 0,
                length: offset,
            },
            has_before: false,
            has_after: false,
            blocks,
        },
        appearance,
    )
}

fn binary_page() -> Page {
    let bytes: Vec<u8> = (0..16 * 1024)
        .map(|index| (index * 7 % 251) as u8)
        .collect();
    Page::binary(
        viewer::BinaryPage {
            revision: 1,
            source: viewer::SourceRange {
                offset: 0,
                length: bytes.len() as u64,
            },
            has_before: false,
            has_after: false,
        },
        &bytes,
    )
}

/// Measurement: layout, first frame, window preparation and a glide's worker cost per
/// mode, without a plugin: Text, Source with colours, Markdown (GitHub) and hex.
#[test]
#[ignore = "measurement; prints worker costs per mode"]
fn measure_prepared_modes() {
    let mut report = String::new();
    let appearance = super::worker().0.appearance;
    for (scale, size) in [
        (1.0, geometry(1312, 720, 1.0)),
        (2.0, geometry(2624, 1440, 2.0)),
    ] {
        let _ = writeln!(report, "-- {scale}x");
        measure_mode(
            "Text, 60-byte rows",
            viewer::MODE_TEXT,
            text_page(rows(600, 60), false, &appearance),
            size,
            &mut report,
        );
        measure_mode(
            "Source, coloured Rust",
            viewer::MODE_SOURCE,
            text_page(rust(500), true, &appearance),
            size,
            &mut report,
        );
        measure_mode(
            "Markdown, GitHub",
            viewer::MODE_DOCUMENT,
            markdown_page(&appearance),
            size,
            &mut report,
        );
        measure_mode("Hex", viewer::MODE_BINARY, binary_page(), size, &mut report);
    }
    println!("{report}");
}

/// The Text view of 100 MB of short rows at 1x, at rest at 30 % of the file.
fn large_text() -> (Engine, Window) {
    let mut engine = Engine::open(
        rows(1_700_000, 60),
        viewer::MODE_TEXT,
        geometry(1312, 720, 1.0),
        1,
    );
    let mut window = Window::default();
    assert!(settle(&mut engine, &mut window));
    window.seek(&mut engine, 0.3);
    assert!(settle(&mut engine, &mut window));
    (engine, window)
}

/// A scrollbar drag that the shown page cannot answer (a Source view reads no page while
/// the thumb moves) starts a new raster context for every position, but its page, its
/// camera and so its pixels stay the same: until a page arrives, the window is given no
/// strip again for rows it had (new strips of the prepared window may still come).
#[test]
fn a_scrollbar_drag_paints_no_strip_again_while_its_page_stays() {
    let size = geometry(1312, 720, 1.0);
    let mut engine = Engine::open(rust(200_000), viewer::MODE_SOURCE, size, 1);
    engine.highlight_replies = Some(highlight);
    let mut window = Window::default();
    assert!(settle(&mut engine, &mut window));
    window.seek(&mut engine, 0.3);
    assert!(settle(&mut engine, &mut window));
    let drag = scripts(100.0, f64::from(size.height)).remove(5);
    let report = run(&mut engine, &mut window, &drag);
    let first_page = report
        .frame_counts
        .iter()
        .position(|counts| counts.layouts > 0)
        .unwrap_or(report.frame_counts.len());
    assert!(first_page > 10, "a page arrived after {first_page} frames");
    let repainted = report.repainted_by_frame[first_page - 1];
    assert_eq!(
        repainted, 0,
        "the window was given {repainted} strips again over {first_page} frames of the same page"
    );
}

/// PageDown moves the camera a whole screen and leaves it at rest; the next page is read
/// while it rests, so eight presses 120 ms apart never wait for a page.
#[test]
fn page_down_never_waits_for_the_next_page() {
    let (mut engine, mut window) = large_text();
    let keys = scripts(100.0, 720.0).remove(3);
    let report = run(&mut engine, &mut window, &keys);
    assert!((report.moved - report.travel).abs() < 1.0);
    assert_eq!(
        report.stalls,
        0,
        "frames waited for pixels: lag p95 {:.1} px, max {:.1} px",
        percentile(&report.lag, 95),
        percentile(&report.lag, 100)
    );
}

/// Forty notches 25 ms apart (about five screens a second) never wait for a page: the
/// glide's requests read the next page while the prepared strips still cover the camera.
#[test]
fn a_fast_wheel_spin_never_waits_for_the_next_page() {
    let (mut engine, mut window) = large_text();
    let spin = scripts(100.0, 720.0).remove(1);
    let report = run(&mut engine, &mut window, &spin);
    assert!((report.moved - report.travel).abs() < 1.0);
    assert_eq!(
        report.stalls,
        0,
        "frames waited for pixels: lag p95 {:.1} px, max {:.1} px",
        percentile(&report.lag, 95),
        percentile(&report.lag, 100)
    );
}

/// Bytes the Source viewer's preparation walks per 16.7 ms frame in the co-simulation. The
/// rate is assumed, not measured: 16 MiB/s, a slow lexer on a slow disk. A walk of 256 KiB, one
/// request, then takes a frame.
const WALK_PER_FRAME: u64 = 16 * 1024 * 1024 / 60;

/// A Source view of `rows` rows of synthetic C++ through the lexical model, at rest at its
/// start, and the gestures a measurement runs on it.
fn cpp_view(rows: usize, size: ViewerGeometry, walk: u64) -> (Engine, Window) {
    let mut engine = Engine::open(cpp(rows), viewer::MODE_SOURCE, size, 1).highlighted();
    engine.lexer = Some(Lexer::new());
    engine.highlight_replies = Some(highlight_cpp);
    engine.walk_per_frame = walk;
    let mut window = Window::default();
    assert!(settle(&mut engine, &mut window));
    (engine, window)
}

/// Measurement: Source highlighting under the measured gestures, timed: stalls, lag, the
/// worker's turns, preparations, refreshes, notices and frames that showed a row uncoloured,
/// over the small C++ file, 2 MiB and 16 MB of it, at 1x and 2x. The larger ones are measured
/// after a jump (to 30 % and 80 %), which reads pages far from the lexer's checkpoints.
/// Preparation walks `PREPARED_WALK` bytes a frame (default `WALK_PER_FRAME`).
#[test]
#[ignore = "measurement; prints Source highlighting work and smoothness of scripted gestures"]
fn measure_source_highlighting() {
    let variable = |name: &str| std::env::var(name).ok();
    let notch: f64 = variable("PREPARED_NOTCH")
        .and_then(|value| value.parse().ok())
        .unwrap_or(100.0);
    let walk: u64 = variable("PREPARED_WALK")
        .and_then(|value| value.parse().ok())
        .unwrap_or(WALK_PER_FRAME);
    let mut report = String::new();
    for (scale, size) in [
        (1.0, geometry(1312, 720, 1.0)),
        (2.0, geometry(2624, 1440, 2.0)),
    ] {
        for (name, rows, jump) in [
            ("C++, 150 rows", 150, None),
            ("C++, 56,000 rows", 56_000, Some(0.3)),
            ("C++, 400,000 rows", 400_000, Some(0.8)),
        ] {
            let (mut engine, mut window) = cpp_view(rows, size, walk);
            let _ = writeln!(
                report,
                "-- {name} ({} bytes) at {scale}x, notch {notch} px, walk {walk} bytes a frame: \
                 opened with {} preparations, {} refreshes, {} notices",
                engine.source.len(),
                engine.preparations,
                engine.refreshes,
                engine.notices
            );
            let mut gestures = scripts(notch * scale, f64::from(size.height));
            if let Some(position) = jump {
                gestures.insert(
                    0,
                    Script {
                        name: if position < 0.5 {
                            "jump to 30 %"
                        } else {
                            "jump to 80 %"
                        },
                        frames: vec![vec![Input::Position(position)]],
                    },
                );
            }
            for script in gestures {
                let measured = run(&mut engine, &mut window, &script);
                let _ = writeln!(report, "{}", measured.line(script.name));
            }
        }
    }
    println!("{report}");
}

/// The owner's small C++ file scrolls without a stalled frame under every measured gesture,
/// and highlighting prepares, refreshes and notices nothing: the page holds the whole file in
/// colour. Before, every gesture's rest prepared the file again and showed the notice.
#[test]
fn a_small_source_file_stalls_no_frame_and_prepares_nothing() {
    let size = geometry(1312, 720, 1.0);
    let (mut engine, mut window) = cpp_view(150, size, WALK_PER_FRAME);
    assert_eq!(
        (engine.preparations, engine.refreshes, engine.notices),
        (0, 0, 0)
    );
    for script in scripts(100.0, f64::from(size.height)) {
        let report = run(&mut engine, &mut window, &script);
        assert_eq!(
            (
                report.stalls,
                report.preparations,
                report.refreshes,
                report.notices,
                report.plain_frames
            ),
            (0, 0, 0, 0, 0),
            "{}",
            report.line(script.name)
        );
    }
}
