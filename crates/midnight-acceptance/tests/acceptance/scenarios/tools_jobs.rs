//! Jobs: a long copy (the large file and the many/ folder) shows progress in its own
//! window; Pause holds it, Resume continues it, Cancel operation asks and stops it
//! keeping only whole files; a second copy completes; the Jobs window lists both.
use crate::{
    Context,
    app::{self, App, Config},
    files_support as support, input, inspect, panel,
    scenarios::transfers::{copy_tree, snapshot},
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CopyOperationState, CopyProgress, PaneId};
use slint::{
    Model,
    platform::{Key, WindowAdapter},
};
use std::{collections::BTreeMap, fs, path::Path, time::Duration};

const SIZE: (u32, u32) = (760, 560);

/// How far the transfer is: files, bytes copied, bytes of the current file, bytes
/// verified.
fn mark(progress: &CopyProgress) -> (u64, u64, u64, u64) {
    (
        progress.files_completed,
        progress.copied_bytes,
        progress.current_file_copied,
        progress.verified_bytes,
    )
}

fn latest(app: &App, start: usize) -> Option<(u64, u64, u64, u64)> {
    app.copies[start..].last().map(|progress| mark(progress))
}

fn finished(app: &App, start: usize) -> Option<CopyOperationState> {
    app.copies[start..]
        .iter()
        .find(|progress| progress.state.is_terminal())
        .map(|progress| progress.state)
}

fn texts(window: &Tool) -> Vec<String> {
    inspect::visible_texts(window.window())
}

/// The progress window of the running copy.
fn progress_window(app: &mut App) -> Result<Tool, String> {
    app.until("the Copy progress window", |app| {
        tools::tool_titled(app, "Copy").is_some()
    });
    let window = tools::tool_titled(app, "Copy").ok_or("no Copy progress window")?;
    app::layout_tool(&window, SIZE.0, SIZE.1);
    app.settle();
    Ok(window)
}

/// The Jobs window's rows: (title, state, summary) from each list item's texts.
fn job_rows(window: &Tool) -> Vec<(String, String, String)> {
    inspect::controls(window.window(), AccessibleRole::ListItem)
        .into_iter()
        .map(|(title, item)| {
            let texts = tools::item_texts(&item);
            (
                title,
                texts.get(1).cloned().unwrap_or_default(),
                texts.get(2).cloned().unwrap_or_default(),
            )
        })
        .collect()
}

fn jobs_window(app: &mut App, before: usize) -> Result<Tool, String> {
    app.until("the Jobs window", move |app| {
        tools::tool_titled(app, "Jobs").is_some() || app.tool_windows().len() > before + 1
    });
    let window = tools::tool_titled(app, "Jobs")
        .ok_or_else(|| format!("no window titled Jobs: {:?}", tools::tool_titles(app)))?;
    app::layout_tool(&window, SIZE.0, SIZE.1);
    app.settle();
    Ok(window)
}

/// Every entry below `target` is a whole copy of its source: no partial file remains.
fn whole_files(source: &Path, target: &Path) -> Result<usize, String> {
    let expected: BTreeMap<String, String> = snapshot(source)?.into_iter().collect();
    let mut files = 0;
    for (path, kind) in snapshot(target)? {
        match expected.get(&path) {
            Some(wanted) if *wanted == kind => {}
            _ => return Err(format!("the cancelled copy left {path}: {kind}")),
        }
        if kind != "folder" {
            files += 1;
        }
    }
    Ok(files)
}

