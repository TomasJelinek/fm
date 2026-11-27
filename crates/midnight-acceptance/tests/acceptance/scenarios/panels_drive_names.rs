//! The drive bar and the drive menu name each drive briefly and keep its whole path in
//! its tooltip and accessible texts (docs/drive-information.md, "Drive names"). For
//! every drive the backend lists, on the left panel:
//! - the drive bar button's caption is the drive's name: unique among the drives; on
//!   Linux never a mount path (`/` itself excepted); at most 16 columns unless only the
//!   whole path tells two drives apart;
//! - the button's accessible name holds the whole path, and hovering a drive's button
//!   shows a tooltip with its path;
//! - the drive menu's row starts with the same name, holds no Linux mount path, and its
//!   accessible description holds the whole path;
//! - choosing a drive other than the root in the menu opens its whole path.
//!
//! A run with the drive fixture, tmpfs mounts at the paths of [`FIXTURE`] (Linux, see
//! docs/developer/acceptance.md, "Drive names"), also checks their exact names: shared
//! folder names take a folder of their path, a long name keeps its start and end.
//! Without it the rules are checked on the drives the machine has. The bar, a drive's
//! tooltip and the open menu are pictured.
use crate::{
    Context, Scenario,
    app::{App, Config},
    files_support as support, input, inspect,
    scenarios::panels_support::{self as ps, LEFT},
    tools_support,
};
use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};
use midnight_core::{Volume, VolumeService};
use midnight_platform::LocalFileSystem;
use slint::{
    Model,
    platform::{Key, WindowAdapter},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// The drive fixture's mount paths and the names they get.
const FIXTURE: [(&str, &str); 5] = [
    ("/media/alice/USB", "alice/USB"),
    ("/mnt/USB", "mnt/USB"),
    ("/mnt/backup/data", "backup/data"),
    ("/srv/data", "srv/data"),
    ("/mnt/a-very-long-mount-folder-name", "a-very-l…er-name"),
];

/// Columns a name takes at most, unless only the whole path tells it apart.
const MAX_COLUMNS: usize = 16;

/// This module's entries in the registry (it owns them; `scenarios/mod.rs` lists the
/// module).
pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "panels_drive_names",
        isolation: crate::Isolation::Parallel,
        area: "panels",
        summary: "Every listed drive's drive bar caption is its short name (unique, at most 16 columns, on Linux never a mount path but `/`); its button's accessible name and hover tooltip give the whole path; its drive menu row starts with the name, shows no Linux mount path and describes the whole path; choosing a drive opens its path. With the Linux tmpfs drive fixture, the exact names (alice/USB, mnt/USB, backup/data, srv/data, a-very-l…er-name).",
        run: drive_names,
        timeout: Duration::from_secs(3 * 60),
    }]
}

/// One drive as the left drive bar shows it.
struct Drive {
    root: PathBuf,
    /// The root as the toolbar's texts spell it out.
    path: String,
    caption: String,
    tooltip: String,
}

/// The drives the harness's filesystem service lists (`App` browses through
/// `LocalFileSystem` without a Home folder), read independently of the window.
fn listed() -> Result<Vec<Volume>, String> {
    LocalFileSystem::new(None)
        .list_volumes()
        .map_err(|error| format!("the drive list could not be read: {error:?}"))
}

/// The left drive bar's drive for each of `volumes`, matched by its name.
fn drives(app: &App, volumes: &[Volume]) -> Result<Vec<Drive>, String> {
    let rows: Vec<_> = app
        .window
        .get_left_locations()
        .iter()
        .filter(|row| row.is_drive)
        .collect();
    if rows.len() != volumes.len() {
        return Err(format!(
            "the drive bar shows {} drives ({:?}), the backend lists {}",
            rows.len(),
            rows.iter()
                .map(|row| row.compact_label.to_string())
                .collect::<Vec<_>>(),
            volumes.len()
        ));
    }
    volumes
        .iter()
        .map(|volume| {
            let row = rows
                .iter()
                .find(|row| row.compact_label.as_str() == volume.label)
                .ok_or_else(|| {
                    format!(
                        "the drive bar has no button named {:?} for {}",
                        volume.label,
                        volume.root.display()
                    )
                })?;
            Ok(Drive {
                root: volume.root.clone(),
                path: volume.root.display().to_string(),
                caption: row.compact_label.to_string(),
                tooltip: row.tooltip.to_string(),
            })
        })
        .collect()
}

