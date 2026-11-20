//! Scrolling in every viewer kind, as a person scrolls. The vertical scrollbar is
//! dragged from 0 % to 100 % (every 1 % for the log, CSV, TSV, CSV as plain text and
//! Markdown with tables, every 2 % for the others) and every drag must land: the
//! content settles at the dragged position without an error, the thumb and the shown
//! content agree, and a text page holds the byte the position names. A wheel notch
//! must move the content as far in every viewer (one notch down, one up, a burst of
//! five), arrows a line, PageDown and PageUp exactly one screen, and Ctrl+End and
//! Ctrl+Home must reach the ends. The long line's horizontal scrollbar is swept too.
//! HTML has no host scrollbar (Chromium draws its own); its keys are checked, and its
//! wheel travel is pending while it is being aligned with the text viewer.
use crate::{
    Context,
    app::{App, Config},
    encode, panel, viewers_stage,
    viewers_support::{self as support, Failures, Viewing, clip, generated, millis, probe},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{LogicalPosition, platform::Key};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(40 * 60);
const OPEN_LIMIT: Duration = Duration::from_secs(120);
const STEP: Duration = Duration::from_secs(15);
/// A drag must settle within this long: it reads at most a few bounded pages.
const LAND_LIMIT: Duration = Duration::from_secs(20);
/// How far the shown position may end up from where the thumb was dragged.
const LAND_TOLERANCE: f32 = 0.02;
/// The thumb must show where the content is.
const THUMB_TOLERANCE: f32 = 0.005;
/// Viewer arrow keys move 28 logical pixels (`midnight-viewer-ui` `scrolling.rs`).
const ARROW: i32 = 28;
const HTML_WHEEL: &str = "the page did not move at all: the HTML viewer takes the wheel as a \
    native window event (web_input), which this harness cannot deliver; viewer.slint drops \
    Slint's own wheel event in the HTML mode";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Text, source, Markdown and binary pages: the status names the page's bytes.
    Text,
    /// A PDF in the Continuous layout: the page counter follows the position.
    Document,
    /// An image at actual pixels.
    Image,
    /// The HTML viewer: no host scrollbar.
    Html,
}

struct Subject {
    file: &'static str,
    label: &'static str,
    kind: Kind,
    /// Alt+F3: the basic plain-text viewer.
    basic: bool,
    /// Scrollbar targets in percent: every `step`, forwards, then back on the others.
    step: u32,
}

const SUBJECTS: &[Subject] = &[
    Subject {
        file: "huge.log",
        label: "large log",
        kind: Kind::Text,
        basic: false,
        step: 1,
    },
    Subject {
        file: "inventory.csv",
        label: "CSV",
        kind: Kind::Text,
        basic: false,
        step: 1,
    },
    Subject {
        file: "inventory.tsv",
        label: "TSV",
        kind: Kind::Text,
        basic: false,
        step: 1,
    },
    Subject {
        file: "inventory.csv",
        label: "CSV as plain text (Alt+F3)",
        kind: Kind::Text,
        basic: true,
        step: 1,
    },
    Subject {
        file: "long.md",
        label: "Markdown with tables",
        kind: Kind::Text,
        basic: false,
        step: 1,
    },
    Subject {
        file: "long.rs",
        label: "Rust source",
        kind: Kind::Text,
        basic: false,
        step: 2,
    },
    Subject {
        file: "huge.bin",
        label: "binary (hex)",
        kind: Kind::Text,
        basic: false,
        step: 2,
    },
    Subject {
        file: "pages.pdf",
        label: "PDF, Continuous layout",
        kind: Kind::Document,
        basic: false,
        step: 2,
    },
    Subject {
        file: "tall.png",
        label: "image at actual pixels",
        kind: Kind::Image,
        basic: false,
        step: 2,
    },
    Subject {
        file: "long.html",
        label: "HTML",
        kind: Kind::Html,
        basic: false,
        step: 2,
    },
];

