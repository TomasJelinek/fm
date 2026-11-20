//! Notice dialogs and the Jobs window's list, driven with the pointer and keys.
//!
//! Notices: entering an unreadable folder shows "Cannot open folder" with the folder,
//! Details (expands to the diagnostic details, Hide details collapses them), Copy
//! details (the clipboard gets the message: the status line says so), Close, Escape and
//! the header's close button each closing it with no other notice following, the
//! panel kept where it was; a very long folder path keeps the dialog inside the window
//! with Close in view and its folder box scrolls to the end of the path.
//!
//! Jobs: create, rename and delete jobs and one that fails are listed with their
//! states; a click and the arrow keys select rows; Open (and Enter) shows a job's
//! window; Copy details reports the copy; Cancel job is disabled for finished jobs;
//! Clear finished without errors keeps the failed job, Clear all finished empties the
//! list ("No background operations"); Close and Escape close the window.
use crate::{
    Context,
    app::{self, App, Config},
    input, inspect, panel,
    tools_support::{self as tools, Tool},
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::PaneId;
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const SIZE: (u32, u32) = (760, 560);
const DENIED: &str = "Access to this folder was denied.";

fn wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} did not happen; notice {:?}, tool windows {:?}, status {:?}",
                tools::shown_notice(app),
                tools::tool_titles(app),
                app.window.get_desktop_status()
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

fn lock(path: &Path) -> Result<(), String> {
    tools::set_mode(path, 0o000)
}

/// Enter `name` with Enter and wait for the "Cannot open folder" notice.
fn enter_locked(app: &mut App, name: &str) -> Result<(String, String), String> {
    panel::select_row(app, LEFT, name)?;
    input::key(app.slint(), Key::Return);
    wait(app, &format!("the notice for {name}"), |app| {
        tools::shown_notice(app).is_some()
    })?;
    app.idle(Duration::from_millis(100));
    Ok(tools::shown_notice(app).unwrap_or_default())
}

/// The notice stays closed for a second: no further notice follows.
fn stays_closed(app: &mut App, how: &str, failures: &mut Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        app.idle(Duration::from_millis(50));
        if let Some((title, _)) = tools::shown_notice(app) {
            failures.push(format!("after {how} another notice appeared: {title:?}"));
            return;
        }
    }
}

/// Text without the bidirectional isolation marks the application wraps names in.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}'))
        .collect()
}

fn notice_texts(app: &App) -> Vec<String> {
    inspect::roots(app.slint())
        .last()
        .map(tools::item_texts)
        .unwrap_or_default()
        .iter()
        .map(|text| plain(text))
        .collect()
}

fn button_shown(app: &App, label: &str) -> bool {
    inspect::find(app.slint(), AccessibleRole::Button, label)
        .and_then(|button| inspect::visible_rect(&button))
        .is_some()
}

/// The events and commands traced since `before` was taken (the trace keeps the last
/// 32 lines).
fn since(before: &[String], app: &App) -> Vec<String> {
    let after = app.recent_events();
    let start = before
        .last()
        .and_then(|last| after.iter().rposition(|line| line == last))
        .map_or(0, |index| index + 1);
    after[start..].to_vec()
}

/// Copy details sends the text to the clipboard and the status line reports it; the
/// in-process clipboard then holds every one of `wanted`.
fn copied(app: &mut App, before: &[String], wanted: &[&str]) -> Result<(), String> {
    let snapshot = before.to_vec();
    wait(app, "Copy details to reach the clipboard", move |app| {
        since(&snapshot, app)
            .iter()
            .any(|line| line.starts_with("→ CopyText") || line.starts_with("→ CopyGeneratedText"))
            && app.window.get_desktop_status().as_str() == "Copied to clipboard"
    })?;
    let text = app
        .clipboard
        .as_ref()
        .and_then(|clipboard| clipboard.text())
        .unwrap_or_default();
    let missing: Vec<&&str> = wanted
        .iter()
        .filter(|part| !text.contains(**part))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Copy details put {text:?} on the clipboard, without {missing:?}"
        ))
    }
}

/// Close a tool window with Escape, or its Close button when Escape does not.
fn close_window(app: &mut App, window: &Tool, what: &str) -> Result<(), String> {
    input::key(window.window(), Key::Escape);
    let watched = window.clone();
    if wait(app, &format!("Escape to close {what}"), move |_| {
        !watched.window().is_visible()
    })
    .is_ok()
    {
        return Ok(());
    }
    tools::click_last(window.window(), "Close")?;
    let watched = window.clone();
    wait(app, &format!("Close to close {what}"), move |_| {
        !watched.window().is_visible()
    })?;
    Err(format!("Escape does not close {what}"))
}