/// The left panel's drive bar button of `drive` (named by its tooltip, as both panels'
/// buttons are: the left one is left of the right panel), whole in view: when the drives
/// do not fit, the strip's arrows scroll it to the start and then towards its end.
fn button(app: &mut App, drive: &Drive) -> Option<ItemRc> {
    let edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    let whole = |app: &App| {
        inspect::find_all(app.slint(), AccessibleRole::Button, &drive.tooltip)
            .into_iter()
            .find(|item| {
                let (x, _, width, _) = inspect::rect(item);
                inspect::center(item).x < edge
                    && inspect::visible_rect(item).is_some_and(|(shown_x, _, shown_width, _)| {
                        (shown_x - x).abs() < 0.5 && (shown_width - width).abs() < 0.5
                    })
            })
    };
    if let Some(item) = whole(app) {
        return Some(item);
    }
    for arrow in [
        "Scroll left panel drives left",
        "Scroll left panel drives right",
    ] {
        for _ in 0..100 {
            let Some(item) = inspect::find(app.slint(), AccessibleRole::Button, arrow)
                .filter(tools_support::enabled)
            else {
                break;
            };
            let (x, y, width, height) = inspect::visible_rect(&item)?;
            input::click_at(
                app.slint(),
                slint::LogicalPosition::new(x + width / 2.0, y + height / 2.0),
            );
            app.settle();
            if let Some(item) = whole(app) {
                return Some(item);
            }
        }
    }
    None
}

/// Columns of `text`: two for East Asian wide characters, as the backend counts them.
fn columns(text: &str) -> usize {
    text.chars()
        .map(|character| match character {
            '\u{1100}'..='\u{115f}'
            | '\u{2e80}'..='\u{a4cf}'
            | '\u{ac00}'..='\u{d7a3}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ff00}'..='\u{ff60}'
            | '\u{1f300}'..='\u{1f64f}'
            | '\u{1f900}'..='\u{1f9ff}'
            | '\u{20000}'..='\u{3fffd}' => 2,
            '\u{0300}'..='\u{036f}' | '\u{200d}' | '\u{fe00}'..='\u{fe0f}' => 0,
            _ => 1,
        })
        .sum()
}

fn drive_names(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let volumes = listed()?;
    // Wide enough for the drive fixture's drives on one panel's bar.
    let mut config = Config::new(&root, &context.work);
    config.width = 2000;
    let mut app = App::start(config);
    let mut failures = Vec::new();
    // Discovery runs on a backend worker; the bar follows once it has published.
    if !ps::wait(&mut app, Duration::from_secs(20), |app| {
        drives(app, &volumes).is_ok()
    }) {
        return Err(drives(&app, &volumes).err().unwrap_or_default());
    }
    let shown = drives(&app, &volumes)?;
    context.note(format!(
        "drives: {:?}",
        shown
            .iter()
            .map(|drive| format!("{} = {}", drive.caption, drive.path))
            .collect::<Vec<_>>()
    ));
    app.screenshot(
        context,
        "drive-bar",
        "The drive bars: each drive by its name",
    );
    fixture_names(context, &shown, &mut failures);
    bar_rules(&mut app, &shown, &mut failures);
    hover(&mut app, context, &shown, &mut failures);
    menu(&mut app, context, &shown, &mut failures)?;
    app.shutdown()?;
    support::report(&failures)
}

/// Captions, accessible names and tooltips of the bar's buttons.
fn bar_rules(app: &mut App, shown: &[Drive], failures: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    for drive in shown {
        if !seen.insert(drive.caption.as_str()) {
            failures.push(format!("two drives show {:?}", drive.caption));
        }
        let whole = drive.path.trim_start_matches('/');
        if columns(&drive.caption) > MAX_COLUMNS && drive.caption != whole {
            failures.push(format!(
                "{:?} ({}) is longer than {MAX_COLUMNS} columns",
                drive.caption, drive.path
            ));
        }
        if cfg!(target_os = "linux") {
            let named_root = drive.root == Path::new("/");
            if named_root != (drive.caption == "/")
                || (!named_root && drive.caption.starts_with('/'))
            {
                failures.push(format!(
                    "the drive bar shows {} as {:?}, not by a name of its own",
                    drive.path, drive.caption
                ));
            }
        }
        if !drive.tooltip.contains(&drive.path) {
            failures.push(format!(
                "the tooltip of {:?} does not give its path {}: {:?}",
                drive.caption, drive.path, drive.tooltip
            ));
        }
        match button(app, drive) {
            None => failures.push(format!(
                "no left drive bar button whose accessible name is {:?}",
                drive.tooltip
            )),
            Some(item) => {
                let texts = tools_support::item_texts(&item);
                if !texts.contains(&drive.caption) {
                    failures.push(format!(
                        "the button of {} shows {texts:?}, not {:?}",
                        drive.path, drive.caption
                    ));
                }
            }
        }
    }
}

