//! Journey: customise, use at once, restart. Staged with the installed plugins so the
//! viewer is part of it.
//!
//! 1. In one Options session: the Light theme, a colour rule for `*.log` moved to the
//!    top, and Copy (`files.copy-to`) rebound from F5 to Ctrl+Alt+Y; OK saves once.
//! 2. At once: the panels are light; `report.log` is red; F5 no longer opens Copy,
//!    Ctrl+Alt+Y does, and the dialog is light; Enter copies, and the copy is red in
//!    the other panel too; F3 opens a light viewer.
//! 3. After a restart on the saved settings: the same theme, colours, keys and viewer;
//!    Options lists its pages in their order of use; a Markdown text, a plain text and a
//!    Word document share the palette's document colour and a source file has the source
//!    colour, each the one it had before the restart.
use super::journeys_support::{self as journey, Journey, wait};
use crate::{
    Context,
    app::{App, Config},
    files_support as files, input, inspect,
    options_support::{self as options, Frame, SettingsStore},
    panel, viewers_stage,
    viewers_support::{self as support, probe},
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::{CopyOperationState, PaneId, PreferenceTheme};
use slint::{SharedString, platform::Key};
use std::{path::PathBuf, sync::Arc, time::Duration};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const STEP: Duration = Duration::from_secs(30);
/// How long a key that should do nothing gets to open a dialog all the same: its handler
/// runs at once, and an Engine round trip after it takes far less.
const INERT: Duration = Duration::from_millis(800);
const OPEN_LIMIT: Duration = Duration::from_secs(60);
pub const TIMEOUT: Duration = Duration::from_secs(12 * 60);
const RED: (u8, u8, u8) = (0xFF, 0x20, 0x50);
const ACTION: &str = "files.copy-to";
const LABEL: &str = "Copy selected files to folder";
const NEW_KEY: &str = "Ctrl+Alt+Y";
const ALIASES: &str = "Shortcuts for selected command";
const CAPTURE: &str = "Press shortcut keys";
const NOTES: &str = "Notes for the viewer, in the saved theme.";
/// A source file: the default palette's source colour (`default_filename_colors`).
const SOURCE: &str = "main.rs";
/// A Markdown text, a plain text and a Word document: the default palette gives them one
/// colour of their own, not the source colour (f153478c).
const DOCUMENTS: [&str; 3] = ["readme.md", "notes.txt", "plan.docx"];

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_options_restart",
        isolation: crate::Isolation::ParallelPlugins,
        area: "journeys",
        summary: "One Options session sets the Light theme, a *.log colour rule and Copy on Ctrl+Alt+Y; at once and after a restart: light panels and dialogs, red rows in both panels, F5 inert, Ctrl+Alt+Y copies, F3 opens a light viewer (installed plugins); a Markdown text, a plain text and a Word document share the palette's document colour and a source file has another, both kept after the restart, and Options lists its pages in their order of use.",
        run: options_restart,
        timeout: TIMEOUT,
    }
}

pub fn options_restart(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

struct Setup {
    folder: PathBuf,
    target: PathBuf,
    plugins: PathBuf,
    store: Arc<SettingsStore>,
}

impl Setup {
    fn config(&self) -> Config {
        let mut config = Config::new(&self.folder, &self.target);
        config.settings = Some(self.store.clone());
        config.plugins = Some(self.plugins.clone());
        config
    }
}

/// What the default (dark) appearance looks like, for comparison, and the palette's
/// colours with the saved choices before the restart.
struct Baseline {
    panel: f32,
    dialog: f32,
    palette: Option<Palette>,
}

/// The colours the default palette gives [`SOURCE`] and [`DOCUMENTS`] in a panel.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Palette {
    source: (u8, u8, u8),
    documents: (u8, u8, u8),
}

