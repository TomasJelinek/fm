//! Properties (Alt+Enter) of several marked items opens the system's own dialog where
//! the platform has one (`Config::native_properties`). Nine files and a folder marked
//! in one folder:
//! - Windows: one Shell property sheet for all ten, whose summary counts 9 files and 1
//!   folder ("9 Files, 1 Folders"; only the numbers are compared, so any display
//!   language passes), and never the box titled "Desktop" that says "The properties for
//!   this item are not available."; the application shows no summary of its own. The
//!   sheet is pictured for the report and closed.
//! - Linux has no system dialog: the application's summary is titled "10 selected
//!   items" and counts "9 files, 1 folder".
use crate::{
    Context, Scenario,
    app::{App, Config},
    files_support as support, input, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use slint::platform::Key;
use std::time::Duration;

/// This module's entries in the registry (it owns them; `scenarios/mod.rs` lists the
/// module).
pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "panels_properties_several",
        // Windows opens and inspects a native Shell property sheet on the desktop.
        isolation: if cfg!(windows) {
            crate::Isolation::Exclusive
        } else {
            crate::Isolation::Parallel
        },
        area: "panels",
        summary: "Alt+Enter on nine files and a folder marked in one folder: Windows opens one Shell property sheet counting 9 files and 1 folder (pictured, then closed), never the \"Desktop\" box \"The properties for this item are not available.\", and no summary of the application's own; Linux shows the application's summary of 10 selected items, 9 files and 1 folder.",
        run: several,
        timeout: Duration::from_secs(3 * 60),
    }]
}

fn several(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "properties-several")?;
    for index in 1..=9_usize {
        ps::write(
            &folder.join(format!("file {index}.txt")),
            vec![b'p'; index * 100],
        )?;
    }
    ps::folder(&folder.join("sub"))?;
    let mut config = Config::new(&folder, &context.work);
    config.desktop = true;
    config.native_properties = true;
    let mut app = App::start(config);
    let mut failures = Vec::new();
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control, Key::Shift], "a");
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        support::selected_names(app, LEFT).len() == 9
    }) {
        return Err(format!(
            "Ctrl+Shift+A marked {:?}, not the nine files",
            support::selected_names(&app, LEFT)
        ));
    }
    // The folder too, by the keyboard, which leaves the files marked.
    let index = panel::index_of(&app, LEFT, "sub")?;
    input::key(app.slint(), Key::Home);
    for _ in 0..index {
        input::key(app.slint(), Key::DownArrow);
    }
    input::key(app.slint(), Key::Insert);
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        support::selected_names(app, LEFT).len() == 10
    }) {
        return Err(format!(
            "Insert on \"sub\" left {:?} marked, not the nine files and the folder",
            support::selected_names(&app, LEFT)
        ));
    }
    input::chord(app.slint(), &[Key::Alt], Key::Return);
    #[cfg(windows)]
    native_sheet(context, &mut app, &mut failures);
    #[cfg(not(windows))]
    summary(context, &mut app, &mut failures);
    support::dismiss_all(&mut app);
    app.shutdown()?;
    support::report(&failures)
}

/// The numbers in `text`, in order: "9 Files, 1 Folders" → [9, 1].
#[cfg(windows)]
fn numbers(text: &str) -> Vec<u64> {
    text.split(|character: char| !character.is_ascii_digit())
        .filter_map(|digits| digits.parse().ok())
        .collect()
}

#[cfg(windows)]
fn native_sheet(context: &mut Context, app: &mut App, failures: &mut Vec<String>) {
    use crate::native_windows::{self as native, NativeWindow};
    let dialog = |window: &NativeWindow| window.class == "#32770";
    if !ps::wait(app, Duration::from_secs(30), |_| {
        native::windows().iter().any(dialog)
    }) {
        failures.push(format!(
            "Alt+Enter on nine files and a folder opened no native window (notice {:?})",
            support::shown_notice(app)
        ));
        return;
    }
    // The sheet fills in its General page (counting, sizes) after it appears.
    let counted = |window: &NativeWindow| {
        dialog(window) && window.texts.iter().any(|text| numbers(text) == [9, 1])
    };
    ps::wait(app, Duration::from_secs(10), |_| {
        native::windows().iter().any(counted)
    });
    app.idle(Duration::from_millis(500));
    let shown = native::windows();
    for window in &shown {
        if window.title == "Desktop"
            || window
                .texts
                .iter()
                .any(|text| text.contains("properties for this item are not available"))
        {
            failures.push(format!(
                "Properties of several items showed the unavailable box {window:?}"
            ));
        }
    }
    match shown.iter().find(|window| counted(window)) {
        None => failures.push(format!(
            "no native sheet counts 9 files and 1 folder; shown: {shown:?}"
        )),
        Some(sheet) => {
            let summary = sheet.texts.iter().find(|text| numbers(text) == [9, 1]);
            let location = sheet
                .texts
                .iter()
                .skip_while(|text| !text.starts_with("Location"))
                .nth(1);
            context.note(format!(
                "Alt+Enter on nine files and a folder: the Shell sheet {:?} says {summary:?}, \
                 location {location:?}",
                sheet.title
            ));
            picture(context, sheet);
        }
    }
    if let Some(notice) = support::shown_notice(app) {
        failures.push(format!(
            "the native sheet opened, but the application also showed {notice:?}"
        ));
    }
    for window in &shown {
        window.close();
    }
    if !ps::wait(app, Duration::from_secs(10), |_| {
        shown.iter().all(|window| !window.exists())
    }) {
        failures.push("the native Properties windows did not close".into());
    }
}

/// The native window's picture in the report, as `App::screenshot` saves its own.
#[cfg(windows)]
fn picture(context: &mut Context, window: &crate::native_windows::NativeWindow) {
    let relative = format!(
        "shots/{}/properties-several.png",
        crate::app::context_name()
    );
    let saved = window.capture().and_then(|(width, height, pixels)| {
        crate::report::save_png(&context.run_dir.join(&relative), width, height, &pixels)
    });
    match saved {
        Ok(()) => context.outcome.shots.push(crate::report::Shot {
            file: relative,
            caption: "Alt+Enter Properties of nine files and a folder: the Shell's sheet".into(),
            flags: 0,
        }),
        Err(error) => context.note(format!("screenshot properties-several failed: {error}")),
    }
}

#[cfg(not(windows))]
fn summary(context: &mut Context, app: &mut App, failures: &mut Vec<String>) {
    if !ps::wait(app, Duration::from_secs(20), |app| {
        support::shown_notice(app).is_some()
    }) {
        failures.push("Alt+Enter on nine files and a folder showed no Properties".into());
        return;
    }
    let (title, body) = support::shown_notice(app).unwrap_or_default();
    context.note(format!("Properties of several items: {title:?} {body:?}"));
    if title != "10 selected items" || !body.lines().any(|line| line == "9 files, 1 folder") {
        failures.push(format!(
            "Properties of nine files and a folder says {title:?} {body:?}, not \"10 \
             selected items\" with \"9 files, 1 folder\""
        ));
    }
    app.screenshot(
        context,
        "properties-several",
        "Alt+Enter Properties of nine files and a folder: the application's summary",
    );
}
