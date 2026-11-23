//! Panel scrolling since cd0b3235, with native wheel input through the panels' acceptance
//! seam since 4c8c17a6 (decision A1). The rows cut at the list's edges are drawn in every
//! sampled frame of a wheel gesture; Detailed and Brief come to rest on a whole row;
//! Thumbnails keeps its exact offset, and a click on its cut top row selects that row; a
//! Brief list scrolled past its cursor takes the keyboard back with a click; every
//! navigation key brings a cursor scrolled out of view back, also when it does not move
//! (finding P-7). A native notch moves the rows nearest the notch distance for the
//! system's lines per notch, or one screen (the seam's settings override stands in for
//! the system), gliding there; with less motion, or with Options › Appearance › Scrolling
//! glide Off, a notch, a half notch and the alignment after it move at once. Offsets are
//! sampled in real time: the exact 100 ms rest and 200 ms glide, and a new wheel stopping a
//! glide, need the scenario's own clock (harness H1) and are SKIP, with the measured times
//! noted and their lower bounds checked.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`.
use crate::{
    Context, Scenario,
    app::{App, Config},
    files_support::{self as support},
    input,
    options_support::Frame,
    panel::{self, ListGeometry},
    scenarios::panels_support::{self as ps, LEFT},
};
use midnight_ui::acceptance_seams::{self as seams, SettingsGuard, SystemSettings, WheelSetting};
use slint::{LogicalPosition, platform::Key};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const ROWS: usize = 120;
/// A small window: every settle renders it, and a fast render samples the gesture finely.
const WIDTH: usize = 820;
const HEIGHT: usize = 520;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "panels_scrolling",
        area: "panels",
        summary: "Panel scrolling: a row cut at the list's top drawn in every frame of a wheel gesture, Detailed and Brief resting on a whole row, Thumbnails keeping its offset and a click on its cut top row, a click focusing a Brief list scrolled past its cursor, Home/Up/End/Down revealing an unmoved cursor (P-7); native notches through the seam: the rows nearest the notch for 1, 3 and 6 lines and one screen, gliding, from a cut Thumbnails offset to a band boundary, and with less motion or Scrolling glide Off at once; the exact rest and glide times (H1) SKIP.",
        run: scrolling,
        timeout: Duration::from_secs(5 * 60),
    }]
}

/// How many pixels of each colour a window rectangle shows.
fn colours(frame: &Frame, (x, y, w, h): panel::Rect) -> HashMap<[u8; 3], usize> {
    let mut counts = HashMap::new();
    let (left, top) = (x.max(0.0) as usize, y.max(0.0) as usize);
    let right = ((x + w).max(0.0) as usize).min(frame.width);
    let bottom = ((y + h).max(0.0) as usize).min(frame.height);
    for row in top..bottom {
        if right <= left {
            break;
        }
        for pixel in &frame.pixels[row * frame.width + left..row * frame.width + right] {
            *counts.entry([pixel.r, pixel.g, pixel.b]).or_insert(0_usize) += 1;
        }
    }
    counts
}

/// The fill of marked rows: the colour most of a marked row's top shows, above its text
/// and artwork (a row in view, neither `..` nor the cursor row, whose outline differs).
fn marked_fill(app: &mut App) -> Option<[u8; 3]> {
    let list = panel::wait_rested(app, LEFT).ok()?;
    let cursor = usize::try_from(app.cursor(LEFT)).ok();
    let whole = list.whole_rows();
    let row = whole
        .iter()
        .copied()
        .filter(|index| *index != 0 && Some(*index) != cursor)
        .nth(whole.len() / 2)?;
    let (x, y, w, _) = list.cell_rect(row);
    let inset = 40.0_f32.min(w / 4.0);
    let frame = Frame::of(app);
    colours(&frame, (x + inset, y + 2.0, w - 2.0 * inset, 2.0))
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(colour, _)| colour)
}

