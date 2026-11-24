//! The polish gate on states that take a running application to reach:
//!
//! - `ui_polish_jobs`: Copy job windows paused (without a Details button), waiting,
//!   running, asking to cancel, cancelled, completed, completed with errors (few and
//!   many) and with notes; the name-conflict and the failure (Retry/Skip) decision
//!   windows; and the Jobs window listing them. Every window is captured at its
//!   preferred and at its minimum size with the static and contrast checks; the windows
//!   also get the focus, hover and tooltip checks, and targeted interactive checks: no
//!   Details button, Escape on an active job asks to cancel, a cancelled or finished
//!   job's window leaves no notice or extra window behind.
//! - `ui_polish_viewers`: viewer windows (text, source with a selection, image,
//!   Markdown, hex), a comparator and the archive password dialog (F3 on an encrypted
//!   member of a ZipCrypto archive; static and interactive checks), in a dark and a
//!   light theme, in a staged process with the installed plugins.
use crate::{
    Context, Scenario,
    app::{self, App, Config},
    encode::{self, Entry},
    files_support::{self as support, Choice, LinkChoice, TransferSpec},
    input, inspect, panel, report,
    scenarios::{
        ui_gate::{self as gate, Issue, Target},
        ui_interact as interact,
    },
    tools_support::{self as tools, Tool},
    viewers_stage, viewers_support,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, PreferenceTheme, UiDensity};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const JOBS_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const VIEWERS_TIMEOUT: Duration = Duration::from_secs(25 * 60);

fn texts(window: &Tool) -> Vec<String> {
    inspect::visible_texts(window.window())
}

fn copy_windows(app: &App) -> Vec<Tool> {
    app.tool_windows()
        .into_iter()
        .filter(|window| tools::title(window) == "Copy")
        .collect()
}

/// Wait (bounded) until `condition` holds; false when it did not.
fn wait_for(app: &mut App, limit: Duration, condition: impl Fn(&App) -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        app.idle(Duration::from_millis(30));
        if condition(app) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
    }
}

/// Size a tool window to its preferred size.
fn preferred(app: &mut App, window: &Tool) {
    let (_, size) = gate::window_sizes(window.window());
    gate::size_tool(window, size, 1.0);
    app.settle();
}

/// The shot path of a capture named `name`.
fn shot(name: &str) -> String {
    format!("shots/{}/{name}.png", app::context_name())
}

/// Capture a tool window at its preferred and its minimum size; returns the shot of
/// the preferred size.
fn capture_sizes(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    surface: &str,
    name: &str,
    caption: &str,
) -> String {
    let target = Target::Tool(window.clone());
    preferred(app, window);
    gate::capture(app, context, &target, surface, name, caption);
    let (min, _) = gate::window_sizes(window.window());
    gate::size_tool(window, min, 1.0);
    app.settle();
    gate::capture(
        app,
        context,
        &target,
        &format!("{surface}@min"),
        &format!("{name}-min"),
        &format!("{caption} (minimum size {:.0}×{:.0})", min.0, min.1),
    );
    preferred(app, window);
    shot(name)
}

/// Focus, hover and tooltip checks of a window, recorded on its shot.
fn interactive_checks(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    surface: &str,
    shot: &str,
) {
    let target = Target::Tool(window.clone());
    let mut issues = interact::focus_checks(app, &target);
    issues.extend(interact::hover_checks(app, &target));
    issues.extend(interact::tooltip_checks(app, &target));
    gate::record(context, shot, surface, issues);
}

/// After closing or finishing: no notice and no window beyond `allowed`.
fn no_extra(app: &mut App, allowed: &[String], what: &str) -> Option<Issue> {
    app.idle(Duration::from_millis(700));
    let mut windows: Vec<String> = app
        .tool_windows()
        .iter()
        .map(|window| tools::title(window))
        .collect();
    for title in allowed {
        if let Some(position) = windows.iter().position(|window| window == title) {
            windows.remove(position);
        }
    }
    let notice = tools::shown_notice(app);
    if windows.is_empty() && notice.is_none() {
        return None;
    }
    Some(Issue::new(
        interact::EXTRA_SURFACE,
        format!("after {what}: windows {windows:?}, notice {notice:?}"),
    ))
}

