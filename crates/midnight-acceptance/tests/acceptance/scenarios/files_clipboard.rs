//! Clipboard and drag and drop between the panels.
//!
//! A headless run has no desktop clipboard: `Config::clipboard` installs an in-process
//! one (`app::MemoryClipboard`) behind the application's clipboard service, so Ctrl+C,
//! Ctrl+X and Ctrl+V run the application's own capture, intent and paste flows. Drag
//! and drop between the panels needs no desktop service at all: the rows are dragged
//! with real pointer events (`panel::drag_row`).
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Content, facts, io_error, tree_facts},
    input, panel,
};
use midnight_core::{CopyOperationState, FileClipboardIntent, PaneId};
use slint::platform::Key;
use std::{fs, path::Path, time::Duration};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;

/// Drag feedback states (`MainWindow.drag-feedback-state`).
const COPY: i32 = 3;
const MOVE: i32 = 4;
const REFUSED: i32 = 5;

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|error| io_error(path, error))
}

/// Wait until a Copy/Move started after `start` ended, confirming its dialog if the
/// action opens one; its last report.
fn finish(
    app: &mut App,
    start: usize,
    what: &str,
) -> Result<std::sync::Arc<midnight_core::CopyProgress>, String> {
    app.until(&format!("{what} to start"), move |app| {
        app.copies.len() > start
            || !app.window.get_copy_draft_token().is_empty()
            || support::shown_notice(app).is_some()
    });
    if !app.window.get_copy_draft_token().is_empty() {
        input::key(app.slint(), Key::Return);
        app.settle();
        app.until(&format!("{what} to be admitted"), move |app| {
            app.copies.len() > start || support::shown_notice(app).is_some()
        });
    }
    if app.copies.len() == start {
        return Err(format!(
            "{what} started no job; notice {:?}",
            support::shown_notice(app)
        ));
    }
    let progress = support::wait_job(app, start);
    app.settle();
    support::dismiss_all(app);
    Ok(progress)
}

fn completed(progress: &midnight_core::CopyProgress, what: &str) -> Result<(), String> {
    if progress.state == CopyOperationState::Completed && progress.failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "{what} ended {:?} (failed {}, skipped {})",
            progress.state, progress.failed, progress.skipped
        ))
    }
}

