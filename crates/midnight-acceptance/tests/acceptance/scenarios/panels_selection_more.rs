//! Selection beyond `files_selection`: Ctrl+=, Ctrl++ and Ctrl+− (no keypad needed)
//! with the remembered mask and Include folders; an invalid mask changes nothing and
//! says why; Ctrl+Shift+click adds a range; Alt+Down and Alt+Up visit the marked rows
//! without wrapping; Insert and Space stop at the last row; `..` is never marked; the
//! rubber band marks the rows it touches (plain replaces, Shift adds, Ctrl toggles);
//! the details strip counts the marked files and folders and their bytes after every
//! change, compared with the disk; the last mask and Include folders survive a restart.
//! In every theme marked rows keep their selected-text
//! contrast in the rendered frame, in the active and the inactive panel.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect,
    options_support::Frame,
    panel,
    scenarios::{
        panels_support::{self as ps, LEFT},
        ui_gate,
    },
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PreferenceTheme;
use midnight_ui::PanelChoices;
use slint::{ComponentHandle, LogicalPosition, platform::Key};
use std::{path::Path, time::Duration};

const FILES: [(&str, usize); 6] = [
    ("a1.txt", 11),
    ("a2.txt", 22),
    ("b1.log", 33),
    ("b2.log", 44),
    ("c.dat", 55),
    ("d.dat", 66),
];
const FOLDERS: [&str; 2] = ["f1", "f2"];

fn mask_open(app: &App) -> bool {
    !app.window
        .global::<PanelChoices>()
        .get_mask_token()
        .is_empty()
}

/// The marks must be `wanted` and the details strip must count them, with the
/// files' bytes taken from the disk.
fn expect(app: &mut App, folder: &Path, what: &str, wanted: &[&str], failures: &mut Vec<String>) {
    let target = ps::sorted(wanted);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT) == target
    }) {
        failures.push(format!(
            "{what}: marks {:?}, expected {target:?}",
            ps::marked(app, LEFT)
        ));
        return;
    }
    let files: Vec<&&str> = wanted
        .iter()
        .filter(|name| folder.join(name).is_file())
        .collect();
    let folders = wanted.len() - files.len();
    let bytes: u64 = files
        .iter()
        .map(|name| std::fs::metadata(folder.join(name)).map_or(0, |metadata| metadata.len()))
        .sum();
    let details = ps::details(app);
    let value = |label: &str| {
        details
            .iter()
            .find(|(field, _)| field == label)
            .map(|(_, value)| value.clone())
    };
    if wanted.is_empty() {
        if value("Selected").is_some() {
            failures.push(format!(
                "{what}: nothing is marked but the strip says {details:?}"
            ));
        }
        return;
    }
    let counted = format!(
        "{} {}, {folders} {}",
        files.len(),
        if files.len() == 1 { "file" } else { "files" },
        if folders == 1 { "folder" } else { "folders" }
    );
    // Updating values settle within moments.
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::details(app)
            .iter()
            .any(|(field, shown)| field == "Selected" && *shown == counted)
    }) {
        failures.push(format!(
            "{what}: the details strip says {:?}, expected Selected {counted:?}",
            ps::details(app)
        ));
    }
    if !files.is_empty() {
        let shown = ps::details(app)
            .into_iter()
            .find(|(field, _)| field == "Selected file bytes")
            .map(|(_, value)| value);
        let digits = shown.as_deref().and_then(crate::tools_support::number);
        if digits != Some(bytes) {
            failures.push(format!(
                "{what}: the details strip shows {shown:?} for the marked files' {bytes} bytes"
            ));
        }
    }
}

/// Open the mask dialog with `modifiers`+`key`, check its kind and starting mask, type
/// `mask` (setting Include folders to `folders`) and press Enter or OK.
#[allow(clippy::too_many_arguments)] // Each argument is one expectation of the dialog.
fn mask(
    app: &mut App,
    key: &str,
    select: bool,
    starts: &str,
    starts_folders: bool,
    mask: &str,
    folders: bool,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], key);
    if !ps::wait(app, Duration::from_secs(5), mask_open) {
        failures.push(format!("Ctrl+{key} opened no mask dialog"));
        return Ok(());
    }
    let choices = app.window.global::<PanelChoices>();
    let shown = ps::value(
        app,
        i_slint_core::items::AccessibleRole::TextInput,
        "Selection mask",
    );
    if choices.get_mask_select() != select || shown.as_deref() != Some(starts) {
        failures.push(format!(
            "Ctrl+{key}: select {} with {shown:?}, expected select {select} starting from {starts:?}",
            choices.get_mask_select()
        ));
    }
    if ps::checked(app, "Include folders") != Some(starts_folders) {
        failures.push(format!(
            "Ctrl+{key}: Include folders starts {:?}, expected {starts_folders} (remembered with the mask)",
            ps::checked(app, "Include folders")
        ));
    }
    input::fill(app.slint(), "Selection mask", mask)?;
    if ps::checked(app, "Include folders") != Some(folders) {
        input::click(
            app.slint(),
            i_slint_core::items::AccessibleRole::Checkbox,
            "Include folders",
        )?;
        app.settle();
        input::click_button(app.slint(), "OK")?;
    } else {
        input::key(app.slint(), Key::Return);
    }
    app.settle();
    Ok(())
}

