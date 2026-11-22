//! Copy/Move errors and decisions through the real windows: Retry after fixing a
//! read-only destination or an unreadable source, Skip all this error type, Cancel job,
//! Skip of an unreadable folder with the rest copied, a write-protected folder moved as
//! it is, a Move out of a write-protected folder (its question, Skip, then Retry), a
//! selected file removed while the dialog is open (said in words, Skip), Decide later and
//! Resolve, Merge all folders, a job whose outcome has errors and notes, and a Copy and a
//! Move to another filesystem when one is available ($XDG_RUNTIME_DIR; on Windows the
//! private installation's drive when the run is on another one): a filesystem that
//! cannot keep a replaced file recoverable (tmpfs) refuses the Move in the dialog.
use crate::{
    Context,
    app::{self, App, Config},
    files_support::{
        self as support, Choice, LinkChoice, TransferSpec, facts, io_error, tree_facts,
    },
    input, inspect, panel,
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CopyOperationState, CopyProgress, PaneId};
use slint::platform::{Key, WindowAdapter};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const LEFT: PaneId = PaneId::Left;
const SIZE: (u32, u32) = (760, 560);
/// The system's own words for a refused access, as the job windows show them.
const DENIED: &str = if cfg!(windows) {
    "Access is denied"
} else {
    "Permission denied"
};
/// Whether `links-ok` holds a dangling link: Unix only (a Windows symbolic link needs
/// Developer Mode or administrator rights to be created).
const LINK_NOTE: bool = cfg!(unix);
/// Why a file cannot be moved out of a write-protected folder, as the question and the
/// job's outcome say it (T-E06).
const MOVE_DENIED: &str = if cfg!(windows) {
    "Permission denied: it cannot be moved out of its folder: the permissions of the item \
     and of its folder do not allow removing it. It stays there; copy it instead."
} else {
    "Permission denied: it cannot be moved out of its folder, which is write-protected or \
     belongs to another user who does not allow it. It stays there; copy it instead."
};
/// The outcome's line when Skip left out the only selected item.
const SKIPPED_ONE: &str = "1 entry or subtree skipped; source data retained.";

/// Set `path`'s permission bits; on Windows their equivalent (`tools::set_mode`).
fn chmod(path: &Path, mode: u32) -> Result<(), String> {
    tools::set_mode(path, mode)
}

/// The permission bits of `path` (links not followed).
#[cfg(unix)]
fn mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o7777)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    fs::write(path, text).map_err(|error| io_error(path, error))
}

fn folder(path: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|error| io_error(path, error))?;
    Ok(path.to_path_buf())
}

/// Clear the marks and mark `names` in the left panel (one name: just the cursor).
fn choose(app: &mut App, names: &[&str]) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    if let [name] = names {
        return panel::select_row(app, LEFT, name);
    }
    for name in names {
        panel::select_row(app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    Ok(())
}

/// Answer each decision of the job starting at `start` with the next of `answers`
/// (Cancel job, recorded as unexpected, when they run out) until it ends. Returns its
/// last report and the decision windows' texts.
fn settle(
    app: &mut App,
    start: usize,
    answers: &[Choice],
) -> Result<(Arc<CopyProgress>, Vec<String>), String> {
    let mut decisions = Vec::new();
    let mut answers = answers.iter();
    loop {
        app.until("the job to end or ask", move |app| {
            support::job(app, start).is_some_and(|job| job.state.is_terminal())
                || support::decision_window(app).is_some()
        });
        if let Some(job) = support::job(app, start).filter(|job| job.state.is_terminal()) {
            app.settle();
            return Ok((job, decisions));
        }
        let window = support::wait_decision(app, "the decision window");
        decisions.push(support::describe_window(&window));
        let choice = answers.next().cloned().unwrap_or(Choice::CancelJob);
        let token = app.window.get_copy_token();
        support::answer(app, &window, &choice)?;
        let answered = window.clone();
        app.until("the decision to be taken", move |app| {
            !answered.window().is_visible() || app.window.get_copy_token() != token
        });
    }
}

fn buttons(window: &Tool) -> Vec<String> {
    inspect::labels(window.window(), AccessibleRole::Button)
}

/// F5 of dup.txt onto its name in `conflict`, the question closed with `how` (Escape, or
/// the window's × as the title bar sends it): the job must wait for a decision, and
/// Resolve on its row in the Jobs window, opened with its window's Jobs button, must bring
/// the question back, which Skip answers.
fn decide_later_by(
    app: &mut App,
    context: &mut Context,
    how: &str,
    conflict: &Path,
) -> Result<(), String> {
    choose(app, &["dup.txt"])?;
    let start = support::start_transfer(
        app,
        context,
        TransferSpec {
            destination: Some(conflict),
            ..TransferSpec::copy()
        },
    )?;
    let window = support::wait_decision(app, "the name conflict");
    if how == "Escape" {
        input::key(window.window(), Key::Escape);
    } else {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::CloseRequested);
    }
    let closed = window.clone();
    app.until("the question to close", move |_| {
        !closed.window().is_visible()
    });
    app.until("the job to wait for a decision", move |app| {
        support::job(app, start).is_some_and(|job| {
            job.state == CopyOperationState::WaitingForDecision || job.state.is_terminal()
        })
    });
    let state = support::job(app, start).map(|job| job.state);
    if state != Some(CopyOperationState::WaitingForDecision) {
        return Err(format!("the job is {state:?}, not waiting for a decision"));
    }
    let copy = tools::tool_titled(app, "Copy").ok_or("no Copy window shows the waiting job")?;
    app::layout_tool(&copy, SIZE.0, SIZE.1);
    app.settle();
    let before = app.tool_windows().len();
    tools::click_last(copy.window(), "Jobs")?;
    let jobs = tools::wait_tool(app, "the Jobs window", before, SIZE);
    let row = inspect::controls(jobs.window(), AccessibleRole::ListItem)
        .into_iter()
        .find(|(_, item)| {
            tools::item_texts(item)
                .iter()
                .any(|text| text.to_lowercase().contains("decision"))
        })
        .map(|(_, item)| item);
    let Some(row) = row else {
        let rows = inspect::visible_texts(jobs.window());
        tools::close_tool(app, &jobs, "the Jobs window");
        return Err(format!(
            "the Jobs window shows no job needing a decision: {rows:?}"
        ));
    };
    input::click_at(jobs.window(), inspect::center(&row));
    app.settle();
    let resolved = tools::click_last(jobs.window(), "Resolve");
    if let Err(error) = resolved {
        tools::close_tool(app, &jobs, "the Jobs window");
        return Err(format!("the Jobs window offers no Resolve: {error}"));
    }
    app.until("Resolve to bring the question back", |app| {
        support::decision_window(app).is_some()
    });
    if jobs.window().is_visible() {
        tools::close_tool(app, &jobs, "the Jobs window");
    }
    let (done, _) = settle(app, start, &[Choice::Skip])?;
    if done.skipped != 1
        || fs::read_to_string(conflict.join("dup.txt")).ok().as_deref() != Some("already here\n")
    {
        return Err(format!(
            "Skip after Resolve from Jobs ended {:?} (skipped {})",
            done.state, done.skipped
        ));
    }
    Ok(())
}

