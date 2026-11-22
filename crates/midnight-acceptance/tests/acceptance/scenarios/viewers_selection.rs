//! Selecting and copying viewed text, and how selected text reads. Ctrl+A selects the
//! rows the loaded page shows and Ctrl+C hands exactly that text to the clipboard
//! (source without its line numbers, Markdown as rendered text, the hex view as its
//! hex rows); a mouse drag and a Shift+click select a range of lines.
//!
//! In every theme the selection is painted in the theme's selection colour and every
//! selected glyph, measured at its solid core right of the line-number gutter, in the
//! theme's selected-text colour (as the viewer window's Theme names them), and that
//! pair keeps the product's minimum: WCAG AA 4.5:1, 7:1 in the Contrast palettes and
//! 12:1 in High Contrast (Rust and CSV columns in all themes; plain text, Markdown
//! with links and struck text, whose strikethrough lines take the selected-text colour
//! too, the hex view and Python, JSON, XML, YAML, CSS and SQL in Blue, Dark and Light).
//! Where the selected-text colour is the page's own (High Contrast), the selected
//! glyphs are measured in it all the same. Unselected text
//! colours keep 5:1 against the page (7:1 in the Contrast and High Contrast palettes)
//! where a colour covers enough glyphs to show its solid core; smaller groups are
//! flagged for review. In every theme the Rust sample also loses the system focus (a
//! native focus event through the viewers' acceptance seam): the selection then takes
//! the theme's dimmer inactive pair, which keeps the same minimum, and the active pair
//! returns with the focus. Each theme is a fresh start of the application.
use crate::{
    Context,
    app::{App, Config},
    input, panel, viewers_stage,
    viewers_support::{self as support, Canvas, Failures, Viewing, clip, generated, probe},
};
use midnight_core::{PaneId, PreferenceTheme};
use slint::{LogicalPosition, platform::Key};
use std::{path::Path, time::Duration};

pub const TIMEOUT: Duration = Duration::from_secs(30 * 60);
const OPEN_LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_secs(15);
/// Glyph colour groups smaller than this are anti-aliasing noise.
const GLYPH_PIXELS: usize = 24;
/// An unselected text colour covering at least this many pixels shows its solid core:
/// its contrast is checked strictly; smaller groups are only flagged.
const CORE_PIXELS: usize = 300;
/// How far a painted colour may lie from the theme's own (per channel, 8-bit rounding of
/// the software renderer).
const PAINTED: u8 = 2;
/// Physical pixels skipped right of the gutter, so its numbers' edges stay out.
const GUTTER_MARGIN: usize = 6;
/// The shortest horizontal run of one colour taken for a decoration line (glyph strokes
/// of this size are at most 16 pixels long).
const DECORATION: usize = 24;

