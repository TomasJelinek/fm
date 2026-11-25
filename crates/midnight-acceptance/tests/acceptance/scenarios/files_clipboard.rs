//! Clipboard and drag and drop between the panels, F5 with marked files during a
//! filename search, more selected items than the 64 a Move once took (F5 of 65 copied,
//! Ctrl+X, Ctrl+V of them moved, F6 of them moved back), drags across volumes (a Move refused up
//! front where the volume cannot keep a replaced file recoverable) and, with the scenario
//! clock held, a drop released within 0.4 s of the press cancelled.
//!
//! A headless run has no desktop clipboard: `Config::clipboard` installs an in-process
//! one (`app::MemoryClipboard`) behind the application's clipboard service, so Ctrl+C,
//! Ctrl+X and Ctrl+V run the application's own capture, intent and paste flows. Drag
//! and drop between the panels needs no desktop service at all: the rows are dragged
//! with real pointer events (`panel::drag_row`).
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Content, TransferSpec, facts, io_error, tree_facts},
    input, inspect, panel,
};
use midnight_core::{CopyOperationState, FileClipboardIntent, PaneId};
use slint::platform::{Key, WindowAdapter};
use std::{fs, path::Path, time::Duration};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;

/// Drag feedback states (`MainWindow.drag-feedback-state`).
const COPY: i32 = 3;
const MOVE: i32 = 4;
const REFUSED: i32 = 5;
/// The most items one Move once took: the selection below is one more.
const LIMIT: usize = 64;

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|error| io_error(path, error))
}

/// Wait until a Copy/Move started after `start` ended, confirming its dialog if the
/// action opens one and answering each question with `answer`; its last report and the
/// questions asked.
fn finish_answering(
    app: &mut App,
    start: usize,
    what: &str,
    answer: &support::Choice,
) -> Result<(std::sync::Arc<midnight_core::CopyProgress>, Vec<String>), String> {
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
    let mut asked = Vec::new();
    loop {
        app.until(&format!("{what} to end or ask"), move |app| {
            support::job(app, start).is_some_and(|job| job.state.is_terminal())
                || support::decision_window(app).is_some()
        });
        if let Some(progress) = support::job(app, start).filter(|job| job.state.is_terminal()) {
            app.settle();
            support::dismiss_all(app);
            return Ok((progress, asked));
        }
        let window = support::wait_decision(app, "the question");
        asked.push(support::describe_window(&window));
        let token = app.window.get_copy_token();
        support::answer(app, &window, answer)?;
        let answered = window.clone();
        app.until("the answer to be taken", move |app| {
            !answered.window().is_visible() || app.window.get_copy_token() != token
        });
    }
}

/// [`finish_answering`] for a job that must not ask: a question is answered with Cancel
/// job and reported.
fn finish(
    app: &mut App,
    start: usize,
    what: &str,
) -> Result<std::sync::Arc<midnight_core::CopyProgress>, String> {
    let (progress, asked) = finish_answering(app, start, what, &support::Choice::CancelJob)?;
    if !asked.is_empty() {
        return Err(format!("{what} asked {asked:?} (answered Cancel job)"));
    }
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
    for name in ["qa-one.txt", "qa-two.txt", "qb-match.txt"] {
        write(&source.join(name), &format!("{name} body\n"))?;
    }
    let many = source.join("many65");
    fs::create_dir(&many).map_err(|error| io_error(&many, error))?;
    for index in 0..LIMIT + 1 {
        write(
            &many.join(format!("m{index:03}.txt")),
            &format!("{index}\n"),
        )?;
    }
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

    // A filename search with two marked files: F5 closes the search and copies both
    // marked files, not the search's match.
    quick_search_copy(&mut app, context, &source, &target, &mut failures)?;

    // More items than a Move once took: F5 copies all 65, Ctrl+X, Ctrl+V moves them all
    // and F6 moves them all back.
    selection_limits(&mut app, context, &source, &target, &mut failures)?;

    // Escape during a drag drops nothing; a drop on a file row lands in that panel's
    // folder.
    drag_variants(&mut app, context, &source, &target, &mut failures)?;
    // A drop released within 0.4 s of the press is taken for an accident.
    quick_release(&mut app, context, &source, &target, &mut failures)?;
    let kinds: Vec<Content> = ["a.txt", "e.txt"]
        .iter()
        .filter_map(|name| facts(&source.join(name)).ok().map(|found| found.content))
        .collect();
    context.note(format!(
        "sources kept by Copy: {} of 2 (a.txt, e.txt)",
        kinds.len()
    ));
    app.shutdown()?;
    // Across volumes a plain drag copies and Shift+drag moves.
    cross_volume_drags(context, &mut failures)?;
    support::report(&failures)
}

