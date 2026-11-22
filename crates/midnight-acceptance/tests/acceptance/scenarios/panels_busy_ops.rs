//! Creating, renaming in place and changing attributes while the shown folder is busy:
//! a background writer keeps creating, growing and deleting files there while F7,
//! Shift+F4 and Ctrl+F2 run. Each result is on disk, listed within 1.5 s of its report
//! (with the cursor on a created entry), no notice or window is left, the marks on
//! untouched files stay, an expanded branch stays expanded through the refreshes, and
//! once the writer stops the listing equals the disk.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Timings},
    input, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use slint::platform::Key;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Stops the writer when dropped, so an early return never leaves it writing.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Creates `w-NNNNN.tmp` every 25 ms, appends to the one two steps back and deletes
/// the one six steps back, until `stop` or 90 s.
fn writer(folder: PathBuf, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(90);
        let name = |index: u64| folder.join(format!("w-{index:05}.tmp"));
        let mut index = 0_u64;
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            let _ = fs::write(name(index), index.to_string());
            if index >= 2
                && let Ok(mut file) = fs::OpenOptions::new().append(true).open(name(index - 2))
            {
                let _ = file.write_all(b" more");
            }
            if index >= 6 {
                let _ = fs::remove_file(name(index - 6));
            }
            index += 1;
            thread::sleep(Duration::from_millis(25));
        }
    })
}

/// After a finished job: the entry is listed within 1.5 s and nothing else is open.
fn listed(
    app: &mut App,
    timings: &mut Timings,
    what: &str,
    name: &str,
    finished: Instant,
    cursor: bool,
    failures: &mut Vec<String>,
) {
    let wanted = name.to_owned();
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.names(LEFT).contains(&wanted)
    }) {
        failures.push(format!("{what}: {name} is not listed"));
        return;
    }
    let delay = finished.elapsed();
    timings.add(what, delay);
    if delay > Duration::from_millis(1500) {
        failures.push(format!(
            "{what}: {name} was listed {} ms after its report",
            delay.as_millis()
        ));
    }
    panel::wait_input(app);
    if cursor && ps::cursor(app, LEFT).as_deref() != Some(name) {
        failures.push(format!(
            "{what}: the cursor is on {:?}, not on {name}",
            ps::cursor(app, LEFT)
        ));
    }
    if let Err(error) = ps::stays_idle(app, Duration::from_millis(300)) {
        failures.push(format!("{what}: {error}"));
        support::dismiss_all(app);
    }
}

pub fn busy_ops(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "busy-ops")?;
    for index in 0..60 {
        ps::write(&folder.join(format!("stable-{index:03}.txt")), "stable")?;
    }
    for name in ["sub/one", "sub/two"] {
        ps::folder(&folder.join(name))?;
    }
    let mut app = App::start(Config::new(&folder, &context.work));
    let mut failures = Vec::new();
    let mut timings = Timings::default();
    let marked = ["stable-010.txt", "stable-020.txt"];
    for name in marked {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    support::toggle_tree(&mut app, "sub")?;
    support::wait_tree_depth(&mut app, "one", Some(1));

    let stop = Arc::new(AtomicBool::new(false));
    let handle = writer(folder.clone(), stop.clone());
    let _stop_writer = StopOnDrop(stop.clone());
    app.idle(Duration::from_millis(500));

    // F7 while busy.
    panel::select_row(&mut app, LEFT, "stable-030.txt")?;
    let start = support::named_operation(&mut app, &[], Key::F7, "made while busy")?;
    let progress = support::wait_operation(&mut app, start);
    let finished = Instant::now();
    match support::operation_ok(&progress, "F7 while busy") {
        Err(error) => failures.push(error),
        Ok(()) if !folder.join("made while busy").is_dir() => {
            failures.push("F7 while busy created no folder".into());
        }
        Ok(()) => listed(
            &mut app,
            &mut timings,
            "F7 while busy",
            "made while busy",
            finished,
            true,
            &mut failures,
        ),
    }
    // Shift+F4 while busy.
    let start = support::named_operation(&mut app, &[Key::Shift], Key::F4, "new while busy.txt")?;
    let progress = support::wait_operation(&mut app, start);
    let finished = Instant::now();
    match support::operation_ok(&progress, "Shift+F4 while busy") {
        Err(error) => failures.push(error),
        Ok(())
            if fs::metadata(folder.join("new while busy.txt")).map_or(true, |m| m.len() != 0) =>
        {
            failures.push("Shift+F4 while busy left no empty file".into());
        }
        Ok(()) => listed(
            &mut app,
            &mut timings,
            "Shift+F4 while busy",
            "new while busy.txt",
            finished,
            true,
            &mut failures,
        ),
    }
    // Ctrl+F2 while busy: octal 600 (on Windows, whose Attributes dialog has attributes
    // instead of permissions, Read-only set) for the marked files: with files marked,
    // Ctrl+F2 acts on them, not on the focused row.
    panel::select_row(&mut app, LEFT, "stable-040.txt")?;
    ps::open_metadata(&mut app, false)?;
    if cfg!(unix) {
        input::fill(app.slint(), "Octal permissions", "600")?;
    } else {
        ps::set_attribute(&mut app, "Read-only", "Set")?;
    }
    app.settle();
    match ps::apply_metadata(&mut app) {
        Err(error) => failures.push(format!("Ctrl+F2 while busy: {error}")),
        Ok((progress, reviewed)) => {
            for name in marked {
                let path = folder.join(name);
                let mode = support::facts(&path).map_or(0, |facts| facts.mode);
                let changed = if cfg!(unix) {
                    mode == 0o600
                } else {
                    ps::read_only_attribute(&path)
                };
                if !changed {
                    failures.push(format!(
                        "Ctrl+F2 while busy did not change the marked {name} (mode {mode:o}); the job ended {:?} ({} done, {} failed) after reviewing {reviewed:?}",
                        progress.state, progress.completed, progress.failed
                    ));
                }
            }
        }
    }
    app.screenshot(
        context,
        "busy-ops",
        "Created and changed while the folder churns",
    );
    // The branch stays expanded and the marks stay through the refreshes.
    let watch = Instant::now();
    let mut collapsed = 0;
    while watch.elapsed() < Duration::from_secs(2) {
        app.idle(Duration::from_millis(50));
        if !app
            .rows(LEFT)
            .iter()
            .any(|row| row.name == "one" && row.tree_depth == 1)
        {
            collapsed += 1;
        }
    }
    if collapsed > 0 {
        failures.push(format!(
            "the expanded sub/ branch was not shown in {collapsed} samples while the folder churned"
        ));
    }
    let marks = ps::marked(&app, LEFT);
    if marks != ps::sorted(&marked) {
        failures.push(format!(
            "the marks changed while the folder churned: {marks:?}"
        ));
    }
    stop.store(true, Ordering::Relaxed);
    let joined = Instant::now();
    while !handle.is_finished() && joined.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    let _ = handle.join();
    // Quiet again: the listing equals the disk.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.idle(Duration::from_millis(50));
        let disk = support::names_on_disk(&folder);
        let mut shown = ps::top_level(&app, LEFT);
        shown.sort();
        if shown == disk {
            break;
        }
        if Instant::now() > deadline {
            failures.push(format!(
                "after the writer stopped the panel lists {} entries, the disk {}",
                shown.len(),
                disk.len()
            ));
            break;
        }
    }
    context.note(timings.summary());
    app.shutdown()?;
    support::report(&failures)
}
