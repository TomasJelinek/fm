//! Selecting and copying viewed text, and how selected text reads. Ctrl+A selects the
//! rows the loaded page shows and Ctrl+C hands exactly that text to the clipboard
//! (source without its line numbers, Markdown as rendered text, the hex view as its
//! hex rows); a mouse drag and a Shift+click select a range of lines. In every theme,
//! selected text keeps at least 4.5:1 contrast (WCAG) against the selection colour
//! painted over the content: Rust and CSV columns in all themes, plain text, Markdown,
//! the hex view and Python, JSON, XML, YAML, CSS and SQL in the representative themes
//! (Blue, Dark, Light). Syntax colours below 3:1 against the unselected background are
//! flagged for review. Each theme is a fresh start of the application.
use crate::{
    Context,
    app::{App, Config},
    input, panel, viewers_stage,
    viewers_support::{self as support, Failures, Viewing, clip, generated, probe},
};
use midnight_core::{PaneId, PreferenceTheme};
use slint::{LogicalPosition, platform::Key};
use std::{path::Path, time::Duration};

pub const TIMEOUT: Duration = Duration::from_secs(30 * 60);
const OPEN_LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_secs(15);
/// The product's minimum for selected text (VIEWER_PRESENTATION.md); a hundredth below
/// it covers rounding to 8-bit colours.
const SELECTED_MINIMUM: f64 = 4.49;
/// Below this an unselected syntax colour is flagged for review (WCAG's minimum for
/// large text).
const UNSELECTED_FLAG: f64 = 3.0;
/// Glyph colour groups smaller than this are anti-aliasing noise.
const GLYPH_PIXELS: usize = 24;

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
}

/// Checked in every theme.
const EVERY_THEME: &[Sample] = &[
    Sample {
        file: "sample.rs",
        label: "Rust source",
        copied: Copied::Prefix(20),
    },
    Sample {
        file: "table.csv",
        label: "CSV columns",
        copied: Copied::Prefix(20),
    },
];