fn click(app: &mut App, name: &str, modifiers: &[Key]) -> Result<(), String> {
    panel::wait_input(app);
    let position = panel::row_center(app, LEFT, name).ok_or_else(|| format!("no row {name}"))?;
    app.idle(Duration::from_millis(650));
    input::click_with(app.slint(), modifiers, position);
    app.last_click = Some((std::time::Instant::now(), position));
    app.settle();
    Ok(())
}

pub fn selection_more(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "selection-more")?;
    for (name, size) in FILES {
        ps::write(&folder.join(name), vec![b'x'; size])?;
    }
    for name in FOLDERS {
        ps::folder(&folder.join(name))?;
    }
    let store = crate::options_support::SettingsStore::new();
    let mut config = Config::new(&folder, &context.work);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let mut failures = Vec::new();

    // Ctrl+= starts from *, Include folders off; a*|*.dat... marks the matches.
    mask(
        &mut app,
        "=",
        true,
        "*",
        false,
        "a*;*.dat",
        false,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl+= a*;*.dat",
        &["a1.txt", "a2.txt", "c.dat", "d.dat"],
        &mut failures,
    );
    // Ctrl++ selects too, from the remembered mask; with Include folders.
    mask(
        &mut app,
        "+",
        true,
        "a*;*.dat",
        false,
        "f1",
        true,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl++ f1 with folders",
        &["a1.txt", "a2.txt", "c.dat", "d.dat", "f1"],
        &mut failures,
    );
    // Ctrl+− deselects, remembering the last mask and its Include folders.
    mask(
        &mut app,
        "-",
        false,
        "f1",
        true,
        "*.dat",
        false,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl+− *.dat",
        &["a1.txt", "a2.txt", "f1"],
        &mut failures,
    );
    // Invalid masks (two |, nothing after |) change nothing and say why.
    mask(
        &mut app,
        "=",
        true,
        "*.dat",
        false,
        "*.dat",
        false,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl+= *.dat again",
        &["a1.txt", "a2.txt", "c.dat", "d.dat", "f1"],
        &mut failures,
    );
    for invalid in ["a|b|c", "*.txt|"] {
        invalid_mask(&mut app, context, invalid, &mut failures)?;
    }
    support::dismiss_all(&mut app);
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    expect(&mut app, &folder, "Ctrl+D", &[], &mut failures);

    // Ctrl+Shift+click adds a range to the marks.
    click(&mut app, "a1.txt", &[Key::Control])?;
    click(&mut app, "b2.log", &[Key::Control])?;
    click(&mut app, "c.dat", &[Key::Control, Key::Shift])?;
    expect(
        &mut app,
        &folder,
        "Ctrl+click a1, b2, Ctrl+Shift+click c.dat",
        &["a1.txt", "b2.log", "c.dat"],
        &mut failures,
    );
    click(&mut app, "f1", &[])?;
    click(&mut app, "a2.txt", &[Key::Control, Key::Shift])?;
    expect(
        &mut app,
        &folder,
        "click f1, Ctrl+Shift+click a2.txt",
        &["a1.txt", "a2.txt", "b2.log", "c.dat", "f1", "f2"],
        &mut failures,
    );

    // Alt+Down and Alt+Up visit the marked rows in displayed order, without wrapping.
    let marked_order: Vec<String> = ps::shown(&app, LEFT)
        .into_iter()
        .filter(|name| ps::marked(&app, LEFT).contains(name))
        .collect();
    panel::select_row(&mut app, LEFT, "..")?;
    let mut visited = Vec::new();
    for _ in 0..marked_order.len() + 1 {
        input::chord(app.slint(), &[Key::Alt], Key::DownArrow);
        app.settle();
        visited.push(ps::cursor(&app, LEFT).unwrap_or_default());
    }
    let mut wanted = marked_order.clone();
    wanted.push(marked_order.last().cloned().unwrap_or_default());
    if visited != wanted {
        failures.push(format!(
            "Alt+Down from .. visited {visited:?}, expected {wanted:?}"
        ));
    }
    input::chord(app.slint(), &[Key::Alt], Key::UpArrow);
    app.settle();
    let previous = marked_order
        .get(marked_order.len().saturating_sub(2))
        .cloned();
    if ps::cursor(&app, LEFT) != previous {
        failures.push(format!(
            "Alt+Up from the last marked row went to {:?}, expected {previous:?}",
            ps::cursor(&app, LEFT)
        ));
    }
    let marks = ps::marked(&app, LEFT);
    if marks.len() != 6 {
        failures.push(format!("Alt+Up/Down changed the marks to {marks:?}"));
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // Insert and Space stop at the last row; `..` is never marked.
    panel::focus_list(&mut app, LEFT)?;
    input::key(app.slint(), Key::End);
    app.settle();
    let last = ps::cursor(&app, LEFT).unwrap_or_default();
    input::key(app.slint(), Key::Insert);
    app.settle();
    input::key(app.slint(), Key::Space);
    app.settle();
    if ps::cursor(&app, LEFT).as_deref() != Some(last.as_str()) {
        failures.push(format!(
            "Insert and Space at the last row moved the cursor to {:?}",
            ps::cursor(&app, LEFT)
        ));
    }
    expect(
        &mut app,
        &folder,
        "Insert then Space on the last row",
        &[],
        &mut failures,
    );
    input::key(app.slint(), Key::Home);
    app.settle();
    input::key(app.slint(), Key::Insert);
    app.settle();
    input::chord(app.slint(), &[Key::Control], "a");
    app.settle();
    input::chord(app.slint(), &[Key::Control], "i");
    app.settle();
    if app
        .rows(LEFT)
        .iter()
        .any(|row| row.parent_entry && row.selected)
    {
        failures.push("`..` was marked by Insert, Ctrl+A or Ctrl+I".into());
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // The rubber band from the gutter beside the rows: plain replaces, Shift adds,
    // Ctrl toggles.
    rubber(&mut app, context, &folder, &mut failures)?;
    // The last mask and its Include folders are saved: after a restart Ctrl+= offers
    // them again.
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "=");
    if !ps::wait(&mut app, Duration::from_secs(5), mask_open) {
        failures.push("Ctrl+= opened no mask dialog before the restart".into());
    } else {
        input::fill(app.slint(), "Selection mask", "b*")?;
        if ps::checked(&app, "Include folders") != Some(true) {
            input::click(
                app.slint(),
                i_slint_core::items::AccessibleRole::Checkbox,
                "Include folders",
            )?;
            app.settle();
        }
        input::click_button(app.slint(), "OK")?;
        ps::wait(&mut app, Duration::from_secs(5), |app| !mask_open(app));
    }
    app.shutdown()?;
    drop(app);
    let mut config = Config::new(&folder, &context.work);
    config.settings = Some(store);
    let mut app = App::start(config);
    mask(&mut app, "=", true, "b*", true, "b*", true, &mut failures)?;
    expect(
        &mut app,
        &folder,
        "Ctrl+= b* with folders after a restart",
        &["b1.log", "b2.log"],
        &mut failures,
    );
    app.shutdown()?;
    drop(app);
    selection_colours(context, &mut failures)?;
    support::report(&failures)
}