/// With the drive fixture mounted, its drives' exact names.
fn fixture_names(context: &mut Context, shown: &[Drive], failures: &mut Vec<String>) {
    let present: Vec<_> = FIXTURE
        .iter()
        .filter(|(path, _)| shown.iter().any(|drive| drive.root == Path::new(path)))
        .collect();
    if present.is_empty() {
        context.note(format!(
            "no drive fixture mounted: the rules were checked on the {} drives of this machine",
            shown.len()
        ));
        return;
    }
    if present.len() != FIXTURE.len() {
        failures.push(format!(
            "the drive fixture is incomplete: only {:?} are listed",
            present.iter().map(|(path, _)| path).collect::<Vec<_>>()
        ));
    }
    for (path, name) in present {
        let caption = shown
            .iter()
            .find(|drive| drive.root == Path::new(path))
            .map(|drive| drive.caption.as_str());
        if caption != Some(name) {
            failures.push(format!("{path} shows {caption:?}, expected {name:?}"));
        }
    }
}

/// Hovering the first drive other than the root shows its whole path.
fn hover(app: &mut App, context: &mut Context, shown: &[Drive], failures: &mut Vec<String>) {
    let mut target = None;
    for drive in shown
        .iter()
        .filter(|drive| drive.root != Path::new("/"))
        .chain(shown.iter())
    {
        if let Some(item) = button(app, drive) {
            target = Some((drive, item));
            break;
        }
    }
    let Some((drive, item)) = target else {
        failures.push("no drive bar button can be brought into view to hover".into());
        return;
    };
    let main = app.main.clone();
    let texts = tools_support::hover_tooltip(app, main.window(), &item).join("\n");
    if !texts.contains(&drive.path) {
        failures.push(format!(
            "hovering {:?} shows {texts:?}, not its path {}",
            drive.caption, drive.path
        ));
    }
    app.screenshot(
        context,
        "drive-tooltip",
        &format!(
            "Hovering {}: the tooltip gives its whole path",
            drive.caption
        ),
    );
    input::move_to(app.slint(), slint::LogicalPosition::new(1.0, 1.0));
    app.settle();
}

/// The drive menu (Alt+F1): rows by name with the path described; a drive opens its path.
fn menu(
    app: &mut App,
    context: &mut Context,
    shown: &[Drive],
    failures: &mut Vec<String>,
) -> Result<(), String> {
    crate::panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Alt], Key::F1);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.window.get_volume_popup_pane() == 0
    }) {
        failures.push("Alt+F1 opened no left drive menu".into());
        return Ok(());
    }
    app.screenshot(
        context,
        "drive-menu",
        "Alt+F1: the drive menu lists drives by name",
    );
    let rows = ps::popup_buttons(app);
    let mut chosen = None;
    for drive in shown {
        let prefix = format!("{} — ", drive.caption);
        let Some((label, item)) = rows.iter().find(|(label, _)| label.starts_with(&prefix)) else {
            failures.push(format!(
                "the drive menu has no row for {:?}; rows {:?}",
                drive.caption,
                rows.iter().map(|(label, _)| label).collect::<Vec<_>>()
            ));
            continue;
        };
        if cfg!(target_os = "linux") && drive.root != Path::new("/") && label.contains(&drive.path)
        {
            failures.push(format!("the drive menu row {label:?} shows the mount path"));
        }
        let description = tools_support::description(item);
        if !description.contains(&drive.path) {
            failures.push(format!(
                "the drive menu row {label:?} describes {description:?}, not the path {}",
                drive.path
            ));
        }
        if chosen.is_none() && drive.root != Path::new("/") {
            chosen = Some((label.clone(), drive.root.clone()));
        }
    }
    match chosen {
        None => {
            input::key(app.slint(), Key::Escape);
            app.settle();
            context.note("only the root is listed: no drive to choose");
        }
        Some((label, root)) => {
            ps::click_popup_button(app, &label)?;
            let reached = ps::wait(app, Duration::from_secs(10), |app| {
                app.location(LEFT).as_deref() == Some(root.as_path()) && !app.loading(LEFT)
            });
            if !reached {
                failures.push(format!(
                    "choosing {label:?} opened {:?}, not {}",
                    app.location(LEFT),
                    root.display()
                ));
            }
        }
    }
    Ok(())
}
