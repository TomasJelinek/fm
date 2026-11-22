//! Options › Colors: add a filename colour rule and see the matching names coloured in
//! the panel; rule priority (first match wins) by moving the rule up; Cancel discards
//! a removal; OK saves; a restart keeps the colours.
use crate::{
    Context,
    app::{App, Config},
    input,
    options_support::{self as options, Frame, SettingsStore},
    panel,
};
use midnight_core::{FilenameColorTarget, FilterMode, PaneId, RgbColor};
use slint::platform::Key;
use std::{fs, path::Path, sync::Arc, time::Duration};

const EXPRESSION: &str = "*.dat;notes.txt";
const RED: (u8, u8, u8) = (0xFF, 0x20, 0x50);
/// The default "text" rule's colour (`default_filename_colors`).
const TEXT_CYAN: (u8, u8, u8) = (6, 164, 199);
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
    // The default rules: text cyan, no colour for .dat and .bin.
    if foreground(&app, "notes.txt")? != Some(TEXT_CYAN)
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
    if foreground(&app, "notes.txt")? != Some(TEXT_CYAN) {
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