fn rubber(
    app: &mut App,
    context: &mut Context,
    folder: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let gutter = app.window.get_left_list_left() + app.window.get_left_list_grid_width() + 4.0;
    let row = |app: &App, name: &str| panel::row_center(app, LEFT, name);
    let (Some(a2), Some(b2), Some(c), Some(d)) = (
        row(app, "a2.txt"),
        row(app, "b2.log"),
        row(app, "c.dat"),
        row(app, "d.dat"),
    ) else {
        failures.push("the rubber band's rows are not shown".into());
        return Ok(());
    };
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    // Plain: a2 through b2.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, a2.y),
        LogicalPosition::new(a2.x, b2.y),
        &[],
    );
    expect(
        app,
        folder,
        "rubber band a2–b2",
        &["a2.txt", "b1.log", "b2.log"],
        failures,
    );
    app.screenshot(context, "rubber-band", "Rubber band from the gutter");
    // Shift adds d.dat.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, d.y),
        LogicalPosition::new(d.x, d.y + 2.0),
        &[Key::Shift],
    );
    expect(
        app,
        folder,
        "Shift rubber band on d.dat",
        &["a2.txt", "b1.log", "b2.log", "d.dat"],
        failures,
    );
    // Ctrl toggles b2 through c.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, b2.y),
        LogicalPosition::new(b2.x, c.y),
        &[Key::Control],
    );
    expect(
        app,
        folder,
        "Ctrl rubber band on b2–c",
        &["a2.txt", "b1.log", "c.dat", "d.dat"],
        failures,
    );
    // Plain again replaces everything with c.dat.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, c.y),
        LogicalPosition::new(c.x, c.y + 2.0),
        &[],
    );
    expect(
        app,
        folder,
        "plain rubber band on c.dat",
        &["c.dat"],
        failures,
    );
    Ok(())
}