/// The question of the Move reported after `start` of `name` out of the write-protected
/// `ro-src` (T-E06), sized for reading: it must offer Retry, Skip, Skip all this error
/// type, Cancel job and Decide later and say why in words. `Err` when no question came.
fn protected_question(
    app: &mut App,
    start: usize,
    name: &str,
    failures: &mut Vec<String>,
) -> Result<Tool, String> {
    app.until(
        "the write-protected source folder's Move error",
        move |app| {
            support::decision_window(app).is_some()
                || support::job(app, start).is_some_and(|job| job.state.is_terminal())
        },
    );
    if support::decision_window(app).is_none() {
        let done = support::job(app, start);
        return Err(format!(
            "no question; the Move of {name} ended {:?} with {:?}",
            done.as_ref().map(|job| job.state),
            done.as_ref().and_then(|job| job.errors.first())
        ));
    }
    let window = support::wait_decision(app, "the Move error");
    let offered = buttons(&window);
    let missing: Vec<&str> = [
        "Retry",
        "Skip",
        "Skip all this error type",
        "Cancel job",
        "Decide later",
    ]
    .into_iter()
    .filter(|wanted| !offered.iter().any(|label| label == wanted))
    .collect();
    if !missing.is_empty() {
        failures.push(format!(
            "T-E06: the question about {name} offers no {missing:?}: {offered:?}"
        ));
    }
    let texts = inspect::visible_texts(window.window());
    let worded = if texts.iter().any(|text| text.contains(MOVE_DENIED)) {
        Ok(())
    } else {
        Err(format!(
            "the question about {name} does not say {MOVE_DENIED:?}: {texts:?}"
        ))
    };
    if let Err(error) = worded {
        failures.push(format!("T-E06: {error}"));
    }
    Ok(window)
}

/// The newest open Copy or Move window, sized for reading.
fn job_window(app: &App) -> Option<Tool> {
    let window = app
        .tool_windows()
        .into_iter()
        .filter(|window| matches!(tools::title(window).as_str(), "Copy" | "Move"))
        .last()?;
    app::layout_tool(&window, SIZE.0, SIZE.1);
    Some(window)
}

/// Close every finished Copy or Move window with its Close button.
fn close_job_windows(app: &mut App) {
    for _ in 0..8 {
        let Some(window) = job_window(app) else {
            return;
        };
        if tools::click_last(window.window(), "Close").is_err() {
            input::key(window.window(), Key::Escape);
        }
        app.settle();
        if window.window().is_visible() {
            return;
        }
    }
}

/// The progress window titled `title` that is still open, if any.
fn progress_window(app: &mut App, title: &str) -> Option<Tool> {
    let window = tools::tool_titled(app, title)?;
    app::layout_tool(&window, SIZE.0, SIZE.1);
    app.settle();
    Some(window)
}

