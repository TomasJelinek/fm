//! Options › Colors: add a filename colour rule and see the matching names coloured in
//! the panel; rule priority (first match wins) by moving the rule up; Cancel discards
//! a removal; the colour picker (OK disabled while it is open, a swatch with Use color,
//! Escape closing only the picker) and every rule field (Regex, Case sensitive, the
//! Folders target, Enable rule, a background, Discard changes, Move down), each seen in
//! the panel and all discarded with Cancel; OK saves; a restart keeps the colours.
use crate::{
    Context,
    app::{App, Config},
    input,
    options_support::{self as options, Frame, SettingsStore},
    panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{FilenameColorTarget, FilterMode, PaneId, RgbColor};
use slint::platform::Key;
use std::{fs, path::Path, sync::Arc, time::Duration};

const EXPRESSION: &str = "*.dat;notes.txt";
const RED: (u8, u8, u8) = (0xFF, 0x20, 0x50);
/// The picker's red swatch, #ef4444.
const PICKED: (u8, u8, u8) = (0xEF, 0x44, 0x44);
/// A folder whose name matches the rule's masks.
const FOLDER: &str = "archive.dat";
/// The default "text" rule's colour (`default_filename_colors`).
const TEXT_BLUE: (u8, u8, u8) = (59, 130, 246);
const RULES: &str = "Filename color rules";

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

/// The custom foreground of `name` in the left panel (None: theme colour).
fn foreground(app: &App, name: &str) -> Result<Option<(u8, u8, u8)>, String> {
    let row = app
        .rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| format!("the panel shows no {name:?}"))?;
    Ok(row.custom_foreground.then(|| {
        (
            row.foreground.red(),
            row.foreground.green(),
            row.foreground.blue(),
        )
    }))
}

#[track_caller]
fn wait_color(app: &mut App, name: &'static str, wanted: Option<(u8, u8, u8)>) {
    app.until(&format!("{name} to be coloured {wanted:?}"), move |app| {
        foreground(app, name).ok() == Some(wanted)
    });
}

fn apply(app: &mut App) -> Result<(), String> {
    options::press(app, "Apply colors")?;
    for _ in 0..200 {
        if options::texts_with(app, "Checking…").is_empty()
            && options::is_enabled(
                app,
                i_slint_core::items::AccessibleRole::Button,
                "Apply colors",
            )
        {
            return Ok(());
        }
        app.idle(Duration::from_millis(20));
    }
    Err("the colour rules stay in validation".into())
}

fn rule_labels(app: &App) -> Vec<String> {
    options::list_rows(app, RULES)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

/// The custom background of `name` in the left panel (None: theme colour).
fn background(app: &App, name: &str) -> Result<Option<(u8, u8, u8)>, String> {
    let row = app
        .rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| format!("the panel shows no {name:?}"))?;
    Ok(row.custom_background.then(|| {
        (
            row.background.red(),
            row.background.green(),
            row.background.blue(),
        )
    }))
}

/// Wait (bounded) until `name` has the foreground `wanted`; an error says what it has.
fn expect_color(
    app: &mut App,
    name: &str,
    wanted: Option<(u8, u8, u8)>,
    what: &str,
) -> Result<(), String> {
    let owned = name.to_owned();
    if options::poll(app, Duration::from_secs(5), move |app| {
        foreground(app, &owned).ok() == Some(wanted)
    }) {
        Ok(())
    } else {
        Err(format!(
            "{what}: {name} is coloured {:?} instead of {wanted:?}",
            foreground(app, name)
        ))
    }
}

/// The colour picker popup of the selected rule is open.
fn picker_open(app: &App) -> bool {
    options::find_any(app, AccessibleRole::Button, "Use color").is_some()
}

/// The hex value a colour field shows, lower case.
fn hex_value(app: &App, label: &str) -> String {
    options::value(app, AccessibleRole::TextInput, label)
        .unwrap_or_default()
        .to_lowercase()
}

