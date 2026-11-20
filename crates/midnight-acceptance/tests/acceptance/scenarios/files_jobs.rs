//! Copy jobs side by side and their windows: "Wait for other operations to finish"
//! off (a second job runs at once beside a paused one) and on (a third waits for the
//! paused one and runs after it), the Jobs window's states and its Cancel job, the
//! progress window's totals, time remaining, speed chart and per-file progress, Cancel
//! operation, the Jobs window's two Clear buttons, and the job windows' wording.
use crate::{
    Context,
    app::{self, App, Config},
    files_support::{self as support, Choice, Pending, TransferSpec, compare_trees, tree_facts},
    inspect, panel,
    scenarios::transfers::copy_tree,
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CopyOperationState, PaneId, TransferWaitReason};
use slint::platform::{Key, WindowAdapter};
use std::{fs, path::Path, time::Duration};

const LEFT: PaneId = PaneId::Left;
const SIZE: (u32, u32) = (760, 560);
/// Announced job-window behavior that may not be implemented yet.
const SLOW_FILE_BAR: &str = "a per-file bar for slow files without Details";
const WAIT_OFF: &str = "a job whose Wait for other operations is off starts at once";
/// The size of the file paused in the middle of its copy.
const LARGE: u64 = 1 << 30;

fn texts(window: &Tool) -> Vec<String> {
    inspect::visible_texts(window.window())
}

/// The visible progress windows titled `title`, oldest first.
fn titled(app: &App, title: &str) -> Vec<Tool> {
    app.tool_windows()
        .into_iter()
        .filter(|window| tools::title(window) == title)
        .collect()
}

/// Wait for a new window titled `title` beyond the `before` already open and size it.
fn new_window(app: &mut App, title: &str, before: usize) -> Result<Tool, String> {
    let wanted = title.to_owned();
    app.until(&format!("a new {title} window"), move |app| {
        titled(app, &wanted).len() > before
    });
    let window = titled(app, title)
        .pop()
        .ok_or_else(|| format!("no {title} window"))?;
    app::layout_tool(&window, SIZE.0, SIZE.1);
    app.settle();
    Ok(window)
}

/// Wait until `window` shows a text that satisfies `wanted` (or `limit` passed).
fn wait_text(
    app: &mut App,
    window: &Tool,
    limit: Duration,
    wanted: impl Fn(&str) -> bool,
) -> Option<String> {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if let Some(text) = texts(window).into_iter().find(|text| wanted(text)) {
            return Some(text);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        app.idle(Duration::from_millis(20));
    }
}

/// Pause the running job `start` with its window's Pause button once it copies, and
/// wait until the window says Paused.
fn pause(app: &mut App, window: &Tool, start: usize, what: &str) -> Result<(), String> {
    app.until(&format!("{what} to run"), move |app| {
        support::job(app, start).is_some_and(|job| {
            job.state.is_terminal()
                || (job.state == CopyOperationState::Running
                    && (job.copied_bytes > 0 || job.files_completed > 0))
        })
    });
    if support::job(app, start).is_some_and(|job| job.state.is_terminal()) {
        return Err(format!("{what} ended before it could be paused"));
    }
    tools::click_last(window.window(), "Pause")?;
    if wait_text(app, window, Duration::from_secs(30), |text| {
        text == "Paused"
    })
    .is_none()
        || support::job(app, start).is_some_and(|job| job.state.is_terminal())
    {
        return Err(format!(
            "{what} could not be paused: {:?}",
            support::job(app, start).map(|job| job.state)
        ));
    }
    Ok(())
}

/// Every file below `target` is a whole copy of the same file below `source`.
fn whole_copies(source: &Path, target: &Path) -> Result<usize, String> {
    let wanted = tree_facts(source)?;
    let found = tree_facts(target)?;
    let mut files = 0;
    for (path, facts) in &found {
        match wanted.get(path) {
            Some(expected) if expected.content == facts.content => {
                if facts.content != support::Content::Folder {
                    files += 1;
                }
            }
            _ => return Err(format!("{path} is not a whole copy: {:?}", facts.content)),
        }
    }
    Ok(files)
}