/// With every file marked, each drawn row shows the selection fill. The list's top band
/// (its first 3 px, across the rows' grid) must show that fill on most of it: a band
/// where no row is drawn shows the list's own background. Not judged while `..` (never
/// marked) is the row at the top of a one-column list.
fn top_band_drawn(frame: &Frame, list: &ListGeometry, fill: [u8; 3]) -> Result<(), String> {
    if list.columns <= 1 && list.content_y > -list.row_height {
        return Ok(());
    }
    let inset = 40.0_f32.min(list.grid_width / 4.0);
    let counts = colours(
        frame,
        (
            list.left + inset,
            list.top,
            list.grid_width - 2.0 * inset,
            3.0,
        ),
    );
    let total: usize = counts.values().sum();
    let filled = counts.get(&fill).copied().unwrap_or(0);
    if total > 0 && filled * 10 >= total * 4 {
        Ok(())
    } else {
        let most = counts
            .iter()
            .max_by_key(|(_, count)| **count)
            .map(|(colour, _)| *colour);
        Err(format!(
            "content y {:.1}: the top band shows {most:?} ({filled} of {total} pixels in the \
             rows' fill {fill:?})",
            list.content_y
        ))
    }
}

fn aligned(list: &ListGeometry, content_y: f32) -> bool {
    ListGeometry { content_y, ..*list }.on_row_boundary()
}

/// The frames sampled after a wheel: each one's offset (with the milliseconds since the
/// wheel) and the frames whose top band showed no row.
struct Gesture {
    offsets: Vec<(u128, f32)>,
    blank: Vec<String>,
}

/// Turn the wheel by `rows` rows over the list and sample frames until it has been still
/// for 500 ms (any alignment starts within 300 ms of the wheel's end).
fn wheel_and_sample(app: &mut App, rows: f32, fill: [u8; 3]) -> Result<Gesture, String> {
    let start = panel::wait_rested(app, LEFT)?;
    panel::wheel_at(app, start.center(), -rows * start.row_height);
    let wheeled = Instant::now();
    let mut offsets = Vec::new();
    let mut blank = Vec::new();
    let mut still = Instant::now();
    let mut last = start.content_y;
    while wheeled.elapsed() < Duration::from_secs(10) {
        let frame = Frame::of(app);
        let list = panel::geometry(app, LEFT);
        if let Err(problem) = top_band_drawn(&frame, &list, fill) {
            blank.push(format!(
                "at {} ms: {problem}",
                wheeled.elapsed().as_millis()
            ));
        }
        offsets.push((wheeled.elapsed().as_millis(), list.content_y));
        if (list.content_y - last).abs() > 0.01 {
            still = Instant::now();
            last = list.content_y;
        } else if still.elapsed() >= Duration::from_millis(500) {
            break;
        }
        app.idle(Duration::from_millis(2));
    }
    Ok(Gesture { offsets, blank })
}

/// A wheel of `rows` rows in a view that settles on rows: a frame shows a row cut at the
/// list's top, every frame draws that band, and the list comes to rest on a whole row.
fn settles(
    app: &mut App,
    context: &mut Context,
    view: &str,
    rows: f32,
    fill: [u8; 3],
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let gesture = wheel_and_sample(app, rows, fill)?;
    let list = panel::geometry(app, LEFT);
    let target = gesture.offsets.last().map_or(list.content_y, |(_, y)| *y);
    context.note(format!(
        "{view}: offsets after a wheel of {rows} rows (ms, px): {:?}",
        gesture.offsets
    ));
    if list.settles_on_rows() && !aligned(&list, target) {
        failures.push(format!(
            "{view}: after a wheel of {rows} rows the list rests at {target:.1}, not on a whole \
             row (rows of {} px)",
            list.row_height
        ));
    }
    if !gesture
        .offsets
        .iter()
        .any(|(_, y)| !aligned(&list, *y) && *y < -list.row_height)
    {
        failures.push(format!(
            "{view}: no sampled frame of the wheel showed a row cut at the list's top: {:?}",
            gesture.offsets
        ));
    }
    if !gesture.blank.is_empty() {
        failures.push(format!(
            "{view}: a frame left the list's top band blank: {}",
            gesture.blank.join("; ")
        ));
    }
    Ok(())
}