/// Set `path`'s permission bits (on Windows their equivalent, `tools_support::set_mode`).
fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    crate::tools_support::set_mode(path, mode)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// The jobs' source and target folders.
fn job_folders(root: &Path) -> Result<(), String> {
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    // Long enough to be paused while it copies: Windows reads a sparse file's zeros fast
    // enough to finish 256 MiB before the window's Pause is pressed, and a Linux page
    // cache (WSL) copied 256 MiB before it, too.
    let big: u64 = if cfg!(windows) { 1 << 30 } else { 2 << 30 };
    tools::sparse(&source.join("big.bin"), big).or_else(|_| {
        fs::create_dir_all(&source).map_err(|error| error.to_string())?;
        tools::sparse(&source.join("big.bin"), big)
    })?;
    for index in 0..3 {
        write(&source.join(format!("small/file-{index}.txt")), "small\n")?;
    }
    write(&source.join("bad/ok.txt"), "readable\n")?;
    for index in 0..3 {
        let path = source.join(format!("bad/secret-{index}.txt"));
        write(&path, "secret\n")?;
        set_mode(&path, 0o000)?;
    }
    for index in 0..40 {
        let path = source.join(format!(
            "many-bad/an-unreadable-file-with-a-long-name-number-{index:02}.txt"
        ));
        write(&path, "secret\n")?;
        set_mode(&path, 0o000)?;
    }
    write(&source.join("conflict.txt"), "new\n")?;
    write(&target.join("conflict.txt"), "old\n")?;
    fs::create_dir_all(source.join("links")).map_err(|error| error.to_string())?;
    // A dangling link: copied as a link with a note whatever the Links choice. Windows
    // has a directory junction (no privilege needed) to a folder that does not exist.
    #[cfg(unix)]
    std::os::unix::fs::symlink("missing-target", source.join("links/dangling"))
        .map_err(|error| error.to_string())?;
    #[cfg(windows)]
    crate::fixtures::junction(
        &source.join("links/dangling"),
        &source.join("links/missing-target"),
    )?;
    write(&source.join("links/kept.txt"), "kept\n")?;
    Ok(())
}

/// Give the folders' unreadable entries their permissions back.
fn unlock(root: &Path) {
    for folder in ["bad", "many-bad"] {
        let folder = root.join("source").join(folder);
        for entry in fs::read_dir(&folder).into_iter().flatten().flatten() {
            let _ = set_mode(&entry.path(), 0o644);
        }
    }
}

fn select_only(app: &mut App, name: &str) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    panel::select_row(app, LEFT, name)
}

/// Start a Copy of `name` (the left panel's cursor row) to the right panel.
fn start_copy(
    app: &mut App,
    context: &mut Context,
    name: &str,
    spec: TransferSpec<'_>,
) -> Result<usize, String> {
    select_only(app, name)?;
    support::start_transfer(app, context, spec)
}

/// A new Copy window beyond the `before` already open, sized to its preferred size.
fn new_copy_window(app: &mut App, before: usize, what: &str) -> Result<Tool, String> {
    if !wait_for(app, Duration::from_secs(30), move |app| {
        copy_windows(app).len() > before
    }) {
        return Err(format!("{what}: no new Copy window"));
    }
    let window = copy_windows(app)
        .pop()
        .ok_or_else(|| format!("{what}: no Copy window"))?;
    preferred(app, &window);
    Ok(window)
}

fn record_failure(context: &mut Context, surface: &str, error: String) {
    context.outcome.flags.push(report::Flag {
        shot: shot("job-step"),
        kind: "job step failed",
        detail: format!("{surface}: {error}"),
    });
}

