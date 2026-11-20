//! Measurements on viewer windows driven with real pointer, wheel and key input: the
//! scrollbars and drags on them, wheel notches, waits until the content settled, the
//! byte range a text page covers, how far rendered content moved between two frames,
//! the colours of rendered text and their contrast, and the UI review's layout checks
//! on content windows.
use super::{Canvas, Viewing, paint_tool_windows};
use crate::{Context, app::App, inspect, report};
use slint::{
    LogicalPosition,
    platform::{PointerEventButton, WindowEvent},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// One wheel notch as Slint's winit backend delivers a line: 60 logical pixels
/// (`LINE_PIXELS` in `midnight-ui-input`), the distance production's eased notch covers.
pub const NOTCH: f32 = 60.0;
/// The share of matching rows below which a measured shift is not trusted.
pub const SHIFT_CONFIDENCE: f32 = 0.8;

/// The middle of the viewer's content.
pub fn content_middle(viewing: &Viewing) -> LogicalPosition {
    let window = &viewing.window;
    LogicalPosition::new(
        window.get_content_left() + window.get_content_width() / 2.0,
        window.get_content_top() + window.get_content_height() / 2.0,
    )
}

/// Turn the wheel over `at` by `notches` (positive: towards the end), one event per
/// notch without waiting between them, as a quick burst arrives.
pub fn wheel(window: &slint::Window, at: LogicalPosition, notches: i32) {
    window.dispatch_event(WindowEvent::PointerMoved { position: at });
    let delta = if notches >= 0 { -NOTCH } else { NOTCH };
    for _ in 0..notches.unsigned_abs() {
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
    pub elapsed: Duration,
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
                elapsed: started.elapsed(),
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

impl Lines<'_> {
    /// The share of the non-blank lines in the overlap that match within anti-aliasing
    /// tolerance when `before[i + shift]` is compared with `after[i]`; None when fewer
    /// than 40 lines overlap with ink.
    fn tolerant(&self, shift: i32, background: [u8; 3]) -> Option<f32> {
        let (mut considered, mut matching) = (0, 0);
        for line in 0..self.count {
            let origin = line as i32 + shift;
            if origin < 0 || origin >= self.count as i32 {
                continue;
            }
            let mut ink = false;
            let mut differing = 0;
            for (new, old) in pixels(self.at, self.length, self.after, line).zip(pixels(
                self.at,
                self.length,
                self.before,
                origin as usize,
            )) {
                ink |= new != background || old != background;
                if (0..3).any(|channel| new[channel].abs_diff(old[channel]) > 40) {
                    differing += 1;
                }
            }
            if !ink {
                continue;
            }
            considered += 1;
            if differing <= (self.length / 50).max(4) {
                matching += 1;
            }
        }
        (considered >= 40).then(|| matching as f32 / considered as f32)
    }

    /// Candidate shifts: the most voted by exactly equal lines, and the best alignments
    /// of the lines' ink profiles; each verified with [`Self::tolerant`].
    fn shift(&self) -> Option<(i32, f32)> {
        let background = dominant(self.before);
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
        // Ink profiles: the sum of each line's distance from the background.
        let profile = |canvas: &Canvas| -> Vec<u32> {
            (0..self.count)
                .map(|line| {
                    pixels(self.at, self.length, canvas, line)
                        .map(|pixel| {
                            (0..3)
                                .map(|channel| {
                                    u32::from(pixel[channel].abs_diff(background[channel]))
                                })
                                .sum::<u32>()
                        })
                        .sum()
                })
                .collect()
        };
        let (old, new) = (profile(self.before), profile(self.after));
        let reach = self.count as i32 - 40;
        let mut aligned: Vec<(u64, i32)> = Vec::new();
        for shift in -reach..=reach {
            let (mut total, mut lines) = (0_u64, 0_u64);
            for line in 0..self.count {
                let origin = line as i32 + shift;
                if origin < 0 || origin >= self.count as i32 {
                    continue;
                }
                let (a, b) = (new[line], old[origin as usize]);
                if a == 0 && b == 0 {
                    continue;
                }
                total += u64::from(a.abs_diff(b));
                lines += 1;
            }
            if lines >= 40 {
                aligned.push((total / lines, shift));
            }
        }
        aligned.sort_unstable();
        candidates.extend(aligned.iter().take(6).map(|(_, shift)| *shift));
        candidates.sort_unstable();
        candidates.dedup();
        candidates
            .into_iter()
            .filter_map(|shift| Some((shift, self.tolerant(shift, background)?)))
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.abs().cmp(&a.0.abs())))
    }
}

/// How far the content moved up between two renders of the same canvas, in physical
/// pixels (positive: towards the end of the file), and the share of the overlapping
/// rows with ink that agree within anti-aliasing tolerance (a settled view may repaint
/// its glyphs at an exact sub-pixel phase). Rows are compared without their right 24
/// pixels, where a page may draw its own scrollbar.
pub fn vertical_shift(before: &Canvas, after: &Canvas) -> Option<(i32, f32)> {
    if before.width != after.width || before.height != after.height || before.width <= 32 {
        return None;
    }
    if before.fingerprint() == after.fingerprint() {
        return Some((0, 1.0));
    }
    Lines {
        before,
        after,
        count: before.height,
        length: before.width - 24,
        at: |canvas, line, position| canvas.at(position, line),
    }
    .shift()
}

/// How far the content moved left between two renders (positive: towards the end of
/// the lines), from whole columns, as [`vertical_shift`] measures rows.
pub fn horizontal_shift(before: &Canvas, after: &Canvas) -> Option<(i32, f32)> {
    if before.width != after.width || before.height != after.height || before.height == 0 {
        return None;
    }
    if before.fingerprint() == after.fingerprint() {
        return Some((0, 1.0));
    }
    Lines {
        before,
        after,
        count: before.width,
        length: before.height,
        at: |canvas, line, position| canvas.at(line, position),
    }
    .shift()
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
            if start > 0 && x < canvas.width && x - start <= LONGEST_RUN {
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
        direction: [f32; 3],
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
        match groups.iter_mut().find(|group| {
            towards
                .iter()
                .zip(group.direction.iter())
                .map(|(a, b)| a * b)
                .sum::<f32>()
                > allowed
        }) {
            Some(group) => group.members.push((colour, count, reach)),
            None if reach >= strongest / 4.0 => groups.push(Group {
                direction: towards,
                members: vec![(colour, count, reach)],
            }),
            None => {}
        }
    }
    let mut glyphs: Vec<([u8; 3], usize)> = groups
        .into_iter()
        .filter_map(|group| {
            let total: usize = group.members.iter().map(|(_, count, _)| count).sum();
            let farthest = group.members.first()?.2;
            let (colour, count, _) = group
                .members
                .iter()
                .filter(|(_, _, reach)| *reach >= farthest * 0.85)
                .max_by_key(|(_, count, _)| *count)?;
            (total >= minimum && *count >= 6).then_some((*colour, total))
        })
        .collect();
    glyphs.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    glyphs
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