/// The system settings the panels read through the seam until the guard drops: the lines
/// per notch (or a screen) and the request for less motion. They take effect with the
/// window's next native focus gain, as in production ([`native_focus`]).
fn system(wheel: WheelSetting, reduce_motion: bool) -> SettingsGuard {
    seams::override_settings(SystemSettings {
        wheel: Some(wheel),
        reduce_motion: Some(reduce_motion),
    })
}

/// A native focus gain: the panels read the system settings again, and must now ask for
/// less motion exactly when `reduced`.
fn native_focus(app: &mut App, reduced: bool, failures: &mut Vec<String>) -> Result<(), String> {
    seams::focus(&app.window, true).ok_or("the main window takes no native focus: not shown")?;
    app.settle();
    if app.window.get_reduce_motion() != reduced {
        failures.push(format!(
            "after a native focus gain the panels' reduced motion is {}, expected {reduced}",
            app.window.get_reduce_motion()
        ));
    }
    Ok(())
}

/// End, then Home in the left list: the cursor on `..` and the list resting at its top.
/// Moving the cursor first keeps this helper independent of `keys_reveal`'s subject (a
/// key that leaves the cursor where it is reveals it too, since 49a8e9c9, finding P-7).
fn to_top(app: &mut App) -> Result<ListGeometry, String> {
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::End);
    app.settle();
    input::key(app.slint(), Key::Home);
    app.settle();
    let list = panel::wait_rested(app, LEFT)?;
    if list.content_y.abs() > 0.5 {
        return Err(format!(
            "End and Home left the list at {:.1}, not at its top",
            list.content_y
        ));
    }
    Ok(list)
}

/// Every navigation key reveals the cursor (the decision on finding P-7): with the list
/// scrolled away from it by the wheel, Home and Up with the cursor already on `..`, and
/// End and Down with it already on the last row, bring the cursor row back into view.
fn keys_reveal(app: &mut App, failures: &mut Vec<String>) -> Result<(), String> {
    for (last, key, name) in [
        (false, Key::Home, "Home"),
        (false, Key::UpArrow, "Up"),
        (true, Key::End, "End"),
        (true, Key::DownArrow, "Down"),
    ] {
        to_top(app)?;
        if last {
            input::key(app.slint(), Key::End);
            app.settle();
        }
        let shown = panel::wait_rested(app, LEFT)?;
        let cursor = usize::try_from(app.cursor(LEFT)).map_err(|_| "no cursor row")?;
        // Three rows away from the cursor: down from the first row, up from the last.
        let rows = if last { 3.0 } else { -3.0 };
        panel::wheel_at(app, shown.center(), rows * shown.row_height);
        let away = panel::wait_rested(app, LEFT)?;
        if away.visible_row(cursor).is_some() {
            failures.push(format!(
                "{name}: the wheel did not scroll the cursor row {cursor} out of view ({:.1})",
                away.content_y
            ));
            continue;
        }
        input::key(app.slint(), key);
        app.settle();
        let after = panel::wait_rested(app, LEFT)?;
        if app.cursor(LEFT) != cursor as i32 || !after.whole_rows().contains(&cursor) {
            failures.push(format!(
                "P-7: {name} with the cursor already on {} row {cursor} left the list at {:.1} \
                 (cursor on row {}): the cursor row stays out of view; every navigation key \
                 reveals the cursor",
                if last { "the last" } else { "the first" },
                after.content_y,
                app.cursor(LEFT)
            ));
        }
    }
    Ok(())
}