/// Ctrl+= with an invalid mask: the marks stay and the reason is shown (in the dialog,
/// the panel's status, the status line, the details strip or a notice).
fn invalid_mask(
    app: &mut App,
    context: &mut Context,
    text: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let before = ps::marked(app, LEFT);
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "=");
    if !ps::wait(app, Duration::from_secs(5), mask_open) {
        failures.push(format!("Ctrl+= for {text:?} opened no dialog"));
        return Ok(());
    }
    input::fill(app.slint(), "Selection mask", text)?;
    input::key(app.slint(), Key::Return);
    app.settle();
    let reason = |app: &App| {
        let mut places = vec![
            app.window
                .global::<PanelChoices>()
                .get_mask_error()
                .to_string(),
            app.window.get_left_status().to_string(),
            app.window.get_desktop_status().to_string(),
        ];
        places.extend(ps::details(app).into_iter().map(|(_, value)| value));
        places.extend(support::shown_notice(app).map(|(title, body)| format!("{title} {body}")));
        places
            .into_iter()
            .find(|text| text.to_lowercase().contains("mask") || text.contains('|'))
    };
    ps::wait(app, Duration::from_secs(2), |app| reason(app).is_some());
    if ps::marked(app, LEFT) != before {
        failures.push(format!(
            "the invalid mask {text:?} changed the marks to {:?}",
            ps::marked(app, LEFT)
        ));
    }
    let open = mask_open(app);
    match reason(app) {
        None => failures.push(format!(
            "the invalid mask {text:?} gave no reason anywhere: dialog open {open}, panel status {:?}, status line {:?}",
            app.window.get_left_status(),
            app.window.get_desktop_status()
        )),
        Some(reason) => context.note(format!("invalid mask {text:?}: {reason:?} (dialog open {open})")),
    }
    support::dismiss_all(app);
    Ok(())
}

/// Selected text's least contrast against the selection fill in a theme (aac18026):
/// 12:1 in the High Contrast themes, 7:1 in the Contrast palettes, otherwise 4.5:1.
fn selected_minimum(theme: PreferenceTheme) -> f64 {
    match theme {
        PreferenceTheme::HighContrastDark | PreferenceTheme::HighContrastLight => 12.0,
        PreferenceTheme::PaperContrast
        | PreferenceTheme::SandstoneContrast
        | PreferenceTheme::GraphiteContrast => 7.0,
        _ => 4.5,
    }
}

/// What a row's name looks like in a rendered frame: the fill most of its name area
/// shows and the colour of its glyph cores (of the colours covering at least three
/// pixels, the one of most contrast against the fill), with that contrast.
#[derive(Debug)]
struct Ink {
    fill: [u8; 3],
    text: [u8; 3],
    ratio: f64,
}

/// The name text item of the left panel's row `name`, as laid out now.
fn name_text(app: &App, name: &str) -> Option<inspect::Rect> {
    let list = panel::geometry(app, LEFT);
    let row = inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .filter(|(label, _)| panel::is_row_label(label, name))
        .map(|(_, item)| item)
        .find(|item| {
            let center = inspect::center(item);
            center.x >= list.left && center.x < list.left + list.width
        })?;
    let mut found = None;
    inspect::walk(&row, &mut |item| {
        if found.is_none()
            && crate::tools_support::text_of(item).is_some()
            && inspect::label(item).as_deref() == Some(name)
        {
            found = inspect::visible_rect(item);
        }
    });
    found
}