/// The checks of a job that ended with errors: its own window shows the error
/// details, no dialog follows it, and Details shows more.
fn check_error_window(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
    what: &str,
    detail: Option<&str>,
    shot: &str,
) {
    app.idle(Duration::from_millis(400));
    let notice = support::shown_notice(app);
    let window = progress_window(app, "Copy").or_else(|| progress_window(app, "Move"));
    let shown = window
        .as_ref()
        .map(|window| inspect::visible_texts(window.window()))
        .unwrap_or_default();
    if let Some(window) = &window {
        app.screenshot_window(context, window, shot, &format!("{what}: its job window"));
    }
    // The job's own window shows how it ended, the error included; no dialog follows.
    if window.is_none() || notice.is_some() {
        failures.push(format!(
            "{what}: the job window is {}; notice {notice:?}",
            if window.is_some() { "open" } else { "gone" }
        ));
    }
    if let Some(detail) = detail
        && !shown.iter().any(|text| text.contains(detail))
    {
        failures.push(format!(
            "{what}: the job window does not show {detail:?}: {:?}",
            shown.iter().take(12).collect::<Vec<_>>()
        ));
    }
    if let Some(window) = &window {
        let before = inspect::visible_texts(window.window()).len();
        let clicked = tools::click_last(window.window(), "Details");
        app.settle();
        let after = inspect::visible_texts(window.window()).len();
        match clicked {
            Err(error) => failures.push(format!("{what}: {error}")),
            Ok(()) if after > before => {}
            Ok(()) => failures.push(format!(
                "{what}: Details showed {before} texts, then {after}"
            )),
        }
    }
    support::dismiss_all(app);
    for title in ["Copy", "Move"] {
        while let Some(window) = tools::tool_titled(app, title) {
            if tools::click_last(window.window(), "Close").is_err() {
                input::key(window.window(), Key::Escape);
            }
            app.settle();
            if window.window().is_visible() {
                break;
            }
        }
    }
}