/// Paused, waiting, running, details, the cancel question, cancelled and completed.
fn long_copy(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    let before = copy_windows(app).len();
    let start = start_copy(app, context, "big.bin", TransferSpec::copy())?;
    let window = new_copy_window(app, before, "the long copy")?;
    tools::click_last(window.window(), "Pause")?;
    let watched = window.clone();
    if !wait_for(app, Duration::from_secs(20), move |_| {
        texts(&watched).iter().any(|text| text == "Paused")
    }) {
        return Err(format!("the copy did not pause: {:?}", texts(&window)));
    }
    let surface = format!("{prefix}/job/paused");
    let paused = capture_sizes(
        app,
        context,
        &window,
        &surface,
        &format!("{prefix}-job-paused"),
        "Copy job, paused",
    );
    interactive_checks(app, context, &window, &surface, &paused);
    // The window has no Details button: its details are in Jobs' Copy details.
    if tools::has(window.window(), AccessibleRole::Button, "Details") {
        gate::record(
            context,
            &paused,
            &surface,
            vec![Issue::new(
                interact::NO_EFFECT,
                "the progress window still offers Details",
            )],
        );
    }
    // A second copy that waits for the paused one.
    let waiting_before = copy_windows(app).len();
    let mut spec = TransferSpec::copy();
    spec.wait = Some(true);
    let second = start_copy(app, context, "small", spec)?;
    let waiting = new_copy_window(app, waiting_before, "the waiting copy")?;
    let surface_waiting = format!("{prefix}/job/waiting");
    let waiting_shot = capture_sizes(
        app,
        context,
        &waiting,
        &surface_waiting,
        &format!("{prefix}-job-waiting"),
        "Copy job waiting for another job",
    );
    let _ = waiting_shot;
    // Running again.
    tools::click_last(window.window(), "Resume")?;
    app.idle(Duration::from_millis(200));
    if support::job(app, start).is_some_and(|progress| !progress.state.is_terminal()) {
        gate::capture(
            app,
            context,
            &Target::Tool(window.clone()),
            &format!("{prefix}/job/running"),
            &format!("{prefix}-job-running"),
            "Copy job running",
        );
        // Escape on an active job asks whether to cancel it.
        input::key(window.window(), Key::Escape);
        let watched = window.clone();
        if wait_for(app, Duration::from_secs(10), move |_| {
            texts(&watched)
                .iter()
                .any(|text| text == "Cancel this operation?")
        }) {
            let question = capture_sizes(
                app,
                context,
                &window,
                &format!("{prefix}/job/cancel-question"),
                &format!("{prefix}-job-cancel-question"),
                "Cancel this operation?",
            );
            interactive_checks(
                app,
                context,
                &window,
                &format!("{prefix}/job/cancel-question"),
                &question,
            );
            tools::click_last(window.window(), "Cancel operation")?;
        } else if support::job(app, start).is_some_and(|progress| !progress.state.is_terminal()) {
            gate::record(
                context,
                &paused,
                &surface,
                vec![Issue::new(
                    interact::ESCAPE,
                    "Escape on an active Copy job neither closes its window nor asks to cancel",
                )],
            );
        } else {
            context.note(format!(
                "SKIP: {prefix}: the cancel question was not captured: the copy ended before it was asked"
            ));
        }
    } else {
        context.note(format!(
            "SKIP: {prefix}: the running copy and its cancel question were not captured: the copy ended before Resume took effect"
        ));
    }
    let allowed: Vec<String> = vec!["Copy".to_owned(), "Copy".to_owned()];
    wait_for(app, Duration::from_secs(60), move |app| {
        support::job(app, start).is_some_and(|progress| progress.state.is_terminal())
    });
    let ended = support::job(app, start).map(|progress| progress.state);
    let surface_ended = format!("{prefix}/job/cancelled");
    if window.window().is_visible() {
        let cancelled = capture_sizes(
            app,
            context,
            &window,
            &surface_ended,
            &format!("{prefix}-job-cancelled"),
            &format!("Copy job ended {ended:?}"),
        );
        if let Some(issue) = no_extra(app, &allowed, "the job ended") {
            gate::record(context, &cancelled, &surface_ended, vec![issue]);
        }
        interactive_checks(app, context, &window, &surface_ended, &cancelled);
        if inspect::find(window.window(), AccessibleRole::Button, "Close").is_some() {
            tools::click_last(window.window(), "Close")?;
            if let Some(issue) = no_extra(app, &allowed[..1], "closing the ended job's window") {
                gate::record(context, &cancelled, &surface_ended, vec![issue]);
            }
        }
    } else {
        context.note(format!(
            "SKIP: {prefix}: the ended copy ({ended:?}) closed its window before it could be captured"
        ));
    }
    // The waiting copy runs once the first one ended and completes.
    wait_for(app, Duration::from_secs(60), move |app| {
        support::job(app, second).is_some_and(|progress| progress.state.is_terminal())
    });
    if waiting.window().is_visible() {
        let completed = capture_sizes(
            app,
            context,
            &waiting,
            &format!("{prefix}/job/completed"),
            &format!("{prefix}-job-completed"),
            "Copy job completed",
        );
        interactive_checks(
            app,
            context,
            &waiting,
            &format!("{prefix}/job/completed"),
            &completed,
        );
    } else {
        context.note(format!(
            "{prefix}: the completed copy closed its window itself"
        ));
    }
    support::dismiss_all(app);
    Ok(())
}

