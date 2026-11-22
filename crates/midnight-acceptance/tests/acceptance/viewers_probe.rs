//! Measurements on viewer windows driven with real pointer, wheel and key input: the
//! scrollbars and drags on them (and the frames shown during a drag), Slint pixel
//! deltas, native wheel notches, focus changes and system settings through the viewers'
//! acceptance seam, waits until the content settled, the byte range a text page covers,
//! how far rendered content moved between two frames, the colours of rendered text and
//! their contrast against the product's minima, how sharp rendered text is, and the UI
//! review's layout checks on content windows.
use super::{Canvas, Failures, Viewing, paint_tool_windows};
use crate::{Context, app::App, inspect, report};
use midnight_core::PreferenceTheme;
use midnight_viewer_ui::acceptance_seams::{self as seams, NativeWheel};
pub use midnight_viewer_ui::acceptance_seams::{SettingsGuard, SystemSettings, WheelSetting};
use slint::{
    LogicalPosition,
    platform::{PointerEventButton, WindowEvent},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// The pixel delta of one synthetic Slint `PointerScrolled` event this harness sends.
/// It is not the product's wheel notch: production converts a winit `LineDelta` notch
/// itself (`midnight_ui_input::notch_pixels_for`: 100 logical pixels at the system's
/// default three lines per notch, in proportion to other settings), which only a native
/// event reaches ([`native_notches`], through the acceptance seam). A Slint pixel delta
/// reaches the content as pixels, so checks with it prove that "a pixel delta moves
/// exactly its pixels".
pub const SLINT_PIXEL_DELTA: f32 = 60.0;
/// The logical pixels of one wheel notch at the default three lines per notch
/// (`midnight_ui_input::notch_pixels_for`, `LINE_PIXELS`).
pub const NOTCH_PIXELS: f32 = 100.0;
/// The share of matching rows below which a measured shift is not trusted.
pub const SHIFT_CONFIDENCE: f32 = 0.8;

/// The viewer windows the application opened: the acceptance seam reaches a window
/// through them.
fn viewer_windows(app: &App) -> Result<std::rc::Rc<midnight_viewer_ui::ViewerWindows>, String> {
    Ok(super::plugins(app)?.viewers.clone())
}

/// Turn a mouse wheel by `notches` (positive: towards the end) over `at`, one native
/// report (winit's `LineDelta`, one line per notch) after the other, as winit delivers
/// them to the viewer's own native observer (`midnight_viewer_ui::acceptance_seams`).
pub fn native_notches(
    app: &App,
    viewing: &Viewing,
    at: LogicalPosition,
    notches: i32,
) -> Result<(), String> {
    let viewers = viewer_windows(app)?;
    let y = if notches >= 0 { -1.0 } else { 1.0 };
    for _ in 0..notches.unsigned_abs() {
        seams::wheel(
            &viewers,
            &viewing.window,
            at,
            NativeWheel::Lines { x: 0.0, y },
        )
        .ok_or("the native input seam does not reach this viewer window")?;
    }
    Ok(())
}

/// The viewer window gains (`true`) or loses the system focus, as winit reports it.
pub fn native_focus(app: &App, viewing: &Viewing, focused: bool) -> Result<(), String> {
    let viewers = viewer_windows(app)?;
    seams::focus(&viewers, &viewing.window, focused)
        .ok_or_else(|| "the native input seam does not reach this viewer window".to_owned())
}

/// The system settings the viewer reads (lines per notch, reduced motion) replaced by
/// `settings` until the guard drops; the viewer reads them as it gains the focus, which
/// this gives it.
pub fn system_settings(
    app: &App,
    viewing: &Viewing,
    settings: SystemSettings,
) -> Result<SettingsGuard, String> {
    let guard = seams::override_settings(settings);
    native_focus(app, viewing, true)?;
    Ok(guard)
}

/// The shown positions a motion passes through: from `start` (the displayed position
/// read before the input, which may move it at once) until the position has held still
/// for [`QUIET`] (at most `limit`), every distinct value of the displayed position, read
/// after each frame (the seam's frames come with Slint's timers).
pub fn motion_steps(app: &App, viewing: &Viewing, start: f32, limit: Duration) -> Vec<f32> {
    let started = Instant::now();
    let mut steps = vec![start];
    let shown = viewing.window.get_displayed_position();
    if shown != start {
        steps.push(shown);
    }
    let mut still = Instant::now();
    while started.elapsed() < limit && still.elapsed() < QUIET {
        slint::platform::update_timers_and_animations();
        paint_tool_windows(app);
        let shown = viewing.window.get_displayed_position();
        if steps.last() != Some(&shown) {
            steps.push(shown);
            still = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    steps
}

/// The middle of the viewer's content.
pub fn content_middle(viewing: &Viewing) -> LogicalPosition {
    let window = &viewing.window;
    LogicalPosition::new(
        window.get_content_left() + window.get_content_width() / 2.0,
        window.get_content_top() + window.get_content_height() / 2.0,
    )
}

/// Send `count` synthetic Slint pixel deltas of [`SLINT_PIXEL_DELTA`] over `at`
/// (positive: towards the end), one event after the other without waiting, as a quick
/// burst arrives. Not a wheel notch (see [`SLINT_PIXEL_DELTA`]).
pub fn pixel_deltas(window: &slint::Window, at: LogicalPosition, count: i32) {
    window.dispatch_event(WindowEvent::PointerMoved { position: at });
    let delta = if count >= 0 {
        -SLINT_PIXEL_DELTA
    } else {
        SLINT_PIXEL_DELTA
    };
    for _ in 0..count.unsigned_abs() {
        window.dispatch_event(WindowEvent::PointerScrolled {
            position: at,
            delta_x: 0.0,
            delta_y: delta,
        });
    }
}

/// The vertical scrollbar as the window lays it out (logical pixels) and its thumb
/// length (`viewer-position-bar.slint`: at least 28 pixels, at most the track).
#[derive(Clone, Copy, Debug)]
pub struct Bar {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub thumb: f32,
}

impl Bar {
    /// The middle of the thumb when it shows `position`.
    pub fn thumb_middle(&self, position: f32) -> LogicalPosition {
        LogicalPosition::new(
            self.left + self.width / 2.0,
            self.top + position.clamp(0.0, 1.0) * (self.height - self.thumb) + self.thumb / 2.0,
        )
    }
}

/// The vertical scrollbar, when the content is taller than the viewport.
pub fn vertical_bar(viewing: &Viewing) -> Option<Bar> {
    let window = &viewing.window;
    let extent = window.get_vertical_extent();
    if extent >= 1.0 {
        return None;
    }
    let (width, height) = (
        window.get_vertical_bar_width(),
        window.get_vertical_bar_height(),
    );
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let thumb = height.min((height * extent.clamp(0.0, 1.0)).max(28.0));
    Some(Bar {
        left: window.get_vertical_bar_left(),
        top: window.get_vertical_bar_top(),
        width,
        height,
        thumb,
    })
}

/// Let `duration` pass while the content windows' timers run and the windows that
/// asked for a frame are painted: what a viewer or comparator needs to make progress.
/// The main window is neither painted nor fed events meanwhile (`App::settle` does
/// both, and its forced main-window frame would dominate every poll).
pub fn pause(app: &App, duration: Duration) {
    let end = Instant::now() + duration;
    loop {
        slint::platform::update_timers_and_animations();
        paint_tool_windows(app);
        let now = Instant::now();
        if now >= end {
            return;
        }
        std::thread::sleep((end - now).min(Duration::from_millis(4)));
    }
}

/// Press at `from`, move to `to` in `steps` moves with a frame (16 ms) between them,
/// and release there, as a person drags.
pub fn drag(
    app: &mut App,
    window: &slint::Window,
    from: LogicalPosition,
    to: LogicalPosition,
    steps: u32,
) {
    let button = PointerEventButton::Left;
    app.settle();
    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    pause(app, Duration::from_millis(16));
    let steps = steps.max(1);
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let position =
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        pause(app, Duration::from_millis(16));
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
}

/// Drag the vertical thumb from where it is shown to `target` and release it there.
pub fn drag_vertical(
    app: &mut App,
    viewing: &Viewing,
    target: f32,
    steps: u32,
) -> Result<(), String> {
    let bar = vertical_bar(viewing).ok_or_else(|| {
        format!(
            "no vertical scrollbar is shown (extent {:.4})",
            viewing.window.get_vertical_extent()
        )
    })?;
    let from = bar.thumb_middle(viewing.window.get_vertical_thumb_position());
    let to = bar.thumb_middle(target);
    drag(app, viewing.slint(), from, to, steps);
    Ok(())
}

/// Drag the vertical thumb from where it is shown to `target` in `steps` moves, `frame`
/// apart, and release it there; the fingerprint of the content shown after each move,
/// before the release: the frames the viewer published while the gesture went on.
pub fn drag_frames(
    app: &mut App,
    viewing: &Viewing,
    target: f32,
    steps: u32,
    frame: Duration,
) -> Result<Vec<u64>, String> {
    let bar = vertical_bar(viewing).ok_or("no vertical scrollbar is shown")?;
    let from = bar.thumb_middle(viewing.window.get_vertical_thumb_position());
    let to = bar.thumb_middle(target);
    let (window, button) = (viewing.slint(), PointerEventButton::Left);
    app.settle();
    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    pause(app, frame);
    let steps = steps.max(1);
    let mut prints = Vec::with_capacity(steps as usize);
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let position =
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        pause(app, frame);
        prints.push(viewing.canvas().fingerprint());
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    Ok(prints)
}

/// Drag the vertical thumb from where it is shown to `target` in `steps` moves a frame
/// (16 ms) apart and release it there, as [`drag_vertical`] does, with `watch` reading the
/// status line throughout.
pub fn drag_vertical_watched(
    app: &mut App,
    viewing: &Viewing,
    target: f32,
    steps: u32,
    watch: &mut StatusWatch,
) -> Result<(), String> {
    let bar = vertical_bar(viewing).ok_or("no vertical scrollbar is shown")?;
    let from = bar.thumb_middle(viewing.window.get_vertical_thumb_position());
    let to = bar.thumb_middle(target);
    let (window, button) = (viewing.slint(), PointerEventButton::Left);
    let frame = Duration::from_millis(16);
    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    watch.pause(app, viewing, frame);
    let steps = steps.max(1);
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let position =
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        watch.pause(app, viewing, frame);
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    watch.pause(app, viewing, frame);
    Ok(())
}

/// What the viewer shows once a scroll settled.
#[derive(Clone, Debug)]
pub struct Settled {
    /// The thumb's position.
    pub position: f32,
    /// The position of the content actually shown.
    pub displayed: f32,
    /// The horizontal position.
    pub horizontal: f32,
    pub status: String,
}

/// How long nothing may change before a view counts as settled: longer than a refill
/// pause of the retained text raster, so a motion that resumes after one is not cut.
pub const QUIET: Duration = Duration::from_millis(300);

/// Run the viewer until it has no requested position pending, does not say it is
/// loading one, and its positions held still for [`QUIET`].
pub fn settle(app: &mut App, viewing: &Viewing, limit: Duration) -> Result<Settled, String> {
    let started = Instant::now();
    let mut last: Option<(f32, f32, f32)> = None;
    let mut since: Option<Instant> = None;
    loop {
        pause(app, Duration::from_millis(15));
        let window = &viewing.window;
        let now = (
            window.get_position(),
            window.get_displayed_position(),
            window.get_horizontal_position(),
        );
        let status = window.get_status().to_string();
        let busy = window.get_position_pending()
            || status.starts_with("Opening")
            || status.contains("Loading requested position");
        if busy || last != Some(now) {
            since = None;
        } else if since.is_none() {
            since = Some(Instant::now());
        }
        last = Some(now);
        if since.is_some_and(|since| since.elapsed() >= QUIET) {
            app.settle();
            return Ok(Settled {
                position: now.0,
                displayed: now.1,
                horizontal: now.2,
                status,
            });
        }
        if started.elapsed() >= limit {
            app.settle();
            return Err(format!(
                "not settled after {limit:?}: thumb {:.4}, shown {:.4}, pending {}, status {status:?}",
                now.0,
                now.1,
                window.get_position_pending()
            ));
        }
    }
}

/// The status lines a viewer showed while a gesture went on: read after every pass of
/// the content windows' timers, which deliver the backend's frames and their statuses.
#[derive(Default)]
pub struct StatusWatch {
    seen: Vec<String>,
}

impl StatusWatch {
    /// Read the status line once.
    pub fn read(&mut self, viewing: &Viewing) {
        let status = viewing.window.get_status().to_string();
        if self.seen.last() != Some(&status) {
            self.seen.push(status);
        }
    }

    /// Let `duration` pass as [`pause`] does, reading the status after every pass.
    pub fn pause(&mut self, app: &App, viewing: &Viewing, duration: Duration) {
        let end = Instant::now() + duration;
        loop {
            slint::platform::update_timers_and_animations();
            paint_tool_windows(app);
            self.read(viewing);
            let now = Instant::now();
            if now >= end {
                return;
            }
            std::thread::sleep((end - now).min(Duration::from_millis(2)));
        }
    }

    /// Run until the content holds still for [`QUIET`] with no position pending (at most
    /// `limit`), reading the status after every pass: the still content.
    pub fn still(
        &mut self,
        app: &App,
        viewing: &Viewing,
        limit: Duration,
    ) -> Result<Canvas, String> {
        let started = Instant::now();
        let mut previous = None;
        let mut since: Option<Instant> = None;
        loop {
            self.pause(app, viewing, Duration::from_millis(30));
            let canvas = viewing.canvas();
            let print = canvas.fingerprint();
            if viewing.window.get_position_pending() || previous != Some(print) {
                since = None;
            } else if since.is_none() {
                since = Some(Instant::now());
            }
            previous = Some(print);
            if since.is_some_and(|since| since.elapsed() >= QUIET) {
                return Ok(canvas);
            }
            if started.elapsed() >= limit {
                return Err(format!(
                    "the content kept changing for {limit:?}; status {:?}",
                    viewing.status()
                ));
            }
        }
    }

    /// The status lines seen that contain `text`.
    pub fn showing(&self, text: &str) -> Vec<String> {
        self.seen
            .iter()
            .filter(|status| status.contains(text))
            .cloned()
            .collect()
    }
}

/// Render the content until it holds still: the same pixels for [`QUIET`], with no
/// position pending.
pub fn still_canvas(app: &mut App, viewing: &Viewing, limit: Duration) -> Result<Canvas, String> {
    app.settle();
    let started = Instant::now();
    let mut previous = None;
    let mut since: Option<Instant> = None;
    loop {
        pause(app, Duration::from_millis(30));
        let canvas = viewing.canvas();
        let print = canvas.fingerprint();
        if viewing.window.get_position_pending() || previous != Some(print) {
            since = None;
        } else if since.is_none() {
            since = Some(Instant::now());
        }
        previous = Some(print);
        if since.is_some_and(|since| since.elapsed() >= QUIET) {
            return Ok(canvas);
        }
        if started.elapsed() >= limit {
            return Err(format!(
                "the content kept changing for {limit:?}; status {:?}",
                viewing.status()
            ));
        }
    }
}

/// The part of a status line that reports a failure, if any.
pub fn status_error(status: &str) -> Option<String> {
    let lower = status.to_lowercase();
    [
        "not completed",
        "error",
        "failed",
        "cannot",
        "could not",
        "unable",
        "panicked",
    ]
    .iter()
    .any(|word| lower.contains(word))
    .then(|| status.to_owned())
}

/// The loaded page's byte range and the file's length from a text status line
/// ("… Bytes 1200–9600 / 81234 …").
pub fn page_bytes(status: &str) -> Option<(u64, u64, u64)> {
    let rest = &status[status.find("Bytes ")? + "Bytes ".len()..];
    let (start, rest) = rest.split_once('–')?;
    let (end, rest) = rest.split_once(" / ")?;
    let length: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((
        start.trim().parse().ok()?,
        end.trim().parse().ok()?,
        length.parse().ok()?,
    ))
}

fn line_hash(pixels: impl Iterator<Item = [u8; 3]>) -> Option<u64> {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut first = None;
    let mut uniform = true;
    for colour in pixels {
        match first {
            None => first = Some(colour),
            Some(seen) if seen != colour => uniform = false,
            Some(_) => {}
        }
        for byte in colour {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    (!uniform).then_some(hash)
}

/// The lines (rows or columns) of two renders, compared along one axis.
struct Lines<'a> {
    before: &'a Canvas,
    after: &'a Canvas,
    /// Lines along the axis of the shift, and pixels per line.
    count: usize,
    length: usize,
    /// The pixel `position` of line `line`.
    at: fn(&Canvas, usize, usize) -> [u8; 3],
}

/// The pixels of line `line` of `canvas`.
fn pixels(
    at: fn(&Canvas, usize, usize) -> [u8; 3],
    length: usize,
    canvas: &Canvas,
    line: usize,
) -> impl Iterator<Item = [u8; 3]> + '_ {
    (0..length).map(move |position| at(canvas, line, position))
}

/// How well two renders agree at one shift ([`Lines::agreement`]).
#[derive(Clone, Copy)]
struct Agreement {
    shift: i32,
    /// The share of the lines with ink that agree but for anti-aliasing: at most a
    /// fiftieth of a line's pixels (at least four) differ.
    share: f32,
    /// The same share where at most a hundredth may differ: it tells the shift from one
    /// a repeating layout's period away, whose rows differ only in their numbers.
    strict: f32,
    /// The fraction of a pixel more that the content moved (see `between`).
    phase: f32,
}

impl Lines<'_> {
    /// How `after[i]` agrees with the rows of `before` at `shift`: with `before[i +
    /// shift]` within anti-aliasing tolerance (40 levels per channel); or, `between`, for
    /// content that moved `shift` lines and a fraction of a line more and whose glyphs the
    /// renderer drew again at their new sub-pixel phase (a text raster refilled at a
    /// camera position between whole pixels), with every pixel between `before[i +
    /// shift]` and `before[i + shift + 1]` within that tolerance; the fraction is what the
    /// values in between give, weighted by how far the two bracketing rows differ. None
    /// when fewer than 40 lines overlap with ink. `sparse` reads every fourth line and
    /// every second pixel, a cheap first ranking of many candidate shifts.
    fn agreement(
        &self,
        shift: i32,
        background: [u8; 3],
        between: bool,
        sparse: bool,
    ) -> Option<Agreement> {
        let next = i32::from(between);
        let (line_step, pixel_step) = if sparse { (4, 2) } else { (1, 1) };
        let lax = (self.length / 50 / pixel_step).max(4);
        let strict_limit = (self.length / 100 / pixel_step).max(4);
        let (mut considered, mut matching, mut strict) = (0_usize, 0_usize, 0_usize);
        // The fraction, in integers: the distance of each value from the first bracketing
        // row towards the second, clamped to the span, over the spans.
        let (mut weighted, mut weights) = (0_i64, 0_i64);
        for line in (0..self.count).step_by(line_step) {
            let origin = line as i32 + shift;
            if origin < 0 || origin + next >= self.count as i32 {
                continue;
            }
            let bracketing = pixels(self.at, self.length, self.before, origin as usize).zip(
                pixels(self.at, self.length, self.before, (origin + next) as usize),
            );
            let (mut ink, mut differing) = (false, 0);
            for (new, (low, high)) in pixels(self.at, self.length, self.after, line)
                .zip(bracketing)
                .step_by(pixel_step)
            {
                ink |= new != background || low != background || high != background;
                let mut outside = false;
                for channel in 0..3 {
                    let (value, low, high) = (
                        i32::from(new[channel]),
                        i32::from(low[channel]),
                        i32::from(high[channel]),
                    );
                    outside |= value < low.min(high) - 40 || value > low.max(high) + 40;
                    let span = high - low;
                    if span.abs() >= 32 {
                        let towards = ((value - low) * span.signum()).clamp(0, span.abs());
                        weighted += i64::from(towards);
                        weights += i64::from(span.abs());
                    }
                }
                differing += usize::from(outside);
            }
            if !ink {
                continue;
            }
            considered += 1;
            matching += usize::from(differing <= lax);
            strict += usize::from(differing <= strict_limit);
        }
        (considered >= 40 / line_step).then(|| Agreement {
            shift,
            share: matching as f32 / considered as f32,
            strict: strict as f32 / considered as f32,
            phase: if weights > 0 {
                weighted as f32 / weights as f32
            } else {
                0.0
            },
        })
    }

    /// How far the content moved: candidate shifts (the most voted by exactly equal
    /// lines, the best alignments of the lines' ink profiles) whose rows agree as they
    /// were. Unless one of them holds within the strict limit for [`SHIFT_CONFIDENCE`] of
    /// the lines, the content may also have moved a fraction of a pixel more and been
    /// drawn again at its new sub-pixel phase: the candidates and their neighbours are
    /// tried `between` too. The best agreement has the highest sum of both shares (so a
    /// repeating layout's period, whose rows differ in their numbers, loses to the shift
    /// whose rows agree within the strict limit), then the shorter shift; an exact one
    /// wins a tie.
    ///
    /// `direction` (the sign of the shown position's change; 0 when unknown) leaves out the
    /// shifts the other way: a layout that repeats (the pages of a PDF) also matches a
    /// period away, on the other side.
    fn motion(&self, direction: i32) -> Option<Motion> {
        let background = dominant(self.before);
        let candidates: Vec<i32> = self
            .candidates(background)
            .into_iter()
            .filter(|shift| direction == 0 || shift.signum() != -direction.signum())
            .collect();
        let score = |found: &Agreement| found.share + found.strict;
        let order = |a: &Agreement, b: &Agreement| {
            score(a)
                .total_cmp(&score(b))
                .then(b.shift.abs().cmp(&a.shift.abs()))
        };
        // A sparse reading ranks the shifts; the three best are read whole.
        let best = |shifts: &[i32], between: bool| {
            let mut ranked: Vec<Agreement> = shifts
                .iter()
                .filter_map(|&shift| self.agreement(shift, background, between, true))
                .collect();
            ranked.sort_by(|a, b| order(b, a));
            ranked
                .iter()
                .take(3)
                .filter_map(|found| self.agreement(found.shift, background, between, false))
                .max_by(order)
        };
        let exact = best(&candidates, false);
        let found = if exact.is_some_and(|found| found.strict >= SHIFT_CONFIDENCE) {
            exact
        } else {
            let mut neighbours: Vec<i32> = candidates
                .iter()
                .flat_map(|&shift| [shift - 1, shift, shift + 1])
                .filter(|shift| direction == 0 || shift.signum() != -direction.signum())
                .collect();
            neighbours.sort_unstable();
            neighbours.dedup();
            match (exact, best(&neighbours, true)) {
                (Some(exact), Some(between)) if score(&between) <= score(&exact) => Some(exact),
                (exact, between) => between.or(exact),
            }
        };
        found.map(|found| Motion {
            pixels: found.shift,
            phase: found.phase,
            share: found.share,
        })
    }

    /// Candidate shifts: the most voted by exactly equal lines, and the best alignments
    /// of the lines' ink profiles.
    fn candidates(&self, background: [u8; 3]) -> Vec<i32> {
        let mut candidates: Vec<i32> = Vec::new();
        // Exact votes.
        let hashes = |canvas: &Canvas| -> Vec<Option<u64>> {
            (0..self.count)
                .map(|line| line_hash(pixels(self.at, self.length, canvas, line)))
                .collect()
        };
        let (old, new) = (hashes(self.before), hashes(self.after));
        let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
        for (line, hash) in old.iter().enumerate() {
            if let Some(hash) = hash {
                index.entry(*hash).or_default().push(line);
            }
        }
        let mut votes: HashMap<i32, usize> = HashMap::new();
        for (line, hash) in new.iter().enumerate() {
            if let Some(found) = hash
                .and_then(|hash| index.get(&hash))
                .filter(|found| found.len() <= 4)
            {
                for origin in found {
                    *votes.entry(*origin as i32 - line as i32).or_default() += 1;
                }
            }
        }
        let mut voted: Vec<(i32, usize)> = votes.into_iter().collect();
        voted.sort_by_key(|(shift, count)| (std::cmp::Reverse(*count), shift.abs()));
        candidates.extend(voted.iter().take(4).map(|(shift, _)| *shift));
        // Ink profiles: each line's distance from the background in blocks of 16 pixels
        // along it, so the lines of a repeating layout differ where their numbers do.
        const BLOCK: usize = 16;
        let blocks = self.length.div_ceil(BLOCK);
        let profile = |canvas: &Canvas| -> Vec<u32> {
            let mut profile = vec![0_u32; self.count * blocks];
            for line in 0..self.count {
                for (position, pixel) in pixels(self.at, self.length, canvas, line).enumerate() {
                    profile[line * blocks + position / BLOCK] += (0..3)
                        .map(|channel| u32::from(pixel[channel].abs_diff(background[channel])))
                        .sum::<u32>();
                }
            }
            profile
        };
        let (old, new) = (profile(self.before), profile(self.after));
        let reach = self.count as i32 - 40;
        let mut aligned: Vec<(u64, i32)> = Vec::new();
        for shift in -reach..=reach {
            let (mut total, mut lines) = (0_u64, 0_u64);
            // Every second line: the lines of text are taller than two pixels.
            for line in (0..self.count).step_by(2) {
                let origin = line as i32 + shift;
                if origin < 0 || origin >= self.count as i32 {
                    continue;
                }
                let a = &new[line * blocks..(line + 1) * blocks];
                let b = &old[origin as usize * blocks..(origin as usize + 1) * blocks];
                if a.iter().chain(b).all(|ink| *ink == 0) {
                    continue;
                }
                total += a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| u64::from(a.abs_diff(*b)))
                    .sum::<u64>();
                lines += 1;
            }
            if lines >= 20 {
                aligned.push((total / lines, shift));
            }
        }
        aligned.sort_unstable();
        candidates.extend(aligned.iter().take(12).map(|(_, shift)| *shift));
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
}

/// How far content moved between two renders, as [`vertical_motion`] measures it.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    /// Whole physical pixels (positive: towards the end of the file).
    pub pixels: i32,
    /// The fraction of a pixel more (0 to 1) when the glyphs were drawn again at their
    /// new sub-pixel phase; 0 when the rows moved as they were.
    pub phase: f32,
    /// The share of the overlapping rows with ink that agree.
    pub share: f32,
}

impl Motion {
    /// The distance to the nearest whole pixel.
    pub fn rounded(&self) -> i32 {
        (self.pixels as f32 + self.phase).round() as i32
    }
}

/// How far the content moved up between two renders of the same canvas, and the share
/// of the overlapping rows with ink that agree within anti-aliasing tolerance. Rows move
/// as they were when the camera rests on the same raster grid; when a refill drew the
/// text again at another sub-pixel phase, the rows of the second render lie between two
/// rows of the first, and the fraction is measured too. Rows are compared without their
/// right 24 pixels, where a page may draw its own scrollbar.
pub fn vertical_motion(before: &Canvas, after: &Canvas) -> Option<Motion> {
    vertical_motion_towards(before, after, 0)
}

/// [`vertical_motion`] of content known to have moved towards the end (`direction` 1) or
/// the start (-1) of its document, from its shown position; 0 when not known.
pub fn vertical_motion_towards(before: &Canvas, after: &Canvas, direction: i32) -> Option<Motion> {
    if before.width != after.width || before.height != after.height || before.width <= 32 {
        return None;
    }
    if before.fingerprint() == after.fingerprint() {
        return Some(Motion {
            pixels: 0,
            phase: 0.0,
            share: 1.0,
        });
    }
    Lines {
        before,
        after,
        count: before.height,
        length: before.width - 24,
        at: |canvas, line, position| canvas.at(position, line),
    }
    .motion(direction)
}

/// [`vertical_motion`] to the nearest whole physical pixel (positive: towards the end of
/// the file), with the share of the rows that agree.
pub fn vertical_shift(before: &Canvas, after: &Canvas) -> Option<(i32, f32)> {
    vertical_motion(before, after).map(|motion| (motion.rounded(), motion.share))
}

/// WCAG 2 relative luminance of an sRGB colour.
pub fn luminance(colour: [u8; 3]) -> f64 {
    let channel = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(colour[0]) + 0.7152 * channel(colour[1]) + 0.0722 * channel(colour[2])
}

/// WCAG 2 contrast ratio of two colours (1 to 21).
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The contrast the product keeps between selected text and the selection in `theme`
/// (`midnight-viewer-ui` tests/selection_contrast.rs, as `midnight-ui` tests/themes.rs):
/// WCAG AA 4.5:1, 7:1 in the Contrast palettes and 12:1 in High Contrast.
pub fn selected_text_minimum(theme: PreferenceTheme) -> f64 {
    match theme {
        PreferenceTheme::PaperContrast
        | PreferenceTheme::SandstoneContrast
        | PreferenceTheme::DeepOcean
        | PreferenceTheme::GraphiteContrast => 7.0,
        PreferenceTheme::HighContrastDark | PreferenceTheme::HighContrastLight => 12.0,
        _ => 4.5,
    }
}

/// The contrast the product keeps between unselected syntax colours and the page in
/// `theme` (the same test): 5:1, a margin over AA, and 7:1 in the Contrast and High
/// Contrast palettes.
pub fn syntax_minimum(theme: PreferenceTheme) -> f64 {
    match theme {
        PreferenceTheme::PaperContrast
        | PreferenceTheme::SandstoneContrast
        | PreferenceTheme::DeepOcean
        | PreferenceTheme::GraphiteContrast
        | PreferenceTheme::HighContrastDark
        | PreferenceTheme::HighContrastLight => 7.0,
        _ => 5.0,
    }
}

/// Whether two colours are the same within `tolerance` per channel.
pub fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

/// A Slint colour as RGB.
pub fn rgb(colour: slint::Color) -> [u8; 3] {
    let colour = colour.to_argb_u8();
    [colour.red, colour.green, colour.blue]
}

/// The part of `canvas` (a render of `viewing`'s content) right of the pinned
/// line-number gutter, `margin` physical pixels further: the text that the gutter's
/// numbers do not touch. A view without a retained gutter keeps its width less `margin`.
pub fn right_of_gutter(viewing: &Viewing, canvas: &Canvas, margin: usize) -> Canvas {
    let window = &viewing.window;
    let gutter = if window.get_retained_raster() {
        (window.get_raster_gutter_width()
            * window.get_content_width()
            * viewing.slint().scale_factor())
        .max(0.0)
        .ceil() as usize
    } else {
        0
    };
    let left = (gutter + margin).min(canvas.width);
    Canvas::crop(
        &canvas.pixels,
        canvas.width,
        left,
        0,
        canvas.width - left,
        canvas.height,
    )
}

/// How sharply dark text is drawn on a light page: along every row, each step between
/// the page colour and solid ink, and the number of pixels in between (the edge's
/// width). A pixel counts as page within a tenth, as ink within a tenth, of the way
/// from the page's luminance to the darkest pixel's. Sharp text at any zoom has edges
/// of one or two pixels; a raster enlarged to the zoom has edges as wide as its scale.
/// Returns the widths, smallest first.
pub fn edge_widths(canvas: &Canvas) -> Vec<usize> {
    let page = luminance(dominant(canvas));
    let ink = canvas
        .pixels
        .iter()
        .map(|pixel| luminance([pixel.r, pixel.g, pixel.b]))
        .fold(page, f64::min);
    if page - ink < 0.3 {
        return Vec::new();
    }
    let level = |x: usize, y: usize| (page - luminance(canvas.at(x, y))) / (page - ink);
    let mut widths = Vec::new();
    for y in 0..canvas.height {
        // The last page or ink pixel seen in this row: (column, is ink).
        let mut last: Option<(usize, bool)> = None;
        for x in 0..canvas.width {
            let share = level(x, y);
            let class = if share <= 0.1 {
                Some(false)
            } else if share >= 0.9 {
                Some(true)
            } else {
                None
            };
            if let Some(ink) = class {
                if let Some((column, was_ink)) = last
                    && was_ink != ink
                {
                    widths.push(x - column - 1);
                }
                last = Some((x, ink));
            }
        }
    }
    widths.sort_unstable();
    widths
}

fn distance(a: [u8; 3], b: [u8; 3]) -> f32 {
    (0..3)
        .map(|channel| (f32::from(a[channel]) - f32::from(b[channel])).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// The most common colour of the canvas.
pub fn dominant(canvas: &Canvas) -> [u8; 3] {
    canvas
        .colours(1)
        .first()
        .map_or([0, 0, 0], |(colour, _)| *colour)
}

/// The colour that most pixels which were `background` in `before` show in `after`: a
/// selection or highlight painted over the background.
pub fn painted_over(
    before: &Canvas,
    after: &Canvas,
    background: [u8; 3],
) -> Option<([u8; 3], usize)> {
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for (old, new) in before.pixels.iter().zip(&after.pixels) {
        let (old, new) = ([old.r, old.g, old.b], [new.r, new.g, new.b]);
        if old == background && new != background {
            *counts.entry(new).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, count)| *count)
}

/// The text colours drawn over `background`: runs of other pixels, at most 16 pixels
/// long (glyph strokes, not the backgrounds of other blocks), that `background` bounds
/// on both sides in a row, grouped by their direction from it. A group's glyph colour is
/// its most frequent colour among those at least 85 % as far from the background as
/// its farthest one (solid glyph pixels repeat; anti-aliased edges are blends nearer
/// the background), seen at least six times. Groups under `minimum` pixels are left
/// out. Returns (glyph colour, pixels of its group), most pixels first.
pub fn glyph_colours(
    canvas: &Canvas,
    background: [u8; 3],
    minimum: usize,
) -> Vec<([u8; 3], usize)> {
    let mut glyphs: Vec<([u8; 3], usize)> = glyph_groups(canvas, background)
        .into_iter()
        .filter_map(|members| {
            let total: usize = members.iter().map(|(_, count, _)| count).sum();
            let farthest = members.first()?.2;
            let (colour, count, _) = members
                .iter()
                .filter(|(_, _, reach)| *reach >= farthest * 0.85)
                .max_by_key(|(_, count, _)| *count)?;
            (total >= minimum && *count >= 6).then_some((*colour, total))
        })
        .collect();
    glyphs.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    glyphs
}

/// The text colours drawn over `background` for a contrast measurement: the groups of
/// [`glyph_colours`], each standing for the colour of its highest contrast against
/// `background` (seen at least twice). WCAG measures the text's own colour, which only
/// the solid cores of glyph strokes show; their anti-aliased edges are blends towards
/// the background and read lower, however often they occur. Groups under `minimum`
/// pixels are left out. Returns (core colour, pixels of its group), most pixels first.
pub fn glyph_cores(canvas: &Canvas, background: [u8; 3], minimum: usize) -> Vec<([u8; 3], usize)> {
    cores(glyph_groups(canvas, background), background, minimum)
}

/// [`glyph_cores`] of the text a selection covers: the runs on `selection` in `after`
/// that count lie between pixels the selection painted (they differ from `before`), so
/// text beside the selection whose own anti-aliased edges come near its colour (line
/// numbers in the gutter) is not taken for selected text; and an edge colour joins the
/// group of any stronger colour of the group it blends towards, not only the colour
/// that started the group, so a glyph's edges never stand for a group of their own.
pub fn selected_glyph_cores(
    after: &Canvas,
    before: &Canvas,
    selection: [u8; 3],
    minimum: usize,
) -> Vec<([u8; 3], usize)> {
    let painted = |x: usize, y: usize| before.at(x, y) != after.at(x, y);
    cores(
        grouped_glyph_runs(after, selection, &painted, true),
        selection,
        minimum,
    )
}

/// Each group's colour of the highest contrast against `background`; groups under
/// `minimum` pixels are left out. Most pixels first.
fn cores(
    groups: Vec<Vec<([u8; 3], usize, f32)>>,
    background: [u8; 3],
    minimum: usize,
) -> Vec<([u8; 3], usize)> {
    let mut glyphs: Vec<([u8; 3], usize)> = groups
        .into_iter()
        .filter_map(|members| {
            let total: usize = members.iter().map(|(_, count, _)| count).sum();
            let (colour, _, _) = members
                .iter()
                .max_by(|a, b| contrast(a.0, background).total_cmp(&contrast(b.0, background)))?;
            (total >= minimum).then_some((*colour, total))
        })
        .collect();
    glyphs.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    glyphs
}

/// The colours of glyph runs over `background` (see [`glyph_colours`]) in groups of
/// one direction from it: each group's members (colour, times seen, distance from the
/// background), farthest first.
fn glyph_groups(canvas: &Canvas, background: [u8; 3]) -> Vec<Vec<([u8; 3], usize, f32)>> {
    grouped_glyph_runs(canvas, background, &|_, _| true, false)
}

/// [`glyph_groups`] where only background pixels for which `bound(x, y)` holds end a
/// run of glyph pixels, and where, with `any_member`, a colour joins a group whose
/// direction matches that of any of its members (each stronger than the colour), not
/// only that of the colour that started it.
fn grouped_glyph_runs(
    canvas: &Canvas,
    background: [u8; 3],
    bound: &dyn Fn(usize, usize) -> bool,
    any_member: bool,
) -> Vec<Vec<([u8; 3], usize, f32)>> {
    const NEAR: f32 = 10.0;
    const LONGEST_RUN: usize = 16;
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for y in 0..canvas.height {
        let mut x = 0;
        while x < canvas.width {
            if distance(canvas.at(x, y), background) <= NEAR {
                x += 1;
                continue;
            }
            let start = x;
            while x < canvas.width && distance(canvas.at(x, y), background) > NEAR {
                x += 1;
            }
            // Bounded by the background on both sides: text on this background.
            if start > 0
                && x < canvas.width
                && x - start <= LONGEST_RUN
                && bound(start - 1, y)
                && bound(x, y)
            {
                for column in start..x {
                    *counts.entry(canvas.at(column, y)).or_default() += 1;
                }
            }
        }
    }
    let mut colours: Vec<([u8; 3], usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .collect();
    colours.sort_by(|a, b| {
        distance(b.0, background)
            .total_cmp(&distance(a.0, background))
            .then(b.1.cmp(&a.1))
    });
    let direction = |colour: [u8; 3]| {
        let delta: [f32; 3] = std::array::from_fn(|channel| {
            f32::from(colour[channel]) - f32::from(background[channel])
        });
        let length = delta
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt()
            .max(1.0);
        delta.map(|value| value / length)
    };
    // A colour joins the strongest group whose direction it shares within what 8-bit
    // rounding allows at its distance (at least 1.5 degrees): anti-aliased edges join
    // their glyph colour, while two similar hues (an orange beside an amber) stay apart.
    // Only colours at least a quarter as far from the background as the strongest one
    // start a group; weaker colours that join none are edge noise.
    let strongest = colours
        .first()
        .map_or(0.0, |(colour, _)| distance(*colour, background));
    struct Group {
        /// The direction of each member, the first one's first.
        directions: Vec<[f32; 3]>,
        members: Vec<([u8; 3], usize, f32)>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (colour, count) in colours {
        let towards = direction(colour);
        let reach = distance(colour, background);
        let allowed = (2.0 / reach.max(1.0))
            .atan()
            .max(1.5_f32.to_radians())
            .cos();
        let shares = |direction: &[f32; 3]| {
            towards
                .iter()
                .zip(direction.iter())
                .map(|(a, b)| a * b)
                .sum::<f32>()
                > allowed
        };
        match groups.iter_mut().find(|group| {
            let compared = if any_member {
                &group.directions[..]
            } else {
                &group.directions[..1]
            };
            compared.iter().any(shares)
        }) {
            Some(group) => {
                group.members.push((colour, count, reach));
                group.directions.push(towards);
            }
            None if reach >= strongest / 4.0 => groups.push(Group {
                directions: vec![towards],
                members: vec![(colour, count, reach)],
            }),
            None => {}
        }
    }
    groups.into_iter().map(|group| group.members).collect()
}

/// Why this machine refuses the HTML viewer's browser, when `error` (the viewer's own
/// report) is one of the refusals that depend on the machine, not on Midnight
/// Explorer: the messages of `midnight_worker_platform`'s `browser_sandbox_linux.rs`,
/// `browser_cgroup_linux.rs` and `browser_libraries_linux.rs`.
pub fn browser_refused(error: &str) -> Option<&'static str> {
    if error.contains("The HTML viewer needs permission for Chromium's sandbox") {
        Some(
            "AppArmor denies Chromium's sandbox to this installation (no \
             midnight-explorer-html-viewer profile covers the stage)",
        )
    } else if error.contains("Chromium requires a writable cgroup-v2 user delegation") {
        Some("no writable cgroup-v2 delegation for the browser on this machine")
    } else if error.contains("The HTML viewer needs system libraries that are not installed") {
        Some("this machine lacks system libraries of the HTML viewer's browser")
    } else {
        None
    }
}

/// Record a check of the HTML viewer: SKIP with the machine's reason when the machine
/// refused the viewer's browser ([`browser_refused`]), the check itself otherwise.
pub fn check_html(failures: &Failures, what: &str, result: Result<(), String>) {
    let refused = result.as_ref().err().and_then(|error| {
        browser_refused(error).map(|reason| format!("{reason}: {}", super::clip(error, 160)))
    });
    match refused {
        Some(reason) => failures.skip(what, &reason),
        None => failures.check(what, result),
    }
}

/// Why the HTML viewer cannot be checked here, known before it opens: its package is not
/// staged, or, on Linux, system libraries its browser links are missing
/// (`viewers_stage::html_viewer_missing`). A machine that refuses the browser only when
/// it starts is told by the viewer's own report ([`browser_refused`]).
pub fn html_unavailable() -> Option<String> {
    crate::viewers_stage::html_viewer_missing()
}

/// `#rrggbb`.
pub fn hex(colour: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2])
}

/// Run the UI review's layout checks on a content window and record each finding as a
/// flag on its screenshot `shot` (the path `App::screenshot_window` returned).
pub fn review(context: &mut Context, window: &slint::Window, shot: &str) -> Vec<String> {
    let size = window.size();
    let scale = window.scale_factor().max(0.01);
    let findings = inspect::check_layout(
        window,
        size.width as f32 / scale,
        size.height as f32 / scale,
    );
    let mut details = Vec::new();
    for finding in findings {
        if let Some(recorded) = context
            .outcome
            .shots
            .iter_mut()
            .find(|recorded| recorded.file == shot)
        {
            recorded.flags += 1;
        }
        details.push(format!("{}: {}", finding.kind, finding.detail));
        context.outcome.flags.push(report::Flag {
            shot: shot.to_owned(),
            kind: finding.kind,
            detail: finding.detail,
        });
    }
    details
}
