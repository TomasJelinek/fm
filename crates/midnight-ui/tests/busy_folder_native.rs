//! A large folder that keeps changing, as a build writing its outputs does,
//! through the installed UI, presenter, production Engine and native watcher.
#[path = "support/operations_native.rs"]
mod native;
use crate::support::ui;

use i_slint_core::model::{ModelChangeListener, ModelChangeListenerContainer};
use midnight_core::PaneId;
use slint::Model;
use std::{
    cell::Cell,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Counts the left row model's resets: a reset replaces every row delegate,
/// which is the refill an in-place re-read avoids.
#[derive(Default)]
struct Resets(Cell<usize>);

impl ModelChangeListener for Resets {
    fn row_changed(self: Pin<&Self>, _: usize) {}
    fn row_added(self: Pin<&Self>, _: usize, _: usize) {}
    fn row_removed(self: Pin<&Self>, _: usize, _: usize) {}
    fn reset(self: Pin<&Self>) {
        self.0.set(self.0.get() + 1);
    }
}

fn count_resets(h: &native::Harness) -> Pin<Box<ModelChangeListenerContainer<Resets>>> {
    let resets = Box::pin(ModelChangeListenerContainer::<Resets>::default());
    h.ui.window
        .get_left_rows()
        .model_tracker()
        .attach_peer(resets.as_ref().model_peer());
    resets
}

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

/// Reports the rows on screen, as the application's visible-row pass does
/// every frame (`thumbnails.rs`); folder presence is probed for them.
fn report_viewport(h: &native::Harness) {
    drop(
        h.presenter
            .borrow_mut()
            .thumbnail_targets([(0, 64), (0, 64)], ["", ""]),
    );
}

/// Settles until the left panel shows `main.rs` colored and the folder rows'
/// presence resolved, then until no listing has started for a second.
///
/// The tests write `main.rs` right after the panel showed its folder, often
/// before the folder's watch subscription took effect. The rows are then
/// checked once the watch adopts them, so the file is shown well before the
/// watch's periodic check (15 s), which this deadline stays below.
fn settle_decorated(h: &mut native::Harness) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        report_viewport(h);
        h.settle();
        let rows = h.rows(PaneId::Left);
        let row = |name: &str| rows.iter().find(|row| row.name == name).cloned();
        if !h.ui.window.get_left_loading()
            && row("main.rs").is_some_and(|row| row.color_rule)
            && row("inside").is_some_and(|row| row.tree_disclosure == 5)
            && row("nested").is_some_and(|row| row.tree_disclosure == 3)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "decorations never settled: {:?}; loading {}, {} listings, {} hints",
            ["main.rs", "inside", "nested"].map(|name| row(name).map(|row| (
                row.name,
                row.color_rule,
                row.tree_disclosure
            ))),
            h.ui.window.get_left_loading(),
            h.listings[0],
            h.hints[0],
        );
        h.idle(Duration::from_millis(10));
    }
    let mut listings = h.listings[0];
    let mut quiet_since = Instant::now();
    while quiet_since.elapsed() < Duration::from_millis(1_500) {
        report_viewport(h);
        h.settle();
        if h.listings[0] != listings {
            listings = h.listings[0];
            quiet_since = Instant::now();
        }
        h.idle(Duration::from_millis(10));
    }
}

/// Whether two rows show the same. Slint never finds two empty images equal,
/// so rows whose icon is not loaded (the parent row's here) compare by every
/// field's value instead.
fn same_row(left: &midnight_ui::FileRow, right: &midnight_ui::FileRow) -> bool {
    let empty = |row: &midnight_ui::FileRow| row.icon.size().width == 0;
    if empty(left) && empty(right) {
        format!("{left:?}") == format!("{right:?}")
    } else {
        left == right
    }
}