/// A name conflict: the decision window, then Skip.
fn conflict(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    let start = start_copy(app, context, "conflict.txt", TransferSpec::copy())?;
    let window = support::wait_decision(app, "the name conflict");
    let surface = format!("{prefix}/decision/conflict");
    let shot = capture_sizes(
        app,
        context,
        &window,
        &surface,
        &format!("{prefix}-decision-conflict"),
        "Name conflict",
    );
    interactive_checks(app, context, &window, &surface, &shot);
    support::answer(app, &window, &Choice::Skip)?;
    wait_for(app, Duration::from_secs(30), move |app| {
        support::job(app, start).is_some_and(|progress| progress.state.is_terminal())
    });
    support::dismiss_all(app);
    Ok(())
}

/// Unreadable files: the failure window (Retry, Skip, Skip all), then a job that
/// completed with errors, its details and Copy details.
fn errors(
    app: &mut App,
    context: &mut Context,
    prefix: &str,
    folder: &str,
    label: &str,
) -> Result<(), String> {
    let before = copy_windows(app).len();
    let start = start_copy(app, context, folder, TransferSpec::copy())?;
    let window = support::wait_decision(app, "the failure question");
    let surface = format!("{prefix}/decision/failure-{label}");
    let shot = capture_sizes(
        app,
        context,
        &window,
        &surface,
        &format!("{prefix}-decision-failure-{label}"),
        "Copy failure with Retry and Skip",
    );
    if label == "few" {
        interactive_checks(app, context, &window, &surface, &shot);
    }
    support::answer(app, &window, &Choice::SkipAllErrors)?;
    wait_for(app, Duration::from_secs(60), move |app| {
        support::job(app, start).is_some_and(|progress| progress.state.is_terminal())
    });
    let ended = support::job(app, start).map(|progress| progress.state);
    let job_window = match new_copy_window(app, before, "the job with errors") {
        Ok(window) => window,
        Err(_) => {
            context.note(format!(
                "SKIP: {prefix}: the {label} errors job ({ended:?}) shows no window to capture"
            ));
            support::dismiss_all(app);
            return Ok(());
        }
    };
    let surface = format!("{prefix}/job/errors-{label}");
    let job_shot = capture_sizes(
        app,
        context,
        &job_window,
        &surface,
        &format!("{prefix}-job-errors-{label}"),
        &format!("Copy job ended {ended:?}"),
    );
    if label == "few" {
        interactive_checks(app, context, &job_window, &surface, &job_shot);

        let commands = app.recent_events().len();
        let notes_before = texts(&job_window);
        if tools::click_last(job_window.window(), "Copy details").is_ok() {
            app.idle(Duration::from_millis(400));
            let sent = app
                .recent_events()
                .iter()
                .rev()
                .take(4)
                .any(|line| line.starts_with('→'));
            if !sent && texts(&job_window) == notes_before && app.recent_events().len() == commands
            {
                gate::record(
                    context,
                    &job_shot,
                    &surface,
                    vec![Issue::new(
                        interact::NO_EFFECT,
                        "\"Copy details\" changes nothing observable",
                    )],
                );
            }
        }
    }
    if inspect::find(job_window.window(), AccessibleRole::Button, "Close").is_some() {
        tools::click_last(job_window.window(), "Close")?;
        if let Some(issue) = no_extra(app, &[], "closing the job with errors") {
            gate::record(context, &job_shot, &surface, vec![issue]);
        }
    }
    support::dismiss_all(app);
    Ok(())
}