/// The left list's offset sampled every couple of milliseconds without drawing (Slint's
/// timers only, which run the seam's wheel frames and the list's alignment), until it has
/// been still for 500 ms: the first offset and each change, with the milliseconds since
/// `since`. Changes as small as an eased glide's last frame (hundredths of a pixel) count.
fn trace(app: &mut App, since: Instant) -> Vec<(u128, f32)> {
    let mut changes = vec![(
        since.elapsed().as_millis(),
        app.window.get_left_list_content_y(),
    )];
    let started = Instant::now();
    let mut still = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(2));
        slint::platform::update_timers_and_animations();
        let y = app.window.get_left_list_content_y();
        let last = changes.last().map_or(y, |(_, last)| *last);
        if (y - last).abs() > 0.0001 {
            changes.push((since.elapsed().as_millis(), y));
            still = Instant::now();
        } else if still.elapsed() >= Duration::from_millis(500) {
            break;
        }
    }
    app.settle();
    changes
}

/// A trace as "ms:px" pairs.
fn show(changes: &[(u128, f32)]) -> String {
    changes
        .iter()
        .map(|(ms, y)| format!("{ms}:{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// How many sampled offsets lie strictly between `from` and `to`: frames of a glide.
fn between(changes: &[(u128, f32)], from: f32, to: f32) -> usize {
    let (low, high) = (from.min(to) + 0.5, from.max(to) - 0.5);
    changes
        .iter()
        .filter(|(_, y)| *y > low && *y < high)
        .count()
}

/// The notch distance the panels use, Chromium's (docs/developer/SCROLLING.md, "Wheel
/// distance"): 100 px at three lines per notch, in proportion to other line counts, 87.5 %
/// of the list's visible height for a screen per notch, and 120 px without a setting
/// (Linux).
fn notch_pixels(setting: WheelSetting, list: &ListGeometry) -> f32 {
    match setting {
        WheelSetting::Lines(lines) => 100.0 * lines.clamp(1, 100) as f32 / 3.0,
        WheelSetting::Page => (list.height * 0.875).max(1.0),
        WheelSetting::Fixed => 120.0,
    }
}

/// Where one notch down from `list`'s offset lands: the rows nearest the notch distance,
/// at least one, rounded to a row boundary (also from a cut offset).
fn landing(setting: WheelSetting, list: &ListGeometry) -> f32 {
    let row = list.row_height;
    let rows = (notch_pixels(setting, list) / row).round().max(1.0);
    (((list.content_y - rows * row) / row).round() * row).max(list.lowest())
}

/// One native notch down from the list's top in `view` for each of `settings`: the list
/// glides over several frames to the rows nearest the notch distance and rests there, on
/// a row boundary. The settings in force before return to the panels afterwards.
fn notches(
    app: &mut App,
    context: &mut Context,
    view: &str,
    settings: &[WheelSetting],
    failures: &mut Vec<String>,
) -> Result<(), String> {
    for setting in settings {
        let _system = system(*setting, false);
        native_focus(app, false, failures)?;
        let base = to_top(app)?;
        let expected = landing(*setting, &base);
        let started = Instant::now();
        panel::notch_at(app, base.center(), -1.0)?;
        let changes = trace(app, started);
        let rested = panel::wait_rested(app, LEFT)?;
        context.note(format!(
            "{view}, {setting:?}: one notch ({:.1} px over rows of {} px) from {:.1} \
             (ms:px): {}",
            notch_pixels(*setting, &base),
            base.row_height,
            base.content_y,
            show(&changes)
        ));
        if (rested.content_y - expected).abs() > 0.5 {
            failures.push(format!(
                "{view}, {setting:?}: one notch down from {:.1} rests at {:.1}, not at \
                 {expected:.1}: the rows nearest {:.1} px (rows of {} px)",
                base.content_y,
                rested.content_y,
                notch_pixels(*setting, &base),
                base.row_height
            ));
        }
        let frames = between(&changes, base.content_y, expected);
        if frames < 3 {
            failures.push(format!(
                "{view}, {setting:?}: the notch did not glide: {frames} sampled frames between \
                 {:.1} and {expected:.1} ({})",
                base.content_y,
                show(&changes)
            ));
        }
    }
    native_focus(app, false, failures)
}

/// Half a native notch down from the list's top: its share of the notch distance (50 px
/// at three lines), then the rest's alignment to the nearest row.
struct HalfNotch {
    /// Sampled offsets between the start and the share, and between the share and the row.
    share_frames: usize,
    align_frames: usize,
    /// Milliseconds from the share's arrival to the alignment's first frame, and from that
    /// frame to the row (0 for a move at once).
    delay: u128,
    glide: u128,
    changes: Vec<(u128, f32)>,
}

fn half_notch(
    app: &mut App,
    view: &str,
    failures: &mut Vec<String>,
) -> Result<Option<HalfNotch>, String> {
    let base = to_top(app)?;
    let row = base.row_height;
    let share = base.content_y - notch_pixels(WheelSetting::default(), &base) / 2.0;
    let target = ((share / row).round() * row).max(base.lowest());
    let started = Instant::now();
    panel::notch_at(app, base.center(), -0.5)?;
    let changes = trace(app, started);
    let rested = panel::wait_rested(app, LEFT)?;
    // The share ends at the sample before the longest pause, the idle interval before
    // the alignment (an eased glide reaches the last half pixel over several frames).
    let reached = (0..changes.len().saturating_sub(1))
        .max_by_key(|index| changes[index + 1].0 - changes[*index].0)
        .filter(|index| (changes[*index].1 - share).abs() <= 0.5);
    // The alignment ends with the last change: its final frame lands on the row (an
    // eased glide comes within half a pixel of it well before its end).
    let arrived = reached.and_then(|start| {
        let end = changes.len() - 1;
        (end > start && (changes[end].1 - target).abs() <= 0.5).then_some(end)
    });
    match (reached, arrived) {
        (Some(start), Some(end)) if (rested.content_y - target).abs() <= 0.5 => {
            let first = if end > start {
                changes[start + 1].0
            } else {
                changes[end].0
            };
            Ok(Some(HalfNotch {
                share_frames: between(&changes[..start], base.content_y, share),
                align_frames: between(&changes[start..end], share, target),
                delay: first - changes[start].0,
                glide: changes[end].0 - first,
                changes,
            }))
        }
        _ => {
            failures.push(format!(
                "{view}: half a notch down from {:.1} should move to {share:.1} and then rest \
                 on the row at {target:.1}; it rests at {:.1} ({})",
                base.content_y,
                rested.content_y,
                show(&changes)
            ));
            Ok(None)
        }
    }
}

/// Motion allowed: half a notch glides to its share and the rest then glides to the row,
/// no sooner than a whole 100 ms interval after the motion ended and for about 200 ms.
/// Less motion (Windows: animation effects off), read on a focus gain: a notch moves its
/// whole distance in one frame, and a half notch's share and its alignment move at once.
fn reduced_motion(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    if let Some(half) = half_notch(app, "Detailed, motion allowed", failures)? {
        context.note(format!(
            "Detailed, half a notch with motion allowed: the share glided over {} sampled \
             frames; the alignment began {} ms after it arrived and glided over {} frames for \
             {} ms (ms:px): {}",
            half.share_frames,
            half.delay,
            half.align_frames,
            half.glide,
            show(&half.changes)
        ));
        if half.share_frames < 3 || half.align_frames < 3 {
            failures.push(format!(
                "Detailed, motion allowed: half a notch moved over {} sampled frames and its \
                 alignment over {}; both glide",
                half.share_frames, half.align_frames
            ));
        }
        if half.delay < 90 {
            failures.push(format!(
                "Detailed: the alignment began {} ms after the motion ended; it waits for a \
                 whole 100 ms interval without motion",
                half.delay
            ));
        }
        if half.glide < 150 {
            failures.push(format!(
                "Detailed: the alignment reached the row {} ms after its first frame; it \
                 glides for 200 ms",
                half.glide
            ));
        }
    }
    let less = system(WheelSetting::default(), true);
    native_focus(app, true, failures)?;
    moves_at_once(app, context, "Detailed with less motion", failures)?;
    drop(less);
    native_focus(app, false, failures)?;
    glide_off(app, context, failures)
}

/// A notch and a half notch with its alignment move at once, without a glide.
fn moves_at_once(
    app: &mut App,
    context: &mut Context,
    label: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let base = to_top(app)?;
    let expected = landing(WheelSetting::default(), &base);
    let started = Instant::now();
    panel::notch_at(app, base.center(), -1.0)?;
    let changes = trace(app, started);
    let rested = panel::wait_rested(app, LEFT)?;
    context.note(format!(
        "{label}: one notch from {:.1} (ms:px): {}",
        base.content_y,
        show(&changes)
    ));
    let frames = between(&changes, base.content_y, expected);
    if (rested.content_y - expected).abs() > 0.5 || frames != 0 {
        failures.push(format!(
            "{label}: one notch down from {:.1} rests at {:.1} after {frames} \
             frames between; expected {expected:.1} at once ({})",
            base.content_y,
            rested.content_y,
            show(&changes)
        ));
    }
    if let Some(half) = half_notch(app, label, failures)? {
        context.note(format!(
            "{label}, half a notch: aligned {} ms after the share arrived \
             (ms:px): {}",
            half.delay,
            show(&half.changes)
        ));
        if half.share_frames != 0 || half.align_frames != 0 {
            failures.push(format!(
                "{label}: half a notch moved over {} sampled frames and its \
                 alignment over {}; both move at once ({})",
                half.share_frames,
                half.align_frames,
                show(&half.changes)
            ));
        }
    }
    Ok(())
}

/// Options › Appearance › Scrolling glide Off, chosen through the window's callback that
/// the Options slider invokes (the slider's own keys and the staging are covered by
/// `midnight-ui`'s tests): without the system's less motion, a notch and a half notch's
/// alignment move at once in the open panels; back at Default the notch glides again.
fn glide_off(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    app.window.invoke_scroll_glide_changed(0);
    if app.window.get_glide_scale() != 0.0 || app.window.get_scroll_glide_index() != 0 {
        failures.push(format!(
            "Scrolling glide Off: the window shows glide scale {} at stop {}",
            app.window.get_glide_scale(),
            app.window.get_scroll_glide_index()
        ));
    }
    moves_at_once(app, context, "Detailed with scrolling glide Off", failures)?;
    app.window.invoke_scroll_glide_changed(3);
    let base = to_top(app)?;
    let expected = landing(WheelSetting::default(), &base);
    let started = Instant::now();
    panel::notch_at(app, base.center(), -1.0)?;
    let changes = trace(app, started);
    panel::wait_rested(app, LEFT)?;
    let frames = between(&changes, base.content_y, expected);
    context.note(format!(
        "Detailed with scrolling glide Default again: one notch from {:.1} over {frames} \
         sampled frames (ms:px): {}",
        base.content_y,
        show(&changes)
    ));
    if frames < 3 {
        failures.push(format!(
            "Scrolling glide Default: one notch moved over {frames} sampled frames; it glides \
             ({})",
            show(&changes)
        ));
    }
    Ok(())
}

pub fn scrolling(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "scrolling")?;
    for index in 0..ROWS {
        ps::write(&folder.join(format!("r{index:03}")), "row")?;
    }
    let mut config = Config::new(&folder, &context.work);
    config.width = WIDTH;
    config.height = HEIGHT;
    let mut app = App::start(config);
    let mut failures = Vec::new();
    // Three lines per notch and motion allowed, whatever this machine is set to; the
    // guard ends the override with the scenario.
    let _system = system(WheelSetting::default(), false);
    native_focus(&mut app, false, &mut failures)?;
    // Every file marked: each drawn row shows the selection fill.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "a");
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        ps::marked(app, LEFT).len() == ROWS
    }) {
        return Err(format!(
            "Ctrl+A marked {} of {ROWS} files",
            ps::marked(&app, LEFT).len()
        ));
    }
    let fill = marked_fill(&mut app).ok_or("no marked row to measure the fill of")?;
    settles(&mut app, context, "Detailed", 2.5, fill, &mut failures)?;
    app.screenshot(
        context,
        "detailed-rested",
        "Detailed: rested on a whole row",
    );
    keys_reveal(&mut app, &mut failures)?;
    notches(
        &mut app,
        context,
        "Detailed",
        &[
            WheelSetting::Lines(3),
            WheelSetting::Lines(1),
            WheelSetting::Lines(6),
            WheelSetting::Page,
        ],
        &mut failures,
    )?;
    reduced_motion(&mut app, context, &mut failures)?;
    // Brief scrolls down its columns page by page and settles on rows too.
    panel::focus_list(&mut app, LEFT)?;
    ps::physical(&mut app, "Digit2", false, true, false);
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        app.window.get_left_view_mode() == 1
    }) {
        failures.push("Alt+2 did not switch to Brief".into());
    } else {
        settles(&mut app, context, "Brief", 1.5, fill, &mut failures)?;
        brief_scrolled_away(&mut app, &mut failures)?;
        notches(
            &mut app,
            context,
            "Brief",
            &[WheelSetting::default()],
            &mut failures,
        )?;
    }
    thumbnails(&mut app, context, fill, &mut failures)?;
    context.note(
        "SKIP: the alignment starting exactly 100 ms after the motion ends and gliding for \
         exactly 200 ms, and a new wheel stopping a glide where it is: need the scenario's \
         own clock (harness H1); checked in real time instead are the glide, a start no \
         sooner than 90 ms and a glide no shorter than 150 ms",
    );
    context.note(
        "SKIP: the system's own settings (Windows: lines to scroll at a time, animation \
         effects; other systems read none): the seam's settings override stands in for \
         them, read on a native focus gain as in production (decision A1)",
    );
    app.shutdown()?;
    support::report(&failures)
}