/// Checked in the representative themes.
const REPRESENTATIVE: &[Sample] = &[
    Sample {
        file: "plain.txt",
        label: "plain text",
        copied: Copied::Prefix(20),
    },
    Sample {
        file: "doc.md",
        label: "Markdown",
        copied: Copied::Rendered("Long document", &["# Long", "| ---", "```"]),
    },
    Sample {
        file: "data.bin",
        label: "hex view",
        copied: Copied::Hex("000102030405060708090A0B0C0D0E0F"),
    },
    Sample {
        file: "sample.py",
        label: "Python",
        copied: Copied::Prefix(20),
    },
    Sample {
        file: "sample.json",
        label: "JSON",
        copied: Copied::Prefix(20),
    },
    Sample {
        file: "sample.xml",
        label: "XML",
        copied: Copied::Prefix(20),
    },
    Sample {
        file: "sample.yaml",
        label: "YAML",
        copied: Copied::Prefix(20),
    },
    Sample {
        file: "sample.css",
        label: "CSS",
        copied: Copied::Prefix(20),
    },
    Sample {
        file: "sample.sql",
        label: "SQL",
        copied: Copied::Prefix(20),
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
            match contrast(&mut app, context, &root, theme, sample) {
                Ok(minimum) => lowest.push(format!("{what} {minimum:.2}")),
                Err(error) => failures.check(&what, Err(error)),
            }
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
        "Lowest contrast of selected text per theme and sample (minimum {SELECTED_MINIMUM}): {}",
        lowest.join(", ")
    ));
    failures.finish(context)
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
    let before = probe::still_canvas(app, viewing, STEP)?;
    let background = probe::dominant(&before);
    let copies = support::plugins(app)?.copied.borrow().len();
    input::chord(viewing.slint(), &[Key::Control], "a");
    viewing.wait(app, "Ctrl+A to paint a selection", STEP, |_| {
        viewing.canvas().fingerprint() != before.fingerprint()
    })?;
    let after = probe::still_canvas(app, viewing, STEP)?;
    let (selection, painted) = probe::painted_over(&before, &after, background)
        .ok_or("Ctrl+A changed no background pixel")?;
    if painted < 2000 {
        return Err(format!(
            "Ctrl+A painted only {painted} pixels in {}",
            probe::hex(selection)
        ));
    }
    let shot = app.screenshot_window(
        context,
        &viewing.software,
        &format!("{theme:?}-{}", sample.file.replace('.', "-")).to_lowercase(),
        &format!(
            "{theme:?}: {} selected with Ctrl+A (selection {} over {})",
            sample.label,
            probe::hex(selection),
            probe::hex(background)
        ),
    );
    // The page's own background showing between selected runs (table cells, the gutter
    // edge) is not text. Each text colour is measured at its glyphs' solid cores: their
    // anti-aliased edges are blends that read lower than the colour itself.
    let glyphs: Vec<([u8; 3], usize)> = probe::glyph_cores(&after, selection, GLYPH_PIXELS)
        .into_iter()
        .filter(|(colour, _)| {
            colour
                .iter()
                .zip(background)
                .map(|(a, b)| u32::from(a.abs_diff(b)))
                .sum::<u32>()
                > 36
        })
        .collect();
    if glyphs.is_empty() {
        return Err(format!(
            "no text found on the selection colour {}",
            probe::hex(selection)
        ));
    }
    let readings: Vec<(String, f64)> = glyphs
        .iter()
        .map(|(colour, _)| (probe::hex(*colour), probe::contrast(*colour, selection)))
        .collect();
    let lowest = readings
        .iter()
        .map(|(_, ratio)| *ratio)
        .fold(f64::INFINITY, f64::min);
    // What the same colours would show without the selection's adjustment: proof that
    // this check can fail where the theme's colours need it.
    let plain = probe::glyph_cores(&before, background, GLYPH_PIXELS);
    let unadjusted: Vec<String> = plain
        .iter()
        .filter(|(colour, _)| probe::contrast(*colour, selection) < 4.5)
        .map(|(colour, _)| {
            format!(
                "{} {:.2}",
                probe::hex(*colour),
                probe::contrast(*colour, selection)
            )
        })
        .collect();
    let dim: Vec<String> = plain
        .iter()
        .filter(|(colour, _)| probe::contrast(*colour, background) < UNSELECTED_FLAG)
        .map(|(colour, _)| {
            format!(
                "{} {:.2}",
                probe::hex(*colour),
                probe::contrast(*colour, background)
            )
        })
        .collect();
    context.note(format!(
        "{theme:?} {}: selection {} on {}; selected text colours {}; unadjusted colours \
         below 4.5:1 on the selection: {}",
        sample.label,
        probe::hex(selection),
        probe::hex(background),
        readings
            .iter()
            .map(|(colour, ratio)| format!("{colour} {ratio:.2}"))
            .collect::<Vec<_>>()
            .join(", "),
        if unadjusted.is_empty() {
            "none".to_owned()
        } else {
            unadjusted.join(", ")
        }
    ));
    if !dim.is_empty() {
        support::flag_shot(
            context,
            &shot,
            "low-contrast text colour",
            format!(
                "{theme:?} {}: text colours below {UNSELECTED_FLAG}:1 against the background {}: {}",
                sample.label,
                probe::hex(background),
                dim.join(", ")
            ),
        );
    }
    let mut problems = Vec::new();
    if lowest < SELECTED_MINIMUM {
        problems.push(format!(
            "selected text reaches only {lowest:.2}:1 against the selection {}: {}",
            probe::hex(selection),
            readings
                .iter()
                .filter(|(_, ratio)| *ratio < SELECTED_MINIMUM)
                .map(|(colour, ratio)| format!("{colour} {ratio:.2}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
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