/// A copy of a folder with a dangling link (on Windows a dangling junction) with Links
/// set to "Copy the files links point to": the link has nothing to copy, stays a link
/// and the job completes with a note.
fn notes(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    let before = copy_windows(app).len();
    let mut spec = TransferSpec::copy();
    // "Copy the files links point to", chosen in the open Links list with the keys.
    spec.links = Some(LinkChoice::CopyFiles);
    let start = start_copy(app, context, "links", spec)?;
    wait_for(app, Duration::from_secs(60), move |app| {
        support::job(app, start).is_some_and(|progress| progress.state.is_terminal())
            || support::decision_window(app).is_some()
    });
    let asked = support::decision_window(app).is_some();
    if let Some(window) = support::decision_window(app) {
        let _ = support::answer(app, &window, &Choice::Skip);
    }
    wait_for(app, Duration::from_secs(60), move |app| {
        support::job(app, start).is_some_and(|progress| progress.state.is_terminal())
    });
    // The state this capture stands for: the job ended with its note, unasked.
    let ended = support::job(app, start).map(|progress| {
        (
            progress.state,
            progress.note_count,
            progress.notes.first().cloned(),
        )
    });
    match &ended {
        Some((state, count, _)) if state.is_terminal() && *count > 0 && !asked => {
            context.note(format!("{prefix}: the job with notes ended {ended:?}"));
        }
        _ => {
            support::dismiss_all(app);
            return Err(format!(
                "the copy of a dangling link did not end with a note (state, notes, first note: {ended:?}; asked a question: {asked})"
            ));
        }
    }
    match new_copy_window(app, before, "the job with notes") {
        Ok(window) => {
            capture_sizes(
                app,
                context,
                &window,
                &format!("{prefix}/job/notes"),
                &format!("{prefix}-job-notes"),
                "Copy job with notes",
            );
        }
        Err(_) => {
            // Its window closed by itself: Jobs › Open shows it again.
            support::dismiss_all(app);
            open_from_jobs(app, context, prefix, "note")?;
        }
    }
    support::dismiss_all(app);
    Ok(())
}

/// Open the Jobs window, select the newest job whose row mentions `wanted`, press Open
/// and capture the job's window.
fn open_from_jobs(
    app: &mut App,
    context: &mut Context,
    prefix: &str,
    wanted: &str,
) -> Result<(), String> {
    let before = app.tool_windows().len();
    app.window.invoke_show_operations();
    if !wait_for(app, Duration::from_secs(20), move |app| {
        app.tool_windows().len() > before
    }) {
        return Err("the Jobs window did not open".into());
    }
    let jobs = tools::tool_titled(app, "Jobs").ok_or("no window titled Jobs")?;
    preferred(app, &jobs);
    let rows = inspect::controls(jobs.window(), AccessibleRole::ListItem);
    let row = rows
        .iter()
        .rev()
        .find(|(_, item)| {
            tools::item_texts(item)
                .iter()
                .any(|text| text.to_lowercase().contains(wanted))
        })
        .or_else(|| rows.last())
        .map(|(_, item)| item.clone())
        .ok_or("the Jobs window lists no job")?;
    input::click_at(jobs.window(), inspect::center(&row));
    app.settle();
    let copies = copy_windows(app).len();
    tools::click_last(jobs.window(), "Open")?;
    let window = new_copy_window(app, copies, "the job opened from Jobs")?;
    capture_sizes(
        app,
        context,
        &window,
        &format!("{prefix}/job/notes"),
        &format!("{prefix}-job-notes"),
        "Copy job with notes (opened from Jobs)",
    );
    support::dismiss_all(app);
    Ok(())
}