/// Brief scrolled until its cursor row is out of view (`panels_view_sort`'s Brief list
/// fits its viewport): after Tab a click gives the list back the keyboard with the
/// cursor and the marks kept, and a click on a row in view puts the cursor on it.
fn brief_scrolled_away(app: &mut App, failures: &mut Vec<String>) -> Result<(), String> {
    let list = panel::wait_rested(app, LEFT)?;
    let cursor = usize::try_from(app.cursor(LEFT)).map_err(|_| "no cursor row in Brief")?;
    if list.visible_row(cursor).is_some() {
        failures.push(format!(
            "Brief: the cursor row {cursor} is still in view at content y {:.1}",
            list.content_y
        ));
        return Ok(());
    }
    input::key(app.slint(), Key::Tab);
    app.settle();
    if app.window.get_focused_pane() != 1 {
        failures.push(format!(
            "Brief: Tab did not move the keyboard to the right panel (focused pane {})",
            app.window.get_focused_pane()
        ));
    }
    match panel::focus_list(app, LEFT) {
        Err(error) => failures.push(format!("Brief, cursor scrolled out of view: {error}")),
        Ok(()) => {
            if app.cursor(LEFT) != cursor as i32 {
                failures.push(format!(
                    "Brief: focusing the scrolled list moved the cursor from row {cursor} to {}",
                    app.cursor(LEFT)
                ));
            }
            if ps::marked(app, LEFT).len() != ROWS {
                failures.push(format!(
                    "Brief: focusing the scrolled list left {} of {ROWS} files marked",
                    ps::marked(app, LEFT).len()
                ));
            }
        }
    }
    let list = panel::wait_rested(app, LEFT)?;
    let whole: Vec<usize> = list
        .whole_rows()
        .into_iter()
        .filter(|index| *index != cursor && *index != 0)
        .collect();
    let Some(&index) = whole.get(whole.len() / 2) else {
        failures.push(format!("Brief: no row is shown whole to click ({list:?})"));
        return Ok(());
    };
    let position = panel::row_position(app, LEFT, index);
    panel::click(app, position);
    app.settle();
    if app.cursor(LEFT) != index as i32 {
        failures.push(format!(
            "Brief: a click at row {index} ({position:?}) put the cursor on row {}",
            app.cursor(LEFT)
        ));
    }
    if ps::marked(app, LEFT).len() != ROWS {
        failures.push(format!(
            "Brief: the click on row {index} left {} of {ROWS} files marked",
            ps::marked(app, LEFT).len()
        ));
    }
    Ok(())
}