fn run(context: &mut Context) -> Result<(), String> {
    let folder = files::scratch(context, "cust")?;
    let target = files::scratch(context, "cust-out")?;
    journey::write(&folder.join("report.log"), "log line\n")?;
    journey::write(&folder.join("notes.txt"), format!("{NOTES}\n"))?;
    journey::write(&folder.join("data.bin"), [0_u8, 1, 2, 3])?;
    journey::write(&folder.join("main.rs"), "fn main() {}\n")?;
    journey::write(&folder.join("readme.md"), "# Readme\n")?;
    journey::write(&folder.join("plan.docx"), "")?;
    let setup = Setup {
        folder,
        target,
        plugins: context.work.join("plugins"),
        store: SettingsStore::seeded(|prefs| prefs.light_theme = PreferenceTheme::Letterhead),
    };
    let mut journey = Journey::default();
    let mut app = App::start(setup.config());
    let ended = before_restart(&mut app, context, &mut journey, &setup);
    files::dismiss_all(&mut app);
    let shutdown = app.shutdown();
    let ended = ended.and_then(|baseline| shutdown.map(|()| baseline));
    drop(app);
    let ended = ended.and_then(|baseline| {
        let mut config = setup.config();
        config.theme = PreferenceTheme::DEFAULT_LIGHT;
        let mut app = App::start(config);
        let after = after_restart(&mut app, context, &mut journey, &setup, &baseline);
        files::dismiss_all(&mut app);
        after.and(app.shutdown())
    });
    journey.finish(context, ended)
}

/// The palette's colours of [`SOURCE`] and [`DOCUMENTS`] in `pane`: each a colour of
/// its own, not the theme's text colour that `data.bin` keeps; the documents all in one,
/// the source in another.
fn palette(app: &App, pane: PaneId) -> Result<Palette, String> {
    let rows = app.rows(pane);
    let shown = |name: &str| {
        rows.iter().find(|row| row.name == name).map(|row| {
            (
                row.custom_foreground,
                (
                    row.foreground.red(),
                    row.foreground.green(),
                    row.foreground.blue(),
                ),
            )
        })
    };
    let plain = shown("data.bin").ok_or("data.bin is not listed")?;
    let source = shown(SOURCE);
    let documents: Vec<_> = DOCUMENTS.iter().map(|name| (*name, shown(name))).collect();
    match (source, documents[0].1) {
        (Some(source), Some(first))
            if source.0
                && first.0
                && source.1 != plain.1
                && first.1 != plain.1
                && source.1 != first.1
                && documents.iter().all(|(_, colour)| *colour == Some(first)) =>
        {
            Ok(Palette {
                source: source.1,
                documents: first.1,
            })
        }
        _ => Err(format!(
            "{SOURCE} {source:?}; documents {documents:?}; data.bin {plain:?}"
        )),
    }
}

/// Open Options: its page buttons from the top in the order of [`options::PAGES`]
/// (order of use); Cancel closes it again.
fn page_order(app: &mut App) -> Result<(), String> {
    options::open(app)?;
    let mut shown: Vec<(f32, String)> = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(label, _)| {
            options::PAGES
                .iter()
                .any(|(page, _)| label.as_str() == *page)
        })
        .map(|(label, item)| (inspect::rect(&item).1, label))
        .collect();
    shown.sort_by(|a, b| a.0.total_cmp(&b.0));
    let order: Vec<&str> = shown.iter().map(|(_, label)| label.as_str()).collect();
    let wanted: Vec<&str> = options::PAGES.iter().map(|(page, _)| *page).collect();
    let result = if order == wanted {
        Ok(())
    } else {
        Err(format!("the pages from the top: {order:?}"))
    };
    options::cancel(app)?;
    result
}

/// The red foreground of `name` in `pane`, if the rule coloured it.
fn red(app: &App, pane: PaneId, name: &str) -> bool {
    app.rows(pane).iter().any(|row| {
        row.name == name
            && row.custom_foreground
            && (
                row.foreground.red(),
                row.foreground.green(),
                row.foreground.blue(),
            ) == RED
    })
}