/// A nested folder chain whose full path is long, ending in an unreadable folder.
fn long_chain(base: &Path) -> Result<PathBuf, String> {
    let mut path = base.to_path_buf();
    for level in 0..7 {
        path = path.join(format!("{level}-{}", "long-folder-name-".repeat(11)));
    }
    let locked = path.join("locked-at-the-end");
    fs::create_dir_all(&locked).map_err(|error| format!("{}: {error}", locked.display()))?;
    Ok(locked)
}

pub fn notices_and_jobs(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("notices");
    let panel_folder = root.join("panel");
    let other = root.join("other");
    let locked = panel_folder.join("locked");
    fs::create_dir_all(&locked).map_err(|error| error.to_string())?;
    fs::create_dir_all(&other).map_err(|error| error.to_string())?;
    fs::write(panel_folder.join("file.txt"), "file\n").map_err(|error| error.to_string())?;
    let long = long_chain(&panel_folder.join("chain"))?;
    lock(&locked)?;
    lock(&long)?;
    let mut config = Config::new(&panel_folder, &other);
    config.clipboard = true;
    let mut app = App::start(config);
    let mut failures: Vec<String> = Vec::new();
    let result = notices(&mut app, context, &panel_folder, &long, &mut failures)
        .and_then(|()| jobs(&mut app, context, &panel_folder, &mut failures));
    // The work folder stays deletable whatever happened.
    let _ = tools::set_mode(&locked, 0o755);
    let _ = tools::set_mode(&long, 0o755);
    app.shutdown()?;
    result?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn notices(
    app: &mut App,
    context: &mut Context,
    folder: &Path,
    long: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // The notice and its parts.
    let (title, body) = enter_locked(app, "locked")?;
    if title != "Cannot open folder" || !body.contains(DENIED) {
        failures.push(format!(
            "entering an unreadable folder shows {title:?}: {body:?}"
        ));
    }
    let shown = notice_texts(app);
    let path = folder.join("locked").display().to_string();
    if !shown.iter().any(|text| text == &path) || !shown.iter().any(|text| text == "Folder") {
        failures.push(format!(
            "the notice does not show the folder {path}: {shown:?}"
        ));
    }
    for label in ["Details", "Copy details", "Close"] {
        if !button_shown(app, label) {
            failures.push(format!("the notice has no {label:?} button"));
        }
    }
    if app.location(LEFT).as_deref() != Some(folder) {
        failures.push(format!(
            "the failed navigation left the panel on {:?}",
            app.location(LEFT)
        ));
    }

    // Details expands the diagnostic details; Hide details collapses them.
    input::click_button(app.slint(), "Details")?;
    app.idle(Duration::from_millis(100));
    let expanded = notice_texts(app);
    let details: Vec<&String> = expanded
        .iter()
        .filter(|text| text.contains("Error kind") || text.contains("Operation:"))
        .collect();
    if details.is_empty() || !button_shown(app, "Hide details") {
        failures.push(format!(
            "Details shows no diagnostic details or no Hide details: {expanded:?}"
        ));
    }
    app.screenshot(
        context,
        "notice-details",
        "A folder notice with its details shown",
    );
    if button_shown(app, "Hide details") {
        input::click_button(app.slint(), "Hide details")?;
        app.idle(Duration::from_millis(100));
        if notice_texts(app)
            .iter()
            .any(|text| text.contains("Error kind"))
            || !button_shown(app, "Details")
        {
            failures.push("Hide details does not collapse the details".into());
        }
    }

    // Copy details: the clipboard receives the message.
    let before = app.recent_events();
    input::click_button(app.slint(), "Copy details")?;
    if let Err(error) = copied(app, &before, &["Cannot open folder", path.as_str(), DENIED]) {
        failures.push(format!("the notice's {error}"));
    }
    if tools::shown_notice(app).is_none() {
        failures.push("Copy details closed the notice".into());
    }

    // Close, Escape and the header's close button each close it; nothing follows.
    input::click_button(app.slint(), "Close")?;
    wait(app, "Close to close the notice", |app| {
        tools::shown_notice(app).is_none()
    })?;
    stays_closed(app, "Close", failures);
    panel::wait_input(app);
    let _ = enter_locked(app, "locked")?;
    input::key(app.slint(), Key::Escape);
    if wait(app, "Escape to close the notice", |app| {
        tools::shown_notice(app).is_none()
    })
    .is_err()
    {
        failures.push("Escape does not close the notice".into());
        input::click_button(app.slint(), "Close")?;
    }
    stays_closed(app, "Escape", failures);
    panel::wait_input(app);
    let _ = enter_locked(app, "locked")?;
    if button_shown(app, "Close dialog") {
        input::click_button(app.slint(), "Close dialog")?;
        if wait(
            app,
            "the header's close button to close the notice",
            |app| tools::shown_notice(app).is_none(),
        )
        .is_err()
        {
            failures.push("the notice header's close button does not close it".into());
            input::click_button(app.slint(), "Close")?;
        }
        stays_closed(app, "the header's close button", failures);
    } else {
        failures.push("the notice has no header close button".into());
        input::click_button(app.slint(), "Close")?;
    }
    panel::wait_input(app);
    panel::focus_list(app, LEFT)?;

    // A very long folder path: the dialog stays inside the window, Close in view, and
    // its folder box scrolls to the end of the path.
    let parent = long.parent().ok_or("the long chain has no parent")?;
    input::chord(app.slint(), &[Key::Control], "g");
    wait(app, "the path editor", |app| {
        tools::has(app.slint(), AccessibleRole::TextInput, "Folder path")
    })?;
    input::replace_text(app.slint(), &parent.display().to_string());
    input::key(app.slint(), Key::Return);
    panel::wait_location(app, LEFT, parent);
    let (_, body) = enter_locked(app, "locked-at-the-end")?;
    if !body.contains(DENIED) {
        failures.push(format!("the long path's notice says {body:?}"));
    }
    let close = inspect::find(app.slint(), AccessibleRole::Button, "Close")
        .ok_or("the long notice has no Close button")?;
    let full = inspect::rect(&close);
    let seen = inspect::visible_rect(&close);
    if seen.is_none_or(|rect| rect.3 < full.3 * 0.9)
        || full.1 + full.3 > app.height as f32
        || full.0 + full.2 > app.width as f32
    {
        failures.push(format!(
            "with a {}-character path the notice's Close button is not fully in view ({full:?}, shown {seen:?})",
            long.display().to_string().len()
        ));
    }
    let long_text = long.display().to_string();
    let path_item = inspect::roots(app.slint()).last().and_then(|root| {
        let mut found = None;
        inspect::walk(root, &mut |item| {
            // The shown path, not the dialog's hidden measuring copy of it.
            if found.is_none()
                && inspect::visible_rect(item).is_some()
                && tools::text_of(item).is_some_and(|text| plain(&text) == long_text)
            {
                found = Some(item.clone());
            }
        });
        found
    });
    match path_item.as_ref().and_then(inspect::clip_rect) {
        Some(clip) => {
            let item = path_item.as_ref().expect("the path text");
            let before = inspect::rect(item);
            if before.3 <= clip.3 + 1.0 {
                context.note("the long folder path fits its box without scrolling");
            } else {
                for _ in 0..40 {
                    app.slint().dispatch_event(WindowEvent::PointerScrolled {
                        position: LogicalPosition::new(
                            clip.0 + clip.2 / 2.0,
                            clip.1 + clip.3 / 2.0,
                        ),
                        delta_x: 0.0,
                        delta_y: -60.0,
                    });
                    app.settle();
                }
                let after = inspect::rect(item);
                let end_shown = after.1 + after.3 <= clip.1 + clip.3 + 2.0;
                if !end_shown {
                    failures.push(format!(
                        "the long folder path does not scroll to its end: text {after:?} in box {clip:?}"
                    ));
                }
            }
        }
        None => failures.push("the long notice does not show its folder path".into()),
    }
    app.screenshot(
        context,
        "notice-long-path",
        "A notice with a very long folder path",
    );
    input::click_button(app.slint(), "Close")?;
    wait(app, "Close to close the long notice", |app| {
        tools::shown_notice(app).is_none()
    })?;
    stays_closed(app, "closing the long notice", failures);
    panel::wait_input(app);
    // Back to the panel's folder for the jobs.
    input::chord(app.slint(), &[Key::Control], "g");
    wait(app, "the path editor", |app| {
        tools::has(app.slint(), AccessibleRole::TextInput, "Folder path")
    })?;
    input::replace_text(app.slint(), &folder.display().to_string());
    input::key(app.slint(), Key::Return);
    panel::wait_location(app, LEFT, folder);
    Ok(())
}

/// Type a name into the open operation dialog and confirm it with Enter.
fn confirm_name(app: &mut App, name: &str) -> Result<(), String> {
    wait(app, "the operation dialog", |app| {
        !app.window.get_operation_token().is_empty()
    })?;
    input::chord(app.slint(), &[Key::Control], "a");
    input::type_text(app.slint(), name);
    input::key(app.slint(), Key::Return);
    wait(app, "the operation dialog to close", |app| {
        app.window.get_operation_token().is_empty()
    })?;
    panel::wait_input(app);
    Ok(())
}

fn job_rows(window: &Tool) -> Vec<(String, String, bool)> {
    let mut rows: Vec<(f32, String, String, bool)> =
        inspect::controls(window.window(), AccessibleRole::ListItem)
            .into_iter()
            .map(|(title, item)| {
                // A failed job's row starts with its error glyph ("!").
                let texts: Vec<String> = tools::item_texts(&item)
                    .into_iter()
                    .filter(|text| text != "!")
                    .collect();
                let checked = item
                    .accessible_string_property(AccessibleStringProperty::Checked)
                    .is_some_and(|value| value == "true");
                (
                    inspect::rect(&item).1,
                    title,
                    texts.get(1).cloned().unwrap_or_default(),
                    checked,
                )
            })
            .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter()
        .map(|(_, title, state, checked)| (title, state, checked))
        .collect()
}

fn open_jobs(app: &mut App) -> Result<Tool, String> {
    panel::wait_input(app);
    let before = app.tool_windows().len();
    input::click_button(app.slint(), "Jobs")?;
    app.until("the Jobs window", move |app| {
        tools::tool_titled(app, "Jobs").is_some() || app.tool_windows().len() > before
    });
    let window = tools::tool_titled(app, "Jobs")
        .ok_or_else(|| format!("no window titled Jobs: {:?}", tools::tool_titles(app)))?;
    app::layout_tool(&window, SIZE.0, SIZE.1);
    app.settle();
    Ok(window)
}

fn jobs(
    app: &mut App,
    context: &mut Context,
    folder: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // Jobs of several kinds: a folder, a file, a rename, a deletion and a failure.
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::F7);
    confirm_name(app, "made")?;
    input::chord(app.slint(), &[Key::Shift], Key::F4);
    confirm_name(app, "new.txt")?;
    app.until("new.txt to be listed", |app| {
        app.names(LEFT).iter().any(|name| name == "new.txt")
    });
    panel::select_row(app, LEFT, "new.txt")?;
    input::key(app.slint(), Key::F2);
    confirm_name(app, "renamed.txt")?;
    app.until("renamed.txt to be listed", |app| {
        app.names(LEFT).iter().any(|name| name == "renamed.txt")
    });
    panel::select_row(app, LEFT, "made")?;
    input::chord(app.slint(), &[Key::Shift], Key::F8);
    wait(app, "the permanent delete confirmation", |app| {
        !app.window.get_operation_token().is_empty() && app.window.get_operation_dangerous()
    })?;
    input::key(app.slint(), Key::Return);
    wait(app, "the delete dialog to close", |app| {
        app.window.get_operation_token().is_empty()
    })?;
    panel::wait_input(app);
    app.until("made to be deleted", |app| {
        !app.names(LEFT).iter().any(|name| name == "made")
    });
    // A folder inside the unreadable one cannot be created: a failed job.
    input::key(app.slint(), Key::F7);
    wait(app, "the Create directory dialog", |app| {
        !app.window.get_operation_token().is_empty()
    })?;
    input::chord(app.slint(), &[Key::Control], "a");
    input::type_text(app.slint(), "locked/inside");
    input::key(app.slint(), Key::Return);
    if wait(app, "the Create directory dialog to close", |app| {
        app.window.get_operation_token().is_empty()
    })
    .is_err()
    {
        context.note(format!(
            "creating inside the unreadable folder kept the dialog open: {:?}",
            inspect::visible_texts(app.slint())
        ));
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(100));
    }
    app.idle(Duration::from_millis(300));
    if let Some((title, _)) = tools::shown_notice(app) {
        context.note(format!("the failed create showed the notice {title:?}"));
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(100));
    }
    if folder.join("made").exists() || !folder.join("renamed.txt").is_file() {
        failures.push("the create, rename and delete jobs did not change the disk".into());
    }

    let window = open_jobs(app)?;
    let rows = job_rows(&window);
    let texts = inspect::visible_texts(window.window());
    if rows.len() < 5 {
        failures.push(format!(
            "Jobs lists {} jobs for five operations: {rows:?}",
            rows.len()
        ));
    }
    let header = format!("{} operations", rows.len());
    if !texts.iter().any(|text| *text == header) {
        failures.push(format!(
            "the Jobs header does not say {header:?}: {texts:?}"
        ));
    }
    if rows
        .iter()
        .all(|(_, state, _)| !state.to_lowercase().contains("fail"))
    {
        failures.push(format!("no job is shown as failed: {rows:?}"));
    }
    app.screenshot_window(
        context,
        &window,
        "jobs-list",
        "Jobs: five finished operations",
    );

    // Selecting: a click, then the arrow keys.
    let first = inspect::controls(window.window(), AccessibleRole::ListItem)
        .into_iter()
        .map(|(_, item)| item)
        .min_by(|a, b| inspect::rect(a).1.total_cmp(&inspect::rect(b).1))
        .ok_or("the Jobs list has no rows")?;
    input::click_at(window.window(), inspect::center(&first));
    app.settle();
    let selected = |window: &Tool| job_rows(window).iter().position(|(_, _, checked)| *checked);
    if selected(&window) != Some(0) {
        failures.push(format!(
            "clicking the first job selects row {:?}",
            selected(&window)
        ));
    }
    input::key(window.window(), Key::DownArrow);
    app.settle();
    if selected(&window) != Some(1) {
        failures.push(format!("Down in Jobs selects row {:?}", selected(&window)));
    }
    input::key(window.window(), Key::UpArrow);
    app.settle();
    if selected(&window) != Some(0) {
        failures.push(format!("Up in Jobs selects row {:?}", selected(&window)));
    }
    // Cancel job is offered only for running jobs.
    if inspect::find(window.window(), AccessibleRole::Button, "Cancel job")
        .is_some_and(|button| tools::enabled(&button))
    {
        failures.push("Cancel job is enabled for a finished job".into());
    }

    // Open shows the job's own window; Enter does too.
    for how in ["Open", "Enter"] {
        let before = app.tool_windows().len();
        if how == "Open" {
            tools::click_last(window.window(), "Open")?;
        } else {
            input::key(window.window(), Key::Return);
        }
        if wait(app, &format!("{how} to show the job's window"), |app| {
            app.tool_windows().len() > before
        })
        .is_err()
        {
            failures.push(format!(
                "{how} in Jobs opens no window for the selected job"
            ));
            continue;
        }
        let opened = app.tool_windows().pop().expect("the job window");
        app::layout_tool(&opened, SIZE.0, SIZE.1);
        app.settle();
        if how == "Open" {
            app.screenshot_window(
                context,
                &opened,
                "job-window",
                "Open: the selected job's window",
            );
        }
        if let Err(error) = close_window(app, &opened, "the job's window") {
            failures.push(error);
        }
    }

    // Copy details reports the copy.
    let selected_title = job_rows(&window)
        .into_iter()
        .find(|(_, _, checked)| *checked)
        .map(|(title, _, _)| title)
        .unwrap_or_default();
    let before = app.recent_events();
    tools::click_last(window.window(), "Copy details")?;
    if let Err(error) = copied(app, &before, &[selected_title.as_str()]) {
        failures.push(format!("Jobs › {error}"));
    }

    // Clear finished without errors keeps the failed job; Clear all finished empties.
    tools::click_last(window.window(), "Clear finished without errors")?;
    app.settle();
    let kept = job_rows(&window);
    if kept.is_empty()
        || kept
            .iter()
            .any(|(_, state, _)| !state.to_lowercase().contains("fail"))
    {
        failures.push(format!(
            "Clear finished without errors left {kept:?}, expected only the failed job"
        ));
    }
    tools::click_last(window.window(), "Clear all finished")?;
    app.settle();
    let texts = inspect::visible_texts(window.window());
    if !job_rows(&window).is_empty()
        || !texts.iter().any(|text| text == "No background operations")
        || !texts.iter().any(|text| text == "0 operations")
    {
        failures.push(format!("Clear all finished leaves {texts:?}"));
    }
    for label in [
        "Clear all finished",
        "Clear finished without errors",
        "Open",
    ] {
        if inspect::find(window.window(), AccessibleRole::Button, label)
            .is_some_and(|button| tools::enabled(&button))
        {
            failures.push(format!("{label} stays enabled in an empty Jobs list"));
        }
    }
    app.screenshot_window(
        context,
        &window,
        "jobs-empty",
        "Jobs after Clear all finished",
    );

    // Close, then Escape after reopening.
    tools::click_last(window.window(), "Close")?;
    let watched = window.clone();
    if wait(app, "Close to close Jobs", move |_| {
        !watched.window().is_visible()
    })
    .is_err()
    {
        failures.push("Close does not close Jobs".into());
    }
    let window = open_jobs(app)?;
    if let Err(error) = close_window(app, &window, "Jobs") {
        failures.push(error);
    }
    Ok(())
}