/// O2: the colour picker and every field of a rule, on the saved first rule
/// (`*.dat;notes.txt`, red): the picker keeps OK disabled while open, a swatch with Use
/// color sets its value, Escape closes only the picker; the syntax (Regex), Case
/// sensitive, the targets (Folders), Enable rule, the background, Discard changes and
/// Move down each change what the panel shows after Apply colors; Cancel discards all
/// of it. Starts and ends with Options closed on the saved rules.
fn rule_fields(app: &mut App, store: &SettingsStore, folder: &Path) -> Result<(), String> {
    let saved = store.current().filename_colors;
    let saves = store.saves();
    // A folder the rule's masks match, listed before Options opens.
    let matching = folder.join(FOLDER);
    fs::create_dir_all(&matching).map_err(|error| error.to_string())?;
    app.until("the panel to list the new folder", |app| {
        app.names(PaneId::Left).iter().any(|name| name == FOLDER)
    });
    options::open(app)?;
    options::page(app, "Colors")?;
    options::click_row(app, RULES, "first rule", &|label| label.starts_with("1. "))?;

    // The picker: OK waits for it; a swatch and Use color set the value.
    options::press(app, "Choose Foreground")?;
    if !options::poll(app, Duration::from_secs(3), picker_open) {
        return Err("Choose Foreground opened no colour picker".into());
    }
    if options::enabled_state(app, AccessibleRole::Button, "OK") != Some(false) {
        return Err(format!(
            "the Options OK button is {:?} while the colour picker is open",
            options::enabled_state(app, AccessibleRole::Button, "OK")
        ));
    }
    options::press(app, "RGB 239, 68, 68")?;
    options::press(app, "Use color")?;
    if !options::poll(app, Duration::from_secs(3), |app| !picker_open(app)) {
        return Err("Use color left the colour picker open".into());
    }
    if hex_value(app, "Foreground hex color") != "#ef4444" {
        return Err(format!(
            "the swatch and Use color set the foreground to {:?}",
            hex_value(app, "Foreground hex color")
        ));
    }
    apply(app)?;
    expect_color(app, "report.dat", Some(PICKED), "the picked colour")?;
    // Escape closes only the picker, which keeps the value.
    options::press(app, "Choose Foreground")?;
    if !options::poll(app, Duration::from_secs(3), picker_open) {
        return Err("Choose Foreground opened no colour picker the second time".into());
    }
    input::key(app.slint(), Key::Escape);
    if !options::poll(app, Duration::from_secs(3), |app| !picker_open(app))
        || !app.window.get_options_open()
    {
        return Err(format!(
            "Escape in the colour picker: picker open {}, Options open {}",
            picker_open(app),
            app.window.get_options_open()
        ));
    }
    if hex_value(app, "Foreground hex color") != "#ef4444" {
        return Err("Escape in the colour picker changed the foreground".into());
    }

    // Regex: only values.dat matches; notes.txt falls back to the text rule.
    options::choose(app, "Color rule syntax", "Regex")?;
    options::fill(app, "Color rule expression", r"^val.*\.dat$")?;
    apply(app)?;
    expect_color(app, "values.dat", Some(PICKED), "a Regex rule")?;
    expect_color(app, "report.dat", None, "a Regex rule")?;
    expect_color(app, "notes.txt", Some(TEXT_BLUE), "a Regex rule")?;

    // Case sensitive: "*.DAT" matches nothing; without it both .dat files.
    options::choose(app, "Color rule syntax", "Masks")?;
    options::fill(app, "Color rule expression", "*.DAT")?;
    options::set_checked(app, "Case sensitive", true)?;
    apply(app)?;
    expect_color(app, "report.dat", None, "a case-sensitive *.DAT")?;
    options::set_checked(app, "Case sensitive", false)?;
    apply(app)?;
    expect_color(app, "report.dat", Some(PICKED), "a case-insensitive *.DAT")?;

    // Folders: the rule colours the folder archive.dat and no file.
    options::choose(app, "Color rule targets", "Folders")?;
    apply(app)?;
    expect_color(app, FOLDER, Some(PICKED), "a Folders rule")?;
    expect_color(app, "report.dat", None, "a Folders rule")?;
    options::choose(app, "Color rule targets", "Files")?;
    apply(app)?;
    expect_color(app, "report.dat", Some(PICKED), "a Files rule")?;
    expect_color(app, FOLDER, None, "a Files rule")?;

    // Enable rule off: the rule colours nothing; on again.
    options::set_checked(app, "Enable rule", false)?;
    apply(app)?;
    expect_color(app, "report.dat", None, "a disabled rule")?;
    options::set_checked(app, "Enable rule", true)?;
    apply(app)?;
    expect_color(app, "report.dat", Some(PICKED), "an enabled rule")?;

    // A background colour.
    options::fill(app, "Background hex color", "#203040")?;
    apply(app)?;
    if !options::poll(app, Duration::from_secs(5), |app| {
        background(app, "report.dat").ok().flatten().is_some()
    }) {
        return Err(format!(
            "a background colour left report.dat with the background {:?}",
            background(app, "report.dat")
        ));
    }

    // Discard changes returns an unapplied edit to the applied rule.
    options::fill(app, "Color rule expression", "*.bin")?;
    options::press(app, "Discard changes")?;
    let expression = options::value(app, AccessibleRole::TextInput, "Color rule expression");
    if expression.as_deref() != Some("*.DAT") {
        return Err(format!(
            "Discard changes left the expression {expression:?} instead of \"*.DAT\""
        ));
    }

    // Move down: the rule leaves the top.
    let first = rule_labels(app).first().cloned().unwrap_or_default();
    options::press(app, "Move down")?;
    let labels = rule_labels(app);
    let moved = first.replacen("1. ", "2. ", 1);
    if labels.get(1) != Some(&moved) {
        return Err(format!(
            "Move down left the rules {labels:?}; expected {moved:?} second"
        ));
    }

    // Cancel discards every change: the saved rules and their colours come back.
    options::cancel(app)?;
    expect_color(app, "report.dat", Some(RED), "Cancel")?;
    if background(app, "report.dat")?.is_some() {
        return Err("Cancel kept the previewed background colour".into());
    }
    if store.saves() != saves || store.current().filename_colors != saved {
        return Err("Cancel kept edited colour rules".into());
    }
    fs::remove_dir(&matching).map_err(|error| error.to_string())?;
    app.until("the panel to drop the folder", |app| {
        !app.names(PaneId::Left).iter().any(|name| name == FOLDER)
    });
    Ok(())
}

