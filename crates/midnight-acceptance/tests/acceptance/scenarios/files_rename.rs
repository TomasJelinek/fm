//! F2 Rename: Unicode names, a case-only change, a collision (Skip, then Replace),
//! invalid names (`/`, empty), the longest valid name and one byte more, and folders.
//! The new name must be listed as soon as the job reports, with the cursor on it.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Choice, Timings, io_error},
    input, panel,
};
use midnight_core::{OperationProgress, OperationState, PaneId};
use slint::platform::Key;
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

/// What one F2 attempt did.
struct Renamed {
    /// The terminal report, if the job reached one.
    progress: Option<Arc<OperationProgress>>,
    /// Texts of the decision windows answered.
    decisions: Vec<String>,
    /// The dialog stayed open after Enter (the name was not accepted).
    still_open: bool,
    warning: String,
    finished: Instant,
    /// The notice the job left (closed with Escape).
    notice: Option<(String, String)>,
}

/// F2 on the focused row, replace the whole name with `text`, press Enter, answer
/// decision windows with `choices`.
fn rename(app: &mut App, text: &str, choices: &[Choice]) -> Result<Renamed, String> {
    let start = support::named_operation(app, &[], Key::F2, text)?;
    let mut decisions = Vec::new();
    let mut choices = choices.iter();
    let confirmed = Instant::now();
    let deadline = confirmed + Duration::from_secs(60);
    loop {
        app.settle();
        if let Some(progress) = app.operations[start..]
            .iter()
            .find(|progress| progress.state.is_terminal())
            .cloned()
        {
            let finished = Instant::now();
            app.settle();
            return Ok(Renamed {
                progress: Some(progress),
                decisions,
                still_open: false,
                warning: String::new(),
                finished,
                notice: support::take_notice(app),
            });
        }
        if support::decision_window(app).is_some() {
            let window = support::wait_decision(app, "the rename decision window");
            let text = support::describe_window(&window);
            let Some(choice) = choices.next() else {
                return Err(format!("unexpected rename decision window: {text}"));
            };
            decisions.push(text);
            let token = app.window.get_copy_token();
            support::answer(app, &window, choice)?;
            app.until("the rename decision to be taken", move |app| {
                app.window.get_copy_token() != token
            });
            continue;
        }
        // A dialog still open seconds after Enter, with no job, did not take the name.
        if !app.window.get_operation_token().is_empty()
            && confirmed.elapsed() > Duration::from_secs(3)
            && app.operations[start..].is_empty()
        {
            return Ok(Renamed {
                progress: None,
                decisions,
                still_open: true,
                warning: app.window.get_operation_warning().to_string(),
                finished: Instant::now(),
                notice: support::shown_notice(app),
            });
        }
        if Instant::now() > deadline {
            return Err(format!(
                "F2 to {text:?} neither finished nor asked within 60 s; notice {:?}",
                support::shown_notice(app)
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|error| io_error(path, error))
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

pub fn rename_cases(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "rename")?;
    for name in [
        "plain.txt",
        "case.txt",
        "a.txt",
        "b.txt",
        "slash.txt",
        "empty.txt",
        "long.txt",
    ] {
        write(&folder.join(name), name)?;
    }
    fs::create_dir(folder.join("folder")).map_err(|error| io_error(&folder, error))?;
    write(&folder.join("folder/inside.txt"), "inside")?;
    let mut app = App::start(Config::new(&folder, &context.work));
    let pane = PaneId::Left;
    let mut failures = Vec::new();
    let mut timings = Timings::default();

    // Unicode, shown at once with the cursor on it.
    let unicode = "Žluťoučký kůň 🐎 日本語 עברית.txt";
    panel::select_row(&mut app, pane, "plain.txt")?;
    let done = rename(&mut app, unicode, &[])?;
    match &done.progress {
        Some(progress) if progress.state == OperationState::Completed => {
            let shown = support::row_delay(&mut app, pane, unicode, true, done.finished);
            timings.add("report to new name listed", shown);
            if app.names(pane).iter().any(|name| name == "plain.txt") {
                failures.push("plain.txt is still listed after the rename".into());
            }
            panel::wait_input(&mut app);
            if support::cursor_name(&app, pane).as_deref() != Some(unicode) {
                failures.push(format!(
                    "after F2 the cursor is on {:?}, not the renamed file",
                    support::cursor_name(&app, pane)
                ));
            }
            if read(&folder.join(unicode)).as_deref() != Some("plain.txt") {
                failures.push("the Unicode-named file lost its content".into());
            }
        }
        other => failures.push(format!("F2 to a Unicode name: {other:?}")),
    }
    app.screenshot(
        context,
        "renamed-unicode",
        "F2 to a Unicode name, cursor on it",
    );

    // Case only.
    panel::select_row(&mut app, pane, "case.txt")?;
    let done = rename(&mut app, "CASE.TXT", &[])?;
    let names = support::names_on_disk(&folder);
    if done
        .progress
        .as_ref()
        .is_none_or(|progress| progress.state != OperationState::Completed)
        || !names.contains(&"CASE.TXT".to_owned())
        || names.contains(&"case.txt".to_owned())
    {
        failures.push(format!(
            "case-only rename: {:?}; disk {names:?}",
            done.progress.map(|progress| progress.state)
        ));
    } else {
        support::wait_row(&mut app, pane, "CASE.TXT", true);
        support::wait_row(&mut app, pane, "case.txt", false);
    }

    // Collision: Skip keeps both, then Replace takes the name.
    panel::select_row(&mut app, pane, "a.txt")?;
    let done = rename(&mut app, "b.txt", &[Choice::Skip])?;
    if done.decisions.is_empty() {
        failures.push(format!(
            "renaming a.txt onto b.txt asked nothing: {:?}",
            done.progress
                .as_ref()
                .map(|progress| (progress.state, &progress.errors))
        ));
    } else {
        context.note(format!("F2 collision asked: {}", done.decisions[0]));
    }
    if read(&folder.join("a.txt")).as_deref() != Some("a.txt")
        || read(&folder.join("b.txt")).as_deref() != Some("b.txt")
    {
        failures.push("Skip on a rename collision changed a.txt or b.txt".into());
    }
    support::dismiss_all(&mut app);
    panel::select_row(&mut app, pane, "a.txt")?;
    let done = rename(&mut app, "b.txt", &[Choice::Replace])?;
    if done
        .progress
        .as_ref()
        .is_none_or(|progress| progress.state != OperationState::Completed)
        || folder.join("a.txt").exists()
        || read(&folder.join("b.txt")).as_deref() != Some("a.txt")
    {
        failures.push(format!(
            "Replace on a rename collision: {:?}; a.txt exists {}, b.txt holds {:?}",
            done.progress.as_ref().map(|progress| progress.state),
            folder.join("a.txt").exists(),
            read(&folder.join("b.txt"))
        ));
    } else {
        support::wait_row(&mut app, pane, "a.txt", false);
    }
    support::dismiss_all(&mut app);

    // A name with a path separator is refused; nothing is created.
    panel::select_row(&mut app, pane, "slash.txt")?;
    let done = rename(&mut app, "x/y.txt", &[])?;
    let notice = done.notice.clone();
    let refused = done.still_open
        || done
            .progress
            .as_ref()
            .is_some_and(|progress| progress.state != OperationState::Completed);
    if !refused || !folder.join("slash.txt").is_file() || folder.join("x").exists() {
        failures.push(format!(
            "F2 to \"x/y.txt\" was not refused: {:?}, slash.txt exists {}, x exists {}",
            done.progress.as_ref().map(|progress| progress.state),
            folder.join("slash.txt").is_file(),
            folder.join("x").exists()
        ));
    }
    context.note(format!(
        "F2 to \"x/y.txt\": dialog open {}, warning {:?}, report {:?}, notice {notice:?}",
        done.still_open,
        done.warning,
        done.progress.as_ref().map(|progress| (
            progress.state,
            progress.errors.first().map(ToString::to_string)
        ))
    ));
    app.screenshot(context, "rename-slash", "F2 to a name with a slash");
    support::dismiss_all(&mut app);

    // An empty name cannot be confirmed.
    panel::select_row(&mut app, pane, "empty.txt")?;
    support::named_operation(&mut app, &[], Key::F2, "")?;
    app.idle(Duration::from_millis(300));
    if app.window.get_operation_token().is_empty() {
        failures.push("Enter with an empty name closed the Rename dialog".into());
    }
    input::key(app.slint(), Key::Escape);
    app.until("the Rename dialog to close", |app| {
        app.window.get_operation_token().is_empty()
    });
    if !folder.join("empty.txt").is_file() {
        failures.push("an empty-name rename changed empty.txt".into());
    }

    // The longest name the filesystem takes (255 bytes), then one byte more.
    let longest = format!("{}.txt", "l".repeat(251));
    panel::select_row(&mut app, pane, "long.txt")?;
    let done = rename(&mut app, &longest, &[])?;
    if done
        .progress
        .as_ref()
        .is_none_or(|progress| progress.state != OperationState::Completed)
        || !folder.join(&longest).is_file()
    {
        failures.push(format!(
            "F2 to a 255-byte name: {:?}",
            done.progress
                .as_ref()
                .map(|progress| (progress.state, &progress.errors))
        ));
    } else {
        support::wait_row(&mut app, pane, &longest, true);
        let too_long = format!("{}.txt", "m".repeat(252));
        panel::select_row(&mut app, pane, &longest)?;
        let done = rename(&mut app, &too_long, &[])?;
        let notice = done.notice.clone();
        let refused = done.still_open
            || done
                .progress
                .as_ref()
                .is_some_and(|progress| progress.state != OperationState::Completed);
        if !refused || !folder.join(&longest).is_file() {
            failures.push(format!(
                "F2 to a 256-byte name was not refused: {:?}",
                done.progress.as_ref().map(|progress| progress.state)
            ));
        } else if done
            .progress
            .as_ref()
            .is_some_and(|progress| progress.errors.is_empty())
            && notice.is_none()
            && done.warning.is_empty()
        {
            failures.push("F2 to a 256-byte name failed without telling why".into());
        }
        context.note(format!(
            "F2 to a 256-byte name: dialog open {}, warning {:?}, report {:?}, notice {notice:?}",
            done.still_open,
            done.warning,
            done.progress.as_ref().map(|progress| (
                progress.state,
                progress.errors.first().map(ToString::to_string)
            ))
        ));
        support::dismiss_all(&mut app);
    }

    // A folder keeps its contents.
    panel::select_row(&mut app, pane, "folder")?;
    let done = rename(&mut app, "renamed folder", &[])?;
    if done
        .progress
        .as_ref()
        .is_none_or(|progress| progress.state != OperationState::Completed)
        || read(&folder.join("renamed folder/inside.txt")).as_deref() != Some("inside")
    {
        failures.push(format!(
            "F2 of a folder: {:?}",
            done.progress.as_ref().map(|progress| progress.state)
        ));
    } else {
        let shown = support::row_delay(&mut app, pane, "renamed folder", true, done.finished);
        timings.add("report to new name listed", shown);
        panel::wait_input(&mut app);
        if support::cursor_name(&app, pane).as_deref() != Some("renamed folder") {
            failures.push(format!(
                "after renaming the folder the cursor is on {:?}",
                support::cursor_name(&app, pane)
            ));
        }
    }
    app.screenshot(context, "after-renames", "After every F2 case");
    held_keys(&mut app, &mut failures)?;
    context.note(timings.summary());
    context.note(
        "SKIP: the status \"Renaming “a” to “b”…\" while a rename holds the panel, and \
         Escape releasing that hold: no seam holds a rename long enough to see it (decision A5)",
    );
    app.shutdown()?;
    support::report(&failures)
}

/// Keys typed right after confirming F2 wait until the new name is listed with the
/// cursor on it, then act from there: Down Down ends two rows below the renamed file.
fn held_keys(app: &mut App, failures: &mut Vec<String>) -> Result<(), String> {
    let pane = PaneId::Left;
    panel::select_row(app, pane, "b.txt")?;
    panel::wait_input(app);
    input::key(app.slint(), Key::F2);
    support::wait_operation_dialog(app, "the Rename dialog");
    input::replace_text(app.slint(), "b held.txt");
    // Enter and the two Downs in one burst, as a fast typist sends them.
    input::key(app.slint(), Key::Return);
    input::key(app.slint(), Key::DownArrow);
    input::key(app.slint(), Key::DownArrow);
    app.until("the rename and the held keys", |app| {
        app.names(PaneId::Left)
            .iter()
            .any(|name| name == "b held.txt")
            && !app.window.get_mutation_input_blocked()
    });
    app.idle(Duration::from_millis(300));
    let names = app.names(pane);
    let renamed = names.iter().position(|name| name == "b held.txt");
    let wanted = renamed.and_then(|index| names.get(index + 2)).cloned();
    let cursor = support::cursor_name(app, pane);
    if wanted.is_none() || cursor != wanted {
        failures.push(format!(
            "Down Down typed right after F2's Enter left the cursor on {cursor:?}, expected two rows below the renamed b held.txt ({wanted:?}); rows {names:?}"
        ));
    }
    Ok(())
}
