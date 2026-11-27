//! The filename colour palette as rendered and as migrated (docs/developer/
//! filename-color-options.md):
//!
//! - Source code (`.rs`) is sky blue `#23AAF2`; word-processor documents (`.docx`) and
//!   text and Markdown (`.txt`, `.md`) are blue `#3B82F6`. Each name's rendered glyph
//!   colour is what the shared readability policy (`midnight_core::readable_rgb`) makes
//!   of its base colour on the row it is drawn on, measured in the pixels of each name in
//!   Blue, Dark and Light (the light theme darkens both), at 4.5:1 or more.
//! - A saved list equal to a shipped palette moves to the current defaults when it is
//!   loaded: the one shipped until 22 November 2026 (blue source code `#3B82F6`, light
//!   cool neutral documents and text `#C4CFE0`), the interim one in which all three
//!   were cyan, and the one shipped until 23 November 2026 (cyan source code `#06A4C7`,
//!   blue documents and text). A customised list is kept, except that a document or text
//!   rule still exactly as the interim build set it (cyan, its default mask) turns blue,
//!   rule by rule; the cyan source rule and a differing text rule keep theirs.
//!   Only the settings codec migrates, so this part runs with the application's own
//!   settings store in a private child process (`viewers_stage::run_private`),
//!   restarting on the saved `preferences.json`.
//!
//! Registered here, not in the registry's list: this module owns its entries.
use crate::{
    Context, Scenario,
    app::{App, Config},
    inspect,
    options_support::{self as options, Frame, SettingsStore},
    panel,
    scenarios::ui_gate::{contrast, hex},
    viewers_stage,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, PreferenceTheme};
use std::{fs, path::Path, time::Duration};

const NAMES: [&str; 4] = ["alpha.rs", "brief.docx", "notes.md", "plain.txt"];
const FONT: &str = "font.ttf";
/// The source rule's default colour, `#23AAF2`.
const SKY_BLUE: [u8; 3] = [0x23, 0xAA, 0xF2];
/// The source rule's default until 23 November 2026, `#06A4C7`.
const CYAN: [u8; 3] = [0x06, 0xA4, 0xC7];
const BLUE: [u8; 3] = [0x3B, 0x82, 0xF6];
/// The fonts rule's default colour, `#92400E`.
const FONTS: [u8; 3] = [0x92, 0x40, 0x0E];
/// The base colour each of [`NAMES`] has by default.
const DEFAULTS: [(&str, [u8; 3]); 4] = [
    ("alpha.rs", SKY_BLUE),
    ("brief.docx", BLUE),
    ("notes.md", BLUE),
    ("plain.txt", BLUE),
];
/// The rules this scenario edits, and the names each one colours.
const SOURCE_RULE: &str = "1. *.rs";
const DOCUMENT_RULE: &str = "4. *.doc";
const TEXT_RULE: &str = "11. *.txt";
const FONT_RULE: &str = "12. *.ttf";
/// Per channel: what anti-aliasing may move a measured glyph colour.
const TOLERANCE: u8 = 10;
/// The registered timeout, which the private child also needs to know.
const TIMEOUT: Duration = Duration::from_secs(12 * 60);
/// A hang guard for the panel to show its filename colours after a start or an OK.
const COLOURED: Duration = Duration::from_secs(15);
const RULES: &str = "Filename color rules";

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "options_palette",
        isolation: crate::Isolation::ParallelStaged,
        area: "options",
        summary: "The filename palette as rendered: .rs sky blue #23AAF2, .docx, .md and .txt blue #3B82F6, each shown as the shared readability policy makes it on its row (≥4.5:1) in Blue, Dark and Light; the shipped palettes (blue/neutral, interim all-cyan, cyan source with blue documents) move to the defaults on loading, a customised list is kept (its cyan source rule included) except its exact interim document/text rules, which turn blue (private child with the real settings store).",
        run: palette,
        timeout: TIMEOUT,
    }]
}

fn files(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    for name in NAMES.iter().chain([FONT].iter()) {
        fs::write(folder.join(name), name).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= TOLERANCE)
}