pub fn colors(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("colors");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    for name in [
        "report.dat",
        "values.dat",
        "notes.txt",
        "script.rs",
        "plain.bin",
    ] {
        fs::write(folder.join(name), name).map_err(|error| error.to_string())?;
    }
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    // The default rules: text blue, no colour for .dat and .bin.
    if foreground(&app, "notes.txt")? != Some(TEXT_BLUE)
        || foreground(&app, "report.dat")?.is_some()
    {
        return Err(format!(
            "unexpected default colours: notes.txt {:?}, report.dat {:?}",
            foreground(&app, "notes.txt")?,
            foreground(&app, "report.dat")?
        ));
    }

    // Add a rule at the end: it colours .dat; notes.txt keeps the earlier text rule.
    options::open(&mut app)?;
    options::page(&mut app, "Colors")?;
    let before = store.current().filename_colors.len();
    options::press(&mut app, "Add rule")?;
    // The list reveals the new, selected rule: the default "*.*" at the end.
    let labels = rule_labels(&app);
    let added = format!("{}. *.*", before + 1);
    if !labels.contains(&added) {
        return Err(format!(
            "Add rule shows the rules {labels:?}, not {added:?}"
        ));
    }
    options::fill(&mut app, "Color rule expression", EXPRESSION)?;
    options::fill(&mut app, "Foreground hex color", "#FF2050")?;
    apply(&mut app)?;
    wait_color(&mut app, "report.dat", Some(RED));
    wait_color(&mut app, "values.dat", Some(RED));
    if foreground(&app, "notes.txt")? != Some(TEXT_BLUE) {
        return Err(format!(
            "a later rule overrode the earlier text rule: notes.txt is {:?}",
            foreground(&app, "notes.txt")?
        ));
    }

    // Move it to the top: it now wins for notes.txt too.
    for _ in 0..64 {
        if !options::is_enabled(&app, i_slint_core::items::AccessibleRole::Button, "Move up") {
            break;
        }
        options::press(&mut app, "Move up")?;
    }
    let labels = rule_labels(&app);
    let top = format!("1. {EXPRESSION}");
    if labels.first() != Some(&top) {
        return Err(format!("after Move up the rules are {labels:?}"));
    }
    apply(&mut app)?;
    wait_color(&mut app, "notes.txt", Some(RED));
    options::review_shot(
        &mut app,
        context,
        "colors-rule",
        "Options › Colors: a new rule moved to the top",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save the colour rules".into());
    }
    let saved = store.current().filename_colors;
    let first = saved.first().ok_or("no colour rules were saved")?;
    if first.expression != EXPRESSION
        || first.mode != FilterMode::Masks
        || first.target != FilenameColorTarget::Files
        || first.style.foreground != Some(RgbColor(RED.0, RED.1, RED.2))
        || !first.enabled
        || saved.len() != before + 1
    {
        return Err(format!(
            "the saved first rule is {first:?} of {} rules",
            saved.len()
        ));
    }
    // Visible: the names render in the rule's colour; others keep theirs.
    let frame = Frame::of(&mut app);
    let red_pixels = frame.count_near(options::list_area(&app, PaneId::Left), RED);
    if red_pixels < 20 {
        return Err(format!(
            "the panel shows no text in the rule's colour ({red_pixels} pixels)"
        ));
    }
    if foreground(&app, "plain.bin")?.is_some() || foreground(&app, "script.rs")? == Some(RED) {
        return Err("the rule coloured names it does not match".into());
    }
    app.screenshot(
        context,
        "colors-panel",
        "The panel with the new colour rule",
    );

    // Cancel discards a removed rule.
    options::open(&mut app)?;
    options::page(&mut app, "Colors")?;
    options::click_row(&mut app, RULES, "first rule", &|label| {
        label.starts_with("1. ")
    })?;
    options::press(&mut app, "Remove rule")?;
    apply(&mut app)?;
    wait_color(&mut app, "report.dat", None);
    let saves = store.saves();
    options::cancel(&mut app)?;
    wait_color(&mut app, "report.dat", Some(RED));
    if store.saves() != saves || store.current().filename_colors.first() != Some(first) {
        return Err("Cancel did not keep the saved colour rule".into());
    }

    // The colour picker and every field of a rule (O2), discarded with Cancel.
    rule_fields(&mut app, &store, &folder)?;
    options::review_shot(
        &mut app,
        context,
        "colors-fields-cancelled",
        "The panel after the rule-field edits were cancelled",
    );

    // Restart: the colours remain.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    for name in ["report.dat", "values.dat", "notes.txt"] {
        if foreground(&app, name)? != Some(RED) {
            return Err(format!(
                "after a restart {name} is coloured {:?}",
                foreground(&app, name)?
            ));
        }
    }
    // The colours follow their rows through sorting and a panel filter.
    panel::focus_list(&mut app, PaneId::Left)?;
    let sort = app.window.get_left_sort();
    input::chord(app.slint(), &[Key::Control], Key::F6);
    app.until("the size order", move |app| {
        app.window.get_left_sort() != sort
    });
    for name in ["report.dat", "values.dat", "notes.txt"] {
        if foreground(&app, name)? != Some(RED) {
            return Err(format!(
                "sorted by size, {name} is coloured {:?}",
                foreground(&app, name)?
            ));
        }
    }
    if foreground(&app, "plain.bin")?.is_some() {
        return Err("sorted by size, plain.bin took a colour".into());
    }
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter dialog", |app| {
        app.window.get_panel_filter_view().open
    });
    options::choose(&mut app, "Filter mode", "Masks")?;
    input::fill(app.slint(), "File filter expression", "*.dat")?;
    input::click_button(app.slint(), "OK")?;
    app.until("the *.dat filter", |app| {
        let mut names = app.names(PaneId::Left);
        names.retain(|name| name != "..");
        names.len() == 2 && names.iter().all(|name| name.ends_with(".dat"))
    });
    for name in ["report.dat", "values.dat"] {
        if foreground(&app, name)? != Some(RED) {
            return Err(format!(
                "filtered to *.dat, {name} is coloured {:?}",
                foreground(&app, name)?
            ));
        }
    }
    let red_pixels = Frame::of(&mut app).count_near(options::list_area(&app, PaneId::Left), RED);
    if red_pixels < 20 {
        return Err(format!(
            "filtered to *.dat, the panel shows no text in the rule's colour ({red_pixels} pixels)"
        ));
    }
    app.screenshot(
        context,
        "colors-sorted-filtered",
        "The coloured names sorted by size and filtered to *.dat",
    );
    context.note("Colors: an added *.dat;notes.txt rule coloured both .dat files at once, lost to the earlier text rule for notes.txt until moved to the top, was saved by OK (first of 13 rules, #FF2050), rendered red pixels in the panel, survived a cancelled removal and a restart, and stayed on their rows sorted by size and filtered to *.dat");
    app.shutdown()
}
