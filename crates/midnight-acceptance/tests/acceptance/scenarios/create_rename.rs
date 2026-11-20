//! Shift+F4 new empty file (including the existing-name prompt), F7 new folder and F2
//! rename, typed into the real dialogs and verified on disk and in the panel.
use crate::{
    Context,
    app::{App, Config},
    input, inspect, panel,
};
use midnight_core::{OperationState, PaneId};
use slint::platform::Key;
use std::fs;

/// Wait for the operation dialog the Engine's preview opens.
#[track_caller]
fn wait_dialog(app: &mut App, what: &str) {
    app.until(what, |app| !app.window.get_operation_token().is_empty());
}

#[track_caller]
fn wait_finished(app: &mut App, start: usize) -> Result<OperationState, String> {
    wait_done(app, start, |_| false)
}

/// Wait for the operation's terminal report, or for `done` to show its effect (not
/// every operation kind needs to report progress); a terminal report other than
/// Completed is returned as such.
#[track_caller]
fn wait_done(
    app: &mut App,
    start: usize,
    done: impl Fn(&App) -> bool,
) -> Result<OperationState, String> {
    app.until("the operation to finish", move |app| {
        app.operations[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
            || done(app)
    });
    Ok(app.operations[start..]
        .iter()
        .find(|progress| progress.state.is_terminal())
        .map_or(OperationState::Completed, |progress| progress.state))
}

fn cursor_name(app: &App, pane: PaneId) -> Option<String> {
    let names = app.names(pane);
    usize::try_from(app.cursor(pane))
        .ok()
        .and_then(|row| names.get(row).cloned())
}

pub fn create_rename(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("operations");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    fs::write(folder.join("rename-me.txt"), "rename me\n").map_err(|error| error.to_string())?;
    fs::write(folder.join("keep.txt"), "keep\n").map_err(|error| error.to_string())?;
    let mut app = App::start(Config::new(&folder, &context.work));
    let pane = PaneId::Left;
    panel::focus_list(&mut app, pane)?;

    // Shift+F4: a new empty file with a Unicode name.
    let name = "nový soubor 文档 🦀.txt";
    let start = app.operations.len();
    input::chord(app.slint(), &[Key::Shift], Key::F4);
    wait_dialog(&mut app, "the New empty file dialog");
    app.screenshot(context, "new-file-dialog", "Shift+F4 New empty file");
    input::type_text(app.slint(), name);
    input::key(app.slint(), Key::Return);
    let created = folder.join(name);
    let state = wait_done(&mut app, start, move |_| created.is_file())?;
    if state != OperationState::Completed {
        return Err(format!("Shift+F4 ended {state:?}"));
    }
    let wanted = name.to_owned();
    app.until("the new file's row", move |app| {
        app.names(pane).contains(&wanted)
    });
    let metadata =
        fs::metadata(folder.join(name)).map_err(|error| format!("the new file: {error}"))?;
    if !metadata.is_file() || metadata.len() != 0 {
        return Err(format!("the new file is not an empty file: {metadata:?}"));
    }
    panel::wait_input(&mut app);
    if cursor_name(&app, pane).as_deref() != Some(name) {
        context.note(format!(
            "after Shift+F4 the cursor is on {:?}, not the new file",
            cursor_name(&app, pane)
        ));
    }

    // Shift+F4 with the name of an existing, non-empty file asks before replacing it.
    fs::write(folder.join("keep.txt"), "keep\n").map_err(|error| error.to_string())?;
    let start = app.operations.len();
    input::chord(app.slint(), &[Key::Shift], Key::F4);
    wait_dialog(&mut app, "the New empty file dialog");
    input::type_text(app.slint(), "keep.txt");
    input::key(app.slint(), Key::Return);
    app.settle();
    if app.window.get_operation_token().is_empty() {
        return Err("Shift+F4 on an existing name closed the dialog without asking".into());
    }
    let warning = app.window.get_operation_warning().to_string();
    if !warning.contains("already exists") {
        return Err(format!(
            "no replace warning for an existing name: {warning:?}"
        ));
    }
    app.screenshot(
        context,
        "new-file-replace-prompt",
        "Shift+F4 on an existing name",
    );
    // The open dialog now asks the replace question: title and confirm button say
    // Replace file, and no unrelated error accompanies it.
    let shown = inspect::visible_texts(app.slint());
    if let Some(text) = shown.iter().find(|text| text.contains("queue is busy")) {
        return Err(format!("the replace question also shows {text:?}"));
    }
    let replace_captions = shown
        .iter()
        .filter(|text| text.as_str() == "Replace file")
        .count();
    if replace_captions < 2 {
        return Err(format!(
            "the dialog's title and confirm button do not both say Replace file; it shows {shown:?}"
        ));
    }
    if fs::read(folder.join("keep.txt")).map_err(|error| error.to_string())? != b"keep\n" {
        return Err("the existing file changed before the replacement was confirmed".into());
    }
    input::key(app.slint(), Key::Return);
    let kept = folder.join("keep.txt");
    let state = wait_done(&mut app, start, move |_| {
        fs::metadata(&kept).is_ok_and(|metadata| metadata.len() == 0)
    })?;
    if state != OperationState::Completed {
        return Err(format!("confirmed replacement ended {state:?}"));
    }
    let replaced = fs::metadata(folder.join("keep.txt")).map_err(|error| error.to_string())?;
    if replaced.len() != 0 {
        return Err(format!(
            "the confirmed replacement left {} bytes",
            replaced.len()
        ));
    }

    // F7: a new folder.
    panel::wait_input(&mut app);
    let start = app.operations.len();
    input::key(app.slint(), Key::F7);
    wait_dialog(&mut app, "the Create directory dialog");
    app.screenshot(context, "create-folder-dialog", "F7 Create directory");
    input::replace_text(app.slint(), "Nová složka");
    input::key(app.slint(), Key::Return);
    let created = folder.join("Nová složka");
    let state = wait_done(&mut app, start, move |_| created.is_dir())?;
    if state != OperationState::Completed || !folder.join("Nová složka").is_dir() {
        return Err(format!(
            "F7 ended {state:?}; folder exists: {}",
            folder.join("Nová složka").is_dir()
        ));
    }
    app.until("the new folder's row", |app| {
        app.names(PaneId::Left)
            .iter()
            .any(|row| row == "Nová složka")
    });

    // F2: the stem is preselected; typing replaces it and keeps the extension.
    panel::select_row(&mut app, pane, "rename-me.txt")?;
    let start = app.operations.len();
    input::key(app.slint(), Key::F2);
    wait_dialog(&mut app, "the Rename dialog");
    app.screenshot(context, "rename-dialog", "F2 Rename with the stem selected");
    input::type_text(app.slint(), "přejmenováno 名前");
    input::key(app.slint(), Key::Return);
    let new_name = "přejmenováno 名前.txt";
    let state = wait_finished(&mut app, start)?;
    if state != OperationState::Completed {
        return Err(format!("F2 ended {state:?}"));
    }
    // The completion itself must show the new name; no watch notification is needed.
    let wanted = new_name.to_owned();
    app.until("the renamed row", move |app| {
        app.names(pane).contains(&wanted)
    });
    if app.names(pane).iter().any(|row| row == "rename-me.txt") {
        return Err("the old name is still listed after the rename".into());
    }
    panel::wait_input(&mut app);
    if cursor_name(&app, pane).as_deref() != Some(new_name) {
        return Err(format!(
            "after F2 the cursor is on {:?}, not the renamed file",
            cursor_name(&app, pane)
        ));
    }
    if !folder.join(new_name).is_file() || folder.join("rename-me.txt").exists() {
        return Err("the rename is not on disk".into());
    }
    app.screenshot(context, "after-operations", "After Shift+F4, F7 and F2");
    context.note("Shift+F4 created an empty Unicode-named file, asked before replacing keep.txt and emptied it on confirmation; F7 created a folder; F2 renamed with the stem preselected");
    app.shutdown()
}