/// The name text of `name`'s row in the left panel: its window rectangle.
fn name_rect(app: &App, name: &str) -> Option<inspect::Rect> {
    let mut found = None;
    for (label, row) in inspect::controls(app.slint(), AccessibleRole::ListItem) {
        if !panel::is_row_label(&label, name) || found.is_some() {
            continue;
        }
        // A name is drawn left to right inside bidirectional isolation marks.
        inspect::walk(&row, &mut |item| {
            let plain = options::text_of(item)
                .map(|text| text.trim_matches(['\u{2066}', '\u{2069}']).to_owned());
            if found.is_none() && plain.as_deref() == Some(name) {
                found = inspect::visible_rect(item);
            }
        });
    }
    found
}

/// The rendered colour of a row's name and the row's background under it: the most
/// common colour of the name's box is the background, and the pixel that contrasts
/// most with it is the glyphs' own colour (anti-aliasing only blends towards the
/// background).
fn rendered(frame: &Frame, area: inspect::Rect) -> Option<([u8; 3], [u8; 3])> {
    let (x, y, w, h) = area;
    let (x0, y0) = (x.max(0.0) as usize, y.max(0.0) as usize);
    let (x1, y1) = (
        ((x + w) as usize).min(frame.width),
        ((y + h) as usize).min(frame.height),
    );
    let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
    for row in y0..y1 {
        for column in x0..x1 {
            let pixel = frame.pixels[row * frame.width + column];
            *counts.entry([pixel.r, pixel.g, pixel.b]).or_default() += 1;
        }
    }
    let background = counts
        .iter()
        .max_by_key(|(colour, count)| (**count, **colour))
        .map(|(colour, _)| *colour)?;
    let text = counts
        .keys()
        .copied()
        .max_by(|a, b| contrast(*a, background).total_cmp(&contrast(*b, background)))?;
    Some((text, background))
}

/// Each of [`NAMES`] in `theme`: its model colour is its default base colour, and its
/// rendered colour is what the shared policy makes of that base on the row measured
/// under it (within [`TOLERANCE`]), at 4.5:1 or more.
fn theme_check(
    context: &mut Context,
    folder: &Path,
    theme: PreferenceTheme,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let mut config = Config::new(folder, folder);
    config.theme = theme;
    config.settings = Some(SettingsStore::new());
    let mut app = App::start(config);
    if !options::poll(&mut app, COLOURED, |app| {
        DEFAULTS
            .iter()
            .all(|(name, _)| model_color(app, name).is_some())
    }) {
        return Err(format!("{theme:?}: {}", colours_shown(&app)));
    }
    app.idle(Duration::from_millis(300));
    let frame = Frame::of(&mut app);
    let mut shown = Vec::new();
    for (name, base) in DEFAULTS {
        let model = model_color(&app, name);
        if model != Some(base) {
            failures.push(format!(
                "{theme:?}: {name}'s colour rule gives {:?}, not {}",
                model.map(hex),
                hex(base)
            ));
        }
        let area =
            name_rect(&app, name).ok_or_else(|| format!("{theme:?}: no name text for {name}"))?;
        let (text, background) =
            rendered(&frame, area).ok_or_else(|| format!("{theme:?}: {name} renders nothing"))?;
        let expected = midnight_core::readable_rgb(base, background);
        let ratio = contrast(text, background);
        if !near(text, expected) || ratio + 0.005 < 4.5 {
            failures.push(format!(
                "{theme:?}: {name} renders {} on {} ({ratio:.2}:1); the shared policy shows {} there, at 4.5:1 or more",
                hex(text),
                hex(background),
                hex(expected)
            ));
        }
        shown.push(format!(
            "{name} {} (policy {}) on {} {ratio:.2}:1",
            hex(text),
            hex(expected),
            hex(background)
        ));
    }
    app.screenshot(
        context,
        &format!("palette-{theme:?}").to_lowercase(),
        &format!("Sources, documents and text in {theme:?}"),
    );
    context.note(format!("{theme:?}: {}", shown.join("; ")));
    app.shutdown()
}

pub fn palette(context: &mut Context) -> Result<(), String> {
    viewers_stage::run_private(context, TIMEOUT, palette_and_migration)
}

fn palette_and_migration(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("palette");
    files(&folder)?;
    let mut failures = Vec::new();
    for theme in [
        PreferenceTheme::Blue,
        PreferenceTheme::Dark,
        PreferenceTheme::Light,
    ] {
        theme_check(context, &folder, theme, &mut failures)?;
    }
    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }
    migration(context)
}

/// The foreground the left panel gives `name` (None: the theme's colour).
fn model_color(app: &App, name: &str) -> Option<[u8; 3]> {
    app.rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == name)
        .filter(|row| row.custom_foreground)
        .map(|row| {
            [
                row.foreground.red(),
                row.foreground.green(),
                row.foreground.blue(),
            ]
        })
}