/// Press a key in the focused left list; whether the Copy dialog opened (closed again
/// with Escape unless `keep`), and the mean luma of the dialog's area. The dialog opens
/// in the key's own handler: when it is `expected` the wait is a hang guard, otherwise
/// [`INERT`] after the key decides that the key did nothing.
fn opens_copy(
    app: &mut App,
    modifiers: &[Key],
    key: impl Into<SharedString>,
    keep: bool,
    expected: bool,
) -> Result<Option<f32>, String> {
    panel::select_row(app, LEFT, "report.log")?;
    input::chord(app.slint(), modifiers, key);
    let limit = if expected { STEP } else { INERT };
    let opened = wait(app, "the Copy dialog", limit, |app| {
        !app.window.get_copy_draft_token().is_empty()
    })
    .is_ok();
    if !opened {
        return Ok(None);
    }
    let area = dialog_area(app).ok_or("the Copy dialog shows no destination and Cancel")?;
    let luma = Frame::of(app).luma(area);
    if !keep {
        input::key(app.slint(), Key::Escape);
        wait(app, "the Copy dialog to close", STEP, |app| {
            app.window.get_copy_draft_token().is_empty()
        })?;
    }
    Ok(Some(luma))
}

/// The area between the Copy dialog's destination field and its Cancel button.
fn dialog_area(app: &App) -> Option<inspect::Rect> {
    let field = inspect::find(app.slint(), AccessibleRole::TextInput, "Destination folder")?;
    let cancel = inspect::find(app.slint(), AccessibleRole::Button, "Cancel")?;
    let (a, b) = (inspect::rect(&field), inspect::rect(&cancel));
    let left = a.0.min(b.0);
    let top = a.1.min(b.1);
    let right = (a.0 + a.2).max(b.0 + b.2);
    let bottom = (a.1 + a.3).max(b.1 + b.3);
    Some((left, top, right - left, bottom - top))
}

/// F3 on notes.txt: the viewer's dominant background luminance (0–1).
fn viewer_light(app: &mut App) -> Result<f64, String> {
    panel::select_row(app, LEFT, "notes.txt")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let shown = viewing.wait(app, "the notes in the viewer", STEP, |window| {
        window.get_accessible_text().contains(NOTES)
    });
    let canvas = probe::still_canvas(app, &viewing, STEP);
    viewing.close(app)?;
    shown?;
    Ok(probe::luminance(probe::dominant(&canvas?)))
}

/// The text directly below "Shortcut currently used by".
fn used_by(app: &App) -> String {
    let Some(caption) = options::find_any(app, AccessibleRole::Text, "Shortcut currently used by")
    else {
        return String::new();
    };
    let (x, y, _, height) = inspect::rect(&caption);
    inspect::controls(app.slint(), AccessibleRole::Text)
        .into_iter()
        .map(|(text, item)| (inspect::rect(&item), text))
        .filter(|((left, top, _, _), _)| *top >= y + height - 1.0 && (left - x).abs() < 4.0)
        .min_by(|a, b| a.0.1.total_cmp(&b.0.1))
        .map(|(_, text)| text)
        .unwrap_or_default()
}

/// Options › Colors: a `*.log` rule in red, moved to the top and applied.
fn colour_rule(app: &mut App) -> Result<(), String> {
    options::page(app, "Colors")?;
    options::press(app, "Add rule")?;
    options::fill(app, "Color rule expression", "*.log")?;
    options::fill(app, "Foreground hex color", "#FF2050")?;
    for _ in 0..64 {
        if !options::is_enabled(app, AccessibleRole::Button, "Move up") {
            break;
        }
        options::press(app, "Move up")?;
    }
    options::press(app, "Apply colors")?;
    wait(app, "the colour rules to apply", STEP, |app| {
        options::texts_with(app, "Checking…").is_empty()
            && options::is_enabled(app, AccessibleRole::Button, "Apply colors")
    })
}

