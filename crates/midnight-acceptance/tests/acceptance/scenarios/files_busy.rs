//! A busy folder: while a background writer keeps creating, modifying and deleting
//! files in the folder the panel shows, the unchanged rows keep their identity and
//! content, the cursor and the selection stay on the same entries and the panel never
//! shows its loading state; a Rename and a Copy finished meanwhile are listed at once.
//! When the focused file vanishes on disk, the cursor moves to a neighbouring row within
//! 1.5 s and the marks stay.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Timings, TransferSpec, io_error},
    input, panel,
};
use midnight_core::{OperationState, PaneId};
use slint::platform::Key;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const STABLE: usize = 120;

/// Stops the writer when dropped, so an early return never leaves it writing.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Creates `w-NNNNN.tmp` every 25 ms, appends to the one created two steps earlier
/// and deletes the one six steps earlier; stops on `stop` or after 90 s.
fn writer(folder: PathBuf, stop: Arc<AtomicBool>, steps: Arc<AtomicU64>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut index = 0_u64;
        let name = |index: u64| folder.join(format!("w-{index:05}.tmp"));
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            let _ = fs::write(name(index), index.to_string());
            if index >= 2
                && let Ok(mut file) = fs::OpenOptions::new().append(true).open(name(index - 2))
            {
                let _ = file.write_all(b" modified");
            }
            if index >= 6 {
                let _ = fs::remove_file(name(index - 6));
            }
            index += 1;
            steps.store(index, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(25));
        }
    })
}

/// The visible facts of the stable rows: identity, size and time text, selection.
fn stable_rows(app: &App) -> BTreeMap<String, (String, String, String, bool)> {
    app.rows(LEFT)
        .into_iter()
        .filter(|row| row.name.starts_with("stable-") || row.name.starts_with("selected-"))
        .map(|row| {
            (
                row.name.to_string(),
                (
                    row.identity.to_string(),
                    row.size.to_string(),
                    format!("{} {}", row.modified_date, row.modified_time),
                    row.selected,
                ),
            )
        })
        .collect()
}

