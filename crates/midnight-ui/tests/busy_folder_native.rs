//! A large folder that keeps changing, as a build writing its outputs does,
//! through the installed UI, presenter, production Engine and native watcher.
#[path = "support/operations_native.rs"]
mod native;
use crate::support::ui;

use midnight_core::PaneId;
use slint::Model;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Writes, renames and removes files in a folder until stopped, as a compiler
/// writes its objects: each step creates a temporary file, renames it, appends
/// to it and removes an object written a few steps earlier.
struct Churn {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<usize>>,
}

impl Churn {
    fn start(folder: &Path, pause: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let folder = folder.to_path_buf();
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            let mut step = 0;
            while !stopping.load(Ordering::Acquire) {
                step += 1;
                let temporary = folder.join(format!("churn-{step:06}.tmp"));
                let object = folder.join(format!("churn-{step:06}.o"));
                fs::write(&temporary, b"object").unwrap();
                fs::rename(&temporary, &object).unwrap();
                fs::OpenOptions::new()
                    .append(true)
                    .open(&object)
                    .and_then(|mut file| std::io::Write::write_all(&mut file, b" data"))
                    .unwrap();
                if step > 16 {
                    fs::remove_file(folder.join(format!("churn-{:06}.o", step - 16))).unwrap();
                }
                thread::sleep(pause);
            }
            step
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }

    /// Stops the writer and returns how many steps it made.
    fn stop(mut self) -> usize {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap()
    }
}

impl Drop for Churn {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The names in `folder`, as the left panel should show them.
fn folder_names(folder: &Path) -> BTreeSet<String> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn shown_names(h: &native::Harness) -> BTreeSet<String> {
    h.rows(PaneId::Left)
        .into_iter()
        .filter(|row| !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect()
}

fn cursor_row(h: &native::Harness) -> Option<midnight_ui::FileRow> {
    let cursor = usize::try_from(h.cursor(PaneId::Left)).ok()?;
    h.ui.window.get_left_rows().row_data(cursor)
}

fn selected_names(h: &native::Harness) -> BTreeSet<String> {
    h.rows(PaneId::Left)
        .into_iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect()
}

/// What the left panel did while its folder kept changing.
#[derive(Debug, Default)]
struct Observation {
    frames: usize,
    /// Frames that showed the panel's loading state (its progress bar).
    loading_frames: usize,
    /// Directory listings the Engine started.
    listings: usize,
    /// Their mean time from start to finish.
    listing_mean: Duration,
    hints: usize,
    /// Row batches the Engine streamed to the UI.
    batches: usize,
    /// Frames whose cursor row got a new identity: the row was replaced
    /// rather than kept.
    replaced_cursor_rows: usize,
}

/// Settles the UI in short frames for `duration`.
fn observe(h: &mut native::Harness, duration: Duration) -> Observation {
    let (listings, hints, batches, time) =
        (h.listings[0], h.hints[0], h.batches[0], h.listing_time[0]);
    let mut observation = Observation::default();
    let mut identity = cursor_row(h).map(|row| row.identity);
    let end = Instant::now() + duration;
    while Instant::now() < end {
        h.settle();
        observation.frames += 1;
        if h.ui.window.get_left_loading() {
            observation.loading_frames += 1;
        }
        let current = cursor_row(h).map(|row| row.identity);
        if current != identity {
            observation.replaced_cursor_rows += 1;
            identity = current;
        }
        h.idle(Duration::from_millis(5));
    }
    observation.listings = h.listings[0] - listings;
    observation.hints = h.hints[0] - hints;
    observation.batches = h.batches[0] - batches;
    observation.listing_mean =
        (h.listing_time[0] - time) / u32::try_from(observation.listings.max(1)).unwrap();
    observation
}

/// How long until the left panel shows exactly what `folder` holds.
fn converge(h: &mut native::Harness, folder: &Path, limit: Duration) -> Option<Duration> {
    let start = Instant::now();
    let expected = folder_names(folder);
    while start.elapsed() < limit {
        h.settle();
        if shown_names(h) == expected {
            return Some(start.elapsed());
        }
        h.idle(Duration::from_millis(10));
    }
    None
}

/// How long until a file created in an otherwise idle `folder` is shown.
fn shown_after_create(h: &mut native::Harness, folder: &Path, name: &str) -> Duration {
    let path: PathBuf = folder.join(name);
    let listings = h.listings[0];
    let start = Instant::now();
    fs::write(&path, b"marker").unwrap();
    let mut loading = false;
    loop {
        h.settle();
        loading |= h.ui.window.get_left_loading();
        if h.rows(PaneId::Left).iter().any(|row| row.name == name) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{name} never appeared"
        );
        h.idle(Duration::from_millis(5));
    }
    let elapsed = start.elapsed();
    eprintln!(
        "  created {name}: shown after {elapsed:?}, {} listings, loading shown: {loading}",
        h.listings[0] - listings
    );
    elapsed
}

/// Measures a folder of `MIDNIGHT_BUSY_FILES` (default 5,800) files that a
/// writer keeps changing for `MIDNIGHT_BUSY_SECONDS` (default 8) seconds.
/// Run with `--ignored --nocapture`; it reports rather than asserts.
#[test]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
fn busy_folder_benchmark() {
    let files = std::env::var("MIDNIGHT_BUSY_FILES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5_800);
    let seconds = std::env::var("MIDNIGHT_BUSY_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let mut h = native::Harness::watched(files);
    let folder = h.left.clone();
    h.until(|h| !h.ui.window.get_left_loading());
    h.focus(PaneId::Left, "file-05.bin", false);
    eprintln!("busy folder benchmark: {files} files, {seconds} s of changes");
    shown_after_create(&mut h, &folder, "idle-marker-1.txt");
    let churn = Churn::start(&folder, Duration::from_millis(2));
    let busy = observe(&mut h, Duration::from_secs(seconds));
    let steps = churn.stop();
    let converged = converge(&mut h, &folder, Duration::from_secs(30));
    eprintln!("  writer steps: {steps}");
    eprintln!("  while busy: {busy:?}");
    eprintln!(
        "  loading shown in {:.1}% of frames; {:.2} listings/s",
        100.0 * busy.loading_frames as f64 / busy.frames.max(1) as f64,
        busy.listings as f64 / seconds as f64
    );
    eprintln!("  converged after the writer stopped: {converged:?}");
    eprintln!(
        "  cursor row: {:?}",
        cursor_row(&h).map(|row| row.name.to_string())
    );
    shown_after_create(&mut h, &folder, "idle-marker-2.txt");
}

/// Selects `file-03.bin` and `file-07.bin`, leaving the cursor on the latter.
fn select_two(h: &mut native::Harness) {
    h.until(|h| !h.ui.window.get_left_loading());
    for name in ["file-03.bin", "file-07.bin"] {
        h.focus(PaneId::Left, name, true);
    }
    h.until(|h| selected_names(h).len() == 2);
}

/// Settles until the left panel shows what `folder` holds, failing if the
/// panel ever shows its loading state meanwhile.
#[track_caller]
fn converge_silently(h: &mut native::Harness, folder: &Path) {
    let expected = folder_names(folder);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        h.settle();
        assert!(
            !h.ui.window.get_left_loading(),
            "a background re-read showed the loading state"
        );
        if shown_names(h) == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the panel never showed the folder"
        );
        h.idle(Duration::from_millis(10));
    }
}

#[test]
fn small_change_bursts_update_rows_in_place_without_the_loading_state() {
    let mut h = native::Harness::watched(600);
    let folder = h.left.clone();
    select_two(&mut h);
    let cursor = cursor_row(&h).unwrap();
    assert_eq!(cursor.name, "file-07.bin");
    let (listings, batches) = (h.listings[0], h.batches[0]);
    // Created, renamed, grown and removed files, as one burst.
    fs::write(folder.join("new 新しい 🦀.o"), b"object").unwrap();
    fs::write(folder.join("temporary.tmp"), b"object").unwrap();
    fs::rename(folder.join("temporary.tmp"), folder.join("renamed.o")).unwrap();
    fs::write(folder.join("bulk-00010.o"), b"grown to more bytes").unwrap();
    fs::remove_file(folder.join("bulk-00020.o")).unwrap();
    converge_silently(&mut h, &folder);
    assert!(h.listings[0] > listings, "the change must be listed");
    assert_eq!(h.batches[0], batches, "a re-read streams no rows");
    let after = cursor_row(&h).unwrap();
    assert_eq!(after.name, cursor.name, "the cursor stays on its file");
    assert_eq!(
        after.identity, cursor.identity,
        "unchanged rows are kept, not replaced"
    );
    assert_eq!(
        selected_names(&h),
        BTreeSet::from(["file-03.bin".to_owned(), "file-07.bin".to_owned()])
    );
    let grown = h
        .rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == "bulk-00010.o")
        .unwrap();
    assert!(grown.size.contains("19"), "{}", grown.size);
}