/// Every test name's model colour, for messages.
fn colours_shown(app: &App) -> String {
    format!(
        "the panel colours {:?}",
        NAMES
            .iter()
            .chain([FONT].iter())
            .map(|name| (*name, model_color(app, name).map(hex)))
            .collect::<Vec<_>>()
    )
}

/// The number a rule row's label starts with ("11. *.txt;…" is 11).
fn rule_number(label: &str) -> Option<u32> {
    label.split_once(". ")?.0.parse().ok()
}

/// Turn the wheel over the rules list until the row whose label starts with `prefix`
/// exists: the list makes only the rows near its viewport. A hang guard of 40 turns;
/// the list not moving any more ends it early.
fn show_rule(app: &mut App, prefix: &str) -> Result<(), String> {
    let wanted = rule_number(prefix).ok_or_else(|| format!("{prefix:?} names no rule number"))?;
    let mut previous = Vec::new();
    for _ in 0..40 {
        let rows: Vec<String> = options::list_rows(app, RULES)
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        if rows.iter().any(|label| label.starts_with(prefix)) || rows == previous {
            return Ok(());
        }
        let last = rows.iter().filter_map(|label| rule_number(label)).max();
        let down = last.is_none_or(|last| last < wanted);
        let area = inspect::find(app.slint(), AccessibleRole::List, RULES)
            .and_then(|list| inspect::visible_rect(&list))
            .ok_or_else(|| format!("the {RULES:?} list is not shown"))?;
        app.slint()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(area.0 + area.2 / 2.0, area.1 + area.3 / 2.0),
                delta_x: 0.0,
                delta_y: if down { -60.0 } else { 60.0 },
            });
        inspect::settle_tree(app.slint());
        app.settle();
        previous = rows;
    }
    Ok(())
}

/// Select the rule whose label starts with `prefix` and set its foreground.
fn set_rule(app: &mut App, prefix: &str, color: [u8; 3]) -> Result<(), String> {
    show_rule(app, prefix)?;
    options::click_row(app, RULES, prefix, &|label| label.starts_with(prefix))?;
    options::fill(app, "Foreground hex color", &hex(color).to_uppercase())?;
    options::press(app, "Apply colors")?;
    if !options::poll(app, Duration::from_secs(10), |app| {
        options::texts_with(app, "Checking…").is_empty()
            && options::is_enabled(app, AccessibleRole::Button, "Apply colors")
    }) {
        return Err(format!("the rule {prefix:?} stays in validation"));
    }
    Ok(())
}

/// The test names a rule colours.
fn names_of(prefix: &str) -> &'static [&'static str] {
    match prefix {
        SOURCE_RULE => &["alpha.rs"],
        DOCUMENT_RULE => &["brief.docx"],
        TEXT_RULE => &["notes.md", "plain.txt"],
        FONT_RULE => &[FONT],
        _ => &[],
    }
}

/// Start on the private profile's own settings store, once the panel shows every test
/// name in its filename colour: the listing and its colour matching follow the start
/// (a slower system read the rows before either).
fn start(folder: &Path) -> Result<App, String> {
    let mut config = Config::new(folder, folder);
    config.real_settings = true;
    let mut app = App::start(config);
    if !options::poll(&mut app, COLOURED, |app| {
        NAMES.iter().all(|name| model_color(app, name).is_some())
    }) {
        return Err(format!(
            "{COLOURED:?} after the start: {}",
            colours_shown(&app)
        ));
    }
    Ok(app)
}

/// Set `rules` (a rule's label prefix and foreground) through Options, OK once the
/// panel shows them, then restart on what was saved.
fn save_and_restart(app: App, folder: &Path, rules: &[(&str, [u8; 3])]) -> Result<App, String> {
    let mut app = app;
    options::open(&mut app)?;
    options::page(&mut app, "Colors")?;
    for (prefix, color) in rules {
        set_rule(&mut app, prefix, *color)?;
    }
    let ok = options::reveal(&mut app, AccessibleRole::Button, "OK")?;
    options::click_item(&mut app, &ok)?;
    app.until("Options to close after OK", |app| {
        !app.window.get_options_open()
    });
    let applied: Vec<(&str, [u8; 3])> = rules
        .iter()
        .flat_map(|(prefix, color)| names_of(prefix).iter().map(move |name| (*name, *color)))
        .collect();
    let watched = applied.clone();
    if !options::poll(&mut app, COLOURED, move |app| {
        watched
            .iter()
            .all(|(name, color)| model_color(app, name) == Some(*color))
    }) {
        return Err(format!(
            "after OK the panel does not show the saved rules {:?}: {}",
            applied
                .iter()
                .map(|(name, color)| (*name, hex(*color)))
                .collect::<Vec<_>>(),
            colours_shown(&app)
        ));
    }
    app.shutdown()?;
    drop(app);
    start(folder)
}