pub fn busy_folder(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "busy")?;
    let other = support::scratch(context, "busy-other")?;
    for index in 0..STABLE {
        let path = folder.join(format!("stable-{index:03}.txt"));
        fs::write(&path, format!("stable {index}")).map_err(|error| io_error(&path, error))?;
    }
    for name in ["selected-a.txt", "selected-b.txt", "cursor.txt"] {
        fs::write(folder.join(name), name).map_err(|error| io_error(&folder, error))?;
    }
    fs::write(other.join("incoming.txt"), "incoming").map_err(|error| io_error(&other, error))?;
    let mut app = App::start(Config::new(&folder, &other));
    let mut failures = Vec::new();
    let mut timings = Timings::default();
    for name in ["selected-a.txt", "selected-b.txt"] {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    panel::select_row(&mut app, LEFT, "cursor.txt")?;
    app.settle();
    let before = stable_rows(&app);
    let selected = vec!["selected-a.txt".to_owned(), "selected-b.txt".to_owned()];

    let stop = Arc::new(AtomicBool::new(false));
    let steps = Arc::new(AtomicU64::new(0));
    let handle = writer(folder.clone(), stop.clone(), steps.clone());
    let _stop_writer = StopOnDrop(stop.clone());
    let watched = Instant::now();
    let mut frames = 0;
    let mut loading_frames = 0;
    let mut cursor_moves = Vec::new();
    let mut selection_changes = Vec::new();
    let mut churned: BTreeMap<String, String> = BTreeMap::new();
    let mut writer_rows_seen = 0;
    while watched.elapsed() < Duration::from_secs(6) {
        app.idle(Duration::from_millis(15));
        frames += 1;
        if app.loading(LEFT) {
            loading_frames += 1;
        }
        let cursor = support::cursor_name(&app, LEFT);
        if cursor.as_deref() != Some("cursor.txt") && cursor_moves.len() < 5 {
            cursor_moves.push(format!(
                "{cursor:?} at {} ms",
                watched.elapsed().as_millis()
            ));
        }
        let mut now_selected = support::selected_names(&app, LEFT);
        now_selected.sort();
        if now_selected != selected && selection_changes.len() < 5 {
            selection_changes.push(format!("{now_selected:?}"));
        }
        let rows = stable_rows(&app);
        for (name, facts) in &before {
            match rows.get(name) {
                Some(found) if found == facts => {}
                found => {
                    churned
                        .entry(name.clone())
                        .or_insert_with(|| format!("{facts:?} → {found:?}"));
                }
            }
        }
        writer_rows_seen = writer_rows_seen.max(
            app.names(LEFT)
                .iter()
                .filter(|name| name.starts_with("w-"))
                .count(),
        );
    }
    let watched_for = watched.elapsed();
    let steps_watched = steps.load(Ordering::Relaxed);
    app.screenshot(
        context,
        "busy-folder",
        "The panel while a writer churns w-*.tmp",
    );
    if loading_frames > 0 {
        failures.push(format!(
            "the panel showed its loading state in {loading_frames} of {frames} frames"
        ));
    }
    if !cursor_moves.is_empty() {
        failures.push(format!("the cursor left cursor.txt: {cursor_moves:?}"));
    }
    if !selection_changes.is_empty() {
        failures.push(format!("the selection changed: {selection_changes:?}"));
    }
    if !churned.is_empty() {
        failures.push(format!(
            "{} unchanged rows were replaced or changed, e.g. {:?}",
            churned.len(),
            churned.iter().take(3).collect::<Vec<_>>()
        ));
    }
    if writer_rows_seen == 0 {
        failures.push("no w-*.tmp row was ever listed while the writer ran".into());
    }

    // A rename finished meanwhile is listed at once, with the cursor on it.
    panel::select_row(&mut app, LEFT, "stable-000.txt")?;
    let start = support::named_operation(&mut app, &[], Key::F2, "renamed while busy.txt")?;
    let progress = support::wait_operation(&mut app, start);
    let finished = Instant::now();
    if progress.state != OperationState::Completed {
        failures.push(format!("F2 while busy ended {:?}", progress.state));
    } else {
        let shown = support::row_delay(&mut app, LEFT, "renamed while busy.txt", true, finished);
        timings.add("rename report to row listed", shown);
        if shown > Duration::from_millis(1500) {
            failures.push(format!(
                "the rename was listed {} ms after its report",
                shown.as_millis()
            ));
        }
        panel::wait_input(&mut app);
        if support::cursor_name(&app, LEFT).as_deref() != Some("renamed while busy.txt") {
            failures.push(format!(
                "after F2 while busy the cursor is on {:?}",
                support::cursor_name(&app, LEFT)
            ));
        }
    }
    // A copy into the busy folder from the other panel is listed at once.
    panel::select_row(&mut app, PaneId::Right, "incoming.txt")?;
    match support::transfer(&mut app, context, TransferSpec::copy()) {
        Err(error) => failures.push(format!("F5 into the busy folder: {error}")),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F5 into the busy folder") {
                failures.push(error);
            }
            let shown = support::row_delay(&mut app, LEFT, "incoming.txt", true, run.finished);
            timings.add("copy report to row listed", shown);
            if shown > Duration::from_millis(1500) {
                failures.push(format!(
                    "the copy was listed {} ms after its report",
                    shown.as_millis()
                ));
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    let joined = Instant::now();
    while !handle.is_finished() && joined.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    if handle.is_finished() {
        let _ = handle.join();
    } else {
        failures.push("the writer thread did not stop within 5 s".into());
    }
    // Once quiet, the panel matches the disk.
    let deadline = Instant::now() + Duration::from_secs(15);
    let disk = loop {
        app.idle(Duration::from_millis(50));
        let disk = support::names_on_disk(&folder);
        let mut shown: Vec<String> = app
            .names(LEFT)
            .into_iter()
            .filter(|name| name != "..")
            .collect();
        shown.sort();
        if shown == disk || Instant::now() > deadline {
            if shown != disk {
                failures.push(format!(
                    "after the writer stopped the panel lists {} entries, the disk {}",
                    shown.len(),
                    disk.len()
                ));
            }
            break disk;
        }
    };
    context.note(format!(
        "writer: {steps_watched} steps in {:.1} s ({} in all); panel sampled in {frames} frames, at most {writer_rows_seen} w-*.tmp rows at once; {} entries at the end",
        watched_for.as_secs_f64(),
        steps.load(Ordering::Relaxed),
        disk.len()
    ));
    context.note(timings.summary());
    neighbour_cursor(&mut app, context, &folder, &mut failures)?;
    context.note(
        "SKIP: the watch fallback line \"Live updates are unavailable for this folder; it is \
         checked for changes regularly instead.\": no seam makes a watch registration fail \
         (decision A5)",
    );
    app.shutdown()?;
    support::report(&failures)
}

/// The focused file is deleted behind the panel's back: within 1.5 s its row is gone,
/// the cursor is on the row before or after it, and the marks are kept.
fn neighbour_cursor(
    app: &mut App,
    context: &mut Context,
    folder: &std::path::Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    const FOCUSED: &str = "stable-050.txt";
    panel::select_row(app, LEFT, FOCUSED)?;
    let names = app.names(LEFT);
    let index = names
        .iter()
        .position(|name| name == FOCUSED)
        .ok_or("the focused row is not listed")?;
    let neighbours = [
        names.get(index + 1).cloned(),
        index
            .checked_sub(1)
            .and_then(|before| names.get(before))
            .cloned(),
    ];
    let marks = support::selected_names(app, LEFT);
    let path = folder.join(FOCUSED);
    let deleted = Instant::now();
    fs::remove_file(&path).map_err(|error| io_error(&path, error))?;
    let gone = |app: &App| !app.names(LEFT).iter().any(|name| name == FOCUSED);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gone(app) && Instant::now() < deadline {
        app.idle(Duration::from_millis(10));
    }
    let after = deleted.elapsed();
    let cursor = support::cursor_name(app, LEFT);
    context.note(format!(
        "the focused {FOCUSED} deleted on disk: its row gone after {} ms, the cursor on {cursor:?}",
        after.as_millis()
    ));
    if !gone(app) || after > Duration::from_millis(1500) {
        failures.push(format!(
            "the deleted focused file's row was {} after {} ms",
            if gone(app) {
                "gone only"
            } else {
                "still listed"
            },
            after.as_millis()
        ));
    }
    if !neighbours
        .iter()
        .flatten()
        .any(|name| Some(name) == cursor.as_ref())
    {
        failures.push(format!(
            "after its row vanished the cursor is on {cursor:?}, not a neighbour {neighbours:?}"
        ));
    }
    if support::selected_names(app, LEFT) != marks {
        failures.push(format!(
            "the marks changed from {marks:?} to {:?} when the focused row vanished",
            support::selected_names(app, LEFT)
        ));
    }
    Ok(())
}