/// Drag `name` from the left panel towards `to` until the drop feedback settles, press
/// Escape, then release: the drag must end there, with nothing dropped.
fn escaped_drag(app: &mut App, name: &str, to: slint::LogicalPosition) -> Result<(), String> {
    use slint::platform::{PointerEventButton, WindowEvent};
    panel::wait_input(app);
    let from = panel::row_center(app, LEFT, name).ok_or_else(|| format!("no {name} row"))?;
    let button = PointerEventButton::Left;
    input::move_to(app.slint(), from);
    app.slint().dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    app.idle(Duration::from_millis(150));
    for step in 1..=24 {
        let t = step as f32 / 24.0;
        input::move_to(
            app.slint(),
            slint::LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t),
        );
        app.idle(Duration::from_millis(16));
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !matches!(app.window.get_drag_feedback_state(), 3..=5)
        && std::time::Instant::now() < deadline
    {
        input::move_to(app.slint(), to);
        app.idle(Duration::from_millis(30));
    }
    let offered = app.window.get_drag_feedback_state();
    input::key(app.slint(), Key::Escape);
    app.settle();
    let active = app.window.get_drag_active();
    app.slint().dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    app.settle();
    if !matches!(offered, 3..=5) || active {
        return Err(format!(
            "the drag offered {offered} and was still active ({active}) after Escape"
        ));
    }
    Ok(())
}

