//! Scrolling in every viewer kind, as a person scrolls. The vertical scrollbar is
//! dragged from 0 % to 100 % (every 1 % for the log, CSV, TSV, CSV as plain text and
//! Markdown with tables, every 2 % for the others) and every drag must land: the
//! content settles at the dragged position without an error, the thumb and the shown
//! content agree, and a text page holds the byte the position names. A Slint pixel
//! delta (the harness's synthetic wheel event) must move the content exactly its pixels
//! in every viewer (one down, one up, a burst of five), and a native wheel notch (winit's
//! `LineDelta`, through the viewers' acceptance seam, at three lines per notch) 100 px,
//! −100 px up and 500 px for five, the HTML viewer included; on the large log six lines
//! per notch move 200 px, one screen per notch a screen, and with reduced motion a notch
//! moves in one step instead of gliding. Arrows move a line, PageDown and PageUp exactly
//! one screen, and Ctrl+End and Ctrl+Home must reach the ends. The PDF shows new frames
//! while its scrollbar is dragged and sharp text at a typed 400 % zoom. The long line's
//! horizontal scrollbar is swept too. HTML has no host scrollbar (Chromium draws its
//! own); its keys are checked, and Slint's own wheel event does not reach it (Chromium
//! takes the wheel only as a native window event). Source highlighting while a person
//! scrolls: in a small C++ file twelve wheel notches down and up, eight PageDowns and
//! PageUps and a scrollbar drag never show "Preparing source highlighting…" in any frame's
//! status line, and after each gesture the rows show their keyword, string and comment
//! tints; in a large one the rows shown after Ctrl+End are coloured within a second and
//! the notice, if shown, is gone once they are.
use crate::{
    Context,
    app::{App, Config},
    encode, input, panel, viewers_stage,
    viewers_support::{self as support, Canvas, Failures, Viewing, clip, generated, millis, probe},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{LogicalPosition, platform::Key};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(55 * 60);
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
const HTML_WHEEL: &str = "the page did not move at all: the HTML viewer takes the wheel only as a \
    native window event (web_input; its notch is checked through the acceptance seam), and \
    viewer.slint drops Slint's own wheel event in the HTML mode";
/// The scrollbar drag whose frames are counted: 20 moves, a frame (30 fps) apart.
const DRAG_STEPS: u32 = 20;
const DRAG_FRAME: Duration = Duration::from_millis(33);
/// The typed zoom at which PDF text must stay sharp, and the widest edge allowed for
/// three quarters of the text's edges (sharp rendering: one or two pixels; a page
/// raster enlarged four times: about four).
const SHARP_ZOOM: &str = "400";
const SHARP_EDGE: usize = 2;
/// The Source viewer's status while it prepares highlighting (`worker_syntax.rs`).
const PREPARING: &str = "Preparing source highlighting";
/// The rows of the highlighting files: 150 (about 6 KB) and 56,000 (about 2.2 MB).
const SMALL_ROWS: usize = 150;
const LARGE_ROWS: usize = 56_000;
/// How soon the rows shown after Ctrl+End must be coloured in the large file.
const COLOURED: Duration = Duration::from_secs(1);

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
    generated::write(&root, "highlight.cpp", highlight_rows(SMALL_ROWS))?;
    generated::write(&root, "highlight-large.cpp", highlight_rows(LARGE_ROWS))?;
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
    // Travel per Slint pixel delta and per native notch, per viewer kind.
    let mut notches: Vec<(String, i32)> = Vec::new();
    let mut native: Vec<(String, i32)> = Vec::new();
    for subject in SUBJECTS {
        if subject.kind == Kind::Html
            && let Some(reason) = probe::html_unavailable()
        {
            failures.skip(subject.label, &reason);
            continue;
        }
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
                    (&mut notches, &mut native),
                );
                failures.check(
                    &format!("{}: close", subject.label),
                    viewing.close(&mut app),
                );
            }
            Err(error) if subject.kind == Kind::Html => {
                probe::check_html(&failures, &format!("{}: open", subject.label), Err(error));
            }
            Err(error) => failures.check(&format!("{}: open", subject.label), Err(error)),
        }
    }
    // Every viewer moves as far per Slint pixel delta as the text viewer.
    let reference = notches
        .iter()
        .find(|(label, _)| label == "large log")
        .map(|(_, travel)| *travel);
    context.note(format!(
        "Travel per Slint pixel delta of {} px (pixels): {}",
        probe::SLINT_PIXEL_DELTA,
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
            "the same travel per Slint pixel delta in every viewer",
            if differ.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "the text viewer moves {reference} px per pixel delta; {}",
                    differ.join(", ")
                ))
            },
        );
    }
    context.note(format!(
        "Travel per native wheel notch at three lines per notch (pixels): {}",
        native
            .iter()
            .map(|(label, travel)| format!("{label} {travel}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    failures.check(
        "horizontal scrollbar of a long line",
        horizontal(&mut app, context),
    );
    source_highlighting(&mut app, context, &failures);
    failures.skip(
        "kinetic (touchpad) coasting",
        "not written yet: Slint's own PointerScrolled events carry no touchpad phases; the \
         acceptance seam's touchpad reports (NativeWheel::Pixels with their phases) can drive \
         such a check now",
    );
    app.shutdown()?;
    failures.finish(context)
}

fn open(app: &mut App, subject: &Subject) -> Result<Viewing, String> {
    panel::select_row(app, PaneId::Left, subject.file)?;
    let viewing = support::view(app, subject.basic, OPEN_LIMIT)?;
    if let Err(error) = viewing.result.clone() {
        // The window shows the viewer's report: closed before the next subject opens.
        let _ = viewing.close(app);
        return Err(format!("{}: {error}", subject.file));
    }
    viewing.wait(app, "the first content", STEP, |window| {
        let status = window.get_status();
        !status.is_empty() && !status.starts_with("Opening")
    })?;
    Ok(viewing)
}

/// Travel per Slint pixel delta and per native notch, per viewer kind.
type Travels<'a> = (&'a mut Vec<(String, i32)>, &'a mut Vec<(String, i32)>);

/// Everything one subject is checked for.
fn exercise(
    app: &mut App,
    context: &mut Context,
    failures: &Failures,
    subject: &Subject,
    viewing: &Viewing,
    (notches, native): Travels<'_>,
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
        // `viewer.slint` accepts and drops Slint's own wheel event in the HTML mode, so a
        // Slint pixel delta moves the page 0 px; any other distance is measured strictly.
        match wheel_travel(app, context, viewing, label) {
            Ok((0, 0, 0)) => failures.skip(&format!("{label}: Slint pixel delta"), HTML_WHEEL),
            Ok(travel) => {
                notches.push((label.to_owned(), travel.0));
                failures.check(&format!("{label}: Slint pixel delta"), exact_travel(travel));
            }
            Err(error) => failures.check(&format!("{label}: Slint pixel delta"), Err(error)),
        }
        native_notch(app, context, failures, viewing, subject, native);
        return;
    }
    failures.check(&format!("{label}: scrollbar shown"), thumb_shown(viewing));
    failures.check(
        &format!("{label}: scrollbar drags land"),
        sweep(app, context, subject, viewing),
    );
    match wheel_travel(app, context, viewing, label) {
        Ok(travel) => {
            notches.push((label.to_owned(), travel.0));
            failures.check(
                &format!("{label}: a Slint pixel delta moves exactly its pixels"),
                exact_travel(travel),
            );
        }
        Err(error) => failures.check(&format!("{label}: Slint pixel delta"), Err(error)),
    }
    failures.check(
        &format!("{label}: keys"),
        keys(app, context, subject, viewing),
    );
    native_notch(app, context, failures, viewing, subject, native);
    if subject.label == "large log" {
        failures.check(
            &format!("{label}: the system's wheel setting and reduced motion"),
            wheel_settings(app, context, viewing),
        );
    }
    if subject.kind == Kind::Document {
        failures.check(
            &format!("{label}: new frames while the scrollbar is dragged"),
            drag_frames(app, context, viewing),
        );
        failures.check(
            &format!("{label}: sharp text at a typed {SHARP_ZOOM} % zoom"),
            sharp_text(app, context, viewing),
        );
    }
}

/// A native wheel notch at three lines per notch (the system's default), through the
/// viewers' acceptance seam: one down, one up and five in a burst move the content
/// [`probe::NOTCH_PIXELS`] pixels each. The HTML viewer's glide stops 120 ms after the page
/// stops painting (WEB_VIEWER_API), which a loaded machine can bring about: a shorter move
/// the right way there is flagged for review (backlog VIEW-09), not failed.
fn native_notch(
    app: &mut App,
    context: &mut Context,
    failures: &Failures,
    viewing: &Viewing,
    subject: &Subject,
    native: &mut Vec<(String, i32)>,
) {
    let label = subject.label;
    let what = format!("{label}: one wheel notch moves 100 px (three lines per notch)");
    let result = (|| {
        let _settings = probe::system_settings(
            app,
            viewing,
            probe::SystemSettings {
                wheel: Some(probe::WheelSetting::Lines(3)),
                reduce_motion: Some(false),
            },
        )?;
        let notch = (probe::NOTCH_PIXELS * viewing.slint().scale_factor()).round() as i32;
        let (travel, late) = notch_travel(app, context, viewing, &format!("{label} notch"), notch)?;
        native.push((label.to_owned(), travel.0));
        let (down, up, burst) = travel;
        if (down - notch).abs() <= 1 && (up + notch).abs() <= 1 && (burst - 5 * notch).abs() <= 2 {
            return Ok(());
        }
        let found = format!(
            "one notch down moved {down} px, one up {up} px, five down {burst} px (expected \
             {notch}, -{notch} and {}){}",
            5 * notch,
            late.iter()
                .map(|late| format!("; {late}"))
                .collect::<String>()
        );
        if subject.kind == Kind::Html && down > 0 && up < 0 && burst > 0 {
            let shot = app.screenshot_window(
                context,
                &viewing.software,
                &format!("notch-{}", slug(label)),
                &format!("{label}: native wheel notches"),
            );
            support::flag_shot(
                context,
                &shot,
                "short HTML wheel glide",
                format!(
                    "{label}: {found} (backlog VIEW-09: the glide stops 120 ms after the page stops painting)"
                ),
            );
            return Ok(());
        }
        Err(found)
    })();
    failures.check(&what, result);
}

/// One native notch down, one up and a burst of five, from the middle of the content,
/// with `notch` pixels expected per notch. A movement that comes to rest short of its
/// distance is watched for [`LATE_LIMIT`] more: the remaining distance may wait at the
/// edge of the prepared pixels for a refill (VIEWER_SCROLLING: an uncovered frontier
/// pauses displayed motion until usable pixels arrive). What happened then is returned
/// with the travels, which count to the first rest.
fn notch_travel(
    app: &mut App,
    context: &mut Context,
    viewing: &Viewing,
    label: &str,
    notch: i32,
) -> Result<NotchTravel, String> {
    if probe::vertical_bar(viewing).is_some() {
        probe::drag_vertical(app, viewing, 0.4, 4)?;
        probe::settle(app, viewing, LAND_LIMIT)?;
    }
    let at = probe::content_middle(viewing);
    let mut travel = [0; 3];
    let mut late = Vec::new();
    for (slot, (notches, what)) in [(1, "one down"), (-1, "one up"), (5, "five down")]
        .into_iter()
        .enumerate()
    {
        let mut sent = Ok(());
        let what = format!("{label} {what}");
        let (distance, before, after) = measure(app, context, viewing, &what, |app| {
            sent = probe::native_notches(app, viewing, at, notches);
        })?;
        sent?;
        travel[slot] = distance;
        if (distance - notches * notch).abs() > 1 {
            late.push(continued(app, viewing, &what, distance, (&before, &after)));
        }
    }
    Ok(((travel[0], travel[1], travel[2]), late))
}

/// The travels of one notch down, one up and five down (pixels to the first rest), and
/// what each movement that rested short of its distance did afterwards.
type NotchTravel = ((i32, i32, i32), Vec<String>);

/// How long a movement that came to rest short of its distance is watched for more.
const LATE_LIMIT: Duration = Duration::from_secs(5);

/// After the movement `what` rested at `distance` (between `frames`): whether the content
/// moves on within [`LATE_LIMIT`], and where it then rests, measured from the start.
fn continued(
    app: &mut App,
    viewing: &Viewing,
    what: &str,
    distance: i32,
    (before, after): (&Canvas, &Canvas),
) -> String {
    let started = Instant::now();
    let rest = after.fingerprint();
    if viewing
        .wait(app, "the movement to go on", LATE_LIMIT, |_| {
            viewing.canvas().fingerprint() != rest
        })
        .is_err()
    {
        return format!(
            "{what} rested after {distance} px and did not move on within {} s",
            LATE_LIMIT.as_secs()
        );
    }
    let paused = started.elapsed();
    match settled_canvas(app, viewing)
        .ok()
        .and_then(|now| probe::vertical_motion(before, &now))
        .filter(|motion| motion.share >= probe::SHIFT_CONFIDENCE)
    {
        Some(total) => format!(
            "{what} rested after {distance} px, moved on after {} ms and reached {} px",
            paused.as_millis(),
            total.rounded()
        ),
        None => format!(
            "{what} rested after {distance} px and changed again after {} ms",
            paused.as_millis()
        ),
    }
}

/// The system's wheel setting and reduced motion, as the large log reads them when it
/// gains the focus: six lines per notch move 200 px, one screen per notch a screen; a
/// notch glides over several frames, and with reduced motion it moves in one step.
fn wheel_settings(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let scale = viewing.slint().scale_factor();
    let at = probe::content_middle(viewing);
    probe::drag_vertical(app, viewing, 0.4, 4)?;
    probe::settle(app, viewing, LAND_LIMIT)?;
    let mut problems = Vec::new();
    let screen = (viewing.window.get_content_height() * scale).round() as i32;
    // A screen leaves no row in common: after that notch, arrows go back half a screen.
    let back = screen / 2 / ARROW;
    for (setting, expected, what) in [
        (
            probe::WheelSetting::Lines(6),
            (2.0 * probe::NOTCH_PIXELS * scale).round() as i32,
            "six lines per notch",
        ),
        (probe::WheelSetting::Page, screen, "one screen per notch"),
    ] {
        let arrows = if setting == probe::WheelSetting::Page {
            back
        } else {
            0
        };
        let _settings = probe::system_settings(
            app,
            viewing,
            probe::SystemSettings {
                wheel: Some(setting),
                reduce_motion: Some(false),
            },
        )?;
        let mut sent = Ok(());
        let distance = moved(
            app,
            context,
            viewing,
            &format!("large log, {what}"),
            |app| {
                sent = probe::native_notches(app, viewing, at, 1);
                arrows_back(app, viewing, Key::UpArrow, arrows);
            },
        )? + arrows * ARROW;
        sent?;
        context.note(format!(
            "Large log, {what}: a notch moved {distance} px (expected {expected})"
        ));
        if (distance - expected).abs() > 1 {
            problems.push(format!(
                "{what}: a notch moved {distance} px, expected {expected}"
            ));
        }
        // Back up for the next measurement.
        let _ = moved(app, context, viewing, "large log, back up", |app| {
            let _ = probe::native_notches(app, viewing, at, -1);
            arrows_back(app, viewing, Key::DownArrow, arrows);
        });
    }
    // Gliding and reduced motion: the positions a notch down shows on its way (a notch up
    // right after a notch down is a check of its own, in `notch_travel`).
    for reduce in [false, true] {
        let _settings = probe::system_settings(
            app,
            viewing,
            probe::SystemSettings {
                wheel: Some(probe::WheelSetting::Lines(3)),
                reduce_motion: Some(reduce),
            },
        )?;
        let _ = settled_canvas(app, viewing)?;
        let start = viewing.window.get_displayed_position();
        probe::native_notches(app, viewing, at, 1)?;
        let steps = probe::motion_steps(app, viewing, start, STEP);
        let changes = steps.len() - 1;
        context.note(format!(
            "Large log, reduced motion {reduce}: a notch showed {changes} position change(s)"
        ));
        let wanted = if reduce { changes == 1 } else { changes >= 3 };
        if !wanted {
            problems.push(format!(
                "with reduced motion {}, a notch showed {changes} position change(s) (expected \
                 {})",
                if reduce { "on" } else { "off" },
                if reduce {
                    "one step"
                } else {
                    "a glide of three or more"
                }
            ));
        }
    }
    // The system's own settings again, for what follows.
    probe::native_focus(app, viewing, true)?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// One Slint pixel delta down, one up and a burst of five move the content exactly
/// [`probe::SLINT_PIXEL_DELTA`] pixels each.
fn exact_travel((down, up, burst): (i32, i32, i32)) -> Result<(), String> {
    let delta = probe::SLINT_PIXEL_DELTA as i32;
    if (down - delta).abs() <= 1 && (up + delta).abs() <= 1 && (burst - 5 * delta).abs() <= 2 {
        Ok(())
    } else {
        Err(format!(
            "one pixel delta of {delta} px down moved {down} px, one up {up} px, five down \
             {burst} px (expected {delta}, -{delta} and {})",
            5 * delta
        ))
    }
}

/// The PDF publishes frames while its scrollbar is dragged, not only after the release:
/// at least two distinct frames over [`DRAG_STEPS`] moves.
fn drag_frames(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    probe::drag_vertical(app, viewing, 0.1, 4)?;
    probe::settle(app, viewing, LAND_LIMIT)?;
    let before = probe::still_canvas(app, viewing, STEP)?.fingerprint();
    let prints = probe::drag_frames(app, viewing, 0.6, DRAG_STEPS, DRAG_FRAME)?;
    let settled = probe::settle(app, viewing, LAND_LIMIT)?;
    // Frames shown during the drag that differ from the page before it and from each
    // other: the page moved at least twice while the button was held.
    let moved: std::collections::HashSet<u64> = prints
        .iter()
        .copied()
        .filter(|print| *print != before)
        .collect();
    let changes = prints.windows(2).filter(|pair| pair[0] != pair[1]).count();
    context.note(format!(
        "PDF scrollbar drag: {} distinct moved frame(s) during {DRAG_STEPS} moves {} ms apart, \
         {changes} change(s) between consecutive moves; landed at {:.3}",
        moved.len(),
        DRAG_FRAME.as_millis(),
        settled.displayed
    ));
    if moved.len() < 2 {
        return Err(format!(
            "the page showed {} distinct moved frame(s) during {DRAG_STEPS} drag moves: it \
             does not follow the drag while the button is held",
            moved.len()
        ));
    }
    Ok(())
}

/// Ctrl+Y and a typed 400 %: the zoom applies, and once the page is still its text has
/// sharp edges (rendered at the zoom, not an enlarged raster); Ctrl+0 fits it again.
fn sharp_text(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    // From the first page's top, the zoom about the view's middle lands on its lines.
    viewing.click_content(app);
    input::chord(viewing.slint(), &[Key::Control], Key::Home);
    probe::settle(app, viewing, LAND_LIMIT)?;
    input::chord(viewing.slint(), &[Key::Control], "y");
    viewing.wait(app, "Ctrl+Y to open the zoom field", STEP, |window| {
        window.get_zoom_editing()
    })?;
    input::replace_text(viewing.slint(), SHARP_ZOOM);
    input::key(viewing.slint(), Key::Return);
    viewing.wait(app, "the typed 400 % zoom", STEP, |window| {
        !window.get_zoom_editing() && (window.get_zoom_percent() - 400.0).abs() < 0.5
    })?;
    let canvas = probe::still_canvas(app, viewing, STEP)?;
    let widths = probe::edge_widths(&canvas);
    let shot = support::save_canvas(
        context,
        &canvas,
        "pdf-400-percent",
        "pages.pdf at a typed 400 % zoom (the text's edges are measured)",
    );
    let quantile = |share: f64| {
        widths
            .get(((widths.len() as f64 * share) as usize).min(widths.len().saturating_sub(1)))
            .copied()
    };
    let (median, upper) = (quantile(0.5), quantile(0.75));
    context.note(format!(
        "PDF at {SHARP_ZOOM} %: {} text edges, median width {median:?} px, 75th percentile \
         {upper:?} px ({shot})",
        widths.len()
    ));
    let result = if widths.len() < 100 {
        Err(format!(
            "only {} edges between the page and solid ink at {SHARP_ZOOM} %: no sharp text",
            widths.len()
        ))
    } else if upper.is_none_or(|upper| upper > SHARP_EDGE) {
        Err(format!(
            "the text's edges are blurred at {SHARP_ZOOM} %: median {median:?} px, 75th \
             percentile {upper:?} px (sharp: at most {SHARP_EDGE} px)"
        ))
    } else {
        Ok(())
    };
    input::chord(viewing.slint(), &[Key::Control], "0");
    viewing.wait(app, "Ctrl+0 to fit the page again", STEP, |window| {
        window.get_zoom_percent() < 400.0
    })?;
    result
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
    measure(app, context, viewing, what, act).map(|(distance, _, _)| distance)
}

/// [`moved`], with the content before and after the movement.
fn measure(
    app: &mut App,
    context: &mut Context,
    viewing: &Viewing,
    what: &str,
    act: impl FnOnce(&mut App),
) -> Result<(i32, Canvas, Canvas), String> {
    let before = settled_canvas(app, viewing)?;
    let shown = viewing.window.get_displayed_position();
    act(app);
    let after = settled_canvas(app, viewing)?;
    if before.fingerprint() == after.fingerprint() {
        return Ok((0, before, after));
    }
    // The shown position tells which way the content went; the pixels, how far.
    let change = viewing.window.get_displayed_position() - shown;
    let direction = if change.abs() > 1e-6 {
        change.signum() as i32
    } else {
        0
    };
    let motion = probe::vertical_motion_towards(&before, &after, direction);
    if !motion.is_some_and(|motion| motion.share >= probe::SHIFT_CONFIDENCE) {
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
    match motion {
        Some(motion) if motion.share >= probe::SHIFT_CONFIDENCE => {
            if motion.phase != 0.0 {
                // A refill drew the text again at the camera's new sub-pixel phase.
                context.note(format!(
                    "{what}: moved {:.2} px, the text drawn again at another sub-pixel phase \
                     ({:.0}% of the rows agree)",
                    motion.pixels as f32 + motion.phase,
                    motion.share * 100.0
                ));
            }
            Ok((motion.rounded(), before, after))
        }
        Some(motion) => Err(format!(
            "{what}: the content changed but did not move as one piece (best match {:.2} px \
             for {:.0}% of the rows)",
            motion.pixels as f32 + motion.phase,
            motion.share * 100.0
        )),
        None => Err(format!("{what}: the two frames cannot be compared")),
    }
}

/// The content once it has come to rest: still for [`probe::QUIET`], and unchanged
/// 700 ms later. A motion that pauses at the edge of the prepared pixels until a refill
/// arrives, then resumes, is not cut short this way.
fn settled_canvas(app: &mut App, viewing: &Viewing) -> Result<Canvas, String> {
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

/// One Slint pixel delta down, one up and a burst of five down, from the middle of the
/// content (see [`probe::SLINT_PIXEL_DELTA`]: not the product's wheel notch).
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
        &format!("{label} one pixel delta down"),
        |_| probe::pixel_deltas(window, at, 1),
    )?;
    let up = moved(
        app,
        context,
        viewing,
        &format!("{label} one pixel delta up"),
        |_| probe::pixel_deltas(window, at, -1),
    )?;
    let burst = moved(
        app,
        context,
        viewing,
        &format!("{label} five pixel deltas"),
        |_| probe::pixel_deltas(window, at, 5),
    )?;
    Ok((down, up, burst))
}

/// After a step of a screen: `count` arrows back. The step comes to rest before the arrows
/// (a reverse key would drop the rest of an eased step), and the arrows go four at a
/// time with rests between them, so a burst's own limits do not enter a measurement.
fn arrows_back(app: &mut App, viewing: &Viewing, key: Key, count: i32) {
    if count <= 0 {
        return;
    }
    let _ = settled_canvas(app, viewing);
    for group in 0..(count + 3) / 4 {
        for _ in 0..(count - group * 4).min(4) {
            press(viewing, &[], key);
        }
        let _ = settled_canvas(app, viewing);
    }
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
    let page_down = moved(app, context, viewing, &format!("{label} PageDown"), |app| {
        press(viewing, &[], Key::PageDown);
        arrows_back(app, viewing, Key::UpArrow, back);
    })? + back * ARROW;
    let page_up = moved(app, context, viewing, &format!("{label} PageUp"), |app| {
        press(viewing, &[], Key::PageUp);
        arrows_back(app, viewing, Key::DownArrow, back);
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
    // Ctrl+End may move in steps (a page is read on the way) with pauses longer than a
    // quiet period: wait for the end itself, within the landing limit, then settle.
    press(viewing, &[Key::Control], Key::End);
    let _ = viewing.wait(app, "Ctrl+End to reach the end", LAND_LIMIT, |window| {
        window.get_displayed_position() >= 0.999 && !window.get_position_pending()
    });
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
    let _ = viewing.wait(app, "Ctrl+Home to reach the start", LAND_LIMIT, |window| {
        window.get_displayed_position() <= 0.001 && !window.get_position_pending()
    });
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

/// C++ rows with a keyword, a string and a comment on each (42 bytes a row).
fn highlight_rows(rows: usize) -> String {
    let mut text = String::with_capacity(rows * 42);
    for row in 1..=rows {
        text.push_str(&format!(
            "const int v{row:05} = f(\"s{row:05}\"); // c{row:05}\n"
        ));
    }
    text
}

/// The colours of the rows right of the gutter, each at its glyphs' solid cores, with
/// the pixels of its group; colours of under a fiftieth of all glyph pixels are edges.
fn row_colours(viewing: &Viewing, canvas: &Canvas) -> Vec<([u8; 3], usize)> {
    let canvas = probe::right_of_gutter(viewing, canvas, 6);
    let background = probe::dominant(&canvas);
    let cores = probe::glyph_cores(&canvas, background, 24);
    let total: usize = cores.iter().map(|(_, pixels)| pixels).sum();
    cores
        .into_iter()
        .filter(|(_, pixels)| *pixels * 50 >= total.max(1))
        .collect()
}

/// The colours of `reference` that the rows of `canvas` do not show.
fn missing_colours(viewing: &Viewing, canvas: &Canvas, reference: &[[u8; 3]]) -> Vec<String> {
    let shown = row_colours(viewing, canvas);
    reference
        .iter()
        .filter(|colour| {
            !shown
                .iter()
                .any(|(shown, _)| probe::near(*shown, **colour, 2))
        })
        .map(|colour| probe::hex(*colour))
        .collect()
}

/// Open `file` in the Source viewer and wait until its rows show at least three colours
/// (the text and two or more tints); those colours.
fn open_highlighted(app: &mut App, file: &'static str) -> Result<(Viewing, Vec<[u8; 3]>), String> {
    let subject = Subject {
        file,
        label: file,
        kind: Kind::Text,
        basic: false,
        step: 2,
    };
    let viewing = open(app, &subject)?;
    let started = Instant::now();
    loop {
        let canvas = match probe::still_canvas(app, &viewing, STEP) {
            Ok(canvas) => canvas,
            Err(error) => {
                let _ = viewing.close(app);
                return Err(format!("{file}: {error}"));
            }
        };
        let colours = row_colours(&viewing, &canvas);
        if colours.len() >= 3 {
            return Ok((
                viewing,
                colours.into_iter().map(|(colour, _)| colour).collect(),
            ));
        }
        if started.elapsed() >= STEP {
            let _ = viewing.close(app);
            return Err(format!(
                "{file}: the rows never showed their syntax tints (colours {:?})",
                colours
                    .iter()
                    .map(|(colour, pixels)| format!("{} ({pixels} px)", probe::hex(*colour)))
                    .collect::<Vec<_>>()
            ));
        }
    }
}

/// Source highlighting while a person scrolls (the owner's complaint about "Preparing
/// source highlighting…" during scrolling).
fn source_highlighting(app: &mut App, context: &mut Context, failures: &Failures) {
    let small = open_highlighted(app, "highlight.cpp").and_then(|(viewing, reference)| {
        let result = highlight_small(app, context, &viewing, &reference);
        let closed = viewing.close(app);
        result.and(closed)
    });
    failures.check(
        "source highlighting: a small file scrolled by wheel, keys and scrollbar shows no \
         preparation notice and keeps its tints",
        small,
    );
    let large = open_highlighted(app, "highlight-large.cpp").and_then(|(viewing, reference)| {
        let result = highlight_large(app, context, &viewing, &reference);
        let closed = viewing.close(app);
        result.and(closed)
    });
    failures.check(
        "source highlighting: the rows after Ctrl+End in a large file are coloured within a \
         second, without a lasting notice",
        large,
    );
}

/// The small file: twelve wheel notches down and up (one every 50 ms, as a hand turns a
/// wheel), eight PageDowns and eight PageUps (one every 100 ms) and a scrollbar drag to
/// 70 %, the status line read after every frame throughout; after each gesture the rows
/// show every colour they showed when the file opened.
fn highlight_small(
    app: &mut App,
    context: &mut Context,
    viewing: &Viewing,
    reference: &[[u8; 3]],
) -> Result<(), String> {
    let mut watch = probe::StatusWatch::default();
    let mut problems = Vec::new();
    let mut after = |app: &mut App, watch: &mut probe::StatusWatch, what: &str| match watch
        .still(app, viewing, LAND_LIMIT)
    {
        Ok(canvas) => {
            let missing = missing_colours(viewing, &canvas, reference);
            if !missing.is_empty() {
                problems.push(format!("after {what} the rows lack {}", missing.join(", ")));
            }
        }
        Err(error) => problems.push(format!("{what}: {error}")),
    };
    let at = probe::content_middle(viewing);
    for (notches, what) in [(1, "twelve notches down"), (-1, "twelve notches up")] {
        for _ in 0..12 {
            probe::native_notches(app, viewing, at, notches)?;
            watch.pause(app, viewing, Duration::from_millis(50));
        }
        after(app, &mut watch, what);
    }
    if !viewing.window.get_content_focused() {
        focus_content(app, viewing);
    }
    for (key, what) in [
        (Key::PageDown, "eight PageDowns"),
        (Key::PageUp, "eight PageUps"),
    ] {
        for _ in 0..8 {
            press(viewing, &[], key);
            watch.pause(app, viewing, Duration::from_millis(100));
        }
        after(app, &mut watch, what);
    }
    probe::drag_vertical_watched(app, viewing, 0.7, 20, &mut watch)?;
    after(app, &mut watch, "a scrollbar drag to 70 %");
    let notices = watch.showing(PREPARING);
    context.note(format!(
        "highlight.cpp ({SMALL_ROWS} rows): colours {}; statuses while scrolling {:?}",
        reference
            .iter()
            .map(|colour| probe::hex(*colour))
            .collect::<Vec<_>>()
            .join(", "),
        watch.showing("")
    ));
    if !notices.is_empty() {
        problems.push(format!(
            "the status line showed {:?} while scrolling",
            notices[0]
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The large file: Ctrl+End, then the rows shown at the end must have every colour of
/// the file's first rows within [`COLOURED`], and the notice, if shown, must be gone once
/// they have (within a quiet period for the status to arrive).
fn highlight_large(
    app: &mut App,
    context: &mut Context,
    viewing: &Viewing,
    reference: &[[u8; 3]],
) -> Result<(), String> {
    if !viewing.window.get_content_focused() {
        focus_content(app, viewing);
    }
    let mut watch = probe::StatusWatch::default();
    press(viewing, &[Key::Control], Key::End);
    let started = Instant::now();
    let coloured = loop {
        watch.pause(app, viewing, Duration::from_millis(20));
        if viewing.window.get_displayed_position() >= 0.999
            && missing_colours(viewing, &viewing.canvas(), reference).is_empty()
        {
            break started.elapsed();
        }
        if started.elapsed() >= STEP {
            return Err(format!(
                "the rows after Ctrl+End were not coloured within {STEP:?} (shown position \
                 {:.4}, status {:?})",
                viewing.window.get_displayed_position(),
                viewing.status()
            ));
        }
    };
    let cleared = Instant::now();
    while viewing.status().contains(PREPARING) && cleared.elapsed() < probe::QUIET {
        watch.pause(app, viewing, Duration::from_millis(10));
    }
    let lingering = viewing.status().contains(PREPARING);
    context.note(format!(
        "highlight-large.cpp ({LARGE_ROWS} rows): the rows after Ctrl+End coloured after {} \
         ms; statuses {:?}",
        coloured.as_millis(),
        watch.showing("")
    ));
    let mut problems = Vec::new();
    if coloured > COLOURED {
        problems.push(format!(
            "the rows after Ctrl+End were coloured after {} ms (at most {} ms)",
            coloured.as_millis(),
            COLOURED.as_millis()
        ));
    }
    if lingering {
        problems.push(format!(
            "{:?} still showed {} ms after the rows were coloured",
            viewing.status(),
            probe::QUIET.as_millis()
        ));
    }
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
