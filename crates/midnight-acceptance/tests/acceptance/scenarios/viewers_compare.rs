//! The file comparator, as a person uses it: mark two files with Insert, Compare
//! (Ctrl+Shift+C), read the verdict and the compared text, move between differences
//! with N, P, F7, Alt+Home, Alt+End and the Next difference button, switch the image
//! comparison modes, and close with Escape or Close. Pairs: identical text, three
//! differences, a 3.5 MiB log, the same text in two encodings and images; binaries are
//! compared byte by byte (hex view, summary, the same keys between differing regions);
//! the 3.5 MiB log against the 4.25 MiB one (over the former 4 MiB text limit) is
//! compared as text, and its Hex button shows the bytes.
//!
//! Further: every difference key (Shift+F7, Alt+Down, Alt+Up, Alt+Right, Alt+Left) and
//! the Previous difference button; Find in both files (the Find… button and Ctrl+F,
//! Enter, F3, Shift+F3, "Match N of M · File · line", Aa, Whole words, Clear Find);
//! scrolling the compared text as the text viewer scrolls (an arrow moves 28 logical
//! pixels, PageDown the whole view, one native wheel notch Chromium's distance through
//! the comparator's acceptance seam, a Slint pixel delta exactly its pixels, the
//! position bar); the window's
//! close request (its ×); the image controls (Fit, Zoom in and out,
//! Ctrl+wheel, the Swipe divider and Right image opacity sliders, dragging the divider
//! on the picture); text over the comparator's limits (64 MiB, 1,000,000 lines, a line
//! over 16 KiB) compared as bytes with the reason; and File A / File B chosen by hand
//! (one side resolved from the panel, a missing path refused in the form, Browse…, the
//! Compare button, Cancel).
use crate::{
    Context,
    app::{App, Config},
    input, inspect, panel, viewers_stage,
    viewers_support::{self as support, Canvas, Failures, clip, generated, millis, probe},
};
use i_slint_core::items::AccessibleRole;
use midnight_compare_ui::ComparatorWindow;
use midnight_core::PaneId;
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(25 * 60);
const VERDICT_LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_secs(10);

/// The `identity-state` a comparison must reach.
const IDENTICAL: i32 = 1;
const SAME_TEXT: i32 = 2;
const DIFFERENT: i32 = 3;
/// The accessible label of the comparator's Hex button.
const HEX: &str = "Compare bytes (hex)";
/// Logical pixels Up and Down move the text, as in the text viewer
/// (`midnight_ui_input::KEY_LINE_PIXELS`).
const KEY_LINE: f32 = 28.0;

pub fn compare(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

struct Comparing {
    window: ComparatorWindow,
    software: Rc<MinimalSoftwareWindow>,
}

impl Comparing {
    fn slint(&self) -> &slint::Window {
        self.window.window()
    }

    fn wait(
        &self,
        app: &mut App,
        what: &str,
        limit: Duration,
        condition: impl Fn(&ComparatorWindow) -> bool,
    ) -> Result<(), String> {
        let deadline = Instant::now() + limit;
        loop {
            app.settle();
            support::paint_tool_windows(app);
            if condition(&self.window) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out after {limit:?} waiting for {what}; status {:?}, identity {}, text {:?}",
                    self.window.get_status(),
                    self.window.get_identity_state(),
                    clip(&self.window.get_accessible_text(), 200)
                ));
            }
            app.idle(Duration::from_millis(10));
        }
    }

    /// The rendered compared content only.
    fn canvas(&self) -> Canvas {
        let (width, _, pixels) = crate::app::render(&self.software);
        let scale = self.slint().scale_factor();
        let window = &self.window;
        Canvas::crop(
            &pixels,
            width,
            (window.get_content_left() * scale).max(0.0) as usize,
            (window.get_content_top() * scale).max(0.0) as usize,
            (window.get_content_width() * scale).max(0.0) as usize,
            (window.get_content_height() * scale).max(0.0) as usize,
        )
    }

    /// Render the content until it holds still for [`probe::QUIET`].
    fn still(&self, app: &mut App) -> Result<Canvas, String> {
        app.settle();
        let started = Instant::now();
        let mut previous = None;
        let mut since: Option<Instant> = None;
        loop {
            probe::pause(app, Duration::from_millis(30));
            let canvas = self.canvas();
            let print = canvas.fingerprint();
            if previous != Some(print) {
                since = None;
            } else if since.is_none() {
                since = Some(Instant::now());
            }
            previous = Some(print);
            if since.is_some_and(|since| since.elapsed() >= probe::QUIET) {
                return Ok(canvas);
            }
            if started.elapsed() >= STEP {
                return Err("the compared content kept changing".into());
            }
        }
    }

    /// How far the compared content moves (rows of pixels, towards the end) after `act`.
    fn moved(&self, app: &mut App, act: impl FnOnce(&slint::Window)) -> Result<i32, String> {
        let before = self.still(app)?;
        act(self.slint());
        let after = self.still(app)?;
        match probe::vertical_shift(&before, &after) {
            Some((shift, share)) if share >= probe::SHIFT_CONFIDENCE => Ok(shift),
            other => Err(format!(
                "the compared content did not move as one piece: {other:?} (shift, share)"
            )),
        }
    }

    /// Click the middle of the compared content, so difference keys go to it.
    fn click_content(&self, app: &mut App) {
        let window = &self.window;
        let position = slint::LogicalPosition::new(
            window.get_content_left() + window.get_content_width() / 2.0,
            window.get_content_top() + window.get_content_height() / 2.0,
        );
        input::click_at(self.slint(), position);
        app.settle();
    }

    fn close(self, app: &mut App, button: bool) -> Result<(), String> {
        if button {
            input::click_button(self.slint(), "Close")?;
        } else {
            input::key(self.slint(), Key::Escape);
        }
        self.closed(app, if button { "Close" } else { "Escape" })
    }

    /// The window's own close button (its title bar's ×): the platform's close request.
    fn close_requested(self, app: &mut App) -> Result<(), String> {
        self.slint().dispatch_event(WindowEvent::CloseRequested);
        self.closed(app, "The window's close request")
    }

    /// Wait until the comparator window is gone after `what`, then give the main window
    /// back its activation.
    fn closed(self, app: &mut App, what: &str) -> Result<(), String> {
        let deadline = Instant::now() + STEP;
        while support::software_window(app, self.slint()).is_some() {
            if Instant::now() >= deadline {
                return Err(format!("{what} did not close the comparator"));
            }
            app.idle(Duration::from_millis(10));
        }
        app.slint()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        app.settle();
        Ok(())
    }
}