/// Escape during a drag drops nothing; a Ctrl+drop on a file row of the other panel
/// copies into that panel's folder, beside the file.
fn drag_variants(
    app: &mut App,
    context: &mut Context,
    source: &Path,
    target: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    for name in ["drag-escape.txt", "drag-onto.txt"] {
        write(&source.join(name), &format!("{name}\n"))?;
    }
    support::wait_row(app, LEFT, "drag-escape.txt", true);
    support::wait_row(app, LEFT, "drag-onto.txt", true);
    let start = app.copies.len();
    let spot = panel::list_spot(app, RIGHT);
    match escaped_drag(app, "drag-escape.txt", spot) {
        Err(error) => failures.push(format!("Escape during a drag: {error}")),
        Ok(()) => {
            // Nothing to wait for: a drop shows its dialog or job within moments (the
            // drops above do), so 2 s without one is an observation window, not a delay.
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while app.copies.len() == start
                && app.window.get_copy_draft_token().is_empty()
                && std::time::Instant::now() < deadline
            {
                app.idle(Duration::from_millis(20));
            }
            if app.copies.len() != start
                || !app.window.get_copy_draft_token().is_empty()
                || target.join("drag-escape.txt").exists()
                || !source.join("drag-escape.txt").is_file()
            {
                failures.push(format!(
                    "Escape during a drag still dropped: {} job(s), dialog open {}",
                    app.copies.len() - start,
                    !app.window.get_copy_draft_token().is_empty()
                ));
                support::dismiss_all(app);
            }
        }
    }
    context.note("Escape during a drag of drag-escape.txt: nothing dropped");
    // b.txt was pasted into the right panel's folder earlier.
    match panel::row_center(app, RIGHT, "b.txt") {
        None => failures.push("the right panel shows no b.txt row to drop on".into()),
        Some(onto) => {
            let start = app.copies.len();
            match panel::drag_row(app, LEFT, "drag-onto.txt", onto, &[Key::Control]) {
                Err(error) => failures.push(error),
                Ok((state, label)) => {
                    context.note(format!(
                        "Ctrl+drag of drag-onto.txt onto the file b.txt: feedback {state} {label:?}"
                    ));
                    match finish(app, start, "the drop onto a file row") {
                        Err(error) => failures.push(error),
                        Ok(progress) => {
                            if let Err(error) = completed(&progress, "the drop onto a file row") {
                                failures.push(error);
                            }
                            if fs::read(target.join("drag-onto.txt")).ok()
                                != fs::read(source.join("drag-onto.txt")).ok()
                                || fs::read_to_string(target.join("b.txt")).ok().as_deref()
                                    != Some("b.txt body\n")
                            {
                                failures.push(
                                    "a drop onto the file b.txt did not copy into its folder \
                                     beside it"
                                        .into(),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// A folder on another volume than the work folder: `$XDG_RUNTIME_DIR` on Unix, the
/// private installation (`MIDNIGHT_ACCEPTANCE_STAGE_ROOT`) on Windows when the run is on
/// another drive.
fn other_volume(here: &Path) -> Result<std::path::PathBuf, &'static str> {
    let candidate = if cfg!(windows) {
        std::env::var_os("MIDNIGHT_ACCEPTANCE_STAGE_ROOT")
    } else {
        crate::fixtures::runtime_volume()
    }
    .map(std::path::PathBuf::from)
    .filter(|path| path.is_absolute() && path.is_dir())
    .ok_or("no folder on another volume")?;
    let volume = |path: &Path| support::identity(path).ok().map(|identity| identity.0);
    if volume(&candidate) == volume(here) {
        return Err("the other folder is on the run's volume");
    }
    Ok(candidate)
}

/// What the source panel's status line says after a drag released too soon.
const QUICK_NOTE: &str = "Drag cancelled: released within 0.4 s";

/// T-X13 (audit T14), with the scenario clock held: a plain drag of `quick.txt` to the
/// other panel released 0.25 s after the press is cancelled ([`QUICK_NOTE`] on the source
/// panel's status line) and starts no dialog or job; the same drag released 0.45 s after
/// the press moves the file (one volume: Move).
fn quick_release(
    app: &mut App,
    context: &mut Context,
    source: &Path,
    target: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    write(&source.join("quick.txt"), "quick.txt\n")?;
    support::wait_row(app, LEFT, "quick.txt", true);
    panel::wait_input(app);
    app.hold_clock();
    let result = held_drags(app, context, source, target, failures);
    app.resume_clock();
    result
}

/// Press on `name` in the left panel, let `before_move` of the held clock pass, drag to
/// the right panel's list and wait (in real time, the clock held) for the drop feedback,
/// let `after_move` pass, release. The feedback state and label at the release.
fn held_drag(
    app: &mut App,
    name: &str,
    before_move: Duration,
    after_move: Duration,
) -> Result<(i32, String), String> {
    use slint::platform::{PointerEventButton, WindowEvent};
    let from = panel::row_center(app, LEFT, name).ok_or_else(|| format!("no {name} row"))?;
    let to = panel::list_spot(app, RIGHT);
    let button = PointerEventButton::Left;
    input::move_to(app.slint(), from);
    app.slint().dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    app.settle();
    app.advance(before_move);
    for step in 1..=24 {
        let t = step as f32 / 24.0;
        input::move_to(
            app.slint(),
            slint::LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t),
        );
        app.settle();
    }
    // The engine prepares the drag and checks the destination asynchronously; real time
    // passing here does not move the held clock, so it is not part of the press.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !matches!(app.window.get_drag_feedback_state(), 3..=5)
        && std::time::Instant::now() < deadline
    {
        input::move_to(app.slint(), to);
        app.idle(Duration::from_millis(30));
    }
    let state = app.window.get_drag_feedback_state();
    let label = app.window.get_drag_label().to_string();
    app.advance(after_move);
    app.slint().dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    app.settle();
    Ok((state, label))
}

fn held_drags(
    app: &mut App,
    context: &mut Context,
    source: &Path,
    target: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let gone = |name: &str| !source.join(name).is_file();
    // Released 0.25 s after the press: nothing happens but the note. A drop shows its
    // dialog or job within moments, so 0.5 s without one is an observation window.
    let start = app.copies.len();
    let (state, label) = held_drag(
        app,
        "quick.txt",
        Duration::from_millis(150),
        Duration::from_millis(100),
    )?;
    app.idle(Duration::from_millis(500));
    let noted = inspect::visible_texts(app.slint())
        .iter()
        .any(|text| text.contains(QUICK_NOTE));
    let started = app.copies.len() - start;
    let dialog = !app.window.get_copy_draft_token().is_empty();
    context.note(format!(
        "a drag released 0.25 s after the press (feedback {state} {label:?}): note {noted}, \
         {started} job(s), dialog {dialog}"
    ));
    if !noted || started != 0 || dialog || gone("quick.txt") || target.join("quick.txt").exists() {
        failures.push(format!(
            "a drag released 0.25 s after the press: {QUICK_NOTE:?} shown {noted}, \
             {started} job(s), dialog {dialog}, source gone {}, arrived {}",
            gone("quick.txt"),
            target.join("quick.txt").exists()
        ));
        support::dismiss_all(app);
    }
    // Released 0.45 s after the press: the drop moves the file.
    let start = app.copies.len();
    let (state, label) = held_drag(
        app,
        "quick.txt",
        Duration::from_millis(150),
        Duration::from_millis(300),
    )?;
    context.note(format!(
        "a drag released 0.45 s after the press: feedback {state} {label:?}"
    ));
    match finish(app, start, "the drag released 0.45 s after the press") {
        Err(error) => failures.push(error),
        Ok(progress) => {
            if progress.state != CopyOperationState::Completed
                || !gone("quick.txt")
                || fs::read_to_string(target.join("quick.txt")).ok().as_deref()
                    != Some("quick.txt\n")
            {
                failures.push(format!(
                    "a drag released 0.45 s after the press ended {:?}: source gone {}, \
                     arrived {}",
                    progress.state,
                    gone("quick.txt"),
                    target.join("quick.txt").exists()
                ));
            }
        }
    }
    Ok(())
}

/// Across volumes: a plain drag offers and does Copy; Shift+drag offers Move, which
/// moves the file or leaves the source untouched, except where the other volume cannot
/// keep a replaced file recoverable (tmpfs): there the drag refuses the Move up front
/// and starts nothing (finding O-1).
fn cross_volume_drags(context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let source = support::scratch(context, "drag-source")?;
    let parent = match other_volume(&source) {
        Ok(parent) => parent,
        Err(reason) => {
            context.note(format!("SKIP: drags across volumes: {reason}"));
            return Ok(());
        }
    };
    let run = context
        .run_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let other = crate::fixtures::create_fresh(&parent, &format!("midnight-acceptance-{run}"))?;
    let takes_move = support::takes_cross_volume_move(&other);
    context.note(format!(
        "the other volume takes a Move from another drive: {takes_move:?}"
    ));
    let result = (|| -> Result<(), String> {
        for name in ["plain-drag.txt", "shift-drag.txt"] {
            write(&source.join(name), &format!("{name}\n"))?;
        }
        let mut app = App::start(Config::new(&source, &other));
        for (name, modifiers, wanted) in [
            ("plain-drag.txt", &[][..], COPY),
            ("shift-drag.txt", &[Key::Shift][..], MOVE),
        ] {
            let before = facts(&source.join(name))?;
            let start = app.copies.len();
            let spot = panel::list_spot(&app, RIGHT);
            let (state, label) = panel::drag_row(&mut app, LEFT, name, spot, modifiers)?;
            context.note(format!(
                "drag of {name} to another volume: feedback {state} {label:?}"
            ));
            if wanted == MOVE && takes_move == Some(false) {
                // Refused while dragging: nothing starts. A drop shows its dialog or job
                // within moments (the drops above do), so 0.5 s without one is an
                // observation window, not a delay.
                app.idle(Duration::from_millis(500));
                let started = app.copies.len() - start;
                let dialog = !app.window.get_copy_draft_token().is_empty();
                let kept = facts(&source.join(name)).ok().as_ref() == Some(&before)
                    && !other.join(name).exists();
                context.note(format!(
                    "{name} across volumes: feedback {state}, {started} job(s), dialog \
                     {dialog}, source kept {kept}"
                ));
                if state != REFUSED || started != 0 || dialog || !kept {
                    failures.push(format!(
                        "finding O-1: a Shift+drag of {name} to a volume that takes no Move \
                         from another drive was not refused up front: feedback {state} \
                         ({label:?}), {started} job(s), dialog {dialog}, source kept {kept}"
                    ));
                }
                support::dismiss_all(&mut app);
                continue;
            }
            if state != wanted {
                failures.push(format!(
                    "a drag of {name} to another volume offered {state} ({label:?}), not {wanted}"
                ));
                support::dismiss_all(&mut app);
                continue;
            }
            // A Move where the other volume takes no verified Move (tmpfs) asks; Skip
            // leaves the source where it is.
            let (progress, asked) = finish_answering(
                &mut app,
                start,
                &format!("the drop of {name}"),
                &support::Choice::Skip,
            )?;
            let arrived = facts(&other.join(name)).ok().map(|found| found.content)
                == Some(before.content.clone());
            let kept = facts(&source.join(name)).ok().as_ref() == Some(&before);
            let consistent = if wanted == COPY {
                progress.state == CopyOperationState::Completed
                    && arrived
                    && kept
                    && asked.is_empty()
            } else {
                (arrived && !source.join(name).exists()) || (kept && !other.join(name).exists())
            };
            context.note(format!(
                "{name} across volumes: {:?}, arrived {arrived}, source kept {kept}, \
                 questions {asked:?}",
                progress.state
            ));
            if !consistent {
                failures.push(format!(
                    "the drop of {name} across volumes ended {:?} (arrived {arrived}, source \
                     kept {kept})",
                    progress.state
                ));
            }
        }
        app.shutdown()
    })();
    let _ = crate::fixtures::remove_tree(&other);
    result
}

/// Mark `names` in the left panel (after clearing the marks); they must be what is marked.
fn mark(app: &mut App, names: &[&str]) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    for name in names {
        panel::select_row(app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    app.settle();
    let mut marked = support::selected_names(app, LEFT);
    marked.sort();
    if marked != names {
        return Err(format!("Insert marked {marked:?}, not {names:?}"));
    }
    Ok(())
}

/// Two marked files, then a filename search that moves the cursor to a third: F5 closes
/// the search and copies the two marked files only.
fn quick_search_copy(
    app: &mut App,
    context: &mut Context,
    source: &Path,
    target: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let destination = target.join("searched");
    fs::create_dir(&destination).map_err(|error| io_error(&destination, error))?;
    mark(app, &["qa-one.txt", "qa-two.txt"])?;
    input::type_text(app.slint(), "qb");
    app.settle();
    if !app.window.get_left_search_active()
        || support::cursor_name(app, LEFT).as_deref() != Some("qb-match.txt")
    {
        failures.push(format!(
            "typing \"qb\" did not search (active {}, cursor on {:?})",
            app.window.get_left_search_active(),
            support::cursor_name(app, LEFT)
        ));
    }
    let run = support::transfer(
        app,
        context,
        TransferSpec {
            destination: Some(destination.as_path()),
            ..TransferSpec::copy()
        },
    );
    if app.window.get_left_search_active() {
        failures.push("F5 during a filename search left the search open".into());
    }
    match run {
        Err(error) => failures.push(format!("F5 during a filename search: {error}")),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F5 during a filename search") {
                failures.push(error);
            }
            let copied = support::names_on_disk(&destination);
            if copied != ["qa-one.txt", "qa-two.txt"]
                || ["qa-one.txt", "qa-two.txt"].iter().any(|name| {
                    fs::read(destination.join(name)).ok() != fs::read(source.join(name)).ok()
                })
            {
                failures.push(format!(
                    "F5 with two marked files during a search of a third copied {copied:?}"
                ));
            }
        }
    }
    support::dismiss_all(app);
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    Ok(())
}

/// 65 marked files, one more than a Move once took: F5 copies all of them, Ctrl+X and
/// Ctrl+V in an empty folder move all of them there, and F6 from there moves all of them
/// back, each a Move of every item, verified against the files as they were.
fn selection_limits(
    app: &mut App,
    context: &mut Context,
    source: &Path,
    target: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let many = source.join("many65");
    let before = tree_facts(&many)?;
    let moved = target.join("moved65");
    let copied = target.join("copies65");
    for folder in [&moved, &copied] {
        fs::create_dir(folder).map_err(|error| io_error(folder, error))?;
    }
    panel::enter(app, LEFT, "many65")?;
    let mark_all = |app: &mut App, pane: PaneId| -> Result<(), String> {
        panel::focus_list(app, pane)?;
        input::chord(app.slint(), &[Key::Control], "a");
        app.settle();
        let marked = support::selected_names(app, pane).len();
        if marked == LIMIT + 1 {
            Ok(())
        } else {
            Err(format!("Ctrl+A marked {marked} of {} files", LIMIT + 1))
        }
    };
    let compare = |what: &str, folder: &Path, failures: &mut Vec<String>| -> Result<(), String> {
        let differences = support::compare_trees(&before, &tree_facts(folder)?, false);
        if !differences.is_empty() {
            failures.push(format!(
                "{what} of {} files left differences: {:?}",
                LIMIT + 1,
                differences.iter().take(5).collect::<Vec<_>>()
            ));
        }
        Ok(())
    };

    // F5 of all 65 copies every one.
    mark_all(app, LEFT)?;
    match support::transfer(
        app,
        context,
        TransferSpec {
            destination: Some(copied.as_path()),
            ..TransferSpec::copy()
        },
    ) {
        Err(error) => failures.push(format!("F5 of {} files: {error}", LIMIT + 1)),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F5 of 65 files") {
                failures.push(error);
            }
            compare("F5", &copied, failures)?;
        }
    }

    // Ctrl+X, then Ctrl+V in the empty folder, moves every one.
    mark_all(app, LEFT)?;
    let clipboard = app.clipboard.clone().ok_or("no clipboard installed")?;
    input::chord(app.slint(), &[Key::Control], "x");
    let held = clipboard.clone();
    app.until_for(
        "Ctrl+X of 65 files to fill the clipboard",
        Duration::from_secs(10),
        move |_| {
            held.files().is_some_and(|files| {
                files.intent == FileClipboardIntent::Move && files.paths.len() == LIMIT + 1
            })
        },
    );
    panel::focus_list(app, RIGHT)?;
    panel::enter(app, RIGHT, "moved65")?;
    let start = app.copies.len();
    input::chord(app.slint(), &[Key::Control], "v");
    match finish_answering(
        app,
        start,
        "Ctrl+V of the 65 cut files",
        &support::Choice::CancelJob,
    ) {
        Err(error) => failures.push(format!("Ctrl+X, Ctrl+V of {} files: {error}", LIMIT + 1)),
        Ok((done, asked)) => {
            context.note(format!(
                "Ctrl+X, Ctrl+V of {} files: {:?}, {} of {} items moved",
                LIMIT + 1,
                done.state,
                done.completed_roots,
                done.requested_roots
            ));
            if done.state != CopyOperationState::Completed || !asked.is_empty() {
                failures.push(format!(
                    "Ctrl+X, Ctrl+V of {} files ended {:?} after {asked:?}: {}",
                    LIMIT + 1,
                    done.state,
                    done.message
                ));
            }
        }
    }
    support::dismiss_all(app);
    compare("Ctrl+X, Ctrl+V", &moved, failures)?;
    if !support::names_on_disk(&many).is_empty() {
        failures.push(format!(
            "Ctrl+X, Ctrl+V of {} files left {:?} in the source",
            LIMIT + 1,
            support::names_on_disk(&many)
        ));
    }

    // F6 from there moves every one back to the left panel's folder.
    support::wait_row(app, RIGHT, "m000.txt", true);
    mark_all(app, RIGHT)?;
    match support::transfer(app, context, TransferSpec::moving()) {
        Err(error) => failures.push(format!("F6 of {} files: {error}", LIMIT + 1)),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F6 of 65 files") {
                failures.push(error);
            }
        }
    }
    compare("F6", &many, failures)?;
    if !support::names_on_disk(&moved).is_empty() {
        failures.push(format!(
            "F6 of {} files left {:?} behind",
            LIMIT + 1,
            support::names_on_disk(&moved)
        ));
    }
    panel::leave(app, RIGHT)?;
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    panel::leave(app, LEFT)?;
    Ok(())
}