/// Measure the row `name`'s name area in `frame` ([`Ink`]).
fn ink(app: &App, frame: &Frame, name: &str) -> Result<Ink, String> {
    let (x, y, w, h) =
        name_text(app, name).ok_or_else(|| format!("no shown name text of {name}"))?;
    let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    let (left, top) = (x.max(0.0) as usize, y.max(0.0) as usize);
    let right = ((x + w) as usize).min(frame.width);
    let bottom = ((y + h) as usize).min(frame.height);
    for row in top..bottom {
        if right <= left {
            break;
        }
        for pixel in &frame.pixels[row * frame.width + left..row * frame.width + right] {
            *counts.entry([pixel.r, pixel.g, pixel.b]).or_default() += 1;
        }
    }
    let fill = counts
        .iter()
        .max_by_key(|(_, count)| **count)
        .map(|(colour, _)| *colour)
        .ok_or_else(|| format!("{name}'s name area is empty"))?;
    let (text, ratio) = counts
        .iter()
        .filter(|(_, count)| **count >= 3)
        .map(|(colour, _)| (*colour, ui_gate::contrast(*colour, fill)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .ok_or_else(|| format!("{name}'s name shows no glyphs"))?;
    Ok(Ink { fill, text, ratio })
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Selection colours in panel rows in every theme (aac18026), measured in the rendered
/// frame: a marked file's name keeps the theme's selected-text contrast against the
/// selection fill (12:1 High Contrast, 7:1 Contrast palettes, otherwise 4.5:1), a marked
/// file in a filename colour rule keeps 4.5:1 (the shared contrast policy), and the
/// cursor row 4.5:1. After Tab the panel is inactive: its cursor row takes the inactive
/// cursor fill, still at 4.5:1, while the marked rows keep their selection.
fn selection_colours(context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let folder = support::scratch(context, "selection-colours")?;
    // `.zzz` is in no filename colour rule; `.txt` is (#3B82F6, Plain text).
    for name in ["alpha.zzz", "beta.zzz", "gamma.zzz", "notes.txt"] {
        ps::write(&folder.join(name), name)?;
    }
    let mut measured = Vec::new();
    for theme in PreferenceTheme::ALL {
        let mut config = Config::new(&folder, &context.work);
        config.theme = theme;
        let mut app = App::start(config);
        for name in ["alpha.zzz", "notes.txt"] {
            panel::select_row(&mut app, LEFT, name)?;
            input::key(app.slint(), Key::Insert);
        }
        panel::select_row(&mut app, LEFT, "gamma.zzz")?;
        if ps::marked(&app, LEFT) != ["alpha.zzz", "notes.txt"] {
            failures.push(format!(
                "{theme:?}: Insert marked {:?}",
                ps::marked(&app, LEFT)
            ));
        }
        let minimum = selected_minimum(theme);
        let mut cursor_fills = Vec::new();
        for (state, tab) in [("active", false), ("inactive", true)] {
            if tab {
                input::key(app.slint(), Key::Tab);
                app.settle();
                if app.window.get_focused_pane() != 1 {
                    failures.push(format!("{theme:?}: Tab did not activate the right panel"));
                }
            }
            let frame = Frame::of(&mut app);
            for (name, needed, what) in [
                ("alpha.zzz", minimum, "a marked file"),
                ("notes.txt", 4.5, "a marked file with a colour rule"),
                ("gamma.zzz", 4.5, "the cursor row"),
            ] {
                match ink(&app, &frame, name) {
                    Err(error) => failures.push(format!("{theme:?}, {state} panel: {error}")),
                    Ok(ink) => {
                        if name == "gamma.zzz" {
                            cursor_fills.push(ink.fill);
                        }
                        measured.push(format!(
                            "{theme:?} {state} {name}: {} on {} {:.2}:1",
                            hex(ink.text),
                            hex(ink.fill),
                            ink.ratio
                        ));
                        if ink.ratio + 0.005 < needed {
                            failures.push(format!(
                                "{theme:?}, {state} panel, {what} ({name}): {} on {} is \
                                 {:.2}:1, needs {needed}:1",
                                hex(ink.text),
                                hex(ink.fill),
                                ink.ratio
                            ));
                        }
                    }
                }
            }
            // A picture of a few representative themes (each frame was measured).
            let pictured = matches!(
                theme,
                PreferenceTheme::Blue
                    | PreferenceTheme::Light
                    | PreferenceTheme::HighContrastDark
                    | PreferenceTheme::PaperContrast
            );
            if !tab && pictured {
                app.screenshot(
                    context,
                    &format!("selection-{theme:?}").to_lowercase(),
                    &format!("{theme:?}: two marked rows and the cursor row"),
                );
            }
        }
        if let [active, inactive] = cursor_fills[..]
            && active == inactive
        {
            failures.push(format!(
                "{theme:?}: the cursor row keeps its fill {} in the inactive panel",
                hex(active)
            ));
        }
        app.shutdown()?;
    }
    context.note(format!(
        "selection colours measured in the rendered rows: {}",
        measured.join("; ")
    ));
    Ok(())
}