/// Settles frames until a listing started after `listings` and `after` more
/// passed, failing when the row model was reset or a row other than
/// `changing` differs from `before` in any frame.
#[track_caller]
fn assert_rows_kept(
    h: &mut native::Harness,
    before: &[midnight_ui::FileRow],
    listings: usize,
    changing: Option<&str>,
    after: Duration,
) {
    let resets = count_resets(h);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut listed: Option<Instant> = None;
    while listed.is_none_or(|listed| listed.elapsed() < after) {
        report_viewport(h);
        h.settle();
        assert_eq!(resets.0.get(), 0, "the row model was reset");
        assert!(
            !h.ui.window.get_left_loading(),
            "the loading state was shown"
        );
        let rows = h.rows(PaneId::Left);
        let differing: Vec<_> = before
            .iter()
            .filter(|row| Some(row.name.as_str()) != changing)
            .filter(|row| !rows.iter().any(|now| same_row(row, now)))
            .map(|row| {
                let now = rows.iter().find(|now| now.name == row.name);
                format!("{row:?} became {now:?}")
            })
            .collect();
        assert!(differing.is_empty(), "rows changed: {differing:#?}");
        assert_eq!(rows.len(), before.len(), "rows were added or removed");
        if listed.is_none() && h.listings[0] > listings {
            listed = Some(Instant::now());
        }
        assert!(Instant::now() < deadline, "no listing started");
        h.idle(Duration::from_millis(5));
    }
}

#[test]
fn a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched() {
    let mut h = native::Harness::watched(0);
    let folder = h.left.clone();
    fs::write(folder.join("main.rs"), b"fn main() {}").unwrap();
    settle_decorated(&mut h);
    select_two(&mut h);
    settle_decorated(&mut h);
    let before = h.rows(PaneId::Left);
    let listings = h.listings[0];
    // Two notifications, and a folder that lists the same as before whenever
    // it is read: the modification time set to what it is. (A file created
    // and removed again is listed by a re-read that starts in between.)
    let main = folder.join("main.rs");
    let modified = fs::metadata(&main).unwrap().modified().unwrap();
    for _ in 0..2 {
        fs::File::options()
            .write(true)
            .open(&main)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }
    assert_rows_kept(
        &mut h,
        &before,
        listings,
        None,
        Duration::from_millis(1_500),
    );
    let after = h.rows(PaneId::Left);
    assert!(
        after.len() == before.len() && after.iter().zip(&before).all(|(a, b)| same_row(a, b)),
        "the rows differ after the refresh"
    );
    assert_eq!(cursor_row(&h).unwrap().name, "file-07.bin");
}

#[test]
fn a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints() {
    let mut h = native::Harness::watched(0);
    let folder = h.left.clone();
    fs::write(folder.join("main.rs"), b"fn main() {}").unwrap();
    settle_decorated(&mut h);
    let before = h.rows(PaneId::Left);
    let listings = h.listings[0];
    fs::write(folder.join("file-02.bin"), vec![2; 9_999]).unwrap();
    assert_rows_kept(
        &mut h,
        &before,
        listings,
        Some("file-02.bin"),
        Duration::from_millis(1_500),
    );
    let changed = h
        .rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == "file-02.bin")
        .unwrap();
    let old = before.iter().find(|row| row.name == "file-02.bin").unwrap();
    assert_ne!(
        changed.size, old.size,
        "the grown file shows its new length"
    );
    assert_eq!(
        changed.identity, old.identity,
        "the row was updated in place"
    );
}

/// Sorted newest first, a rewritten file jumps from the bottom of a long list
/// to its top: the re-read moves that one row instead of refilling the rows
/// it passed, as it would when every passed row counted as an edit.
#[test]
fn a_file_that_jumps_to_the_top_moves_without_refilling_the_rows_it_passed() {
    let mut h = native::Harness::watched(600);
    let folder = h.left.clone();
    h.until(|h| !h.ui.window.get_left_loading());
    // The modification time column; its second click sorts newest first.
    let resorts = count_resets(&h);
    for _ in 0..2 {
        h.ui.window.invoke_sort(0, 2);
        h.until(|h| !h.ui.window.get_left_loading());
    }
    assert!(resorts.0.get() > 0, "a new order refills the rows");
    let rows = h.rows(PaneId::Left);
    let oldest = rows
        .iter()
        .rposition(|row| row.name.ends_with(".o"))
        .unwrap();
    assert!(oldest > 500, "the oldest object is far down: {oldest}");
    let name = rows[oldest].name.to_string();
    let resets = count_resets(&h);
    let listings = h.listings[0];
    fs::write(folder.join(&name), b"rewritten, now the newest file").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        h.settle();
        let rows = h.rows(PaneId::Left);
        if h.listings[0] > listings
            && rows
                .iter()
                .position(|row| row.name == name)
                .is_some_and(|row| row < 10)
        {
            break;
        }
        assert!(Instant::now() < deadline, "{name} never moved to the top");
        h.idle(Duration::from_millis(10));
    }
    assert!(!h.ui.window.get_left_loading());
    assert_eq!(resets.0.get(), 0, "the rows were refilled");
}