/// Mark `first` and `second` with Insert, press Compare, wait for the verdict.
fn open(
    app: &mut App,
    context: &mut Context,
    first: &str,
    second: &str,
    wanted: i32,
) -> Result<Comparing, String> {
    open_in(app, context, PaneId::Left, first, second, wanted)
}

/// Mark `first` and `second` in `pane` with Insert, press Compare, wait for the verdict.
fn open_in(
    app: &mut App,
    context: &mut Context,
    pane: PaneId,
    first: &str,
    second: &str,
    wanted: i32,
) -> Result<Comparing, String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    for name in [first, second] {
        panel::select_row(app, pane, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    let before = support::plugins(app)?.compared.borrow().len();
    let started = Instant::now();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let compared = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .cloned();
    let Some(compared) = compared else {
        return Err(format!(
            "Ctrl+Shift+C opened no comparator for {first} and {second}; notice {:?}",
            app.notice()
        ));
    };
    let window = compared
        .window
        .upgrade()
        .ok_or("the comparator closed at once")?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let software = support::software_window(app, window.window())
        .ok_or("the comparator window is not visible")?;
    let comparing = Comparing { window, software };
    comparing.wait(app, "the comparison verdict", VERDICT_LIMIT, |window| {
        window.get_identity_state() == wanted
    })?;
    let elapsed = started.elapsed();
    context.note(format!(
        "Compare {first} / {second}: verdict {} after {}; status {:?}",
        comparing.window.get_identity_state(),
        millis(elapsed),
        comparing.window.get_status()
    ));
    context.note(format!(
        "latency\tcomparison\t{first} / {second}\t{}\t{}",
        elapsed.as_millis(),
        clip(&comparing.window.get_status(), 80)
    ));
    Ok(comparing)
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let limits = context.work.join("limits");
    prepare_limits(&limits)?;
    let mut config = Config::new(&root, &limits);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let failures = Failures::default();
    panel::enter(&mut app, PaneId::Left, "compare")?;

    failures.check(
        "identical text",
        open(&mut app, context, "same-a.txt", "same-b.txt", IDENTICAL).and_then(|comparing| {
            comparing.wait(
                &mut app,
                "identical text at the beginning",
                STEP,
                |window| {
                    window.get_status().contains("No line differences")
                        && top_shows(window, "000 ", "000 ")
                        && window.get_position() == 0.0
                },
            )?;
            app.screenshot_window(
                context,
                &comparing.software,
                "identical",
                "same-a.txt and same-b.txt: identical files, all bytes verified",
            );
            comparing.close(&mut app, false)
        }),
    );

    failures.check(
        "three differences",
        open(&mut app, context, "left.txt", "right.txt", DIFFERENT)
            .and_then(|comparing| differences(&mut app, context, comparing)),
    );

    failures.check(
        "large logs",
        open(&mut app, context, "large-a.log", "large-b.log", DIFFERENT).and_then(|comparing| {
            comparing.wait(
                &mut app,
                "the first changed line on opening",
                STEP,
                |window| {
                    window.get_status().contains("Difference 1 of 2")
                        && top_shows(window, "00002048 ", "00002048 FIRST CHANGE")
                },
            )?;
            app.screenshot_window(
                context,
                &comparing.software,
                "large",
                "The opening view shows both sides of the first change after 2,048 matching lines",
            );
            comparing.click_content(&mut app);
            input::key(comparing.slint(), Key::F7);
            comparing.wait(&mut app, "F7 to reach the other change", STEP, |window| {
                window.get_status().contains("Difference 2 of 2")
                    && top_shows(window, "", "LAST CHANGE")
            })?;
            comparing.close(&mut app, false)
        }),
    );

    failures.check(
        "same text in two encodings",
        open(
            &mut app,
            context,
            "text-utf16.txt",
            "text-utf8.txt",
            SAME_TEXT,
        )
        .and_then(|comparing| {
            app.screenshot_window(
                context,
                &comparing.software,
                "encodings",
                "text-utf8.txt and text-utf16.txt: identical text, bytes differ (encoding/BOM)",
            );
            // The window's own close button: the platform's close request.
            comparing.close_requested(&mut app)
        }),
    );

    failures.check(
        "images",
        open(&mut app, context, "image-a.png", "image-b.png", DIFFERENT)
            .and_then(|comparing| images(&mut app, context, comparing)),
    );

    failures.check(
        "binary files",
        open(&mut app, context, "binary-a.bin", "binary-b.bin", DIFFERENT)
            .and_then(|comparing| binaries(&mut app, context, comparing)),
    );

    failures.check(
        "text over the former 4 MiB limit",
        open(
            &mut app,
            context,
            "large-a.log",
            "over-limit.log",
            DIFFERENT,
        )
        .and_then(|comparing| over_former_limit(&mut app, context, comparing)),
    );
    failures.check(
        "Find in the compared files",
        open(&mut app, context, "left.txt", "right.txt", DIFFERENT)
            .and_then(|comparing| find(&mut app, context, comparing)),
    );
    match open(&mut app, context, "left.txt", "right.txt", DIFFERENT).and_then(|comparing| {
        let chosen = font_choice(&mut app, context, &comparing);
        comparing.close(&mut app, false)?;
        chosen
    }) {
        Ok(true) => failures.check("the text font chooser", Ok(())),
        Ok(false) => failures.skip(
            "the text font chooser",
            "every other fixed-pitch family offered here renders exactly like the current one",
        ),
        Err(error) => failures.check("the text font chooser", Err(error)),
    }
    failures.check(
        "scrolling the compared text",
        open(&mut app, context, "large-a.log", "large-b.log", DIFFERENT)
            .and_then(|comparing| scrolling(&mut app, context, comparing)),
    );
    input::chord(app.slint(), &[Key::Control], "d");
    failures.check(
        "File A and File B chosen by hand",
        choose_files(&mut app, context),
    );
    for (first, second, reason) in [
        (
            "big-a.txt",
            "big-b.txt",
            "text comparison supports files up to 64 MiB",
        ),
        (
            "lines-a.txt",
            "lines-b.txt",
            "text comparison supports up to 1,000,000 lines per file",
        ),
        (
            "wide-a.txt",
            "wide-b.txt",
            "text comparison supports lines up to 16 KiB",
        ),
    ] {
        failures.check(
            &format!("{first} and {second} over a text limit"),
            open_in(&mut app, context, PaneId::Right, first, second, DIFFERENT)
                .and_then(|comparing| over_limit(&mut app, context, comparing, reason)),
        );
    }
    input::chord(app.slint(), &[Key::Control], "d");
    panel::focus_list(&mut app, PaneId::Left)?;
    panel::leave(&mut app, PaneId::Left)?;
    app.shutdown()?;
    failures.finish(context)
}

/// Text pairs over each of the comparator's text limits, each differing in one line.
fn prepare_limits(folder: &std::path::Path) -> Result<(), String> {
    let line = "0123456789 abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ text\n";
    let big = line.repeat((65 << 20) / line.len() + 1);
    generated::write(folder, "big-a.txt", &big)?;
    generated::write(folder, "big-b.txt", big.replacen("text\n", "TEXT\n", 1))?;
    let lines = "x\n".repeat(1_000_001);
    generated::write(folder, "lines-a.txt", &lines)?;
    generated::write(folder, "lines-b.txt", lines.replacen("x\n", "y\n", 1))?;
    let wide = format!("{}\nshort line\n", "w".repeat(17 * 1024));
    generated::write(folder, "wide-a.txt", &wide)?;
    generated::write(folder, "wide-b.txt", wide.replacen("short", "SHORT", 1))?;
    Ok(())
}

/// A pair the text comparison does not admit is compared as bytes, and the status
/// says why.
fn over_limit(
    app: &mut App,
    context: &mut Context,
    comparing: Comparing,
    reason: &str,
) -> Result<(), String> {
    let window = comparing.window.clone_strong();
    let shown = comparing.wait(
        app,
        "the reason and the byte summary",
        VERDICT_LIMIT,
        |window| window.get_bytes_mode() && window.get_status().contains("compared as bytes"),
    );
    let status = window.get_status().to_string();
    app.screenshot_window(
        context,
        &comparing.software,
        &format!("limit-{}", reason.split_whitespace().last().unwrap_or("x")),
        &format!("Over a text limit: {}", clip(&status, 140)),
    );
    comparing.close(app, false)?;
    shown?;
    if !window.get_bytes_only() {
        return Err(format!(
            "a pair over a text limit can still switch to text: {status:?}"
        ));
    }
    if !status.contains(reason) {
        return Err(format!("the status does not say {reason:?}: {status:?}"));
    }
    Ok(())
}

/// Find in both files: the Find… button opens the bar; Enter, F3 and Shift+F3 step
/// through the two matches with their file and line; Aa and Whole words narrow them;
/// Clear Find empties the query.
fn find(app: &mut App, context: &mut Context, comparing: Comparing) -> Result<(), String> {
    let window = comparing.window.clone_strong();
    let result = (|| {
        input::click_button(comparing.slint(), "Find in compared files (Ctrl+F)")?;
        comparing.wait(app, "the find bar", STEP, |window| {
            window.get_search_visible()
        })?;
        input::type_text(comparing.slint(), "line");
        input::key(comparing.slint(), Key::Return);
        let summary = |wanted: &'static str| {
            move |window: &ComparatorWindow| window.get_search_summary().as_str() == wanted
        };
        comparing.wait(
            app,
            "the first match",
            STEP,
            summary("Match 1 of 2 · File B · line 11"),
        )?;
        input::key(comparing.slint(), Key::F3);
        comparing.wait(
            app,
            "F3 to the second match",
            STEP,
            summary("Match 2 of 2 · File B · line 62"),
        )?;
        app.screenshot_window(
            context,
            &comparing.software,
            "find-second-match",
            "Find \"line\" in left.txt and right.txt: the second match",
        );
        input::key(comparing.slint(), Key::F3);
        comparing.wait(
            app,
            "F3 to wrap to the first match",
            STEP,
            summary("Match 1 of 2 · File B · line 11"),
        )?;
        input::chord(comparing.slint(), &[Key::Shift], Key::F3);
        comparing.wait(
            app,
            "Shift+F3 back to the second",
            STEP,
            summary("Match 2 of 2 · File B · line 62"),
        )?;
        input::chord(comparing.slint(), &[Key::Shift], Key::Return);
        comparing.wait(
            app,
            "Shift+Enter back to the first",
            STEP,
            summary("Match 1 of 2 · File B · line 11"),
        )?;
        input::click(
            comparing.slint(),
            AccessibleRole::Checkbox,
            "Case sensitive",
        )?;
        input::click(
            comparing.slint(),
            AccessibleRole::TextInput,
            "Find in compared files",
        )?;
        input::replace_text(comparing.slint(), "LINE");
        input::key(comparing.slint(), Key::Return);
        comparing.wait(app, "Aa: no upper-case LINE", STEP, summary("No matches"))?;
        input::click(
            comparing.slint(),
            AccessibleRole::Checkbox,
            "Case sensitive",
        )?;
        input::click(comparing.slint(), AccessibleRole::Checkbox, "Whole words")?;
        input::click(
            comparing.slint(),
            AccessibleRole::TextInput,
            "Find in compared files",
        )?;
        input::replace_text(comparing.slint(), "lin");
        input::key(comparing.slint(), Key::Return);
        comparing.wait(app, "Whole words: no word lin", STEP, summary("No matches"))?;
        input::click(comparing.slint(), AccessibleRole::Checkbox, "Whole words")?;
        input::click_button(comparing.slint(), "Clear Find")?;
        comparing.wait(app, "Clear Find", STEP, |window| {
            window.get_query().is_empty() && window.get_search_summary().is_empty()
        })
    })();
    context.note(format!(
        "Find in comparison: summary {:?}",
        window.get_search_summary()
    ));
    comparing.close(app, false)?;
    result
}