/// What a copied selection must look like.
#[derive(Clone, Copy)]
enum Copied {
    /// A prefix of the file of at least this many lines (the page opens at its top).
    Prefix(usize),
    /// Rendered Markdown: contains the first fragment, none of the markup.
    Rendered(&'static str, &'static [&'static str]),
    /// The hex view: these bytes in hex, whitespace ignored.
    Hex(&'static str),
}

struct Sample {
    file: &'static str,
    label: &'static str,
    copied: Copied,
    /// The page shows struck text, whose strikethrough lines the selection recolours.
    /// (The Markdown viewer colours links without underlining them: its `build.rs`
    /// never sets the underline flag, so no bundled viewer draws underlines.)
    struck: bool,
}

/// Checked in every theme.
const EVERY_THEME: &[Sample] = &[
    Sample {
        file: "sample.rs",
        label: "Rust source",
        copied: Copied::Prefix(20),
        struck: false,
    },
    Sample {
        file: "table.csv",
        label: "CSV columns",
        copied: Copied::Prefix(20),
        struck: false,
    },
];

/// Checked in the representative themes.
const REPRESENTATIVE: &[Sample] = &[
    Sample {
        file: "plain.txt",
        label: "plain text",
        copied: Copied::Prefix(20),
        struck: false,
    },
    Sample {
        file: "doc.md",
        label: "Markdown",
        copied: Copied::Rendered("Long document", &["# Long", "| ---", "```"]),
        struck: false,
    },
    Sample {
        file: "links.md",
        label: "Markdown links and struck text",
        copied: Copied::Rendered("Midnight link one", &["# Links", "](", "~~"]),
        struck: true,
    },
    Sample {
        file: "data.bin",
        label: "hex view",
        copied: Copied::Hex("000102030405060708090A0B0C0D0E0F"),
        struck: false,
    },
    Sample {
        file: "sample.py",
        label: "Python",
        copied: Copied::Prefix(20),
        struck: false,
    },
    Sample {
        file: "sample.json",
        label: "JSON",
        copied: Copied::Prefix(20),
        struck: false,
    },
    Sample {
        file: "sample.xml",
        label: "XML",
        copied: Copied::Prefix(20),
        struck: false,
    },
    Sample {
        file: "sample.yaml",
        label: "YAML",
        copied: Copied::Prefix(20),
        struck: false,
    },
    Sample {
        file: "sample.css",
        label: "CSS",
        copied: Copied::Prefix(20),
        struck: false,
    },
    Sample {
        file: "sample.sql",
        label: "SQL",
        copied: Copied::Prefix(20),
        struck: false,
    },
];

const REPRESENTATIVE_THEMES: &[PreferenceTheme] = &[
    PreferenceTheme::Blue,
    PreferenceTheme::Dark,
    PreferenceTheme::Light,
];

pub fn selection(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn prepare(context: &Context) -> Result<std::path::PathBuf, String> {
    let root = context.work.join("select");
    generated::write(&root, "sample.rs", generated::rust(400))?;
    generated::write(&root, "table.csv", generated::table(200, ','))?;
    let mut plain = String::new();
    for line in 1..=400 {
        plain.push_str(&format!(
            "line {line:04}: the quick viewer selects {} words\n",
            line * 7 % 31
        ));
    }
    generated::write(&root, "plain.txt", plain)?;
    generated::write(&root, "doc.md", generated::markdown(6))?;
    // Paragraphs of links and struck words between selected text (no tables, whose
    // borders are long lines too).
    let mut links = String::from("# Links\n\n");
    for paragraph in 0..12 {
        links.push_str(&format!(
            "Paragraph {paragraph}: [Midnight link one](https://example.invalid/one) and \
             ~~these struck words~~ beside [another link two](https://example.invalid/two) \
             in running text.\n\n"
        ));
    }
    generated::write(&root, "links.md", links)?;
    generated::write(
        &root,
        "data.bin",
        (0..=255_u8).cycle().take(64 * 1024).collect::<Vec<u8>>(),
    )?;
    for (name, content, _) in generated::source_samples() {
        if matches!(
            name.as_str(),
            "sample.py"
                | "sample.json"
                | "sample.xml"
                | "sample.yaml"
                | "sample.css"
                | "sample.sql"
        ) {
            // Repeated, so every token colour has many glyphs on the page.
            generated::write(&root, &name, content.repeat(8))?;
        }
    }
    Ok(root)
}

fn themes() -> Vec<PreferenceTheme> {
    (0..64).map_while(PreferenceTheme::from_index).collect()
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = prepare(context)?;
    let failures = Failures::default();
    let mut lowest: Vec<String> = Vec::new();
    let themes = themes();
    context.note(format!("Themes checked: {}", themes.len()));
    for theme in themes {
        let mut config = Config::new(&root, &context.work);
        config.plugins = Some(context.work.join("plugins"));
        // Ctrl+C hands the text on to the application's clipboard service.
        config.clipboard = true;
        config.theme = theme;
        let mut app = App::start(config);
        let mut samples: Vec<&Sample> = EVERY_THEME.iter().collect();
        if REPRESENTATIVE_THEMES.contains(&theme) {
            samples.extend(REPRESENTATIVE);
        }
        for sample in samples {
            let what = format!("{theme:?} {}", sample.label);
            let result = contrast(&mut app, context, &root, theme, sample).map(|ratio| {
                lowest.push(format!("{what} {ratio:.2}"));
            });
            failures.check(&what, result);
        }
        if theme == PreferenceTheme::Dark {
            failures.check(
                "mouse selection (drag and Shift+click)",
                mouse_selection(&mut app, context, &root),
            );
        }
        app.shutdown()?;
    }
    context.note(format!(
        "Contrast of the selected-text colour on the painted selection per theme and sample \
         (minimum 4.5, 7 in the Contrast and 12 in the High Contrast palettes): {}",
        lowest.join(", ")
    ));
    failures.finish(context)
}

/// The viewer loses the system focus (a native focus event): the selection takes the
/// theme's inactive pair as the viewer window's Theme names it, painted over the page as
/// the active pair was, with every selected glyph core in the inactive selected-text
/// colour, and that pair keeps the theme's minimum; the focus coming back brings the
/// active selection back. `before` is the unselected page, `selected` the active
/// selection, both right of the gutter; `minimum` the theme's.
fn inactive_pair(
    app: &mut App,
    viewing: &Viewing,
    before: &Canvas,
    selected: &Canvas,
    minimum: f64,
) -> Result<String, String> {
    let (fill, ink) = (
        probe::rgb(viewing.window.get_inactive_selection_background()),
        probe::rgb(viewing.window.get_inactive_selection_text()),
    );
    let active = (
        probe::rgb(viewing.window.get_selection_background()),
        probe::rgb(viewing.window.get_selection_text()),
    );
    probe::native_focus(app, viewing, false)?;
    let crop = |canvas: &Canvas| probe::right_of_gutter(viewing, canvas, GUTTER_MARGIN);
    if (fill, ink) != active {
        viewing.wait(app, "the inactive selection", STEP, |_| {
            crop(&viewing.canvas()).fingerprint() != selected.fingerprint()
        })?;
    }
    let inactive = crop(&probe::still_canvas(app, viewing, STEP)?);
    let background = probe::dominant(before);
    let mut problems = Vec::new();
    match probe::painted_over(before, &inactive, background) {
        Some((painted, _)) if probe::near(painted, fill, PAINTED) => {
            let stray: Vec<String> =
                probe::selected_glyph_cores(&inactive, before, painted, GLYPH_PIXELS)
                    .into_iter()
                    .filter(|(colour, _)| {
                        colour
                            .iter()
                            .zip(background)
                            .map(|(a, b)| u32::from(a.abs_diff(b)))
                            .sum::<u32>()
                            > 36
                            && !probe::near(*colour, ink, PAINTED)
                    })
                    .map(|(colour, pixels)| format!("{} ({pixels} px)", probe::hex(colour)))
                    .collect();
            if !stray.is_empty() {
                problems.push(format!(
                    "inactive selected glyphs are drawn in other colours than the inactive \
                     selected text {}: {}",
                    probe::hex(ink),
                    stray.join(", ")
                ));
            }
        }
        other => problems.push(format!(
            "the inactive selection is painted {:?}, the theme's inactive selection is {}",
            other.map(|(colour, _)| probe::hex(colour)),
            probe::hex(fill)
        )),
    }
    let pair = probe::contrast(ink, fill);
    if pair < minimum {
        problems.push(format!(
            "the inactive pair {} on {} is {pair:.2}:1, below {minimum}:1",
            probe::hex(ink),
            probe::hex(fill)
        ));
    }
    // The focus back: the selection in the active pair again.
    probe::native_focus(app, viewing, true)?;
    viewing.wait(app, "the active selection again", STEP, |_| {
        probe::painted_over(before, &crop(&viewing.canvas()), background)
            .is_some_and(|(painted, _)| probe::near(painted, active.0, PAINTED))
    })?;
    if problems.is_empty() {
        Ok(format!(
            "inactive {} on {} {pair:.2}:1",
            probe::hex(ink),
            probe::hex(fill)
        ))
    } else {
        Err(problems.join("; "))
    }
}

/// Open `sample`, select its page with Ctrl+A and check the selected text's contrast
/// and what Ctrl+C copies; returns the lowest contrast found.
fn contrast(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    theme: PreferenceTheme,
    sample: &Sample,
) -> Result<f64, String> {
    panel::select_row(app, PaneId::Left, sample.file)?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let result = (|| {
        viewing.result.clone()?;
        viewing.wait(app, "the content", STEP, |window| {
            let status = window.get_status();
            !status.is_empty() && !status.starts_with("Opening")
        })?;
        select_page(app, context, root, theme, sample, &viewing)
    })();
    viewing.close(app)?;
    result
}

fn select_page(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    theme: PreferenceTheme,
    sample: &Sample,
    viewing: &Viewing,
) -> Result<f64, String> {
    if !viewing.window.get_content_focused() {
        return Err("the content has no keyboard focus after opening".into());
    }
    let whole = probe::still_canvas(app, viewing, STEP)?;
    let copies = support::plugins(app)?.copied.borrow().len();
    input::chord(viewing.slint(), &[Key::Control], "a");
    viewing.wait(app, "Ctrl+A to paint a selection", STEP, |_| {
        viewing.canvas().fingerprint() != whole.fingerprint()
    })?;
    let selected = probe::still_canvas(app, viewing, STEP)?;
    // The text right of the line-number gutter: the gutter's numbers are not selected.
    let before = probe::right_of_gutter(viewing, &whole, GUTTER_MARGIN);
    let after = probe::right_of_gutter(viewing, &selected, GUTTER_MARGIN);
    let background = probe::dominant(&before);
    let (selection, painted) = probe::painted_over(&before, &after, background)
        .ok_or("Ctrl+A changed no background pixel")?;
    if painted < 2000 {
        return Err(format!(
            "Ctrl+A painted only {painted} pixels in {}",
            probe::hex(selection)
        ));
    }
    // The theme's pair, as the viewer window's Theme names it.
    let (fill, ink) = (
        probe::rgb(viewing.window.get_selection_background()),
        probe::rgb(viewing.window.get_selection_text()),
    );
    let (minimum, syntax) = (
        probe::selected_text_minimum(theme),
        probe::syntax_minimum(theme),
    );
    let shot = app.screenshot_window(
        context,
        &viewing.software,
        &format!("{theme:?}-{}", sample.file.replace('.', "-")).to_lowercase(),
        &format!(
            "{theme:?}: {} selected with Ctrl+A (selection {} over {}, selected text {})",
            sample.label,
            probe::hex(selection),
            probe::hex(background),
            probe::hex(ink)
        ),
    );
    let mut problems = Vec::new();
    if !probe::near(selection, fill, PAINTED) {
        problems.push(format!(
            "the selection is painted {}, the theme's selection is {}",
            probe::hex(selection),
            probe::hex(fill)
        ));
    }
    let pair = probe::contrast(ink, fill);
    if pair < minimum {
        problems.push(format!(
            "the theme's selected text {} on its selection {} is {pair:.2}:1, below {minimum}:1",
            probe::hex(ink),
            probe::hex(fill)
        ));
    }
    // Text beside the selection (the page's own background between selected runs, a
    // table cell) is not selected text, unless the theme's selected text is the page's
    // own colour (High Contrast): the two then read the same. Each colour is measured at
    // its glyphs' solid cores: their anti-aliased edges are blends towards the selection.
    let ink_is_page = probe::near(ink, background, PAINTED);
    let glyphs: Vec<([u8; 3], usize)> =
        probe::selected_glyph_cores(&after, &before, selection, GLYPH_PIXELS)
            .into_iter()
            .filter(|(colour, _)| {
                ink_is_page
                    || colour
                        .iter()
                        .zip(background)
                        .map(|(a, b)| u32::from(a.abs_diff(b)))
                        .sum::<u32>()
                        > 36
            })
            .collect();
    let readings: Vec<String> = glyphs
        .iter()
        .map(|(colour, pixels)| {
            format!(
                "{} ({pixels} px) {:.2}:1",
                probe::hex(*colour),
                probe::contrast(*colour, selection)
            )
        })
        .collect();
    if glyphs.is_empty() {
        problems.push(format!(
            "no text found on the selection colour {}",
            probe::hex(selection)
        ));
    }
    let stray: Vec<String> = glyphs
        .iter()
        .filter(|(colour, _)| !probe::near(*colour, ink, PAINTED))
        .map(|(colour, pixels)| {
            format!(
                "{} ({pixels} px, {:.2}:1)",
                probe::hex(*colour),
                probe::contrast(*colour, selection)
            )
        })
        .collect();
    if !stray.is_empty() {
        problems.push(format!(
            "selected glyphs are drawn in other colours than the theme's selected text {}: {}",
            probe::hex(ink),
            stray.join(", ")
        ));
    }
    let lowest = glyphs
        .iter()
        .map(|(colour, _)| probe::contrast(*colour, selection))
        .fold(f64::INFINITY, f64::min);
    // The inactive pair, once per theme.
    if sample.file == "sample.rs" {
        match inactive_pair(app, viewing, &before, &after, minimum) {
            Ok(reading) => context.note(format!("{theme:?} {}: {reading}", sample.label)),
            Err(error) => problems.push(error),
        }
    }
    if sample.struck {
        let lines = decoration_runs(&after, selection, background);
        let wrong: Vec<String> = lines
            .iter()
            .filter(|(colour, _)| !probe::near(*colour, ink, PAINTED))
            .map(|(colour, runs)| format!("{} ({runs} runs)", probe::hex(*colour)))
            .collect();
        let found: usize = lines.iter().map(|(_, runs)| runs).sum();
        let unselected = decoration_runs(&before, background, selection);
        context.note(format!(
            "{theme:?} {}: strikethrough lines over the selection {:?}; before it {:?}",
            sample.label,
            lines
                .iter()
                .map(|(colour, runs)| format!("{} x{runs}", probe::hex(*colour)))
                .collect::<Vec<_>>(),
            unselected
                .iter()
                .map(|(colour, runs)| format!("{} x{runs}", probe::hex(*colour)))
                .collect::<Vec<_>>()
        ));
        if found < 6 {
            problems.push(format!(
                "only {found} strikethrough line(s) found over the selected struck words"
            ));
        } else if !wrong.is_empty() {
            problems.push(format!(
                "selected struck words are crossed out in other colours than the selected \
                 text {}: {}",
                probe::hex(ink),
                wrong.join(", ")
            ));
        }
    }
    // Unselected text: every colour keeps the theme's syntax minimum against the page.
    let plain = probe::glyph_cores(&before, background, GLYPH_PIXELS);
    let weak = |strict: bool| -> Vec<String> {
        plain
            .iter()
            .filter(|(colour, pixels)| {
                probe::contrast(*colour, background) < syntax && (*pixels >= CORE_PIXELS) == strict
            })
            .map(|(colour, pixels)| {
                format!(
                    "{} ({pixels} px) {:.2}:1",
                    probe::hex(*colour),
                    probe::contrast(*colour, background)
                )
            })
            .collect()
    };
    let (dim, faint) = (weak(true), weak(false));
    if !dim.is_empty() {
        problems.push(format!(
            "unselected text colours below {syntax}:1 against the page {}: {}",
            probe::hex(background),
            dim.join(", ")
        ));
    }
    if !faint.is_empty() {
        support::flag_shot(
            context,
            &shot,
            "low-contrast text colour",
            format!(
                "{theme:?} {}: text colours of fewer than {CORE_PIXELS} pixels (their cores may \
                 be blends) below {syntax}:1 against the page {}: {}",
                sample.label,
                probe::hex(background),
                faint.join(", ")
            ),
        );
    }
    context.note(format!(
        "{theme:?} {}: selection {} (theme {}) on {}; selected text {} ({pair:.2}:1, minimum \
         {minimum}:1); selected glyph cores {}; unselected colours {}",
        sample.label,
        probe::hex(selection),
        probe::hex(fill),
        probe::hex(background),
        probe::hex(ink),
        readings.join(", "),
        plain
            .iter()
            .map(|(colour, pixels)| format!(
                "{} ({pixels} px) {:.2}:1",
                probe::hex(*colour),
                probe::contrast(*colour, background)
            ))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let record = support::plugins(app)?.copied.clone();
    input::chord(viewing.slint(), &[Key::Control], "c");
    if let Err(error) = viewing.wait(app, "Ctrl+C to copy", STEP, |_| {
        record.borrow().len() > copies
    }) {
        problems.push(error);
    } else {
        let copied = record.borrow().last().cloned().unwrap_or_default();
        if let Err(error) = check_copy(root, sample, &copied) {
            problems.push(error);
        }
    }
    if problems.is_empty() {
        Ok(lowest)
    } else {
        Err(problems.join("; "))
    }
}

/// Horizontal runs of at least [`DECORATION`] pixels of one colour other than `around`
/// and `page`, with `around` within three pixels before and after them: the decoration
/// lines drawn over `around` (the selection, or the page before it), such as a
/// strikethrough through the glyphs of its colour. The renderer fills decorations on
/// whole pixels, so a decoration row is its colour exactly. Returns (colour, runs), most
/// runs first.
fn decoration_runs(canvas: &Canvas, around: [u8; 3], page: [u8; 3]) -> Vec<([u8; 3], usize)> {
    let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    for y in 0..canvas.height {
        let mut x = 0;
        while x < canvas.width {
            let colour = canvas.at(x, y);
            let start = x;
            while x < canvas.width && canvas.at(x, y) == colour {
                x += 1;
            }
            if colour == around || colour == page || x - start < DECORATION {
                continue;
            }
            let left = (start.saturating_sub(3)..start).any(|i| canvas.at(i, y) == around);
            let right = (x..(x + 3).min(canvas.width)).any(|i| canvas.at(i, y) == around);
            if left && right {
                *counts.entry(colour).or_default() += 1;
            }
        }
    }
    let mut runs: Vec<([u8; 3], usize)> = counts.into_iter().collect();
    runs.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    runs
}

fn check_copy(root: &Path, sample: &Sample, copied: &str) -> Result<(), String> {
    match sample.copied {
        Copied::Prefix(lines) => {
            let file = std::fs::read_to_string(root.join(sample.file))
                .map_err(|error| error.to_string())?;
            let copied_lines = copied.trim_end_matches('\n').lines().count();
            if !file.starts_with(copied.trim_end_matches('\n')) || copied_lines < lines {
                return Err(format!(
                    "Ctrl+C copied {copied_lines} line(s) that are not the start of the file \
                     (want at least {lines}): {:?}",
                    clip(copied, 160)
                ));
            }
        }
        Copied::Rendered(fragment, markup) => {
            if !copied.contains(fragment) || markup.iter().any(|mark| copied.contains(mark)) {
                return Err(format!(
                    "Ctrl+C copied {:?}, expected rendered text with {fragment:?} and none of \
                     {markup:?}",
                    clip(copied, 200)
                ));
            }
        }
        Copied::Hex(bytes) => {
            let compact: String = copied
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>()
                .to_uppercase();
            if !compact.contains(bytes) {
                return Err(format!(
                    "Ctrl+C copied {:?}, without the hex bytes {bytes}",
                    clip(copied, 200)
                ));
            }
        }
    }
    Ok(())
}

/// A mouse drag over several lines of plain text, then a click and a Shift+click:
/// Ctrl+C copies exactly a run of the file's text spanning those lines.
fn mouse_selection(app: &mut App, context: &mut Context, root: &Path) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, "plain.txt")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let result = (|| {
        viewing.result.clone()?;
        viewing.wait(app, "the text", STEP, |window| {
            window.get_accessible_text().contains("line 0001")
        })?;
        let file =
            std::fs::read_to_string(root.join("plain.txt")).map_err(|error| error.to_string())?;
        let window = &viewing.window;
        let (left, top) = (window.get_content_left(), window.get_content_top());
        // Drag from the second line's start to about the ninth line's middle.
        let from = LogicalPosition::new(left + 3.0, top + 26.0);
        let to = LogicalPosition::new(left + 160.0, top + 150.0);
        probe::drag(app, viewing.slint(), from, to, 8);
        let dragged = copy(app, &viewing)?;
        app.screenshot_window(
            context,
            &viewing.software,
            "mouse-drag-selection",
            "Dark: a mouse drag over several lines of plain text",
        );
        let lines = dragged.matches('\n').count();
        if !file.contains(&dragged) || !(4..=12).contains(&lines) {
            return Err(format!(
                "the dragged selection copied {lines} line break(s), {}: {:?}",
                if file.contains(&dragged) {
                    "a run of the file"
                } else {
                    "not a run of the file"
                },
                clip(&dragged, 200)
            ));
        }
        // A click, then a Shift+click further down, select between them.
        input::click_at(
            viewing.slint(),
            LogicalPosition::new(left + 3.0, top + 60.0),
        );
        app.settle();
        input::click_with(
            viewing.slint(),
            &[Key::Shift],
            LogicalPosition::new(left + 120.0, top + 220.0),
        );
        let extended = copy(app, &viewing)?;
        let lines = extended.matches('\n').count();
        if !file.contains(&extended) || !(6..=16).contains(&lines) {
            return Err(format!(
                "Shift+click copied {lines} line break(s), {}: {:?}",
                if file.contains(&extended) {
                    "a run of the file"
                } else {
                    "not a run of the file"
                },
                clip(&extended, 200)
            ));
        }
        if extended == dragged {
            return Err("Shift+click did not change the selection".into());
        }
        context.note(format!(
            "Mouse selection: drag copied {:?}; Shift+click copied {:?}",
            clip(&dragged, 80),
            clip(&extended, 80)
        ));
        Ok(())
    })();
    viewing.close(app)?;
    result
}

/// Ctrl+C in the content; the text the viewer handed to the clipboard.
fn copy(app: &mut App, viewing: &Viewing) -> Result<String, String> {
    let record = support::plugins(app)?.copied.clone();
    let before = record.borrow().len();
    input::chord(viewing.slint(), &[Key::Control], "c");
    viewing.wait(app, "Ctrl+C to copy the selection", STEP, |_| {
        record.borrow().len() > before
    })?;
    let copied = record.borrow().last().cloned().unwrap_or_default();
    Ok(copied)
}