/// The PDF's pages.
const PAGES: usize = 24;

pub fn scrolling(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

/// Link a fixture file into `folder` (copy it where links are refused).
fn link(context: &Context, relative: &str, folder: &Path, name: &str) -> Result<(), String> {
    let (from, to) = (context.fixture.path(relative), folder.join(name));
    std::fs::hard_link(&from, &to)
        .or_else(|_| std::fs::copy(&from, &to).map(|_| ()))
        .map_err(|error| format!("{}: {error}", to.display()))
}

fn prepare(context: &Context) -> Result<std::path::PathBuf, String> {
    let root = context.work.join("scroll");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    link(context, "text/huge.log", &root, "huge.log")?;
    link(context, "sizes/huge.bin", &root, "huge.bin")?;
    link(context, "text/long-line.txt", &root, "long-line.txt")?;
    generated::write(&root, "inventory.csv", generated::table(1500, ','))?;
    generated::write(&root, "inventory.tsv", generated::table(1500, '\t'))?;
    generated::write(&root, "long.md", generated::markdown(120))?;
    generated::write(&root, "long.rs", generated::rust(6000))?;
    generated::write(&root, "pages.pdf", generated::pdf(PAGES))?;
    generated::write(&root, "long.html", generated::html(400))?;
    // Every row differs: the red channel counts rows, green their hundreds, blue columns.
    let tall = encode::png(800, 3000, |x, y| {
        [
            (y % 256) as u8,
            ((y / 256) * 20 % 256) as u8,
            (x * 7 % 256) as u8,
        ]
    });
    generated::write(&root, "tall.png", tall)?;
    Ok(root)
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = prepare(context)?;
    let mut config = Config::new(&root, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let failures = Failures::default();
    let mut notches: Vec<(String, i32)> = Vec::new();
    for subject in SUBJECTS {
        match open(&mut app, subject) {
            Ok(viewing) => {
                context.note(format!(
                    "latency\tscroll subject\t{}\t{}\t{}",
                    subject.label,
                    viewing.latency.as_millis(),
                    clip(&viewing.status(), 80)
                ));
                exercise(
                    &mut app,
                    context,
                    &failures,
                    subject,
                    &viewing,
                    &mut notches,
                );
                failures.check(
                    &format!("{}: close", subject.label),
                    viewing.close(&mut app),
                );
            }
            Err(error) => failures.check(&format!("{}: open", subject.label), Err(error)),
        }
    }
    // Every viewer moves as far per notch as the text viewer.
    let reference = notches
        .iter()
        .find(|(label, _)| label == "large log")
        .map(|(_, travel)| *travel);
    context.note(format!(
        "Wheel travel per notch (pixels): {}",
        notches
            .iter()
            .map(|(label, travel)| format!("{label} {travel}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if let Some(reference) = reference {
        let differ: Vec<String> = notches
            .iter()
            .filter(|(_, travel)| (travel - reference).abs() > 1)
            .map(|(label, travel)| format!("{label} {travel} px"))
            .collect();
        failures.check(
            "the same wheel travel per notch in every viewer",
            if differ.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "the text viewer moves {reference} px per notch; {}",
                    differ.join(", ")
                ))
            },
        );
    }
    failures.check(
        "horizontal scrollbar of a long line",
        horizontal(&mut app, context),
    );
    failures.skip(
        "kinetic (touchpad) coasting",
        "synthetic PointerScrolled events carry no touchpad phases, and this harness \
         delivers no native window events, which production's kinetic observer reads",
    );
    app.shutdown()?;
    failures.finish(context)
}

fn open(app: &mut App, subject: &Subject) -> Result<Viewing, String> {
    panel::select_row(app, PaneId::Left, subject.file)?;
    let viewing = support::view(app, subject.basic, OPEN_LIMIT)?;
    viewing
        .result
        .clone()
        .map_err(|error| format!("{}: {error}", subject.file))?;
    viewing.wait(app, "the first content", STEP, |window| {
        let status = window.get_status();
        !status.is_empty() && !status.starts_with("Opening")
    })?;
    Ok(viewing)
}

/// Everything one subject is checked for.
fn exercise(
    app: &mut App,
    context: &mut Context,
    failures: &Failures,
    subject: &Subject,
    viewing: &Viewing,
    notches: &mut Vec<(String, i32)>,
) {
    let label = subject.label;
    if let Err(error) = ready(app, subject, viewing) {
        failures.check(&format!("{label}: preparation"), Err(error));
        return;
    }
    let shot = app.screenshot_window(
        context,
        &viewing.software,
        &format!("open-{}", slug(label)),
        &format!("{label}: opened ({})", clip(&viewing.status(), 80)),
    );
    let findings = probe::review(context, viewing.slint(), &shot);
    if !findings.is_empty() {
        context.note(format!("{label}: layout review: {}", findings.join("; ")));
    }
    if subject.kind == Kind::Html {
        failures.check(&format!("{label}: keys"), html_keys(app, context, viewing));
        // The browser surface takes the wheel as a native window event (`web_input`);
        // `viewer.slint` accepts and drops Slint's own wheel event in the HTML mode.
        // This harness delivers only Slint's event, so no movement at all means the
        // wheel did not reach the page here; any other distance is measured strictly.
        match wheel_travel(app, context, viewing, label) {
            Ok((0, 0, 0)) => failures.skip(&format!("{label}: wheel travel per notch"), HTML_WHEEL),
            Ok((down, up, burst)) => {
                notches.push((label.to_owned(), down));
                let notch = probe::NOTCH as i32;
                failures.check(
                    &format!("{label}: wheel"),
                    if (down - notch).abs() <= 1
                        && (up + notch).abs() <= 1
                        && (burst - 5 * notch).abs() <= 2
                    {
                        Ok(())
                    } else {
                        Err(format!(
                            "one notch down moved the page {down} px, one up {up} px, five \
                             down {burst} px (the text viewer: {notch}, -{notch}, {})",
                            5 * notch
                        ))
                    },
                );
            }
            Err(error) => failures.check(&format!("{label}: wheel"), Err(error)),
        }
        return;
    }
    failures.check(&format!("{label}: scrollbar shown"), thumb_shown(viewing));
    failures.check(
        &format!("{label}: scrollbar drags land"),
        sweep(app, context, subject, viewing),
    );
    match wheel_travel(app, context, viewing, label) {
        Ok((down, up, burst)) => {
            notches.push((label.to_owned(), down));
            let notch = probe::NOTCH as i32;
            failures.check(
                &format!("{label}: wheel"),
                if (down - notch).abs() <= 1
                    && (up + notch).abs() <= 1
                    && (burst - 5 * notch).abs() <= 2
                {
                    Ok(())
                } else {
                    Err(format!(
                        "one notch down moved {down} px, one up {up} px, five down {burst} px \
                         (expected {notch}, -{notch} and {})",
                        5 * notch
                    ))
                },
            );
        }
        Err(error) => failures.check(&format!("{label}: wheel"), Err(error)),
    }
    failures.check(
        &format!("{label}: keys"),
        keys(app, context, subject, viewing),
    );
}

fn slug(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// What a subject needs before scrolling: the PDF's Continuous layout, the image's
/// actual pixels.
fn ready(app: &mut App, subject: &Subject, viewing: &Viewing) -> Result<(), String> {
    match subject.kind {
        Kind::Document => {
            viewing.wait(app, "the PDF page counter", STEP, |window| {
                window.get_document_pages() == PAGES as i32
            })?;
            crate::input::click(
                viewing.slint(),
                AccessibleRole::Button,
                "Continuous document layout",
            )?;
            viewing.wait(app, "the Continuous layout", STEP, |window| {
                window.get_document_layout() == 1 && window.get_vertical_extent() < 1.0
            })
        }
        Kind::Image => {
            viewing.wait(app, "the image", STEP, |window| window.get_image_mode())?;
            viewing.click_content(app);
            crate::input::chord(viewing.slint(), &[Key::Control], "1");
            viewing.wait(app, "actual pixels with a scrollbar", STEP, |window| {
                (window.get_zoom_percent() - 100.0).abs() < 0.5
                    && window.get_vertical_extent() < 1.0
            })
        }
        Kind::Html => viewing.wait(app, "the page", STEP, |window| {
            window.get_web_mode() && viewing.canvas().colours(4).len() >= 2
        }),
        Kind::Text => viewing.wait(app, "a scrollable page", STEP, |window| {
            window.get_vertical_extent() < 1.0 && probe::page_bytes(&window.get_status()).is_some()
        }),
    }
}

/// The vertical thumb is drawn: accent pixels over its length where it should be.
fn thumb_shown(viewing: &Viewing) -> Result<(), String> {
    let bar = probe::vertical_bar(viewing).ok_or_else(|| {
        format!(
            "content taller than the window but no scrollbar (extent {:.4}, bar {}x{})",
            viewing.window.get_vertical_extent(),
            viewing.window.get_vertical_bar_width(),
            viewing.window.get_vertical_bar_height()
        )
    })?;
    let (width, height, pixels) = viewing.render();
    let scale = viewing.slint().scale_factor();
    let accent = viewing.window.get_accent().to_argb_u8();
    let accent = [accent.red, accent.green, accent.blue];
    let column = ((bar.left + bar.width / 2.0) * scale) as usize;
    let top = (bar.top * scale) as usize;
    let bottom = (((bar.top + bar.height) * scale) as usize).min(height);
    let rows: Vec<usize> = (top..bottom)
        .filter(|y| {
            let pixel = pixels[y * width + column.min(width - 1)];
            [pixel.r, pixel.g, pixel.b]
                .iter()
                .zip(accent)
                .all(|(value, wanted)| value.abs_diff(wanted) <= 40)
        })
        .collect();
    let expected = (bar.thumb * scale * 0.8) as usize;
    if rows.len() < expected.max(8) {
        return Err(format!(
            "the scrollbar shows {} thumb rows, expected about {:.0} (bar {:.0}x{:.0} at {:.0},{:.0})",
            rows.len(),
            bar.thumb * scale,
            bar.width,
            bar.height,
            bar.left,
            bar.top
        ));
    }
    Ok(())
}

/// The targets of a sweep: every `step` percent forwards, then the ones between back.
fn targets(step: u32) -> Vec<u32> {
    let forward = (0..=100).step_by(step.max(1) as usize * 2);
    let backward = (0..=100)
        .rev()
        .filter(move |percent| percent % (step.max(1) * 2) == step.max(1));
    forward.chain(backward).collect()
}

/// Drag the thumb to every target and check where each drag landed.
fn sweep(
    app: &mut App,
    context: &mut Context,
    subject: &Subject,
    viewing: &Viewing,
) -> Result<(), String> {
    let mut problems = Vec::new();
    let mut times = Vec::new();
    let mut previous_page = 0;
    for percent in targets(subject.step) {
        let target = percent as f32 / 100.0;
        let started = Instant::now();
        let landed = probe::drag_vertical(app, viewing, target, 4)
            .and_then(|()| probe::settle(app, viewing, LAND_LIMIT));
        let settled = match landed {
            Ok(settled) => settled,
            Err(error) => {
                problems.push(format!("{percent}%: {error}"));
                continue;
            }
        };
        times.push(started.elapsed());
        if let Some(error) = probe::status_error(&settled.status) {
            problems.push(format!("{percent}%: the status reports {error:?}"));
        }
        if (settled.displayed - target).abs() > LAND_TOLERANCE {
            problems.push(format!(
                "{percent}%: the content shows {:.3}",
                settled.displayed
            ));
        }
        if (settled.position - settled.displayed).abs() > THUMB_TOLERANCE {
            problems.push(format!(
                "{percent}%: the thumb shows {:.4} but the content {:.4}",
                settled.position, settled.displayed
            ));
        }
        match subject.kind {
            Kind::Text => match probe::page_bytes(&settled.status) {
                Some((start, end, length)) => {
                    // The view starts on a whole row: allow two of the longest rows
                    // (or 0.5 % of the file) around the byte the position names.
                    let extent = f64::from(viewing.window.get_vertical_extent());
                    let byte = f64::from(target) * (1.0 - extent) * length as f64;
                    let slack = (length as f64 * 0.005).max(520.0);
                    if byte + slack < start as f64 || byte - slack > end as f64 {
                        problems.push(format!(
                            "{percent}%: the page holds bytes {start}–{end}, not byte {byte:.0}"
                        ));
                    }
                }
                None => problems.push(format!(
                    "{percent}%: no page range in the status {:?}",
                    settled.status
                )),
            },
            Kind::Document => {
                let page = viewing.window.get_document_page();
                let expected = 1.0 + f64::from(target) * (PAGES as f64 - 1.0);
                if (f64::from(page) - expected).abs() > 1.5 {
                    problems.push(format!(
                        "{percent}%: page {page} is shown, expected about {expected:.1}"
                    ));
                }
                if percent > 0 && page < previous_page && percent % (subject.step * 2) == 0 {
                    problems.push(format!(
                        "{percent}%: the page went back from {previous_page} to {page}"
                    ));
                }
                previous_page = page;
            }
            Kind::Image | Kind::Html => {}
        }
        if percent == 50 {
            app.screenshot_window(
                context,
                &viewing.software,
                &format!("drag-50-{}", slug(subject.label)),
                &format!(
                    "{}: scrollbar dragged to 50 % ({})",
                    subject.label,
                    clip(&settled.status, 80)
                ),
            );
        }
    }
    times.sort();
    if let (Some(median), Some(slowest)) = (times.get(times.len() / 2), times.last()) {
        context.note(format!(
            "{}: {} drags; landing median {}, slowest {}",
            subject.label,
            times.len(),
            millis(*median),
            millis(*slowest)
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} drags did not land: {}",
            problems.len(),
            targets(subject.step).len(),
            clip(&problems.join("; "), 1500)
        ))
    }
}

/// Measured content movement after `act`, from a still frame to the next still frame.
fn moved(
    app: &mut App,
    context: &mut Context,
    viewing: &Viewing,
    what: &str,
    act: impl FnOnce(&mut App),
) -> Result<i32, String> {
    let before = settled_canvas(app, viewing)?;
    act(app);
    let after = settled_canvas(app, viewing)?;
    if before.fingerprint() == after.fingerprint() {
        return Ok(0);
    }
    let shift = probe::vertical_shift(&before, &after);
    if !shift.is_some_and(|(_, share)| share >= probe::SHIFT_CONFIDENCE) {
        // What the measurement saw, for the report.
        for (canvas, when) in [(&before, "before"), (&after, "after")] {
            support::save_canvas(
                context,
                canvas,
                &format!("{}-{when}", slug(what)),
                &format!("{what}: the content {when} (the movement could not be measured)"),
            );
        }
    }
    match shift {
        Some((shift, share)) if share >= probe::SHIFT_CONFIDENCE => Ok(shift),
        Some((shift, share)) => Err(format!(
            "{what}: the content changed but did not move as one piece (best match {shift} px \
             for {:.0}% of the rows)",
            share * 100.0
        )),
        None => Err(format!("{what}: the two frames cannot be compared")),
    }
}

/// The content once it has come to rest: still for [`probe::QUIET`], and unchanged
/// 700 ms later. A motion that pauses at the edge of the prepared pixels until a refill
/// arrives, then resumes, is not cut short this way.
fn settled_canvas(
    app: &mut App,
    viewing: &Viewing,
) -> Result<crate::viewers_support::Canvas, String> {
    let started = Instant::now();
    loop {
        let still = probe::still_canvas(app, viewing, STEP)?;
        probe::pause(app, Duration::from_millis(700));
        if viewing.canvas().fingerprint() == still.fingerprint() {
            return Ok(still);
        }
        if started.elapsed() >= STEP * 2 {
            return Err("the content kept moving after pauses".into());
        }
    }
}

/// One notch down, one up and a burst of five down, from the middle of the content.
fn wheel_travel(
    app: &mut App,
    context: &mut Context,
    viewing: &Viewing,
    label: &str,
) -> Result<(i32, i32, i32), String> {
    if probe::vertical_bar(viewing).is_some() {
        probe::drag_vertical(app, viewing, 0.4, 4)?;
        probe::settle(app, viewing, LAND_LIMIT)?;
    }
    let at = probe::content_middle(viewing);
    let window = viewing.slint();
    let down = moved(
        app,
        context,
        viewing,
        &format!("{label} one notch down"),
        |_| probe::wheel(window, at, 1),
    )?;
    let up = moved(
        app,
        context,
        viewing,
        &format!("{label} one notch up"),
        |_| probe::wheel(window, at, -1),
    )?;
    let burst = moved(
        app,
        context,
        viewing,
        &format!("{label} five notches"),
        |_| probe::wheel(window, at, 5),
    )?;
    Ok((down, up, burst))
}

fn press(viewing: &Viewing, modifiers: &[Key], key: Key) {
    crate::input::chord(viewing.slint(), modifiers, key);
}

/// Arrows move 28 pixels, PageDown and PageUp one screen, Ctrl+End and Ctrl+Home reach
/// the ends.
fn keys(
    app: &mut App,
    context: &mut Context,
    subject: &Subject,
    viewing: &Viewing,
) -> Result<(), String> {
    probe::drag_vertical(app, viewing, 0.3, 4)?;
    probe::settle(app, viewing, LAND_LIMIT)?;
    // Scrolling keys belong to the content, which has the focus unless a control took it.
    if !viewing.window.get_content_focused() {
        focus_content(app, viewing);
    }
    let label = subject.label;
    let down = moved(app, context, viewing, &format!("{label} Down"), |_| {
        press(viewing, &[], Key::DownArrow)
    })?;
    let up = moved(app, context, viewing, &format!("{label} Up"), |_| {
        press(viewing, &[], Key::UpArrow)
    })?;
    let mut problems = Vec::new();
    if down != ARROW || up != -ARROW {
        problems.push(format!(
            "Down moved {down} px and Up {up} px (expected {ARROW} and -{ARROW})"
        ));
    }
    // A screen leaves no row in common: PageDown, then arrows back up half a screen.
    let screen =
        (viewing.window.get_content_height() * viewing.slint().scale_factor()).round() as i32;
    let back = screen / 2 / ARROW;
    // The page comes to rest before the arrows (a reverse key would drop the rest of an
    // eased page step), and the arrows go four at a time with rests between them, so a
    // burst's own limits (checked below) do not enter this measurement.
    let arrows_back = |app: &mut App, key: Key| {
        let _ = settled_canvas(app, viewing);
        for group in 0..(back + 3) / 4 {
            for _ in 0..(back - group * 4).min(4) {
                press(viewing, &[], key);
            }
            let _ = settled_canvas(app, viewing);
        }
    };
    let page_down = moved(app, context, viewing, &format!("{label} PageDown"), |app| {
        press(viewing, &[], Key::PageDown);
        arrows_back(app, Key::UpArrow);
    })? + back * ARROW;
    let page_up = moved(app, context, viewing, &format!("{label} PageUp"), |app| {
        press(viewing, &[], Key::PageUp);
        arrows_back(app, Key::DownArrow);
    })? - back * ARROW;
    if (page_down - screen).abs() > 1 || (page_up + screen).abs() > 1 {
        problems.push(format!(
            "PageDown moved {page_down} px and PageUp {page_up} px; the screen is {screen} px"
        ));
    }
    // A held key: twenty Down presses in a row move twenty steps.
    let held = moved(
        app,
        context,
        viewing,
        &format!("{label} 20 × Down"),
        |_| {
            for _ in 0..20 {
                press(viewing, &[], Key::DownArrow);
            }
        },
    )?;
    if held != 20 * ARROW {
        problems.push(format!(
            "twenty Down presses in a row moved {held} px, not {}",
            20 * ARROW
        ));
    }
    press(viewing, &[Key::Control], Key::End);
    let end = probe::settle(app, viewing, LAND_LIMIT)?;
    if end.displayed < 0.999 {
        problems.push(format!("Ctrl+End shows position {:.4}", end.displayed));
    }
    if subject.kind == Kind::Text
        && let Some((_, last, length)) = probe::page_bytes(&end.status)
        && last != length
    {
        problems.push(format!(
            "after Ctrl+End the page ends at byte {last} of {length}"
        ));
    }
    if subject.kind == Kind::Document && viewing.window.get_document_page() != PAGES as i32 {
        problems.push(format!(
            "Ctrl+End shows page {} of {PAGES}",
            viewing.window.get_document_page()
        ));
    }
    app.screenshot_window(
        context,
        &viewing.software,
        &format!("end-{}", slug(subject.label)),
        &format!("{}: Ctrl+End", subject.label),
    );
    press(viewing, &[Key::Control], Key::Home);
    let home = probe::settle(app, viewing, LAND_LIMIT)?;
    if home.displayed > 0.001 {
        problems.push(format!("Ctrl+Home shows position {:.4}", home.displayed));
    }
    if subject.kind == Kind::Text
        && let Some((first, _, _)) = probe::page_bytes(&home.status)
        && first != 0
    {
        problems.push(format!("after Ctrl+Home the page starts at byte {first}"));
    }
    context.note(format!(
        "{}: Down {down} px, Up {up} px, PageDown {page_down} px, PageUp {page_up} px \
         (screen {screen} px); Ctrl+End {:.4}, Ctrl+Home {:.4}",
        subject.label, end.displayed, home.displayed
    ));
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Give the content the keyboard focus with a click on its left margin, as a person
/// does before using scrolling keys (an image or a document page takes no selection).
fn focus_content(app: &mut App, viewing: &Viewing) {
    let window = &viewing.window;
    let position = LogicalPosition::new(
        window.get_content_left() + 4.0,
        window.get_content_top() + window.get_content_height() / 2.0,
    );
    crate::input::click_at(viewing.slint(), position);
    app.settle();
}

/// HTML: Chromium's own keys. Down and PageDown move the page towards its end, Ctrl+End
/// shows another part and Ctrl+Home the first view again, pixel for pixel.
fn html_keys(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let first = probe::still_canvas(app, viewing, STEP)?;
    let down = moved(app, context, viewing, "HTML Down", |_| {
        press(viewing, &[], Key::DownArrow)
    })?;
    let screen =
        (viewing.window.get_content_height() * viewing.slint().scale_factor()).round() as i32;
    let page = moved(app, context, viewing, "HTML PageDown", |_| {
        press(viewing, &[], Key::PageDown)
    })?;
    let mut problems = Vec::new();
    if !(10..=100).contains(&down) {
        problems.push(format!("Down moved the page {down} px"));
    }
    if page < screen / 2 || page > screen {
        problems.push(format!(
            "PageDown moved the page {page} px; the screen is {screen} px"
        ));
    }
    press(viewing, &[Key::Control], Key::End);
    let end = probe::still_canvas(app, viewing, STEP)?;
    if end.fingerprint() == first.fingerprint() {
        problems.push("Ctrl+End did not move the page".into());
    }
    app.screenshot_window(context, &viewing.software, "end-html", "HTML: Ctrl+End");
    press(viewing, &[Key::Control], Key::Home);
    let home = probe::still_canvas(app, viewing, STEP)?;
    if home.fingerprint() != first.fingerprint() {
        problems.push("Ctrl+Home did not return to the first view".into());
    }
    context.note(format!(
        "HTML keys: Down {down} px, PageDown {page} px (screen {screen} px)"
    ));
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The long line with Wrap off: its horizontal scrollbar dragged across, each drag
/// landing where it was dropped, and Right and Left moving 40 pixels.
fn horizontal(app: &mut App, context: &mut Context) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, "long-line.txt")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let result = horizontal_checks(app, context, &viewing);
    viewing.close(app)?;
    result
}

fn horizontal_checks(
    app: &mut App,
    context: &mut Context,
    viewing: &Viewing,
) -> Result<(), String> {
    viewing.result.clone()?;
    if viewing.window.get_wrap_enabled() {
        crate::input::chord(viewing.slint(), &[Key::Control], "w");
    }
    viewing.wait(
        app,
        "Wrap off with a horizontal scrollbar",
        STEP,
        |window| !window.get_wrap_enabled() && window.get_horizontal_extent() < 1.0,
    )?;
    let mut problems = Vec::new();
    for percent in (0..=100).step_by(5).chain([97, 63, 31, 3]) {
        let target = percent as f32 / 100.0;
        let thumb = horizontal_thumb(viewing)?;
        let window = &viewing.window;
        let left = window.get_content_left();
        let track =
            viewing.slint().size().width as f32 / viewing.slint().scale_factor() - 2.0 * left;
        let length = thumb.1 - thumb.0;
        let from = LogicalPosition::new((thumb.0 + thumb.1) / 2.0, thumb.2);
        let to = LogicalPosition::new(left + target * (track - length) + length / 2.0, thumb.2);
        probe::drag(app, viewing.slint(), from, to, 4);
        let settled = probe::settle(app, viewing, LAND_LIMIT)?;
        if let Some(error) = probe::status_error(&settled.status) {
            problems.push(format!("{percent}%: the status reports {error:?}"));
        }
        if (settled.horizontal - target).abs() > LAND_TOLERANCE {
            problems.push(format!(
                "{percent}%: the line shows {:.3}",
                settled.horizontal
            ));
        }
    }
    app.screenshot_window(
        context,
        &viewing.software,
        "horizontal-drag",
        "The 200,000-character line after horizontal scrollbar drags",
    );
    problems
        .is_empty()
        .then_some(())
        .ok_or_else(|| problems.join("; "))
}

/// The horizontal thumb in logical pixels: its left and right edge and its middle row,
/// found as the accent-coloured run in the bar below the content.
fn horizontal_thumb(viewing: &Viewing) -> Result<(f32, f32, f32), String> {
    let (width, height, pixels) = viewing.render();
    let scale = viewing.slint().scale_factor();
    let window = &viewing.window;
    let accent = window.get_accent().to_argb_u8();
    let accent = [accent.red, accent.green, accent.blue];
    let top = ((window.get_content_top() + window.get_content_height()) * scale) as usize;
    let mut best: Option<(usize, usize, usize)> = None;
    for y in top..(top + (24.0 * scale) as usize).min(height) {
        let columns: Vec<usize> = (0..width)
            .filter(|x| {
                let pixel = pixels[y * width + x];
                [pixel.r, pixel.g, pixel.b]
                    .iter()
                    .zip(accent)
                    .all(|(value, wanted)| value.abs_diff(wanted) <= 40)
            })
            .collect();
        if let (Some(first), Some(last)) = (columns.first(), columns.last())
            && best.is_none_or(|(a, b, _)| last - first > b - a)
        {
            best = Some((*first, *last, y));
        }
    }
    let (first, last, y) = best.ok_or("no horizontal thumb is drawn below the content")?;
    Ok((
        first as f32 / scale,
        (last + 1) as f32 / scale,
        y as f32 / scale,
    ))
}