/// The Jobs window listing every job of the session.
fn jobs_list(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    let before = app.tool_windows().len();
    app.window.invoke_show_operations();
    if !wait_for(app, Duration::from_secs(20), move |app| {
        app.tool_windows().len() > before
    }) {
        return Err("the Jobs window did not open".into());
    }
    let window = tools::tool_titled(app, "Jobs").ok_or("no window titled Jobs")?;
    let surface = format!("{prefix}/jobs-list");
    let shot = capture_sizes(
        app,
        context,
        &window,
        &surface,
        &format!("{prefix}-jobs-list"),
        "Jobs with finished jobs",
    );
    interactive_checks(app, context, &window, &surface, &shot);
    tools::close_tool(app, &window, "the Jobs window");
    Ok(())
}

fn jobs_body(context: &mut Context) -> Result<(), String> {
    for (theme, density) in [
        (PreferenceTheme::Blue, UiDensity::Desktop),
        (PreferenceTheme::Dark, UiDensity::Touch),
    ] {
        let prefix = format!("{theme:?}-{density:?}");
        let root = context.work.join(&prefix);
        job_folders(&root)?;
        let mut config = Config::new(root.join("source"), root.join("target"));
        config.theme = theme;
        config.density = density;
        config.clipboard = true;
        let mut app = App::start(config);
        type Step = fn(&mut App, &mut Context, &str) -> Result<(), String>;
        let steps: [(&str, Step); 6] = [
            ("long copy", long_copy),
            ("conflict", conflict),
            ("few errors", |app, context, prefix| {
                errors(app, context, prefix, "bad", "few")
            }),
            ("many errors", |app, context, prefix| {
                errors(app, context, prefix, "many-bad", "many")
            }),
            ("notes", notes),
            ("jobs list", jobs_list),
        ];
        for (name, step) in steps {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                step(&mut app, context, &prefix)
            }));
            let error = match result {
                Ok(Ok(())) => continue,
                Ok(Err(error)) => error,
                Err(panic) => panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                    .unwrap_or_else(|| "panicked".to_owned()),
            };
            record_failure(
                context,
                &format!("{prefix}/{name}"),
                error.lines().next().unwrap_or_default().to_owned(),
            );
            context.note(format!("{prefix} {name} failed: {error}"));
            support::dismiss_all(&mut app);
        }
        app.shutdown()?;
        unlock(&root);
        // The pass's copies are not needed any more; free their space at once.
        crate::fixtures::restore_owner_write(&root);
        let _ = fs::remove_dir_all(&root);
    }
    gate::verdict(context, "ui_polish_jobs")
}

pub fn jobs(context: &mut Context) -> Result<(), String> {
    jobs_body(context)
}

// ---------------------------------------------------------------------------------------
// Viewers and comparators

fn viewer_capture(
    app: &mut App,
    context: &mut Context,
    viewing: &viewers_support::Viewing,
    surface: &str,
    name: &str,
    caption: &str,
) {
    gate::size_tool(&viewing.software, (1100.0, 720.0), 1.0);
    app.settle();
    viewers_support::paint_tool_windows(app);
    gate::capture(
        app,
        context,
        &Target::Tool(viewing.software.clone()),
        surface,
        name,
        caption,
    );
}

fn view_file(
    app: &mut App,
    context: &mut Context,
    prefix: &str,
    folder: &str,
    file: &str,
    name: &str,
    select: bool,
) -> Result<(), String> {
    panel::enter(app, LEFT, folder)?;
    panel::select_row(app, LEFT, file)?;
    let viewing = viewers_support::view(app, false, Duration::from_secs(30))?;
    viewing
        .result
        .clone()
        .map_err(|error| format!("{folder}/{file}: {error}"))?;
    viewer_capture(
        app,
        context,
        &viewing,
        &format!("{prefix}/viewer/{name}"),
        &format!("{prefix}-viewer-{name}"),
        &format!("Viewer: {folder}/{file}"),
    );
    if select {
        viewing.click_content(app);
        input::chord(viewing.slint(), &[Key::Control], "a");
        app.idle(Duration::from_millis(300));
        viewers_support::paint_tool_windows(app);
        viewer_capture(
            app,
            context,
            &viewing,
            &format!("{prefix}/viewer/{name}-selected"),
            &format!("{prefix}-viewer-{name}-selected"),
            &format!("Viewer: {folder}/{file}, all selected"),
        );
    }
    viewing.close(app)?;
    panel::leave(app, LEFT)?;
    Ok(())
}