#[test]
fn a_large_burst_replaces_the_rows_and_still_converges_without_the_loading_state() {
    let mut h = native::Harness::watched(300);
    let folder = h.left.clone();
    select_two(&mut h);
    // More changes than row edits describe cheaply: the rows are replaced.
    for index in 0..400 {
        fs::write(folder.join(format!("burst-{index:04}.o")), b"object").unwrap();
    }
    for index in 0..150 {
        fs::remove_file(folder.join(format!("bulk-{index:05}.o"))).unwrap();
    }
    converge_silently(&mut h, &folder);
    assert_eq!(cursor_row(&h).unwrap().name, "file-07.bin");
    assert_eq!(selected_names(&h).len(), 2);
}

#[test]
fn a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges() {
    let mut h = native::Harness::watched(1_000);
    let folder = h.left.clone();
    select_two(&mut h);
    let churn = Churn::start(&folder, Duration::from_millis(2));
    let busy = observe(&mut h, Duration::from_secs(3));
    churn.stop();
    eprintln!("while busy: {busy:?}");
    assert_eq!(busy.loading_frames, 0, "{busy:?}");
    assert_eq!(busy.replaced_cursor_rows, 0, "{busy:?}");
    assert_eq!(busy.batches, 0, "{busy:?}");
    // At most one hint per second of changes, plus the burst that started it.
    assert!((1..=5).contains(&busy.listings), "{busy:?}");
    converge_silently(&mut h, &folder);
    assert_eq!(cursor_row(&h).unwrap().name, "file-07.bin");
    assert_eq!(selected_names(&h).len(), 2);
}