/// Every test name's model colour against `expected`; the mismatches.
fn compare(app: &App, expected: &[(&str, [u8; 3])]) -> Vec<String> {
    expected
        .iter()
        .filter(|(name, color)| model_color(app, name) != Some(*color))
        .map(|(name, color)| {
            format!(
                "{name} is {:?}, not {}",
                model_color(app, name).map(hex),
                hex(*color)
            )
        })
        .collect()
}

/// The private child: the real settings store, restarted on what Options saved.
fn migration(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("palette-migration");
    files(&folder)?;
    let defaults: Vec<(&str, [u8; 3])> = DEFAULTS.iter().copied().chain([(FONT, FONTS)]).collect();
    let app = start(&folder)?;
    let wrong = compare(&app, &defaults);
    if !wrong.is_empty() {
        return Err(format!("a new profile: {}", wrong.join("; ")));
    }
    // The palette shipped until 22 November: loaded again, it moves to the defaults.
    let neutral = [0xC4, 0xCF, 0xE0];
    let app = save_and_restart(
        app,
        &folder,
        &[
            (SOURCE_RULE, BLUE),
            (DOCUMENT_RULE, neutral),
            (TEXT_RULE, neutral),
        ],
    )?;
    let wrong = compare(&app, &defaults);
    if !wrong.is_empty() {
        return Err(format!(
            "the saved palette shipped until 22 November was not moved to the defaults: {}",
            wrong.join("; ")
        ));
    }
    // The interim palette (all three cyan): it moves to the defaults too.
    let app = save_and_restart(
        app,
        &folder,
        &[
            (SOURCE_RULE, CYAN),
            (DOCUMENT_RULE, CYAN),
            (TEXT_RULE, CYAN),
        ],
    )?;
    let wrong = compare(&app, &defaults);
    if !wrong.is_empty() {
        return Err(format!(
            "the saved interim all-cyan palette was not moved to the defaults: {}",
            wrong.join("; ")
        ));
    }
    // The palette shipped until 23 November (cyan source, blue documents and text): it
    // moves to the defaults, so the source rule turns sky blue.
    let app = save_and_restart(app, &folder, &[(SOURCE_RULE, CYAN)])?;
    let wrong = compare(&app, &defaults);
    if !wrong.is_empty() {
        return Err(format!(
            "the saved palette shipped until 23 November was not moved to the defaults: {}",
            wrong.join("; ")
        ));
    }
    // A customised list is kept, except that its exact interim document rule turns blue:
    // the source rule in cyan, the earlier default, and a green text rule keep their
    // colours.
    let green = [0x22, 0xC5, 0x5E];
    let custom_font = [0x92, 0x40, 0x0F];
    let app = save_and_restart(
        app,
        &folder,
        &[
            (SOURCE_RULE, CYAN),
            (DOCUMENT_RULE, CYAN),
            (TEXT_RULE, green),
            (FONT_RULE, custom_font),
        ],
    )?;
    let kept = [
        ("alpha.rs", CYAN),
        ("brief.docx", BLUE),
        ("notes.md", green),
        ("plain.txt", green),
        (FONT, custom_font),
    ];
    let wrong = compare(&app, &kept);
    if !wrong.is_empty() {
        return Err(format!(
            "a customised list was not kept (with its interim document rule turned blue): {}",
            wrong.join("; ")
        ));
    }
    let mut app = app;
    app.screenshot(
        context,
        "palette-customised-kept",
        "A customised list kept after a restart, its interim document rule turned blue",
    );
    context.note(format!(
        "Migration (real settings store): the palettes shipped until 22 November and until 23 November and the interim all-cyan one all came back as the defaults (.rs {}, .docx/.md/.txt {}, fonts {}); a customised list kept its cyan source rule, green text rule and font colour and turned its interim cyan document rule blue",
        hex(SKY_BLUE),
        hex(BLUE),
        hex(FONTS)
    ));
    app.shutdown()
}