fn comparator(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    panel::enter(app, LEFT, "compare")?;
    input::chord(app.slint(), &[Key::Control], "d");
    for name in ["left.txt", "right.txt"] {
        panel::select_row(app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    let before = viewers_support::plugins(app)?.compared.borrow().len();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let compared = viewers_support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .cloned()
        .ok_or("Ctrl+Shift+C opened no comparator")?;
    let window = compared
        .window
        .upgrade()
        .ok_or("the comparator closed at once")?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let software = viewers_support::software_window(app, window.window())
        .ok_or("the comparator window is not visible")?;
    gate::size_tool(&software, (1100.0, 720.0), 1.0);
    app.idle(Duration::from_secs(2));
    viewers_support::paint_tool_windows(app);
    gate::capture(
        app,
        context,
        &Target::Tool(software.clone()),
        &format!("{prefix}/compare/text"),
        &format!("{prefix}-compare-text"),
        "Comparator: two texts with differences",
    );
    input::key(window.window(), Key::Escape);
    wait_for(app, Duration::from_secs(10), move |app| {
        viewers_support::software_window(app, window.window()).is_none()
    });
    input::chord(app.slint(), &[Key::Control], "d");
    panel::leave(app, LEFT)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------
// The archive password dialog

/// A ZipCrypto archive whose one member needs a password. The gate never types it, so
/// nothing it presses can unlock the archive.
const LOCKED: &str = "locked.zip";
const SECRET: &str = "secret.txt";
const LOCKED_PASSWORD: &str = "polish-gate-secret";
/// A hang guard for the archive's listing and for the dialog.
const LOCKED_WAIT: Duration = Duration::from_secs(30);

fn write_locked(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let bytes = encode::zip_entries(&[
        Entry::file(SECRET, "The polish gate's secret.\n").zip_crypto(LOCKED_PASSWORD)
    ]);
    fs::write(folder.join(LOCKED), bytes).map_err(|error| format!("{LOCKED}: {error}"))
}

/// The password surface's idle state: inside `locked.zip` with the cursor on its
/// encrypted member and nothing open over the panel.
fn locked_member(app: &mut App) -> Result<(), String> {
    if viewers_support::password_dialog(app).is_some() {
        viewers_support::dismiss_password(app, viewers_support::Dismiss::Escape)?;
    }
    support::dismiss_all(app);
    if !app.names(LEFT).iter().any(|name| name == SECRET) {
        panel::select_row(app, LEFT, LOCKED)?;
        input::key(app.slint(), Key::Return);
        if !wait_for(app, LOCKED_WAIT, |app| {
            app.names(LEFT).iter().any(|name| name == SECRET)
        }) {
            return Err(format!(
                "{LOCKED} did not open as a folder: rows {:?}",
                app.names(LEFT)
            ));
        }
    }
    panel::select_row(app, LEFT, SECRET)
}

/// F3 on the encrypted member opens the archive password dialog.
fn open_password(app: &mut App) -> Result<Target, String> {
    locked_member(app)?;
    input::key(app.slint(), Key::F3);
    viewers_support::wait_password_dialog(app, LOCKED_WAIT)?;
    Ok(Target::Main)
}

/// The archive password dialog over the main window: the static and contrast checks,
/// then the interactive ones (Tab, focus, Escape, Enter with the empty field, every
/// button, hover, no extra window or notice after a dismissal).
fn password_dialog(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    let surface_name = format!("{prefix}/password");
    let shot = format!("{prefix}-password");
    open_password(app)?;
    gate::capture(
        app,
        context,
        &Target::Main,
        &surface_name,
        &shot,
        "Archive password",
    );
    locked_member(app)?;
    let mut surface = interact::Surface::new(
        surface_name.clone(),
        open_password,
        |app: &App, _: &Target| viewers_support::password_dialog(app).is_some(),
        |app: &mut App| {
            let _ = locked_member(app);
        },
    );
    let outcome = interact::check_surface(app, &mut surface);
    interact::record(
        context,
        &surface_name,
        &format!("shots/{}/{shot}.png", app::context_name()),
        outcome,
    );
    locked_member(app)
}

fn viewers_body(context: &mut Context) -> Result<(), String> {
    if viewers_stage::installed_packages("viewers") == 0 {
        return Err("SKIP: no viewer packages are staged".into());
    }
    for (theme, density) in [
        (PreferenceTheme::Dark, UiDensity::Desktop),
        (PreferenceTheme::Light, UiDensity::Touch),
    ] {
        let prefix = format!("{theme:?}-{density:?}");
        let root = context.fixture.root.clone();
        let mut config = Config::new(&root, &context.work);
        config.theme = theme;
        config.density = density;
        config.plugins = Some(context.work.join(format!("plugins-{prefix}")));
        let mut app = App::start(config);
        let files: [(&str, &str, &str, bool); 5] = [
            ("text", "utf8.txt", "text", true),
            ("code", "main.rs", "source", true),
            ("images", "photo.png", "image", false),
            ("docs", "README.md", "markdown", false),
            ("compare", "binary-a.bin", "hex", true),
        ];
        for (folder, file, name, select) in files {
            if let Err(error) = view_file(&mut app, context, &prefix, folder, file, name, select) {
                record_failure(context, &format!("{prefix}/viewer/{name}"), error.clone());
                context.note(format!("{prefix} viewer {name} failed: {error}"));
                support::dismiss_all(&mut app);
                let fixture_root = context.fixture.root.clone();
                if app.location(LEFT).as_deref() != Some(fixture_root.as_path()) {
                    let _ = panel::leave(&mut app, LEFT);
                }
            }
        }
        if let Err(error) = comparator(&mut app, context, &prefix) {
            record_failure(context, &format!("{prefix}/compare"), error.clone());
            context.note(format!("{prefix} comparator failed: {error}"));
        }
        app.shutdown()?;
        drop(app);
        // The archive password dialog, from a folder of this run holding the archive.
        let locked = context.work.join(format!("locked-{prefix}"));
        write_locked(&locked)?;
        let mut config = Config::new(&locked, &context.work);
        config.theme = theme;
        config.density = density;
        config.plugins = Some(context.work.join(format!("plugins-locked-{prefix}")));
        let mut app = App::start(config);
        if let Err(error) = password_dialog(&mut app, context, &prefix) {
            record_failure(context, &format!("{prefix}/password"), error.clone());
            context.note(format!("{prefix} archive password dialog failed: {error}"));
        }
        app.shutdown()?;
    }
    gate::verdict(context, "ui_polish_viewers")
}

pub fn viewers(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, VIEWERS_TIMEOUT, viewers_body)
}

/// The registrations of the two scenarios (`scenarios::all`).
pub fn jobs_scenario() -> Scenario {
    Scenario {
        name: "ui_polish_jobs",
        area: "ui",
        summary: "Polish gate on job windows (paused, waiting, running, cancel question, cancelled, completed, errors, notes), decision windows (conflict, Retry/Skip failure) and Jobs, at preferred and minimum size.",
        run: jobs,
        timeout: JOBS_TIMEOUT,
    }
}

pub fn viewers_scenario() -> Scenario {
    Scenario {
        name: "ui_polish_viewers",
        area: "ui",
        summary: "Polish gate on viewer windows (text, source with a selection, image, Markdown, hex), a comparator and the archive password dialog in a dark and a light theme (staged with the installed plugins).",
        run: viewers,
        timeout: VIEWERS_TIMEOUT,
    }
}