/// The words a job window must not use; returns the offending texts.
fn published(texts: &[String]) -> Vec<String> {
    texts
        .iter()
        .filter(|text| text.to_lowercase().contains("publish"))
        .cloned()
        .collect()
}

pub fn transfer_jobs(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "source")?;
    let targets: Vec<std::path::PathBuf> = (1..=6)
        .map(|index| support::scratch(context, &format!("t{index}")))
        .collect::<Result<_, _>>()?;
    copy_tree(&context.fixture.path("many"), &source.join("many"))?;
    let small = source.join("small");
    fs::create_dir(&small).map_err(|error| support::io_error(&small, error))?;
    for index in 0..3 {
        let path = small.join(format!("small-{index}.txt"));
        fs::write(&path, format!("small {index}\n"))
            .map_err(|error| support::io_error(&path, error))?;
    }
    // A file large enough to pause in the middle of its copy: 1 GiB, sparse, so only
    // its copy takes disk space (removed once checked).
    let large_file = source.join("large.bin");
    fs::File::create(&large_file)
        .and_then(|file| file.set_len(LARGE))
        .map_err(|error| support::io_error(&large_file, error))?;
    // A folder whose copy ends with an error: one of its files cannot be read.
    let faulty = source.join("faulty");
    fs::create_dir(&faulty).map_err(|error| support::io_error(&faulty, error))?;
    for name in ["ok.txt", "unreadable.txt"] {
        let path = faulty.join(name);
        fs::write(&path, name).map_err(|error| support::io_error(&path, error))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = faulty.join("unreadable.txt");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000))
            .map_err(|error| support::io_error(&path, error))?;
    }
    let faulty_target = support::scratch(context, "t-faulty")?;
    let mut app = App::start(Config::new(&source, &targets[0]));
    let mut failures = Vec::new();
    let mut pending = Pending::default();
    let mut seen_texts: Vec<String> = Vec::new();

    // The dialog offers the queueing option, unchecked by default.
    panel::select_row(&mut app, LEFT, "small")?;
    panel::wait_input(&mut app);
    crate::input::key(app.slint(), Key::F5);
    app.until("the Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    let default_wait = inspect::find(app.slint(), AccessibleRole::Checkbox, support::WAIT_OPTION)
        .map(|item| tools::checked(&item));
    if default_wait != Some(false) {
        failures.push(format!(
            "the Copy dialog's {:?} option is {default_wait:?}, not shown unchecked",
            support::WAIT_OPTION
        ));
    }
    app.screenshot(
        context,
        "copy-dialog-options",
        "Copy dialog with its options",
    );
    crate::input::key(app.slint(), Key::Escape);
    app.until("the Copy dialog to close", |app| {
        app.window.get_copy_draft_token().is_empty()
    });

    // Job 1 copies many/ and is paused as soon as it runs.
    let before = titled(&app, "Copy").len();
    panel::select_row(&mut app, LEFT, "many")?;
    let first = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(targets[0].as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let window1 = new_window(&mut app, "Copy", before)?;
    pause(&mut app, &window1, first, "job 1")?;
    seen_texts.extend(texts(&window1));
    // A framed window's title bar closes it; the header has no × of its own.
    let corner = support::header_close_buttons(&window1);
    if !corner.is_empty() {
        failures.push(format!(
            "the progress window's header has an internal ×: {corner:?}"
        ));
    }
    let held = support::job(&app, first).map(|job| job.files_completed);

    // Job 2, without waiting: it runs at once and completes beside the paused job 1.
    panel::select_row(&mut app, LEFT, "small")?;
    let second = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(targets[1].as_path()),
            wait: Some(false),
            ..TransferSpec::copy()
        },
    )?;
    // At most 20 s: a job that is not told to wait must not wait for the paused one.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !support::job(&app, second).is_some_and(|job| job.state.is_terminal())
        && std::time::Instant::now() < deadline
    {
        app.idle(Duration::from_millis(50));
    }
    let job2 = support::job(&app, second);
    let second_done = job2
        .as_ref()
        .is_some_and(|job| job.state == CopyOperationState::Completed);
    pending.check(
        "Wait off: a second job runs at once beside a paused one",
        WAIT_OFF,
        if second_done {
            Ok(())
        } else {
            Err(format!(
                "after 20 s job 2 is {:?} waiting for {:?}",
                job2.as_ref().map(|job| job.state),
                job2.as_ref().and_then(|job| job.wait_reason)
            ))
        },
    );
    let job1 = support::job(&app, first);
    if job1
        .as_ref()
        .is_none_or(|job| job.state.is_terminal() || !job.paused)
        || job1.as_ref().map(|job| job.files_completed) != held
    {
        failures.push(format!(
            "job 1 did not stay paused while job 2 ran: {:?}",
            job1.map(|job| (job.state, job.paused, job.files_completed))
        ));
    }

    // Job 3 waits for the earlier, paused job 1.
    let before = titled(&app, "Copy").len();
    let third = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(targets[2].as_path()),
            wait: Some(true),
            ..TransferSpec::copy()
        },
    )?;
    // At most 20 s for job 3 to report why it waits.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !support::job(&app, third)
        .is_some_and(|job| job.wait_reason.is_some() || job.state.is_terminal())
        && std::time::Instant::now() < deadline
    {
        app.idle(Duration::from_millis(50));
    }
    app.idle(Duration::from_secs(1));
    let job3 = support::job(&app, third);
    let waits = job3.as_ref().is_some_and(|job| {
        !job.state.is_terminal()
            && job.files_completed == 0
            && job.wait_reason == Some(TransferWaitReason::EarlierTransfers)
    });
    let observed = format!(
        "job 3 (waiting) is {:?}",
        job3.map(|job| (job.state, job.wait_reason, job.files_completed))
    );
    if second_done {
        if !waits {
            failures.push(format!("{observed}; it should wait for the paused job 1"));
        }
    } else {
        // Job 2 still waits too (pending above), which changes what job 3 waits for.
        pending.check(
            "Wait on: a third job waits for the earlier jobs",
            WAIT_OFF,
            if waits { Ok(()) } else { Err(observed) },
        );
    }
    match new_window(&mut app, "Copy", before) {
        Err(error) => failures.push(error),
        Ok(window3) => {
            let queued = texts(&window3);
            if !queued
                .iter()
                .any(|text| text.starts_with("Queued — waiting") || text.starts_with("Waiting for"))
            {
                failures.push(format!("job 3's window does not say it waits: {queued:?}"));
            }
            if second_done
                && !queued
                    .iter()
                    .any(|text| text == "Queued — waiting for earlier transfers")
            {
                failures.push(format!(
                    "job 3's window does not say it waits for earlier transfers: {queued:?}"
                ));
            }
            seen_texts.extend(queued);
            app.screenshot_window(context, &window3, "job3-waiting", "Job 3 waits for job 1");
        }
    }
    // The Jobs window lists the three jobs and their states.
    let before = app.tool_windows().len();
    app.window.invoke_show_operations();
    let jobs = tools::wait_tool(&mut app, "the Jobs window", before, SIZE);
    let states: Vec<String> = inspect::controls(jobs.window(), AccessibleRole::ListItem)
        .into_iter()
        .map(|(_, item)| tools::item_texts(&item).get(1).cloned().unwrap_or_default())
        .collect();
    let corner = support::header_close_buttons(&jobs);
    if !corner.is_empty() {
        failures.push(format!(
            "the Jobs window's header has an internal ×: {corner:?}"
        ));
    }
    seen_texts.extend(texts(&jobs));
    app.screenshot_window(
        context,
        &jobs,
        "jobs-three",
        "Jobs: waiting, completed and paused",
    );
    let expect: &[&str] = if second_done {
        &["Queued — waiting for earlier transfers", "Completed"]
    } else {
        &[]
    };
    if states.len() != 3
        || !expect
            .iter()
            .all(|state| states.iter().any(|found| found == state))
        || !states.iter().any(|state| state.contains("Paused"))
    {
        failures.push(format!(
            "the Jobs window lists {states:?}; expected waiting, completed and paused jobs"
        ));
    }
    tools::close_tool(&mut app, &jobs, "the Jobs window");

    // Resume job 1: it completes, and only then job 3 runs. Its thousands of small
    // files, each written, synced and read back, take minutes on a busy disk.
    tools::click_last(window1.window(), "Resume")?;
    let resumed = std::time::Instant::now();
    app.until_for(
        "job 1 to end after Resume",
        Duration::from_secs(300),
        move |app| support::job(app, first).is_some_and(|job| job.state.is_terminal()),
    );
    let done = support::wait_job(&mut app, first);
    context.note(format!(
        "job 1 ({} files) ended {:?} {:.1} s after Resume",
        done.files_completed,
        done.state,
        resumed.elapsed().as_secs_f64()
    ));
    if done.state != CopyOperationState::Completed {
        failures.push(format!("job 1 ended {:?} after Resume", done.state));
    }
    let done2 = support::wait_job(&mut app, second);
    if done2.state != CopyOperationState::Completed {
        failures.push(format!("job 2 (not waiting) ended {:?}", done2.state));
    }
    let done3 = support::wait_job(&mut app, third);
    if done3.state != CopyOperationState::Completed {
        failures.push(format!("job 3 ended {:?}", done3.state));
    }
    let first_id = support::job_id(&app, first).ok_or("job 1 reported nothing")?;
    let third_id = support::job_id(&app, third).ok_or("job 3 reported nothing")?;
    let job1_end = app
        .copies
        .iter()
        .position(|job| job.id == first_id && job.state.is_terminal());
    // Job 3's first report of copied work.
    let job3_ran = app
        .copies
        .iter()
        .position(|job| job.id == third_id && (job.files_completed > 0 || job.copied_bytes > 0));
    if job3_ran.is_none() || job3_ran < job1_end {
        failures.push(format!(
            "job 3 ran before job 1 ended (report {job3_ran:?}, job 1 ended at {job1_end:?})"
        ));
    }
    for (folder, target) in [
        ("many", &targets[0]),
        ("small", &targets[1]),
        ("small", &targets[2]),
    ] {
        let differences = compare_trees(
            &tree_facts(&source.join(folder))?,
            &tree_facts(&target.join(folder))?,
            false,
        );
        if !differences.is_empty() {
            failures.push(format!(
                "{folder}/ in {} differs: {:?}",
                target.display(),
                differences.iter().take(5).collect::<Vec<_>>()
            ));
        }
    }
    support::dismiss_all(&mut app);

    // The progress window of the large file: totals, time remaining, the speed chart
    // and, under Details, the current file's progress.
    let before = titled(&app, "Copy").len();
    panel::select_row(&mut app, LEFT, "large.bin")?;
    let large = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(targets[3].as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let window4 = new_window(&mut app, "Copy", before)?;
    // Paused in the middle of the file, the window holds still for reading.
    pause(&mut app, &window4, large, "the large file's copy")?;
    let without_details = texts(&window4)
        .into_iter()
        .find(|text| text.starts_with("Current file written"));
    tools::click_last(window4.window(), "Details")?;
    let per_file = wait_text(&mut app, &window4, Duration::from_secs(10), |text| {
        text.starts_with("Current file written")
    });
    let shown = texts(&window4);
    seen_texts.extend(shown.clone());
    app.screenshot_window(
        context,
        &window4,
        "progress-details",
        "A large file's progress with Details",
    );
    let has = |prefix: &str| shown.iter().any(|text| text.starts_with(prefix));
    for (prefix, what) in [
        ("Time remaining:", "the time remaining"),
        ("Scale:", "the speed chart's scale"),
    ] {
        if !has(prefix) {
            failures.push(format!("the progress window shows no {what}: {shown:?}"));
        }
    }
    if !shown
        .iter()
        .any(|text| text.starts_with("Files: ") && text.contains("Bytes: "))
    {
        failures.push(format!(
            "the progress window shows no totals of files and bytes: {shown:?}"
        ));
    }
    if per_file.is_none() && !has("Current file written") {
        failures.push(format!(
            "Details shows no progress of the current file: {shown:?}"
        ));
    }
    pending.check(
        "the progress of a slow file shows without Details",
        SLOW_FILE_BAR,
        without_details.map(|_| ()).ok_or_else(|| {
            "no \"Current file written\" progress before Details was pressed".to_owned()
        }),
    );
    context.note(format!(
        "large file progress: {:?}",
        shown
            .iter()
            .filter(|text| [
                "Files: ",
                "Time remaining",
                "Scale:",
                "Current file",
                "Write:"
            ]
            .iter()
            .any(|prefix| text.starts_with(prefix)))
            .collect::<Vec<_>>()
    ));
    if tools::has(window4.window(), AccessibleRole::Button, "Resume") {
        tools::click_last(window4.window(), "Resume")?;
    }
    let done = support::wait_job(&mut app, large);
    let copied = targets[3].join("large.bin");
    let size = fs::metadata(&copied).map(|metadata| metadata.len()).ok();
    if done.state != CopyOperationState::Completed || size != Some(LARGE) {
        failures.push(format!(
            "the large file's copy ended {:?} with {size:?} bytes",
            done.state
        ));
    }
    // Its 1 GiB is not kept in the run folder.
    let _ = fs::remove_file(&copied);
    let watched = window4.clone();
    app.until("the completed copy's window to close", move |_| {
        !watched.window().is_visible()
    });

    // Cancel operation: the question at a safe checkpoint, then only whole files.
    let before = titled(&app, "Copy").len();
    panel::select_row(&mut app, LEFT, "many")?;
    let cancelled = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(targets[4].as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let window5 = new_window(&mut app, "Copy", before)?;
    tools::click_last(window5.window(), "Cancel operation")?;
    if wait_text(&mut app, &window5, Duration::from_secs(30), |text| {
        text == "Cancel this operation?"
    })
    .is_none()
    {
        failures.push(format!(
            "Cancel operation asked no question: {:?}",
            texts(&window5)
        ));
    } else {
        tools::click_last(window5.window(), "Cancel operation")?;
    }
    let done = support::wait_job(&mut app, cancelled);
    app.idle(Duration::from_millis(500));
    let notice = support::shown_notice(&app);
    seen_texts.extend(texts(&window5));
    if let Some((title, body)) = &notice {
        seen_texts.push(format!("{title} {body}"));
    }
    if done.state != CopyOperationState::Cancelled {
        failures.push(format!("the cancelled copy ended {:?}", done.state));
    }
    // The window shows how the job ended; no notice repeats it, and the outcome
    // replaces the progress block.
    if let Some(notice) = &notice {
        failures.push(format!(
            "a notice follows the cancelled copy whose window is open: {notice:?}"
        ));
    }
    let ended = texts(&window5);
    if !ended
        .iter()
        .any(|text| text.contains("Copy cancelled; files already copied are kept."))
    {
        failures.push(format!(
            "the cancelled copy's window does not say what was kept: {ended:?}"
        ));
    }
    if ended.iter().any(|text| text.starts_with("Items remaining")) {
        failures.push(format!(
            "the ended copy still shows Items remaining: {ended:?}"
        ));
    }
    match whole_copies(&source.join("many"), &targets[4].join("many")) {
        Ok(files) => context.note(format!("Cancel operation kept {files} whole files")),
        Err(error) => failures.push(format!("the cancelled copy left {error}")),
    }
    app.screenshot_window(context, &window5, "cancelled", "The cancelled copy");
    support::dismiss_all(&mut app);
    if window5.window().is_visible() {
        let _ = tools::click_last(window5.window(), "Close");
    }

    // The Jobs window's Cancel job stops a paused job at once, without a question.
    let before = titled(&app, "Copy").len();
    panel::select_row(&mut app, LEFT, "many")?;
    let stopped = support::start_transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(targets[5].as_path()),
            ..TransferSpec::copy()
        },
    )?;
    let window6 = new_window(&mut app, "Copy", before)?;
    pause(&mut app, &window6, stopped, "the copy to cancel from Jobs")?;
    let before = app.tool_windows().len();
    app.window.invoke_show_operations();
    let jobs = tools::wait_tool(&mut app, "the Jobs window", before, SIZE);
    let rows = inspect::controls(jobs.window(), AccessibleRole::ListItem);
    match rows.iter().find(|(_, item)| {
        tools::item_texts(item)
            .iter()
            .any(|text| text.contains("Paused"))
    }) {
        None => failures.push(format!(
            "the Jobs window shows no paused job: {:?}",
            texts(&jobs)
        )),
        Some((_, item)) => {
            crate::input::click_at(jobs.window(), inspect::center(item));
            app.settle();
            tools::click_last(jobs.window(), "Cancel job")?;
            let done = support::wait_job(&mut app, stopped);
            if done.state != CopyOperationState::Cancelled {
                failures.push(format!("Jobs' Cancel job ended the job {:?}", done.state));
            }
            if texts(&window6)
                .iter()
                .any(|text| text == "Cancel this operation?")
            {
                failures.push("Jobs' Cancel job asked the cancel question".into());
            }
        }
    }
    seen_texts.extend(texts(&jobs));
    tools::close_tool(&mut app, &jobs, "the Jobs window");
    support::dismiss_all(&mut app);
    for window in titled(&app, "Copy") {
        let _ = tools::click_last(window.window(), "Close");
    }

    // A job that ends with an error: the unreadable file is skipped.
    panel::select_row(&mut app, LEFT, "faulty")?;
    match support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(faulty_target.as_path()),
            choices: vec![Choice::Skip],
            ..TransferSpec::copy()
        },
    ) {
        Err(error) => failures.push(format!("the copy with an error: {error}")),
        Ok(run) => context.note(format!(
            "the copy with an error ended {:?} ({} question(s), failed {}, skipped {}, {} error(s))",
            run.progress.state,
            run.decisions.len(),
            run.progress.failed,
            run.progress.skipped,
            run.progress.errors.len()
        )),
    }
    support::dismiss_all(&mut app);
    for window in titled(&app, "Copy") {
        let _ = tools::click_last(window.window(), "Close");
    }

    // Clear finished without errors removes the jobs that completed cleanly and keeps
    // the one with an error; Clear all finished then empties the list.
    let before = app.tool_windows().len();
    app.window.invoke_show_operations();
    let jobs = tools::wait_tool(&mut app, "the Jobs window", before, SIZE);
    let rows = |jobs: &Tool| -> Vec<Vec<String>> {
        inspect::controls(jobs.window(), AccessibleRole::ListItem)
            .into_iter()
            .map(|(_, item)| tools::item_texts(&item))
            .collect()
    };
    let state = |row: &Vec<String>| row.get(1).cloned().unwrap_or_default();
    // A row with an error says so: its state or its "… · N errors" summary.
    let with_error = |row: &Vec<String>| {
        row.iter().any(|text| {
            text.contains("with errors")
                || text.split(" · ").any(|part| {
                    part.ends_with(" error")
                        || (part.ends_with(" errors") && !part.starts_with("0 "))
                })
        })
    };
    let all = rows(&jobs);
    app.screenshot_window(
        context,
        &jobs,
        "jobs-finished",
        "Jobs: finished jobs before clearing",
    );
    if !all.iter().any(|row| with_error(row)) {
        failures.push(format!(
            "the Jobs window shows no job with an error: {all:?}"
        ));
    }
    match tools::click_last(jobs.window(), "Clear finished without errors") {
        Err(error) => failures.push(format!("Jobs: {error}")),
        Ok(()) => {
            app.settle();
            app.idle(Duration::from_millis(200));
            let left = rows(&jobs);
            if left.iter().any(|row| state(row) == "Completed")
                || !left.iter().any(|row| with_error(row))
                || left.len() >= all.len()
            {
                failures.push(format!(
                    "Clear finished without errors left {left:?} of {all:?}"
                ));
            }
        }
    }
    match tools::click_last(jobs.window(), "Clear all finished") {
        Err(error) => failures.push(format!("Jobs: {error}")),
        Ok(()) => {
            let cleared = wait_text(&mut app, &jobs, Duration::from_secs(5), |text| {
                text == "No background operations"
            });
            if cleared.is_none() {
                failures.push(format!("Clear all finished left {:?}", rows(&jobs)));
            }
        }
    }
    seen_texts.extend(texts(&jobs));
    tools::close_tool(&mut app, &jobs, "the Jobs window");

    let words = published(&seen_texts);
    if !words.is_empty() {
        failures.push(format!(
            "job windows or notices say published: {:?}",
            words.iter().take(4).collect::<Vec<_>>()
        ));
    }
    app.shutdown()?;
    support::report_with(context, &failures, &pending)
}