/// Options › Keyboard: Copy loses F5 and gets Ctrl+Alt+Y.
fn rebind(app: &mut App) -> Result<(), String> {
    options::page(app, "Keyboard")?;
    options::fill(app, "Search commands", LABEL)?;
    let entry = options::reveal(app, AccessibleRole::ListItem, LABEL)?;
    let id = entry
        .accessible_string_property(AccessibleStringProperty::Value)
        .map(|value| value.to_string());
    if id.as_deref() != Some(ACTION) {
        return Err(format!("{LABEL:?} is the action {id:?}, not {ACTION}"));
    }
    options::click_item(app, &entry)?;
    let aliases = options::value(app, AccessibleRole::Combobox, ALIASES);
    if aliases.as_deref() != Some("F5") {
        return Err(format!("{LABEL} shows the shortcuts {aliases:?}, not F5"));
    }
    options::press(app, "Remove")?;
    let field = options::reveal(app, AccessibleRole::TextInput, CAPTURE)?;
    options::click_item(app, &field)?;
    input::chord(app.slint(), &[Key::Control, Key::Alt], "y");
    // A wrong capture is reported below, with what was captured.
    let _ = wait(app, "Ctrl+Alt+Y to be captured", STEP, |app| {
        options::value(app, AccessibleRole::TextInput, CAPTURE).as_deref() == Some(NEW_KEY)
    });
    let captured = options::value(app, AccessibleRole::TextInput, CAPTURE);
    if captured.as_deref() != Some(NEW_KEY) || used_by(app) != "Unassigned" {
        return Err(format!(
            "Ctrl+Alt+Y captured {captured:?}, used by {:?}",
            used_by(app)
        ));
    }
    options::press(app, "Assign")?;
    let aliases = options::value(app, AccessibleRole::Combobox, ALIASES);
    if aliases.as_deref() != Some(NEW_KEY) {
        return Err(format!("after Assign {LABEL} has {aliases:?}"));
    }
    Ok(())
}

fn before_restart(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    setup: &Setup,
) -> Result<Baseline, String> {
    // The default appearance.
    let panel = Frame::of(app).luma(options::list_area(app, LEFT));
    let dialog =
        opens_copy(app, &[], Key::F5, false, true)?.ok_or("F5 does not open Copy by default")?;
    let mut baseline = Baseline {
        panel,
        dialog,
        palette: None,
    };
    journey.check(
        "report.log is not red before the rule",
        if red(app, LEFT, "report.log") {
            Err("it already is".into())
        } else {
            Ok(())
        },
    );
    journey.step(format!(
        "defaults: panel luma {panel:.0}, Copy dialog luma {dialog:.0}"
    ));

    // 1. One Options session.
    options::open(app)?;
    options::page(app, "Appearance")?;
    options::choose(app, "Light theme", "Light")?;
    app.set_host_dark(false);
    colour_rule(app)?;
    rebind(app)?;
    options::review_shot(
        app,
        context,
        "options",
        "Options › Keyboard: Copy on Ctrl+Alt+Y",
    );
    if !options::ok(app, &setup.store)? {
        return Err("OK did not save the settings".into());
    }
    journey.step("Options: Light theme, *.log red rule on top, Copy F5 → Ctrl+Alt+Y; OK saved");

    // 2. At once.
    check_effects(app, context, journey, &baseline, "at once")?;
    let colours = palette(app, LEFT);
    baseline.palette = colours.as_ref().ok().copied();
    journey.check(
        "at once: a Markdown text, a plain text and a Word document share one colour, and a \
         source file has another",
        colours.map(|_| ()),
    );
    let start = app.copies.len();
    match opens_copy(app, &[Key::Control, Key::Alt], "y", true, true)? {
        None => journey.check("Ctrl+Alt+Y opens Copy", Err("no dialog".into())),
        Some(_) => {
            input::key(app.slint(), Key::Return);
            let copied = wait(app, "the copy to end", STEP, move |app| {
                files::job(app, start).is_some_and(|job| job.state.is_terminal())
            })
            .and_then(|()| match files::job(app, start) {
                Some(job) if job.state == CopyOperationState::Completed => Ok(()),
                other => Err(format!("{:?}", other.map(|job| job.state))),
            })
            .and_then(|()| journey::holds(&setup.target.join("report.log"), "log line\n"));
            journey.check("Ctrl+Alt+Y, Enter copies report.log", copied)
        }
    };
    journey.check(
        "the copy is red in the other panel too",
        wait(app, "report.log to be red on the right", STEP, |app| {
            red(app, RIGHT, "report.log")
        }),
    );
    app.screenshot(context, "at-once", "The saved choices at once");
    journey.step("at once: light panel and dialog, red rows, Ctrl+Alt+Y copied, light viewer");
    Ok(baseline)
}