pub fn transfer_errors(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "errors")?;
    let target = support::scratch(context, "target")?;
    for name in [
        "one.txt",
        "a.txt",
        "b.txt",
        "c.txt",
        "secret.txt",
        "dup.txt",
        "x.txt",
        "y.txt",
    ] {
        write(&source.join(name), &format!("{name}\n"))?;
    }
    write(&source.join("tree/a.txt"), "a\n")?;
    write(&source.join("tree/b/c.txt"), "c\n")?;
    write(&source.join("tree/zz/hidden.txt"), "hidden\n")?;
    write(&source.join("locked/inside.txt"), "inside\n")?;
    write(&source.join("ro-src/held.txt"), "held\n")?;
    write(&source.join("ro-src/kept.txt"), "kept\n")?;
    write(&source.join("gone-copy.txt"), "gone\n")?;
    write(&source.join("gone-move.txt"), "gone\n")?;
    write(&source.join("m1/new1.txt"), "new1\n")?;
    write(&source.join("m2/new2.txt"), "new2\n")?;
    write(&source.join("links-ok/ok.txt"), "ok\n")?;
    write(&source.join("zz-alone/hidden.txt"), "hidden\n")?;
    write(&source.join("links-ok/zz-inner/hidden.txt"), "hidden\n")?;
    write(&source.join("err.txt"), "err\n")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("missing-target", source.join("links-ok/dangling"))
        .map_err(|error| io_error(&source, error))?;
    let ro = [
        folder(&target.join("ro-dest"))?,
        folder(&target.join("ro-dest2"))?,
        folder(&target.join("ro-dest3"))?,
    ];
    let plain = folder(&target.join("plain-dest"))?;
    let tree_dest = folder(&target.join("tree-dest"))?;
    let move_dest = folder(&target.join("move-dest"))?;
    let conflict = folder(&target.join("conflict-dest"))?;
    write(&conflict.join("dup.txt"), "already here\n")?;
    let merge = folder(&target.join("merge-dest"))?;
    write(&merge.join("m1/old1.txt"), "old1\n")?;
    write(&merge.join("m2/old2.txt"), "old2\n")?;
    let mixed_dest = folder(&target.join("mixed-dest"))?;
    let skip_root_dest = folder(&target.join("skip-root-dest"))?;
    let ro_move = folder(&target.join("ro-move"))?;
    for path in &ro {
        chmod(path, 0o555)?;
    }
    chmod(&source.join("secret.txt"), 0o000)?;
    chmod(&source.join("tree/zz"), 0o000)?;
    chmod(&source.join("zz-alone"), 0o000)?;
    chmod(&source.join("links-ok/zz-inner"), 0o000)?;
    chmod(&ro_move, 0o555)?;
    chmod(&source.join("locked"), 0o555)?;
    chmod(&source.join("ro-src"), 0o555)?;
    let mut app = App::start(Config::new(&source, &target));
    let mut failures = Vec::new();

    // A read-only destination: the error offers Retry, Skip, Skip all this error type
    // and Cancel job; after the folder is made writable, Retry copies the file.
    choose(&mut app, &["one.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(ro[0].as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let window = support::wait_decision(&mut app, "the read-only destination's error");
    let offered = buttons(&window);
    for wanted in ["Retry", "Skip", "Skip all this error type", "Cancel job"] {
        if !offered.iter().any(|label| label == wanted) {
            failures.push(format!(
                "the read-only destination's error offers no {wanted:?}: {offered:?}"
            ));
        }
    }
    let corner = support::header_close_buttons(&window);
    if !corner.is_empty() {
        failures.push(format!(
            "the framed decision window has a × of its own: its header has {corner:?}"
        ));
    }
    app.screenshot_window(
        context,
        &window,
        "error-decision",
        "Copy into a read-only folder",
    );
    chmod(&ro[0], 0o755)?;
    let token = app.window.get_copy_token();
    support::answer(&mut app, &window, &Choice::Retry)?;
    let answered = window.clone();
    app.until("Retry to be taken", move |app| {
        !answered.window().is_visible() || app.window.get_copy_token() != token
    });
    let (done, decisions) = settle(&mut app, start, &[])?;
    if done.state != CopyOperationState::Completed || done.failed != 0 || !decisions.is_empty() {
        failures.push(format!(
            "Retry into the now writable folder ended {:?} (failed {}, further questions {decisions:?})",
            done.state, done.failed
        ));
    }
    if facts(&ro[0].join("one.txt"))
        .map(|found| found.content)
        .ok()
        != facts(&source.join("one.txt"))
            .map(|found| found.content)
            .ok()
    {
        failures.push("Retry did not copy one.txt into the destination".into());
    }
    support::dismiss_all(&mut app);

    // Skip all this error type: one question for three files, nothing written.
    choose(&mut app, &["a.txt", "b.txt", "c.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(ro[1].as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let (done, decisions) = settle(&mut app, start, &[Choice::SkipAllErrors])?;
    if decisions.len() != 1 || done.skipped != 3 || !support::names_on_disk(&ro[1]).is_empty() {
        failures.push(format!(
            "Skip all this error type: {} question(s), {:?}, skipped {}, failed {}, destination {:?}",
            decisions.len(),
            done.state,
            done.skipped,
            done.failed,
            support::names_on_disk(&ro[1])
        ));
    }
    check_error_window(
        &mut app,
        context,
        &mut failures,
        "Skip all this error type",
        Some(DENIED),
        "skip-all-errors",
    );

    // Cancel job at the error: the job ends Cancelled and writes nothing.
    choose(&mut app, &["a.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(ro[2].as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let (done, _) = settle(&mut app, start, &[Choice::CancelJob])?;
    if done.state != CopyOperationState::Cancelled || !support::names_on_disk(&ro[2]).is_empty() {
        failures.push(format!(
            "Cancel job at the error ended {:?}, destination {:?}",
            done.state,
            support::names_on_disk(&ro[2])
        ));
    }
    app.idle(Duration::from_millis(400));
    let notice = support::shown_notice(&app);
    if let Some(notice) = notice {
        failures.push(format!("a notice follows Cancel job: {notice:?}"));
    }
    support::dismiss_all(&mut app);

    // An unreadable source file: the preparation error offers Retry, which succeeds once
    // the file is readable.
    choose(&mut app, &["secret.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(plain.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let window = support::wait_decision(&mut app, "the unreadable file's error");
    let offered = buttons(&window);
    if !offered.iter().any(|label| label == "Retry") {
        failures.push(format!(
            "the unreadable file's error offers no Retry: {offered:?}"
        ));
    }
    chmod(&source.join("secret.txt"), 0o644)?;
    let token = app.window.get_copy_token();
    support::answer(&mut app, &window, &Choice::Retry)?;
    let answered = window.clone();
    app.until("Retry to be taken", move |app| {
        !answered.window().is_visible() || app.window.get_copy_token() != token
    });
    let (done, decisions) = settle(&mut app, start, &[])?;
    if done.state != CopyOperationState::Completed
        || !decisions.is_empty()
        || facts(&plain.join("secret.txt"))
            .map(|found| found.content)
            .ok()
            != facts(&source.join("secret.txt"))
                .map(|found| found.content)
                .ok()
    {
        failures.push(format!(
            "Retry of the now readable file ended {:?} (questions {decisions:?})",
            done.state
        ));
    }
    support::dismiss_all(&mut app);

    // An unreadable folder inside a copied tree: Skip leaves it out and the rest is
    // copied.
    choose(&mut app, &["tree"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(tree_dest.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let (done, decisions) = settle(&mut app, start, &[Choice::Skip])?;
    context.note(format!("unreadable folder in a tree: {decisions:?}"));
    let copied = tree_facts(&tree_dest.join("tree")).unwrap_or_default();
    let missing: Vec<&str> = ["a.txt", "b", "b/c.txt"]
        .into_iter()
        .filter(|wanted| !copied.contains_key(*wanted))
        .collect();
    if !missing.is_empty() {
        failures.push(format!(
            "after Skip of an unreadable folder the rest of the tree is copied: not copied: \
             {missing:?}; the copy holds {:?}",
            copied.keys().collect::<Vec<_>>()
        ));
    }
    if decisions.len() != 1 || done.skipped == 0 || copied.contains_key("zz/hidden.txt") {
        failures.push(format!(
            "the unreadable folder: {} question(s), {:?}, skipped {}",
            decisions.len(),
            done.state,
            done.skipped
        ));
    }
    check_error_window(
        &mut app,
        context,
        &mut failures,
        "Skip of an unreadable folder",
        Some("zz"),
        "skip-unreadable",
    );

    // An owned write-protected folder (mode 0555, with contents) moves as it is: no
    // question, and the moved folder keeps its mode.
    choose(&mut app, &["locked"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(move_dest.as_path()),
            ..TransferSpec::moving()
        },
    )?;
    let (done, decisions) = settle(&mut app, start, &[])?;
    let moved = !source.join("locked").exists() && move_dest.join("locked/inside.txt").is_file();
    #[cfg(unix)]
    let kept_mode = mode(&move_dest.join("locked")) == Some(0o555);
    #[cfg(not(unix))]
    let kept_mode = crate::fixtures::write_protected(&move_dest.join("locked"));
    if !decisions.is_empty() || done.state != CopyOperationState::Completed || !moved || !kept_mode
    {
        failures.push(format!(
            "Move of a write-protected folder ended {:?} (questions {decisions:?}, moved {moved}, \
             mode kept {kept_mode})",
            done.state
        ));
    }
    support::dismiss_all(&mut app);
    close_job_windows(&mut app);

    // T-E06: a file in a write-protected folder (outside the moved item) cannot leave it.
    // One question while the Move is prepared offers Retry, Skip, Skip all this error
    // type, Cancel job and Decide later and says why in words. Skip leaves the file where
    // it is with its bytes, writes nothing, and ends the job Failed with the item's error
    // and the skipped line; Retry once the folder is writable moves the file.
    panel::enter(&mut app, LEFT, "ro-src")?;
    let kept_before = facts(&source.join("ro-src/kept.txt"))?;
    choose(&mut app, &["kept.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(move_dest.as_path()),
            ..TransferSpec::moving()
        },
    )?;
    match protected_question(&mut app, start, "kept.txt", &mut failures) {
        Err(error) => failures.push(format!("T-E06 (Skip): {error}")),
        Ok(window) => {
            let token = app.window.get_copy_token();
            support::answer(&mut app, &window, &Choice::Skip)?;
            let answered = window.clone();
            app.until("Skip to be taken", move |app| {
                !answered.window().is_visible() || app.window.get_copy_token() != token
            });
            let (done, decisions) = settle(&mut app, start, &[])?;
            let skipped = SKIPPED_ONE;
            let lines: Vec<&str> = done.message.lines().collect();
            if done.state != CopyOperationState::Failed
                || (done.skipped, done.failed) != (1, 1)
                || !decisions.is_empty()
                || !lines.contains(&skipped)
                || !lines.iter().any(|line| line.contains("kept.txt"))
            {
                failures.push(format!(
                    "T-E06: after Skip the Move ended {:?} (skipped {}, failed {}, further \
                     questions {decisions:?}) with {:?}, not Failed with the item's error and \
                     {skipped:?}",
                    done.state, done.skipped, done.failed, done.message
                ));
            }
            let worded = if lines
                .iter()
                .any(|line| line.contains("kept.txt") && line.contains(MOVE_DENIED))
            {
                Ok(())
            } else {
                Err(format!(
                    "the outcome does not give kept.txt's reason {MOVE_DENIED:?}: {:?}",
                    done.message
                ))
            };
            if let Err(error) = worded {
                failures.push(format!("T-E06: {error}"));
            }
            if facts(&source.join("ro-src/kept.txt")).ok().as_ref() != Some(&kept_before)
                || move_dest.join("kept.txt").exists()
            {
                failures.push(
                    "T-E06: after Skip kept.txt is not where it was with its bytes, or \
                     something arrived at the destination"
                        .into(),
                );
            }
            context.note(format!(
                "finding W-2 (wording): one skipped item is reported as {skipped:?}"
            ));
        }
    }
    support::dismiss_all(&mut app);
    close_job_windows(&mut app);
    choose(&mut app, &["held.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(move_dest.as_path()),
            ..TransferSpec::moving()
        },
    )?;
    let mut refused_retry = None;
    match protected_question(&mut app, start, "held.txt", &mut failures) {
        Err(error) => failures.push(format!("T-E06 (Retry): {error}")),
        Ok(window) => {
            let question = support::describe_window(&window);
            app.screenshot_window(
                context,
                &window,
                "move-protected-source",
                "Move out of a write-protected folder",
            );
            chmod(&source.join("ro-src"), 0o755)?;
            let token = app.window.get_copy_token();
            support::answer(&mut app, &window, &Choice::Retry)?;
            let answered = window.clone();
            app.until("Retry to be taken", move |app| {
                !answered.window().is_visible() || app.window.get_copy_token() != token
            });
            let (done, decisions) = settle(&mut app, start, &[])?;
            let moved =
                !source.join("ro-src/held.txt").exists() && move_dest.join("held.txt").is_file();
            if !moved || done.state != CopyOperationState::Completed || !decisions.is_empty() {
                refused_retry = Some(format!(
                    "finding ACC-ERR-01: Retry after making the source folder writable did \
                     not move the file ({:?}, further questions {decisions:?}; first question \
                     {question})",
                    done.state
                ));
            }
        }
    }
    support::dismiss_all(&mut app);
    close_job_windows(&mut app);
    panel::leave(&mut app, LEFT)?;

    // A selected file removed while the dialog is open: the question says so in words
    // ("removed from the source while copying", "… while moving"), never with the
    // system's text, and Skip of the only item ends the job Failed with the skipped line.
    for (name, spec) in [
        ("gone-copy.txt", TransferSpec::copy()),
        ("gone-move.txt", TransferSpec::moving()),
    ] {
        let verb = if spec.key == Key::F6 {
            "moving"
        } else {
            "copying"
        };
        let gone = source.join(name);
        let remove = || fs::remove_file(&gone).map_err(|error| io_error(&gone, error));
        choose(&mut app, &[name])?;
        let what = format!("{name} removed while its {verb} dialog was open");
        // The file must be removable while the dialog holds its selection.
        let start = match support::start_transfer(
            &mut app,
            context,
            TransferSpec {
                destination: Some(plain.as_path()),
                before_confirm: Some(&remove),
                ..spec
            },
        ) {
            Ok(start) => start,
            Err(error) => {
                failures.push(format!("{what}: {error}"));
                support::dismiss_all(&mut app);
                continue;
            }
        };
        app.until("the removed source's question", move |app| {
            support::decision_window(app).is_some()
                || support::job(app, start).is_some_and(|job| job.state.is_terminal())
        });
        let Some(window) = support::decision_window(&app)
            .is_some()
            .then(|| support::wait_decision(&mut app, "the removed source's question"))
        else {
            failures.push(format!(
                "{what}: no question; the job ended {:?}",
                support::job(&app, start).map(|job| (job.state, job.message.clone()))
            ));
            continue;
        };
        let texts = inspect::visible_texts(window.window());
        let wording = format!("removed from the source while {verb}");
        if !texts.iter().any(|text| text.contains(&wording))
            || texts.iter().any(|text| text.contains("os error"))
        {
            failures.push(format!(
                "{what}: the question does not say {wording:?} in words: {texts:?}"
            ));
        }
        if verb == "copying" {
            app.screenshot_window(
                context,
                &window,
                "removed-source",
                "A selected file removed while the Copy dialog was open",
            );
        }
        let (done, decisions) = settle(&mut app, start, &[Choice::Skip])?;
        if done.state != CopyOperationState::Failed
            || !done.message.lines().any(|line| line == SKIPPED_ONE)
            || done.message.contains("os error")
            || decisions.len() != 1
            || plain.join(name).exists()
        {
            failures.push(format!(
                "{what}: Skip ended the job {:?} with {:?} after {} question(s), not Failed \
                 with {SKIPPED_ONE:?}",
                done.state,
                done.message,
                decisions.len()
            ));
        }
        support::dismiss_all(&mut app);
        close_job_windows(&mut app);
    }

    // Decide later leaves the job waiting; Resolve brings the question back.
    choose(&mut app, &["dup.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(conflict.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let window = support::wait_decision(&mut app, "the name conflict");
    support::answer(&mut app, &window, &Choice::DecideLater)?;
    let deferred = window.clone();
    app.until("Decide later to close the question", move |_| {
        !deferred.window().is_visible()
    });
    app.idle(Duration::from_millis(400));
    let waiting = support::job(&app, start).map(|job| job.state);
    if waiting != Some(CopyOperationState::WaitingForDecision) {
        failures.push(format!("after Decide later the job is {waiting:?}"));
    }
    match tools::tool_titled(&app, "Copy") {
        None => failures.push("no Copy window shows the deferred question".into()),
        Some(window) => {
            app::layout_tool(&window, SIZE.0, SIZE.1);
            app.settle();
            app.screenshot_window(context, &window, "decide-later", "A deferred question");
            match tools::click_last(window.window(), "Resolve") {
                Err(error) => failures.push(format!("the deferred job offers no Resolve: {error}")),
                Ok(()) => {
                    app.until("Resolve to bring the question back", |app| {
                        support::decision_window(app).is_some()
                    });
                    let (done, _) = settle(&mut app, start, &[Choice::Skip])?;
                    if done.skipped != 1
                        || fs::read_to_string(conflict.join("dup.txt")).ok().as_deref()
                            != Some("already here\n")
                    {
                        failures.push(format!(
                            "Skip after Resolve ended {:?} (skipped {})",
                            done.state, done.skipped
                        ));
                    }
                }
            }
        }
    }
    support::dismiss_all(&mut app);

    close_job_windows(&mut app);

    // Escape and the window's × in a question act as Decide later: the job waits for a
    // decision, and Resolve in the Jobs window (opened with the job window's Jobs button)
    // brings the question back.
    for how in ["Escape", "the window's ×"] {
        if let Err(error) = decide_later_by(&mut app, context, how, &conflict) {
            failures.push(format!("{how} in a question: {error}"));
        }
        support::dismiss_all(&mut app);
        close_job_windows(&mut app);
    }

    // Merge all folders: one question for two folder conflicts.
    choose(&mut app, &["m1", "m2"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(merge.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let (done, decisions) = settle(&mut app, start, &[Choice::MergeAll])?;
    let merged = ["m1/new1.txt", "m1/old1.txt", "m2/new2.txt", "m2/old2.txt"]
        .iter()
        .all(|path| merge.join(path).is_file());
    if decisions.len() != 1 || done.state != CopyOperationState::Completed || !merged {
        failures.push(format!(
            "Merge all folders: {} question(s), {:?}, merged {merged}",
            decisions.len(),
            done.state
        ));
    }
    support::dismiss_all(&mut app);

    // A selected folder that cannot be read fails while the transfer is prepared: Skip
    // leaves it out and the other selected file is copied.
    close_job_windows(&mut app);
    choose(&mut app, &["one.txt", "zz-alone"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(skip_root_dest.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    app.until("the unreadable folder's question", move |app| {
        support::decision_window(app).is_some()
            || support::job(app, start).is_some_and(|job| job.state.is_terminal())
    });
    // Size the question like a desktop dialog before reading its buttons, as
    // `wait_decision` does for the other questions here: unsized, the window shows only
    // "Decide later", so an offered Skip would go unseen.
    let offered = support::decision_window(&app).is_some().then(|| {
        let window = support::wait_decision(&mut app, "the unreadable folder's question");
        inspect::labels(window.window(), AccessibleRole::Button)
    });
    let skippable = offered
        .as_ref()
        .is_some_and(|labels| labels.iter().any(|label| label == "Skip"));
    // Without Skip, the question is answered with Cancel job.
    let answers = if skippable {
        vec![Choice::Skip]
    } else {
        Vec::new()
    };
    let (done, _) = settle(&mut app, start, &answers)?;
    let copied = skip_root_dest.join("one.txt").is_file();
    let skip_root = match &offered {
        None => Err(format!("no question; the job ended {:?}", done.state)),
        Some(labels) if !skippable => Err(format!(
            "the question offers {labels:?}; Cancel job ended it {:?}",
            done.state
        )),
        Some(_) if !copied => Err(format!(
            "after Skip the job ended {:?} without one.txt",
            done.state
        )),
        Some(_) => Ok(()),
    };
    if let Err(error) = skip_root {
        failures.push(format!(
            "Skip of a selected folder that cannot be read copies the rest of the selection: \
             {error}"
        ));
    }
    support::dismiss_all(&mut app);
    close_job_windows(&mut app);

    // An outcome with a skipped folder and a link copied as a link: both are counts, the
    // window lists no item for them, and Details shows and hides the full list.
    choose(&mut app, &["links-ok"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(mixed_dest.as_path()),
            links: Some(LinkChoice::CopyFiles),
            ..TransferSpec::copy()
        },
    )?;
    let (done, decisions) = settle(&mut app, start, &[Choice::Skip])?;
    app.idle(Duration::from_millis(400));
    let window = job_window(&app);
    let outcome = window
        .as_ref()
        .map(|window| inspect::visible_texts(window.window()))
        .unwrap_or_default();
    context.note(format!(
        "outcome of a skipped folder and a link ({:?}, skipped {}, questions {}): {:?}",
        done.state,
        done.skipped,
        decisions.len(),
        outcome.iter().take(16).collect::<Vec<_>>()
    ));
    if decisions.len() != 1 || !mixed_dest.join("links-ok/ok.txt").is_file() {
        failures.push(format!(
            "the readable files were not copied beside the skipped folder ({} question(s))",
            decisions.len()
        ));
    }
    let has = |wanted: &str| outcome.iter().any(|text| text.contains(wanted));
    if !LINK_NOTE {
        context.note(
            "no link in links-ok on Windows (links need a privilege or Developer Mode, and \
             Windows copies never recreate links): the link's note is not checked",
        );
    }
    for wanted in [
        "Files copied: ",
        "1 skipped item was not copied.",
        "1 link was copied as a link; Details says why.",
    ] {
        if wanted.contains("link") && !LINK_NOTE {
            continue;
        }
        if !has(wanted) {
            failures.push(format!(
                "the outcome does not say {wanted:?}: {:?}",
                outcome.iter().take(12).collect::<Vec<_>>()
            ));
        }
    }
    if !has("1 error") || !has("zz-inner") {
        failures.push(format!(
            "the outcome does not list the skipped folder's error: {:?}",
            outcome.iter().take(12).collect::<Vec<_>>()
        ));
    }
    if has("because its target does not exist") {
        failures.push(format!(
            "the outcome lists informational items one by one: {:?}",
            outcome.iter().take(12).collect::<Vec<_>>()
        ));
    }
    if let Some(window) = &window {
        app.screenshot_window(
            context,
            window,
            "outcome-counts",
            "Counts of a skipped folder and a link",
        );
        let before = inspect::visible_texts(window.window()).len();
        let shown = tools::click_last(window.window(), "Details").map(|()| {
            app.settle();
            inspect::visible_texts(window.window())
        });
        match shown {
            Err(error) => failures.push(format!("the outcome's Details: {error}")),
            Ok(details) => {
                if details.len() <= before
                    || (LINK_NOTE
                        && !details
                            .iter()
                            .any(|text| text.contains("because its target does not exist")))
                {
                    failures.push(format!(
                        "Details does not show the link's reason: {:?}",
                        details.iter().take(16).collect::<Vec<_>>()
                    ));
                }
                let hidden = tools::click_last(window.window(), "Fewer details")
                    .or_else(|_| tools::click_last(window.window(), "Details"));
                app.settle();
                let after = inspect::visible_texts(window.window()).len();
                if hidden.is_err() || after >= details.len() {
                    failures.push(format!(
                        "Details did not hide again ({before} texts, {} with Details, {after} after)",
                        details.len()
                    ));
                }
            }
        }
    } else {
        failures.push("no window shows the outcome of the skipped folder".into());
    }
    if let Some(notice) = support::shown_notice(&app) {
        failures.push(format!(
            "a notice follows a job whose window shows it: {notice:?}"
        ));
    }
    close_job_windows(&mut app);

    // An error the job records: its count and row open the path, message and operation
    // in place, by click and by Space.
    choose(&mut app, &["err.txt"])?;
    let start = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(ro_move.as_path()),
            ..TransferSpec::moving()
        },
    )?;
    let (done, decisions) = settle(&mut app, start, &[Choice::Skip])?;
    app.idle(Duration::from_millis(400));
    context.note(format!(
        "F6 into a read-only folder: {:?}, failed {}, skipped {}, questions {decisions:?}",
        done.state, done.failed, done.skipped
    ));
    if !source.join("err.txt").is_file() || ro_move.join("err.txt").exists() {
        failures.push("a refused Move of err.txt changed source or destination".into());
    }
    match job_window(&app) {
        None => failures.push("no window shows the refused Move".into()),
        Some(window) if done.failed > 0 => {
            app.screenshot_window(
                context,
                &window,
                "error-row",
                "A Move that ended with an error",
            );
            let count = inspect::controls(window.window(), AccessibleRole::Button)
                .into_iter()
                .find(|(label, _)| label.ends_with(" error") || label.ends_with(" errors"));
            match count {
                None => failures.push(format!(
                    "the window has no error count button: {:?}",
                    inspect::visible_texts(window.window())
                )),
                Some((label, item)) => {
                    let collapsed = inspect::visible_texts(window.window());
                    input::click_at(window.window(), inspect::center(&item));
                    app.settle();
                    let opened = inspect::visible_texts(window.window());
                    let detail = |texts: &[String]| {
                        texts.iter().any(|text| text.contains(DENIED))
                            && texts.iter().any(|text| text.contains("Operation:"))
                            && texts.iter().any(|text| text.contains("err.txt"))
                    };
                    if !detail(&opened) || detail(&collapsed) {
                        failures.push(format!(
                            "clicking {label:?} did not open the error's path, message and operation: {:?}",
                            opened.iter().take(16).collect::<Vec<_>>()
                        ));
                    }
                    // Again to close it, then Space on the focused count opens it again.
                    input::click_at(window.window(), inspect::center(&item));
                    app.settle();
                    input::key(window.window(), " ");
                    app.settle();
                    if !detail(&inspect::visible_texts(window.window())) {
                        failures.push(format!("Space on {label:?} did not open the error"));
                    }
                }
            }
        }
        Some(_) => context.note(format!(
            "the refused Move recorded no error ({:?}); its rows were not checked",
            done.state
        )),
    }
    close_job_windows(&mut app);

    // Another filesystem: Copy works; Move either moves or is refused cleanly.
    cross_volume(&mut app, context, &source, &mut failures)?;

    app.shutdown()?;
    if let Some(finding) = refused_retry {
        failures.push(finding);
    }
    support::report(&failures)
}

/// A private folder's parent on another filesystem than `source`, where Copy and Move
/// must cross volumes: `$XDG_RUNTIME_DIR` (usually tmpfs) on Unix; on Windows the
/// private installation (`MIDNIGHT_ACCEPTANCE_STAGE_ROOT`, below the user profile) when
/// it is on another drive than the run. None, with the reason, when there is none.
fn other_volume(source: &Path) -> Result<PathBuf, &'static str> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let device = |path: &Path| fs::metadata(path).map(|metadata| metadata.dev()).ok();
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_dir())
            .filter(|runtime| device(runtime) != device(source))
            .ok_or("no second filesystem ($XDG_RUNTIME_DIR)")
    }
    #[cfg(not(unix))]
    {
        let drive = |path: &Path| {
            path.components()
                .next()
                .map(|component| component.as_os_str().to_ascii_uppercase())
        };
        std::env::var_os("MIDNIGHT_ACCEPTANCE_STAGE_ROOT")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_dir())
            .filter(|root| drive(root) != drive(source))
            .ok_or("no private folder on another drive (MIDNIGHT_ACCEPTANCE_STAGE_ROOT)")
    }
}

/// Copy and Move to a private folder on another filesystem ([`other_volume`]), when
/// there is one; removed afterwards.
fn cross_volume(
    app: &mut App,
    context: &mut Context,
    source: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let runtime = match other_volume(source) {
        Ok(runtime) => runtime,
        Err(reason) => {
            context.note(format!("cross-volume Copy and Move not run: {reason}"));
            return Ok(());
        }
    };
    let run = context
        .run_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let volume = crate::fixtures::create_fresh(&runtime, &format!("midnight-acceptance-{run}"))?;
    chmod(&volume, 0o700)?;
    let result = (|| -> Result<(), String> {
        choose(app, &["x.txt"])?;
        let start = support::start_transfer(
            app,
            context,
            TransferSpec {
                destination: Some(volume.as_path()),
                ..TransferSpec::copy()
            },
        )?;
        let (done, decisions) = settle(app, start, &[])?;
        if done.state != CopyOperationState::Completed
            || fs::read(volume.join("x.txt")).ok() != fs::read(source.join("x.txt")).ok()
        {
            failures.push(format!(
                "Copy to another filesystem ended {:?} (questions {decisions:?})",
                done.state
            ));
        }
        support::dismiss_all(app);
        let before = facts(&source.join("y.txt"))?;
        choose(app, &["y.txt"])?;
        // A volume that cannot keep a replaced file recoverable (tmpfs) takes no Move from
        // another drive: the dialog refuses it before any job (finding O-1).
        let takes_move = support::takes_cross_volume_move(&volume);
        let kept = || {
            facts(&source.join("y.txt")).ok().as_ref() == Some(&before)
                && !volume.join("y.txt").exists()
        };
        let started = support::start_transfer(
            app,
            context,
            TransferSpec {
                destination: Some(volume.as_path()),
                ..TransferSpec::moving()
            },
        );
        match started {
            Err(error) if support::is_move_refusal(&error) => {
                context.note(format!(
                    "Move to another filesystem ({}; takes such a Move: {takes_move:?}): {error}",
                    volume.display()
                ));
                if takes_move != Some(false) || !kept() {
                    failures.push(format!(
                        "a Move to another filesystem that takes one ({takes_move:?}) was \
                         refused, or the refusal changed the source: {error}"
                    ));
                }
            }
            Err(error) => failures.push(format!("Move to another filesystem: {error}")),
            Ok(start) => {
                let (done, decisions) = settle(app, start, &[Choice::Skip])?;
                let moved = !source.join("y.txt").exists()
                    && fs::read(volume.join("y.txt")).ok().as_deref()
                        == Some(b"y.txt\n".as_slice());
                let kept = kept();
                context.note(format!(
                    "Move to another filesystem ({}; takes such a Move: {takes_move:?}): {:?}, \
                     moved {moved}, source kept {kept}, questions {decisions:?}",
                    volume.display(),
                    done.state
                ));
                if takes_move == Some(false) {
                    failures.push(format!(
                        "finding O-1: a Move from another drive to {} was admitted as a job \
                         ({:?}), not refused in the dialog",
                        volume.display(),
                        done.state
                    ));
                }
                if !moved && !kept {
                    failures.push(format!(
                        "Move to another filesystem left both sides inconsistent: {:?}",
                        done.state
                    ));
                }
            }
        }
        support::dismiss_all(app);
        Ok(())
    })();
    let _ = crate::fixtures::remove_tree(&volume);
    result
}