/// Thumbnails keeps the exact offset a wheel leaves, so a tile row stays cut at the
/// list's top; a click 2 px into the list selects a tile of that row. A native notch from
/// a cut offset moves the bands nearest the notch distance (at least one) and lands on a
/// band boundary.
fn thumbnails(
    app: &mut App,
    context: &mut Context,
    fill: [u8; 3],
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    ps::physical(app, "Digit5", false, true, false);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.window.get_left_view_mode() == 2
    }) {
        failures.push("Alt+5 did not switch to Thumbnails".into());
        return Ok(());
    }
    let start = panel::wait_rested(app, LEFT)?;
    let gesture = wheel_and_sample(app, 1.4, fill)?;
    let list = panel::wait_rested(app, LEFT)?;
    let after_wheel = gesture.offsets.last().map_or(list.content_y, |(_, y)| *y);
    context.note(format!(
        "Thumbnails: offsets after a wheel of 1.4 grid rows (ms, px): {:?}",
        gesture.offsets
    ));
    if (list.content_y - after_wheel).abs() > 0.01
        || (after_wheel - start.content_y).abs() < 1.0
        || aligned(&list, list.content_y)
    {
        failures.push(format!(
            "Thumbnails: the wheel moved the list {:.1} → {after_wheel:.1} and it rests at \
             {:.1} (grid rows of {} px); expected it to keep the cut offset",
            start.content_y, list.content_y, list.row_height
        ));
    }
    if !gesture.blank.is_empty() {
        failures.push(format!(
            "Thumbnails: a frame left the list's top band blank: {}",
            gesture.blank.join("; ")
        ));
    }
    // The cut row at the top takes a click 2 px into the list.
    let band = ((2.0 - list.content_y) / list.row_height).floor() as usize;
    let column = 1.min(list.columns - 1);
    let index = band * list.columns + column;
    let point = LogicalPosition::new(
        list.left + (column as f32 + 0.5) * list.grid_width / list.columns as f32,
        list.top + 2.0,
    );
    panel::click(app, point);
    app.settle();
    if app.cursor(LEFT) != index as i32 {
        failures.push(format!(
            "a click 2 px into the Thumbnails list (content y {:.1}) put the cursor on row {}, \
             not on the cut row's tile {index}",
            list.content_y,
            app.cursor(LEFT)
        ));
    }
    if ps::marked(app, LEFT).len() != ROWS {
        failures.push("the click on the cut tile changed the marks".into());
    }
    app.screenshot(
        context,
        "thumbnails-cut",
        "Thumbnails: a cut tile row kept at the top",
    );
    // A native notch from a cut offset (the click may have revealed its whole tile).
    let mut base = panel::wait_rested(app, LEFT)?;
    if aligned(&base, base.content_y) {
        panel::wheel_at(app, base.center(), -0.5 * base.row_height);
        base = panel::wait_rested(app, LEFT)?;
    }
    if aligned(&base, base.content_y) {
        failures.push(format!(
            "Thumbnails: half a band's wheel left the grid on a band boundary at {:.1}",
            base.content_y
        ));
    } else {
        let expected = landing(WheelSetting::default(), &base);
        let started = Instant::now();
        panel::notch_at(app, base.center(), -1.0)?;
        let changes = trace(app, started);
        let rested = panel::wait_rested(app, LEFT)?;
        context.note(format!(
            "Thumbnails: one notch from the cut offset {:.1} (bands of {} px) (ms:px): {}",
            base.content_y,
            base.row_height,
            show(&changes)
        ));
        if (rested.content_y - expected).abs() > 0.5 {
            failures.push(format!(
                "Thumbnails: one notch down from the cut offset {:.1} rests at {:.1}, not on \
                 the band boundary {expected:.1}",
                base.content_y, rested.content_y
            ));
        }
    }
    ps::physical(app, "Digit3", false, true, false);
    Ok(())
}