pub fn clipboard_and_drag(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "source")?;
    let target = support::scratch(context, "target")?;
    for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt"] {
        write(&source.join(name), &format!("{name} body\n"))?;
    }
    let tree = source.join("tree");
    fs::create_dir_all(tree.join("inner")).map_err(|error| io_error(&tree, error))?;
    write(&tree.join("top.txt"), "top\n")?;
    write(&tree.join("inner/deep.txt"), "deep\n")?;
    fs::create_dir(target.join("into")).map_err(|error| io_error(&target, error))?;
    let mut config = Config::new(&source, &target);
    config.clipboard = true;
    let mut app = App::start(config);
    let clipboard = app.clipboard.clone().ok_or("no clipboard installed")?;
    let mut failures = Vec::new();

    // Ctrl+C of two marked files: the clipboard holds them, to be copied.
    panel::select_row(&mut app, LEFT, "a.txt")?;
    input::key(app.slint(), Key::Insert);
    panel::select_row(&mut app, LEFT, "b.txt")?;
    input::key(app.slint(), Key::Insert);
    input::chord(app.slint(), &[Key::Control], "c");
    let wanted = vec![source.join("a.txt"), source.join("b.txt")];
    let held = clipboard.clone();
    app.until_for(
        "Ctrl+C to fill the clipboard",
        Duration::from_secs(10),
        move |_| held.files().is_some(),
    );
    match clipboard.files() {
        Some(files) if files.intent == FileClipboardIntent::Copy && files.paths == wanted => {}
        other => failures.push(format!(
            "Ctrl+C put {other:?} on the clipboard instead of a.txt and b.txt to copy"
        )),
    }
    // Ctrl+V in the other panel copies them there; the sources stay.
    panel::focus_list(&mut app, RIGHT)?;
    let start = app.copies.len();
    input::chord(app.slint(), &[Key::Control], "v");
    match finish(&mut app, start, "Ctrl+V of the copied files") {
        Err(error) => failures.push(error),
        Ok(progress) => {
            if let Err(error) = completed(&progress, "the pasted copy") {
                failures.push(error);
            }
            for name in ["a.txt", "b.txt"] {
                if facts(&target.join(name)).map(|found| found.content).ok()
                    != facts(&source.join(name)).map(|found| found.content).ok()
                    || !source.join(name).is_file()
                {
                    failures.push(format!(
                        "after Ctrl+C, Ctrl+V {name} is not a copy beside its kept source"
                    ));
                }
            }
            support::wait_row(&mut app, RIGHT, "b.txt", true);
        }
    }
    app.screenshot(
        context,
        "pasted-copy",
        "Ctrl+C in the left panel, Ctrl+V in the right",
    );

    // Ctrl+X of a file, Ctrl+V in the other panel: it moves.
    let before = facts(&source.join("c.txt"))?;
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    panel::select_row(&mut app, LEFT, "c.txt")?;
    input::chord(app.slint(), &[Key::Control], "x");
    let held = clipboard.clone();
    app.until_for(
        "Ctrl+X to fill the clipboard",
        Duration::from_secs(10),
        move |_| {
            held.files()
                .is_some_and(|files| files.intent == FileClipboardIntent::Move)
        },
    );
    panel::focus_list(&mut app, RIGHT)?;
    let start = app.copies.len();
    input::chord(app.slint(), &[Key::Control], "v");
    match finish(&mut app, start, "Ctrl+V of the cut file") {
        Err(error) => failures.push(error),
        Ok(progress) => {
            if let Err(error) = completed(&progress, "the pasted move") {
                failures.push(error);
            }
            if fs::symlink_metadata(source.join("c.txt")).is_ok() {
                failures.push("Ctrl+X, Ctrl+V left c.txt in the source folder".into());
            }
            match facts(&target.join("c.txt")) {
                Ok(after) if after == before => {}
                other => failures.push(format!(
                    "the moved c.txt is {other:?} instead of {before:?}"
                )),
            }
            support::wait_row(&mut app, LEFT, "c.txt", false);
        }
    }
    context.note(format!(
        "clipboard after the cut was pasted: {:?}",
        clipboard.files()
    ));

    // Drag d.txt to the other panel without modifiers: the same volume moves it.
    let before = facts(&source.join("d.txt"))?;
    let start = app.copies.len();
    let spot = panel::list_spot(&app, RIGHT);
    match panel::drag_row(&mut app, LEFT, "d.txt", spot, &[]) {
        Err(error) => failures.push(error),
        Ok((state, label)) => {
            context.note(format!("drag of d.txt: feedback {state} {label:?}"));
            if state != MOVE {
                failures.push(format!(
                    "a plain drag on one volume offered {state} ({label:?}), not Move"
                ));
            }
            match finish(&mut app, start, "the dropped d.txt") {
                Err(error) => failures.push(error),
                Ok(progress) => {
                    if let Err(error) = completed(&progress, "the drop of d.txt") {
                        failures.push(error);
                    }
                    if fs::symlink_metadata(source.join("d.txt")).is_ok()
                        || facts(&target.join("d.txt")).ok().as_ref() != Some(&before)
                    {
                        failures.push("the dragged d.txt was not moved to the right panel".into());
                    }
                }
            }
        }
    }

    // Ctrl+drag copies.
    let start = app.copies.len();
    let spot = panel::list_spot(&app, RIGHT);
    match panel::drag_row(&mut app, LEFT, "e.txt", spot, &[Key::Control]) {
        Err(error) => failures.push(error),
        Ok((state, label)) => {
            context.note(format!("Ctrl+drag of e.txt: feedback {state} {label:?}"));
            if state != COPY {
                failures.push(format!("Ctrl+drag offered {state} ({label:?}), not Copy"));
            }
            match finish(&mut app, start, "the Ctrl+dropped e.txt") {
                Err(error) => failures.push(error),
                Ok(progress) => {
                    if let Err(error) = completed(&progress, "the drop of e.txt") {
                        failures.push(error);
                    }
                    if !source.join("e.txt").is_file()
                        || facts(&target.join("e.txt")).map(|found| found.content).ok()
                            != facts(&source.join("e.txt")).map(|found| found.content).ok()
                    {
                        failures.push("Ctrl+drag did not copy e.txt beside its source".into());
                    }
                }
            }
        }
    }

    // A folder dragged onto a folder row with Ctrl lands inside that folder.
    let expected = tree_facts(&tree)?;
    let start = app.copies.len();
    match panel::row_center(&app, RIGHT, "into") {
        None => failures.push("the right panel shows no into/ row to drop on".into()),
        Some(onto) => match panel::drag_row(&mut app, LEFT, "tree", onto, &[Key::Control]) {
            Err(error) => failures.push(error),
            Ok((state, label)) => {
                context.note(format!(
                    "Ctrl+drag of tree onto into/: feedback {state} {label:?}"
                ));
                match finish(&mut app, start, "the folder dropped onto into/") {
                    Err(error) => failures.push(error),
                    Ok(progress) => {
                        if let Err(error) = completed(&progress, "the folder drop") {
                            failures.push(error);
                        }
                        let copied = tree_facts(&target.join("into/tree"))?;
                        let differences = support::compare_trees(&expected, &copied, false);
                        if !differences.is_empty() {
                            failures
                                .push(format!("tree/ dropped onto into/ differs: {differences:?}"));
                        }
                    }
                }
            }
        },
    }

    // Ctrl+Shift is not a drag action: the drop does nothing.
    let start = app.copies.len();
    let spot = panel::list_spot(&app, RIGHT);
    match panel::drag_row(&mut app, LEFT, "f.txt", spot, &[Key::Control, Key::Shift]) {
        Err(error) => failures.push(error),
        Ok((state, label)) => {
            app.idle(Duration::from_millis(500));
            context.note(format!(
                "Ctrl+Shift+drag of f.txt: feedback {state} {label:?}"
            ));
            if state != REFUSED || app.copies.len() != start {
                failures.push(format!(
                    "Ctrl+Shift+drag offered {state} ({label:?}) and started {} job(s)",
                    app.copies.len() - start
                ));
            }
            if !source.join("f.txt").is_file() || target.join("f.txt").exists() {
                failures.push("a refused drop changed f.txt".into());
            }
        }
    }
    app.screenshot(context, "after-drops", "After the drops between the panels");
    let kinds: Vec<Content> = ["a.txt", "e.txt"]
        .iter()
        .filter_map(|name| facts(&source.join(name)).ok().map(|found| found.content))
        .collect();
    context.note(format!(
        "sources kept by Copy: {} of 2 (a.txt, e.txt)",
        kinds.len()
    ));
    app.shutdown()?;
    support::report(&failures)
}
