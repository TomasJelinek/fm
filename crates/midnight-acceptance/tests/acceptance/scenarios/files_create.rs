//! F7 New folder and Shift+F4 New file: nested path segments, a path outside the
//! panel's folder, existing names (F7 refuses, Shift+F4 asks before replacing),
//! cancelling, and names with leading and trailing spaces. The created entry is listed
//! at once with the cursor on it.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Timings, io_error},
    input, panel,
};
use midnight_core::{OperationState, PaneId};
use slint::platform::Key;
use std::{
    fs,
    time::{Duration, Instant},
};

pub fn create_cases(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "create")?;
    fs::write(folder.join("existing.txt"), "keep\n").map_err(|error| io_error(&folder, error))?;
    fs::create_dir(folder.join("existing-folder")).map_err(|error| io_error(&folder, error))?;
    let mut app = App::start(Config::new(&folder, &context.work));
    let pane = PaneId::Left;
    let mut failures = Vec::new();
    let mut timings = Timings::default();
    panel::focus_list(&mut app, pane)?;

    // F7 with one name: the new folder is listed with the cursor on it.
    let start = support::named_operation(&mut app, &[], Key::F7, "single")?;
    let progress = support::wait_operation(&mut app, start);
    let finished = Instant::now();
    if let Err(error) = support::operation_ok(&progress, "F7 single") {
        failures.push(error);
    } else {
        timings.add(
            "report to row listed",
            support::row_delay(&mut app, pane, "single", true, finished),
        );
        panel::wait_input(&mut app);
        if support::cursor_name(&app, pane).as_deref() != Some("single") {
            failures.push(format!(
                "after F7 single the cursor is on {:?}, not the new folder",
                support::cursor_name(&app, pane)
            ));
        }
    }

    // F7 with nested segments creates every missing folder; the cursor should go to
    // the listed ancestor of the new path.
    let start = support::named_operation(&mut app, &[], Key::F7, "alpha/beta/gamma")?;
    let progress = support::wait_operation(&mut app, start);
    let finished = Instant::now();
    if let Err(error) = support::operation_ok(&progress, "F7 alpha/beta/gamma") {
        failures.push(error);
    } else if !folder.join("alpha/beta/gamma").is_dir() {
        failures.push("F7 alpha/beta/gamma did not create the nested folders".into());
    } else {
        timings.add(
            "report to row listed",
            support::row_delay(&mut app, pane, "alpha", true, finished),
        );
        panel::wait_input(&mut app);
        let cursor = support::cursor_name(&app, pane);
        context.note(format!(
            "after F7 alpha/beta/gamma the cursor is on {cursor:?}"
        ));
        if cursor.as_deref() != Some("alpha") {
            failures.push(format!(
                "[AF-5] after F7 alpha/beta/gamma the cursor is on {cursor:?}, not on the created alpha"
            ));
        }
    }

    // F7 on an existing folder name is refused and changes nothing.
    let start = support::named_operation(&mut app, &[], Key::F7, "existing-folder")?;
    app.idle(Duration::from_millis(500));
    let report = app.operations[start..]
        .iter()
        .find(|p| p.state.is_terminal())
        .cloned();
    let notice = support::shown_notice(&app);
    let open = !app.window.get_operation_token().is_empty();
    let warning = app.window.get_operation_warning().to_string();
    if report
        .as_ref()
        .is_some_and(|progress| progress.state == OperationState::Completed)
    {
        failures.push("F7 on an existing folder name reported success".into());
    }
    if report.is_none() && !open && notice.is_none() {
        failures.push("F7 on an existing folder name gave no feedback".into());
    }
    context.note(format!(
        "F7 existing-folder: dialog open {open}, warning {warning:?}, report {:?}, notice {notice:?}",
        report.as_ref().map(|progress| (
            progress.state,
            progress.errors.first().map(ToString::to_string)
        ))
    ));
    app.screenshot(context, "mkdir-existing", "F7 with an existing folder name");
    support::dismiss_all(&mut app);

    // Escape cancels F7 and Shift+F4 without creating anything.
    for (modifiers, key, name) in [
        (&[][..], Key::F7, "never-folder"),
        (&[Key::Shift][..], Key::F4, "never-file.txt"),
    ] {
        panel::wait_input(&mut app);
        input::chord(app.slint(), modifiers, key);
        support::wait_operation_dialog(&mut app, "the name dialog");
        input::replace_text(app.slint(), name);
        input::key(app.slint(), Key::Escape);
        app.until("the dialog to close", |app| {
            app.window.get_operation_token().is_empty()
        });
        app.idle(Duration::from_millis(200));
        if folder.join(name).exists() {
            failures.push(format!("Escape still created {name}"));
        }
    }

    // A relative path above the panel's folder.
    let outside = context.work.join("created-outside");
    let start = support::named_operation(&mut app, &[], Key::F7, "../created-outside")?;
    let progress = support::wait_operation(&mut app, start);
    if let Err(error) = support::operation_ok(&progress, "F7 ../created-outside") {
        failures.push(error);
    } else if !outside.is_dir() {
        failures.push("F7 ../created-outside created nothing above the panel folder".into());
    }

    // Shift+F4 with nested segments.
    let start = support::named_operation(&mut app, &[Key::Shift], Key::F4, "sub/dir/new.txt")?;
    app.idle(Duration::from_millis(300));
    let nested = if app.window.get_operation_token().is_empty() {
        let progress = support::wait_operation(&mut app, start);
        format!(
            "{:?} {:?}; sub/dir/new.txt is a file: {}",
            progress.state,
            progress.errors.first().map(ToString::to_string),
            folder.join("sub/dir/new.txt").is_file()
        )
    } else {
        format!(
            "dialog stayed open: {:?}",
            app.window.get_operation_warning().to_string()
        )
    };
    context.note(format!("Shift+F4 sub/dir/new.txt: {nested}"));
    support::dismiss_all(&mut app);
    if folder.join("sub/dir/new.txt").exists()
        && fs::metadata(folder.join("sub/dir/new.txt")).map_or(true, |metadata| metadata.len() != 0)
    {
        failures.push("Shift+F4 sub/dir/new.txt created a non-empty file".into());
    }

    // Shift+F4 on an existing file asks; Cancel keeps it.
    let start = support::named_operation(&mut app, &[Key::Shift], Key::F4, "existing.txt")?;
    app.settle();
    let warning = app.window.get_operation_warning().to_string();
    if app.window.get_operation_token().is_empty() || !warning.contains("already exists") {
        failures.push(format!(
            "Shift+F4 on an existing name did not ask: dialog open {}, warning {warning:?}",
            !app.window.get_operation_token().is_empty()
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.until("the dialog to close", |app| {
        app.window.get_operation_token().is_empty()
    });
    app.idle(Duration::from_millis(300));
    if app.operations[start..]
        .iter()
        .any(|progress| progress.state == OperationState::Completed)
        || fs::read(folder.join("existing.txt")).ok().as_deref() != Some(b"keep\n".as_slice())
    {
        failures.push("cancelling the replace question changed existing.txt".into());
    }

    // Shift+F4 on an existing file, confirmed with the dialog's Replace file button: the
    // file is emptied. A notice naming a recovery copy, when shown, must point at the
    // original bytes.
    let original = b"original bytes\n";
    fs::write(folder.join("replace-me.txt"), original).map_err(|error| io_error(&folder, error))?;
    support::wait_row(&mut app, pane, "replace-me.txt", true);
    let start = support::named_operation(&mut app, &[Key::Shift], Key::F4, "replace-me.txt")?;
    app.until("the replace question", |app| {
        app.window
            .get_operation_warning()
            .to_string()
            .contains("already exists")
    });
    input::click_button(app.slint(), "Replace file")?;
    let replaced = folder.join("replace-me.txt");
    app.until("the replacement", move |app| {
        app.operations[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
            || fs::metadata(&replaced).is_ok_and(|metadata| metadata.len() == 0)
    });
    app.idle(Duration::from_millis(300));
    if fs::metadata(folder.join("replace-me.txt")).map_or(true, |metadata| metadata.len() != 0) {
        failures.push("Replace file did not empty replace-me.txt".into());
    }
    match support::take_notice(&mut app) {
        None => context.note("Shift+F4 Replace file: replaced, no notice"),
        Some((title, body)) => {
            // The notice names the recovery copy on a line of its own.
            let recovery = body
                .lines()
                .map(str::trim)
                .find(|line| line.contains(".midnight-recovery-"))
                .map(str::to_owned);
            match recovery {
                Some(path) if fs::read(&path).ok().as_deref() == Some(original.as_slice()) => {
                    context.note(format!(
                        "Shift+F4 Replace file: notice {title:?}; the recovery copy holds the original bytes"
                    ));
                }
                other => failures.push(format!(
                    "the replace notice names no readable recovery copy with the original bytes ({other:?}): {title:?} {body:?}"
                )),
            }
        }
    }

    // Leading and trailing spaces are kept exactly.
    let spaced = "  spaced name .txt";
    let start = support::named_operation(&mut app, &[Key::Shift], Key::F4, spaced)?;
    let progress = support::wait_operation(&mut app, start);
    let finished = Instant::now();
    let on_disk = support::names_on_disk(&folder);
    if let Err(error) = support::operation_ok(&progress, "Shift+F4 with spaces") {
        failures.push(error);
    } else if on_disk.iter().any(|name| name == spaced) {
        timings.add(
            "report to row listed",
            support::row_delay(&mut app, pane, spaced, true, finished),
        );
        panel::wait_input(&mut app);
        if support::cursor_name(&app, pane).as_deref() != Some(spaced) {
            failures.push(format!(
                "after Shift+F4 the cursor is on {:?}, not the new file",
                support::cursor_name(&app, pane)
            ));
        }
    } else {
        context.note(format!(
            "Shift+F4 {spaced:?} created {:?} (spaces not kept exactly)",
            on_disk
                .iter()
                .filter(|name| name.contains("spaced"))
                .collect::<Vec<_>>()
        ));
    }
    app.screenshot(context, "after-create", "After the F7 and Shift+F4 cases");
    context.note(timings.summary());
    app.shutdown()?;
    support::report(&failures)
}