pub fn jobs_copy_control(context: &mut Context) -> Result<(), String> {
    let source = context.work.join("source");
    let target = context.work.join("target");
    let second = context.work.join("second");
    for folder in [&source, &target, &second] {
        fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let huge = context.fixture.path("sizes/huge.bin");
    if fs::hard_link(&huge, source.join("huge.bin")).is_err() {
        fs::copy(&huge, source.join("huge.bin")).map_err(|error| error.to_string())?;
    }
    copy_tree(&context.fixture.path("many"), &source.join("many"))?;
    let mut app = App::start(Config::new(&source, &target));
    let (left, right) = (PaneId::Left, PaneId::Right);

    // F5 on the large file and many/, confirmed with Enter.
    for name in ["huge.bin", "many"] {
        panel::select_row(&mut app, left, name)?;
        input::key(app.slint(), Key::Insert);
    }
    let start = app.copies.len();
    input::key(app.slint(), Key::F5);
    app.until("the Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    input::key(app.slint(), Key::Return);
    let window = progress_window(&mut app)?;
    app.until("the copy to make progress", move |app| {
        latest(app, start).is_some_and(|mark| mark != (0, 0, 0, 0))
    });
    if let Some(state) = finished(&app, start) {
        return Err(format!(
            "the copy ended ({state:?}) before it could be paused"
        ));
    }
    if !texts(&window)
        .iter()
        .any(|text| text.starts_with("Time remaining:"))
    {
        return Err(format!("no time remaining line: {:?}", texts(&window)));
    }
    app.screenshot_window(context, &window, "progress", "A long copy in progress");

    // Pause: the header says Paused and the transfer holds still.
    tools::click_last(window.window(), "Pause")?;
    let watched = window.clone();
    app.until("the copy to pause", move |_| {
        texts(&watched).iter().any(|text| text == "Paused")
    });
    let held = latest(&app, start);
    app.idle(Duration::from_millis(800));
    if latest(&app, start) != held || finished(&app, start).is_some() {
        return Err(format!(
            "the paused copy moved on: {held:?} → {:?}",
            latest(&app, start)
        ));
    }
    if !tools::has(window.window(), AccessibleRole::Button, "Resume") {
        return Err("no Resume button while paused".into());
    }
    app.screenshot_window(context, &window, "paused", "The paused copy");
    // The Jobs window lists the paused job.
    let before = app.tool_windows().len();
    tools::click_last(window.window(), "Jobs")?;
    let jobs = jobs_window(&mut app, before)?;
    let paused_rows = job_rows(&jobs);
    if paused_rows.len() != 1 || paused_rows[0].0 != "Copy" {
        return Err(format!("the Jobs window lists {paused_rows:?}"));
    }
    app.screenshot_window(
        context,
        &jobs,
        "jobs-paused",
        "Jobs while the copy is paused",
    );
    tools::close_tool(&mut app, &jobs, "the Jobs window");

    // Resume: the transfer moves again.
    tools::click_last(window.window(), "Resume")?;
    app.until("the copy to resume", move |app| {
        latest(app, start) != held || finished(app, start).is_some()
    });
    if let Some(state) = finished(&app, start) {
        return Err(format!(
            "the copy ended ({state:?}) before it could be cancelled"
        ));
    }

    // Cancel operation: a confirmation at a safe checkpoint, then the cancellation.
    tools::click_last(window.window(), "Cancel operation")?;
    let watched = window.clone();
    app.until("the cancel confirmation", move |_| {
        texts(&watched)
            .iter()
            .any(|text| text == "Cancel this operation?")
    });
    app.screenshot_window(
        context,
        &window,
        "cancel-confirmation",
        "Cancel this operation?",
    );
    tools::click_last(window.window(), "Cancel operation")?;
    app.until("the copy to end", move |app| finished(app, start).is_some());
    if finished(&app, start) != Some(CopyOperationState::Cancelled) {
        return Err(format!(
            "the cancelled copy ended {:?}",
            finished(&app, start)
        ));
    }
    let watched = window.clone();
    app.until("the window to offer Close", move |_| {
        watched.window().is_visible()
            && tools::has(watched.window(), AccessibleRole::Button, "Close")
    });
    app.screenshot_window(context, &window, "cancelled", "The cancelled copy");
    // While its window shows the outcome, no notice repeats it.
    app.idle(Duration::from_millis(300));
    if let Some((title, body)) = tools::shown_notice(&app) {
        return Err(format!(
            "a notice repeats the cancellation while its window is open: {title:?}: {body:?}"
        ));
    }
    // The open progress window says how the copy ended, in the user's words.
    let kept = "Copy cancelled; files already copied are kept.";
    if !texts(&window).iter().any(|text| text.contains(kept)) {
        return Err(format!(
            "the cancelled copy's window does not say {kept:?}: {:?}",
            texts(&window)
        ));
    }
    let whole = whole_files(&source, &target)?;
    let before = app.tool_windows().len();
    tools::click_last(window.window(), "Jobs")?;
    let jobs = jobs_window(&mut app, before)?;
    let rows = job_rows(&jobs);
    if rows.len() != 1 || rows[0].1 != "Cancelled" {
        return Err(format!("after cancelling, Jobs lists {rows:?}"));
    }
    tools::close_tool(&mut app, &jobs, "the Jobs window");
    tools::click_last(window.window(), "Close")?;
    let watched = window.clone();
    app.until("the progress window to close", move |_| {
        !watched.window().is_visible()
    });
    // Its progress window showed the outcome, so no main-window notice repeats it.
    app.idle(Duration::from_millis(300));
    let notice = tools::shown_notice(&app);
    if let Some((title, body)) = &notice {
        return Err(format!(
            "a notice repeats the cancellation its progress window showed: {title:?}: {body:?}"
        ));
    }

    // A second copy of the large file into an empty folder completes.
    panel::focus_list(&mut app, right)?;
    panel::leave(&mut app, right)?;
    panel::enter(&mut app, right, "second")?;
    panel::focus_list(&mut app, left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    panel::select_row(&mut app, left, "huge.bin")?;
    let start = app.copies.len();
    input::key(app.slint(), Key::F5);
    app.until("the second Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    input::key(app.slint(), Key::Return);
    app.until("the second copy to finish", move |app| {
        finished(app, start).is_some()
    });
    if finished(&app, start) != Some(CopyOperationState::Completed) {
        return Err(format!("the second copy ended {:?}", finished(&app, start)));
    }
    if fs::read(second.join("huge.bin")).map_err(|error| error.to_string())?
        != fs::read(&huge).map_err(|error| error.to_string())?
    {
        return Err("the completed copy's bytes differ".into());
    }
    // A clean completion closes its progress window by itself.
    app.until("the completed copy's window to close", |app| {
        tools::tool_titled(app, "Copy").is_none()
    });
    let before = app.tool_windows().len();
    support::open_jobs(&mut app)?;
    let jobs = tools::wait_tool(&mut app, "the Jobs window", before, SIZE);
    let rows = job_rows(&jobs);
    if rows.len() != 2 || rows[0].1 != "Completed" || rows[1].1 != "Cancelled" {
        return Err(format!(
            "Jobs lists {rows:?}; expected Completed then Cancelled"
        ));
    }
    app.screenshot_window(
        context,
        &jobs,
        "jobs-finished",
        "Jobs: completed and cancelled copies",
    );
    tools::click_last(jobs.window(), "Clear all finished")?;
    tools::wait_text(&mut app, &jobs, "the cleared list", |text| {
        text == "No background operations"
    });
    if app.window.get_operations().row_count() != 0 {
        return Err("Clear all finished left operations behind".into());
    }
    tools::close_tool(&mut app, &jobs, "the Jobs window");
    context.note(format!(
        "paused at {held:?} (files, copied, current file, verified) and held for 0.8 s; Jobs while paused: {paused_rows:?}; cancel kept {whole} whole files and no partial ones; notice after closing the cancelled job: {notice:?}; the second copy completed and closed its window; Jobs listed Completed then Cancelled and cleared"
    ));
    app.shutdown()?;
    // Reported last, after every other check ran.
    if !paused_rows[0].1.contains("Paused") {
        return Err(format!(
            "finding ACC-TOOLS-04: while the copy was paused (its progress window said Paused) the Jobs window listed it as {:?}",
            paused_rows[0].1
        ));
    }
    Ok(())
}