/// The font chooser: another installed fixed-pitch family repaints the compared text.
/// Ok(false) when every other family offered renders exactly like the current one on
/// this machine (System monospace may be the same face), so nothing can be seen.
fn font_choice(
    app: &mut App,
    context: &mut Context,
    comparing: &Comparing,
) -> Result<bool, String> {
    comparing.wait(app, "the font list", VERDICT_LIMIT, |window| {
        window.get_fonts_ready()
    })?;
    let window = &comparing.window;
    let families: Vec<String> = window
        .get_font_families()
        .iter()
        .map(|family| family.to_string())
        .collect();
    let first = usize::try_from(window.get_font_index()).unwrap_or(usize::MAX);
    let before = comparing.still(app)?.fingerprint();
    for (index, other) in families
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != first)
    {
        input::click(
            comparing.slint(),
            AccessibleRole::Combobox,
            "Text comparison font",
        )?;
        app.settle();
        input::click(comparing.slint(), AccessibleRole::Button, other)?;
        comparing.wait(app, "the chosen font", STEP, |window| {
            window.get_font_index() == index as i32
        })?;
        let after = comparing.still(app)?.fingerprint();
        context.note(format!(
            "Comparison fonts {families:?}: {:?} → {other:?} {}",
            families.get(first),
            if after == before {
                "renders alike"
            } else {
                "repaints"
            }
        ));
        if after != before {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The compared text scrolls as the text viewer's: an arrow moves 28 logical pixels and
/// PageDown the whole view, a native wheel notch (through the comparator's acceptance
/// seam) glides Chromium's distance for the system's wheel setting, a Slint pixel delta
/// moves exactly its pixels; the position bar jumps where it is clicked.
fn scrolling(app: &mut App, context: &mut Context, comparing: Comparing) -> Result<(), String> {
    let result = (|| {
        comparing.click_content(app);
        // The text viewer's key distances (`midnight_ui_input::key_scroll`): 28 logical
        // pixels a line, the whole view a page, at once.
        let scale = comparing.slint().scale_factor();
        let line = (KEY_LINE * scale).round() as i32;
        let row = comparing.moved(app, |window| input::key(window, Key::DownArrow))?;
        let back = comparing.moved(app, |window| input::key(window, Key::UpArrow))?;
        if row.abs_diff(line) > 1 || back != -row {
            return Err(format!(
                "Down moved {row} px and Up {back} px; the text viewer moves {line} px"
            ));
        }
        let view = comparing.window.get_content_height();
        let screen = (view * scale).round() as i32;
        // PageDown leaves no row in common: page, then arrows back half a screen.
        let lines_back = (view / 2.0 / KEY_LINE) as i32;
        let page = comparing.moved(app, |window| {
            input::key(window, Key::PageDown);
            for _ in 0..lines_back {
                input::key(window, Key::UpArrow);
            }
        })?;
        let expected = ((view - lines_back as f32 * KEY_LINE) * scale).round() as i32;
        if page.abs_diff(expected) > 1 {
            return Err(format!(
                "PageDown and {lines_back} Up moved {page} px, not {expected} px: PageDown \
                 moves the whole view ({screen} px) as in the text viewer"
            ));
        }
        let at = slint::LogicalPosition::new(
            comparing.window.get_content_left() + comparing.window.get_content_width() / 2.0,
            comparing.window.get_content_top() + comparing.window.get_content_height() / 2.0,
        );
        // One native wheel notch glides the text the text viewer's distance.
        let notch_pixels = probe::system_notch_pixels(view);
        let wanted = (notch_pixels * scale).round() as i32;
        let before = comparing.still(app)?;
        probe::comparator_notches(app, &comparing.window, at, 1)?;
        let after = comparing.still(app)?;
        let notch = match probe::vertical_shift(&before, &after) {
            Some((shift, share)) if share >= probe::SHIFT_CONFIDENCE => shift,
            other => {
                return Err(format!(
                    "a wheel notch did not move the compared content as one piece: {other:?}"
                ));
            }
        };
        if notch.abs_diff(wanted) > 1 {
            return Err(format!(
                "one wheel notch moved the compared text {notch} px; the text viewer moves \
                 {wanted} px ({notch_pixels} logical px)"
            ));
        }
        // A Slint pixel delta, not the product's notch (`probe::SLINT_PIXEL_DELTA`).
        let delta = comparing.moved(app, |window| probe::pixel_deltas(window, at, 1))?;
        if delta.abs_diff(probe::SLINT_PIXEL_DELTA as i32) > 1 {
            return Err(format!(
                "a Slint pixel delta of {} px moved the compared text {delta} px",
                probe::SLINT_PIXEL_DELTA
            ));
        }
        // The position bar right of the content: a click at three quarters.
        let window = &comparing.window;
        let bar = slint::LogicalPosition::new(
            window.get_content_left() + window.get_content_width() + 4.0 + 7.0,
            window.get_content_top() + window.get_content_height() * 0.75,
        );
        input::click_at(comparing.slint(), bar);
        comparing.wait(app, "the position bar to seek", STEP, |window| {
            (window.get_position() - 0.75).abs() < 0.03
        })?;
        app.screenshot_window(
            context,
            &comparing.software,
            "position-bar",
            "large-a.log and large-b.log after a click at three quarters of the position bar",
        );
        context.note(format!(
            "Comparison scrolling: Down {row} px, PageDown and {lines_back} Up {page} px, wheel \
             notch {notch} px, Slint pixel delta {delta} px, position bar {:.3}",
            comparing.window.get_position()
        ));
        Ok(())
    })();
    comparing.close(app, false)?;
    result
}

/// Compare with nothing marked: File A is the focused file, File B has no same-name
/// file to come from, so the form asks; a missing path is refused there, Browse…
/// fills File B, Compare compares; Cancel closes a new form.
fn choose_files(app: &mut App, context: &mut Context) -> Result<(), String> {
    let folder = context.fixture.path("compare");
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    panel::focus_list(app, PaneId::Right)?;
    input::chord(app.slint(), &[Key::Control], "d");
    panel::select_row(app, PaneId::Left, "left.txt")?;
    let before = support::plugins(app)?.compared.borrow().len();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let window = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .and_then(|compared| compared.window.upgrade())
        .ok_or_else(|| {
            format!(
                "Ctrl+Shift+C opened no comparator; notice {:?}",
                app.notice()
            )
        })?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let software = support::software_window(app, window.window())
        .ok_or("the comparator window is not visible")?;
    let comparing = Comparing { window, software };
    let result = (|| {
        comparing.wait(app, "the File A / File B form", STEP, |window| {
            window.get_choosing_files()
        })?;
        let file_a = comparing.window.get_file_a().to_string();
        if file_a != folder.join("left.txt").to_string_lossy()
            || !comparing.window.get_file_b().is_empty()
        {
            return Err(format!(
                "the form shows File A {file_a:?} and File B {:?}",
                comparing.window.get_file_b()
            ));
        }
        let shot = app.screenshot_window(
            context,
            &comparing.software,
            "choose-files",
            "Ctrl+Shift+C with nothing marked: File B is asked for",
        );
        for finding in probe::review(context, comparing.slint(), &shot) {
            context.note(format!("layout review: {finding}"));
        }
        // A missing file is refused in the form, which keeps both drafts.
        let missing = folder.join("missing.txt").to_string_lossy().into_owned();
        input::fill(comparing.slint(), "File B", &missing)?;
        input::key(comparing.slint(), Key::Return);
        comparing.wait(app, "the missing file's error", STEP, |window| {
            window.get_choosing_files() && window.get_choice_error() && !window.get_choice_busy()
        })?;
        if comparing.window.get_file_a().as_str() != file_a
            || comparing.window.get_file_b().as_str() != missing
        {
            return Err("the refused form lost its drafts".into());
        }
        context.note(format!(
            "A missing File B: {:?}",
            comparing.window.get_choice_message()
        ));
        // Browse… answers with right.txt (the scripted file chooser).
        if !comparing.window.get_picker_available() {
            return Err("Browse… is unavailable".into());
        }
        app.picker.answer(Some(folder.join("right.txt")));
        input::click_button(comparing.slint(), "Browse for File B")?;
        let wanted = folder.join("right.txt").to_string_lossy().into_owned();
        comparing.wait(app, "Browse… to fill File B", STEP, |window| {
            window.get_file_b().as_str() == wanted && !window.get_choice_busy()
        })?;
        input::click_button(comparing.slint(), "Compare")?;
        comparing.wait(app, "the chosen files' verdict", VERDICT_LIMIT, |window| {
            !window.get_choosing_files() && window.get_identity_state() == DIFFERENT
        })?;
        app.screenshot_window(
            context,
            &comparing.software,
            "chosen-files",
            "left.txt and right.txt chosen by hand (Browse… for File B)",
        );
        Ok(())
    })();
    comparing.close(app, false)?;
    result?;
    // Cancel closes a new form.
    let before = support::plugins(app)?.compared.borrow().len();
    panel::select_row(app, PaneId::Left, "left.txt")?;
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let window = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .and_then(|compared| compared.window.upgrade())
        .ok_or("the second Ctrl+Shift+C opened no comparator")?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    app.settle();
    input::click_button(window.window(), "Cancel")?;
    let deadline = Instant::now() + STEP;
    while support::software_window(app, window.window()).is_some() {
        if Instant::now() >= deadline {
            return Err("Cancel did not close the File A / File B form".into());
        }
        app.idle(Duration::from_millis(10));
    }
    app.slint()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    app.settle();
    Ok(())
}

/// The first visible aligned text row contains each side's expected text.
fn top_shows(window: &ComparatorWindow, left: &str, right: &str) -> bool {
    window
        .get_accessible_text()
        .lines()
        .next()
        .and_then(|row| row.split_once('\t'))
        .is_some_and(|(a, b)| a.contains(left) && b.contains(right))
}

fn differences(app: &mut App, context: &mut Context, comparing: Comparing) -> Result<(), String> {
    let window = comparing.window.clone_strong();
    let changed = "010 this line differs in right.txt";
    let inserted = "inserted line only in right.txt";
    comparing.wait(app, "the opening difference", STEP, |window| {
        window.get_status().contains("Difference 1 of 3") && top_shows(window, "010 ", changed)
    })?;
    let shot = app.screenshot_window(
        context,
        &comparing.software,
        "differences",
        "left.txt and right.txt: three differences",
    );
    for finding in probe::review(context, comparing.slint(), &shot) {
        context.note(format!("layout review: {finding}"));
    }
    // Click into the content so difference keys go to it.
    comparing.click_content(app);
    comparing.still(app)?;
    let first = window.get_position();
    input::key(comparing.slint(), Key::F7);
    comparing.wait(app, "F7 to reach the inserted line", STEP, |window| {
        window.get_status().contains("Difference 2 of 3")
            && top_shows(window, "", inserted)
            && window.get_position() > first
    })?;
    app.screenshot_window(
        context,
        &comparing.software,
        "second-difference",
        "F7: the second difference (a line only in right.txt)",
    );
    // A changed visible row count confirms the worker painted the new geometry.
    // Each resize must keep the second difference where the user navigated.
    let size = comparing.slint().size();
    let mut previous_rows = window.get_accessible_text().lines().count();
    for height in [size.height / 2, size.height] {
        crate::app::layout_tool(&comparing.software, size.width, height);
        comparing.wait(app, "resized second difference", STEP, |window| {
            let text = window.get_accessible_text();
            text.lines().count() != previous_rows
                && text
                    .lines()
                    .next()
                    .is_some_and(|row| row.contains(inserted))
                && window.get_status().contains("Difference 2 of 3")
        })?;
        previous_rows = window.get_accessible_text().lines().count();
    }
    input::key(comparing.slint(), "p");
    comparing.wait(app, "P to return to the first difference", STEP, |window| {
        window.get_position() < first + 0.0001 && window.get_accessible_text().contains(changed)
    })?;
    input::chord(comparing.slint(), &[Key::Alt], Key::End);
    comparing.wait(
        app,
        "Alt+End to reach the last difference",
        STEP,
        |window| window.get_position() > first,
    )?;
    comparing.still(app)?;
    let last = window.get_position();
    input::chord(comparing.slint(), &[Key::Alt], Key::Home);
    comparing.wait(
        app,
        "Alt+Home to return to the first difference",
        STEP,
        |window| window.get_position() < last && window.get_accessible_text().contains(changed),
    )?;
    input::key(comparing.slint(), "n");
    comparing.wait(app, "N to reach the inserted line", STEP, |window| {
        top_shows(window, "", inserted)
    })?;
    input::key(comparing.slint(), "p");
    comparing.wait(app, "P to reach the changed line", STEP, |window| {
        top_shows(window, "010 ", changed)
    })?;
    input::click_button(comparing.slint(), "Next difference")?;
    comparing.wait(app, "the Next difference button", STEP, |window| {
        window.get_accessible_text().contains(inserted)
    })?;
    input::click_button(comparing.slint(), "Previous difference")?;
    comparing.wait(app, "the Previous difference button", STEP, |window| {
        window.get_position() < first + 0.0001 && window.get_accessible_text().contains(changed)
    })?;
    // The other aliases: Alt+Down, Alt+Right (next); Alt+Up, Alt+Left, Shift+F7 (back).
    comparing.click_content(app);
    for (keys, name, forwards) in [
        (&[Key::Alt][..], Key::DownArrow, true),
        (&[Key::Alt][..], Key::UpArrow, false),
        (&[Key::Alt][..], Key::RightArrow, true),
        (&[Key::Alt][..], Key::LeftArrow, false),
        (&[][..], Key::F7, true),
        (&[Key::Shift][..], Key::F7, false),
    ] {
        input::chord(comparing.slint(), keys, name);
        let what = format!("{keys:?}+{name:?}");
        if forwards {
            comparing.wait(app, &what, STEP, |window| {
                window.get_accessible_text().contains(inserted) && window.get_position() > first
            })?;
        } else {
            comparing.wait(app, &what, STEP, |window| {
                window.get_position() < first + 0.0001
                    && window.get_accessible_text().contains(changed)
            })?;
        }
    }
    context.note(format!(
        "Difference navigation: first at {first:.4}, last at {last:.4}; status {:?}",
        window.get_status()
    ));
    comparing.close(app, false)
}

fn images(app: &mut App, context: &mut Context, comparing: Comparing) -> Result<(), String> {
    let window = comparing.window.clone_strong();
    if !window.get_image_mode() {
        return Err("PNG files are not compared as images".into());
    }
    for (index, label) in ["Side by side", "Swipe", "Onion skin"].iter().enumerate() {
        input::click_button(comparing.slint(), label)?;
        comparing.wait(app, label, STEP, |window| {
            window.get_rendered_mode() == index as i32
        })?;
        let shot = app.screenshot_window(
            context,
            &comparing.software,
            &format!("image-{}", label.to_lowercase().replace(' ', "-")),
            &format!("image-a.png and image-b.png: {label}"),
        );
        for finding in probe::review(context, comparing.slint(), &shot) {
            context.note(format!("layout review: {finding}"));
        }
        // Swipe and Onion skin have a slider for their amount.
        if index > 0 {
            let slider = if index == 1 {
                "Swipe divider"
            } else {
                "Right image opacity"
            };
            let before = window.get_amount();
            let item = inspect::find(comparing.slint(), AccessibleRole::Slider, slider)
                .ok_or_else(|| format!("no {slider} slider"))?;
            let (x, y, width, height) = inspect::rect(&item);
            input::click_at(
                comparing.slint(),
                slint::LogicalPosition::new(
                    x + width * if before > 0.5 { 0.1 } else { 0.9 },
                    y + height / 2.0,
                ),
            );
            comparing.wait(app, slider, STEP, |window| {
                (window.get_amount() - before).abs() > 0.2
            })?;
        }
    }
    // Swipe: dragging on the picture moves the divider to the pointer.
    input::click_button(comparing.slint(), "Swipe")?;
    comparing.wait(app, "Swipe again", STEP, |window| {
        window.get_rendered_mode() == 1
    })?;
    let content = (
        window.get_content_left(),
        window.get_content_top(),
        window.get_content_width(),
        window.get_content_height(),
    );
    probe::drag(
        app,
        comparing.slint(),
        slint::LogicalPosition::new(content.0 + content.2 * 0.5, content.1 + content.3 / 2.0),
        slint::LogicalPosition::new(content.0 + content.2 * 0.25, content.1 + content.3 / 2.0),
        4,
    );
    comparing.wait(app, "the dragged divider", STEP, |window| {
        (window.get_amount() - 0.25).abs() < 0.05
    })?;
    // Zoom in and out, Ctrl+wheel and Fit change the picture.
    let fitted = comparing.still(app)?.fingerprint();
    input::click_button(comparing.slint(), "Zoom in")?;
    let zoomed = comparing.still(app)?.fingerprint();
    input::click_button(comparing.slint(), "Zoom out")?;
    input::click_button(comparing.slint(), "Zoom out")?;
    let smaller = comparing.still(app)?.fingerprint();
    let at = slint::LogicalPosition::new(content.0 + content.2 / 2.0, content.1 + content.3 / 2.0);
    input::press(comparing.slint(), Key::Control);
    probe::pixel_deltas(comparing.slint(), at, -1);
    input::release(comparing.slint(), Key::Control);
    let wheeled = comparing.still(app)?.fingerprint();
    input::click_button(comparing.slint(), "Fit")?;
    let refit = comparing.still(app)?.fingerprint();
    if zoomed == fitted || smaller == zoomed || wheeled == smaller || refit != fitted {
        return Err(format!(
            "the zoom controls: Fit {fitted:x}, Zoom in {zoomed:x}, Zoom out twice {smaller:x}, \
             Ctrl+wheel {wheeled:x}, Fit again {refit:x}"
        ));
    }
    comparing.close(app, true)
}

/// The hex view's first visible row: its offset and how many bytes a row shows.
fn first_row(window: &ComparatorWindow) -> Option<(u64, u64)> {
    let text = window.get_accessible_text();
    let left = text.lines().next()?.split('\t').next()?;
    let (offset, cells) = left.split_once("  ")?;
    let bytes = cells.split('│').next()?.split_whitespace().count() as u64;
    Some((u64::from_str_radix(offset, 16).ok()?, bytes))
}

/// The first visible row holds the byte at `offset`.
fn shows(offset: u64) -> impl Fn(&ComparatorWindow) -> bool {
    move |window| {
        first_row(window).is_some_and(|(start, bytes)| start <= offset && offset < start + bytes)
    }
}

/// binary-a.bin and binary-b.bin differ in the bytes at 100, 4000 and 8000.
fn binaries(app: &mut App, context: &mut Context, comparing: Comparing) -> Result<(), String> {
    let window = comparing.window.clone_strong();
    if !window.get_bytes_mode() || !window.get_bytes_only() {
        return Err(format!(
            "binary files are not shown as bytes; status {:?}",
            window.get_status()
        ));
    }
    comparing.wait(app, "the byte summary", STEP, |window| {
        let status = window.get_status();
        status.contains("3 differing bytes in 3 regions")
            && status.contains("first difference at 0x0064 (100)")
            && status.contains("8,192 bytes each")
    })?;
    let shot = app.screenshot_window(
        context,
        &comparing.software,
        "binary",
        "binary-a.bin and binary-b.bin: hex view, 3 differing bytes in 3 regions",
    );
    for finding in probe::review(context, comparing.slint(), &shot) {
        context.note(format!("layout review: {finding}"));
    }
    comparing.click_content(app);
    input::key(comparing.slint(), "n");
    comparing.wait(app, "N to reach the byte at 100", STEP, shows(100))?;
    app.screenshot_window(
        context,
        &comparing.software,
        "binary-first",
        "N: the row of the first differing byte (offset 100), marked in both files",
    );
    input::key(comparing.slint(), Key::F7);
    comparing.wait(app, "F7 to reach the byte at 4000", STEP, shows(4000))?;
    input::chord(comparing.slint(), &[Key::Alt], Key::End);
    comparing.wait(app, "Alt+End to reach the byte at 8000", STEP, shows(8000))?;
    input::key(comparing.slint(), "p");
    comparing.wait(app, "P to return to the byte at 4000", STEP, shows(4000))?;
    input::chord(comparing.slint(), &[Key::Alt], Key::Home);
    comparing.wait(
        app,
        "Alt+Home to return to the byte at 100",
        STEP,
        shows(100),
    )?;
    input::click_button(comparing.slint(), "Next difference")?;
    comparing.wait(app, "the Next difference button", STEP, shows(4000))?;
    context.note(format!(
        "Byte comparison: {} bytes per row; status {:?}",
        first_row(&window).map_or(0, |(_, bytes)| bytes),
        window.get_status()
    ));
    comparing.close(app, false)
}

/// over-limit.log is large-a.log with 768 KiB of lines appended.
fn over_former_limit(
    app: &mut App,
    context: &mut Context,
    comparing: Comparing,
) -> Result<(), String> {
    let window = comparing.window.clone_strong();
    if window.get_bytes_mode() {
        return Err(format!(
            "the 4.25 MiB log is not compared as text; status {:?}",
            window.get_status()
        ));
    }
    comparing.wait(
        app,
        "the opening view to show the lines only over-limit.log has",
        STEP,
        |window| {
            window
                .get_accessible_text()
                .lines()
                .next()
                .is_some_and(|row| row.starts_with('\t') && row.len() > 1)
        },
    )?;
    app.screenshot_window(
        context,
        &comparing.software,
        "over-former-limit",
        "large-a.log and the 4.25 MiB over-limit.log open at the first appended line",
    );
    let text_status = window.get_status();
    input::click_button(comparing.slint(), HEX)?;
    comparing.wait(
        app,
        "the Hex button to show the bytes",
        VERDICT_LIMIT,
        |window| {
            window.get_bytes_mode() && window.get_status().contains("differing bytes in 1 region")
        },
    )?;
    if window.get_bytes_only() || !window.get_status().contains("File B") {
        return Err(format!("the text pair's bytes: {:?}", window.get_status()));
    }
    app.screenshot_window(
        context,
        &comparing.software,
        "over-former-limit-hex",
        "the same pair as bytes (Hex): one region, the bytes only over-limit.log has",
    );
    let byte_status = window.get_status();
    input::click_button(comparing.slint(), HEX)?;
    comparing.wait(
        app,
        "the Hex button to return to the text",
        STEP,
        |window| !window.get_bytes_mode(),
    )?;
    context.note(format!(
        "Over the former limit: text {text_status:?}; bytes {byte_status:?}"
    ));
    comparing.close(app, false)
}