/// The theme, colours, keys and viewer as the saved settings say.
fn check_effects(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    baseline: &Baseline,
    when: &str,
) -> Result<(), String> {
    let light = Frame::of(app).luma(options::list_area(app, LEFT));
    journey.check(
        &format!("{when}: the panels are light"),
        if light > baseline.panel + 80.0 {
            Ok(())
        } else {
            Err(format!(
                "panel luma {light:.0} (default {:.0})",
                baseline.panel
            ))
        },
    );
    journey.check(
        &format!("{when}: report.log is red"),
        wait(app, "report.log to be red", STEP, |app| {
            red(app, LEFT, "report.log")
        }),
    );
    journey.check(
        &format!("{when}: notes.txt keeps its colour"),
        if red(app, LEFT, "notes.txt") {
            Err("the *.log rule coloured notes.txt".into())
        } else {
            Ok(())
        },
    );
    journey.check(
        &format!("{when}: F5 no longer opens Copy"),
        match opens_copy(app, &[], Key::F5, false, false)? {
            None => Ok(()),
            Some(_) => Err("it does".into()),
        },
    );
    journey.check(
        &format!("{when}: Ctrl+Alt+Y opens a light Copy dialog"),
        match opens_copy(app, &[Key::Control, Key::Alt], "y", false, true)? {
            None => Err("Ctrl+Alt+Y opened nothing".into()),
            Some(luma) if luma > baseline.dialog + 60.0 => Ok(()),
            Some(luma) => Err(format!(
                "dialog luma {luma:.0} (default {:.0})",
                baseline.dialog
            )),
        },
    );
    let viewer = viewer_light(app);
    journey.check(
        &format!("{when}: F3 opens a light viewer"),
        match viewer {
            Ok(luminance) if luminance > 0.6 => Ok(()),
            Ok(luminance) => Err(format!("viewer background luminance {luminance:.2}")),
            Err(error) => Err(error),
        },
    );
    app.screenshot(
        context,
        &format!("effects-{}", when.replace(' ', "-")),
        &format!("The saved choices {when}"),
    );
    Ok(())
}

fn after_restart(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    setup: &Setup,
    baseline: &Baseline,
) -> Result<(), String> {
    let saved = setup.store.current();
    journey.check(
        "the saved settings hold the three choices",
        if saved.light_theme == PreferenceTheme::Light
            && saved.dark_theme == PreferenceTheme::DEFAULT_DARK
            && saved
                .filename_colors
                .first()
                .is_some_and(|rule| rule.expression == "*.log")
            && saved
                .shortcuts
                .iter()
                .any(|binding| binding.action == ACTION && binding.keys == [NEW_KEY])
        {
            Ok(())
        } else {
            Err(format!(
                "theme {:?}, first rule {:?}, shortcuts {:?}",
                saved.light_theme,
                saved.filename_colors.first().map(|rule| &rule.expression),
                saved.shortcuts
            ))
        },
    );
    check_effects(app, context, journey, baseline, "after a restart")?;
    journey.check(
        "after a restart the copy is red in the other panel",
        wait(app, "report.log to be red on the right", STEP, |app| {
            red(app, RIGHT, "report.log")
        }),
    );
    journey.check(
        "after a restart the source and the documents keep the colours they had before it",
        palette(app, LEFT).and_then(|colours| match baseline.palette {
            Some(before) if before == colours => Ok(()),
            before => Err(format!("{colours:?} after, {before:?} before")),
        }),
    );
    journey.check(
        "after a restart Options lists its pages in their order of use",
        page_order(app),
    );
    app.screenshot(context, "restarted", "The saved choices after a restart");
    journey.step(
        "after a restart: the same theme, colours, keys and viewer; the palette's source \
         and document colours kept; the Options pages in their order",
    );
    Ok(())
}
