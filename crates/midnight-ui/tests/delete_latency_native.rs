//! How soon the panels show a deletion: the removed row and the space it freed,
//! through the installed UI, presenter, production Engine and native watcher.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/operations_native.rs"]
mod native;
use crate::support::ui;

use midnight_core::{PaneId, StorageService};
use midnight_platform::LocalFileSystem;
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

/// Large enough that the space it frees outweighs what other programs write
/// to the same volume meanwhile.
const FILE_SIZE: u64 = 2 << 30;
const GIB: f64 = (1u64 << 30) as f64;

/// The free space a panel shows ("119.8 GiB free"), in bytes, to a tenth of its unit.
fn shown_space(text: &str) -> Option<u64> {
    let mut words = text.split(' ');
    let number = words.next()?;
    let unit = match words.next()? {
        "TiB" => 1u64 << 40,
        "GiB" => 1 << 30,
        "MiB" => 1 << 20,
        "KiB" => 1 << 10,
        "B" => 1,
        _ => return None,
    };
    // Grouping separators and the decimal separator aside, the digits are tenths.
    let tenths = number
        .chars()
        .filter_map(|digit| digit.to_digit(10))
        .fold(0u64, |value, digit| value * 10 + u64::from(digit));
    Some(tenths * unit / 10)
}

fn space(h: &native::Harness, pane: PaneId) -> u64 {
    let text = match pane {
        PaneId::Left => h.ui.window.get_left_free_space(),
        PaneId::Right => h.ui.window.get_right_free_space(),
    };
    shown_space(&text).unwrap_or_else(|| panic!("unreadable free space {text:?}"))
}

/// What the volume reports now, as the application reads it.
fn actual(h: &native::Harness) -> u64 {
    LocalFileSystem::new(None)
        .space_info(&h.left)
        .unwrap()
        .available
}

fn shows(h: &native::Harness, pane: PaneId, name: &str) -> bool {
    h.rows(pane).iter().any(|row| row.name == name)
}

/// Writes `size` bytes that occupy disk blocks, then waits for them to reach the disk.
fn write_file(path: &Path, size: u64) {
    let mut file = fs::File::create(path).unwrap();
    let chunk = vec![0x5a_u8; 8 << 20];
    let mut written = 0;
    while written < size {
        file.write_all(&chunk).unwrap();
        written += chunk.len() as u64;
    }
    file.sync_all().unwrap();
}

/// Removes `path` on another thread, as another program does: unlinking a large
/// file can block for a second while the disk is busy, and the UI loop must keep
/// running meanwhile. The thread returns when the removal returned.
fn remove_elsewhere(path: &Path) -> std::thread::JoinHandle<Instant> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        fs::remove_file(&path).unwrap();
        Instant::now()
    })
}

/// Settles the UI in short frames for `duration`, as the app's event loop runs.
fn run(h: &mut native::Harness, duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        h.settle();
        h.idle(Duration::from_millis(2));
    }
}

/// When each panel showed the deletion, measured from `start`, and every change
/// of the shown free space meanwhile.
#[derive(Debug, Default)]
struct Shown {
    row: Option<Duration>,
    left: Option<Duration>,
    right: Option<Duration>,
    /// (elapsed, pane, shown GiB, actual GiB)
    timeline: Vec<(Duration, PaneId, f64, f64)>,
    /// (elapsed, what happened): watch hints, listings and operation ends.
    events: Vec<(Duration, String)>,
}

impl Shown {
    /// Notes when another program's removal (`remove_elsewhere`) returned.
    fn removed(&mut self, start: Instant, removal: std::thread::JoinHandle<Instant>) {
        let returned = removal.join().unwrap();
        self.events
            .push((returned.duration_since(start), "rm returned".into()));
    }
}

/// Settles until the left panel lacks `name` (when given) and each panel shows at
/// least half of `freed` over `before` (`freed` is negative for space taken), or
/// until `limit`.
fn observe(
    h: &mut native::Harness,
    start: Instant,
    name: Option<&str>,
    before: [u64; 2],
    freed: i64,
    limit: Duration,
) -> Shown {
    let mut shown = Shown::default();
    let mut last = before;
    let (mut hints, mut listings, mut operations) = (h.hints, h.listings, h.operations.len());
    let reached = |value: u64, before: u64| {
        let change = value as i64 - before as i64;
        if freed >= 0 {
            change >= freed / 2
        } else {
            change <= freed / 2
        }
    };
    loop {
        h.settle();
        let elapsed = start.elapsed();
        if shown.row.is_none() && name.is_some_and(|name| !shows(h, PaneId::Left, name)) {
            shown.row = Some(elapsed);
        }
        if h.hints != hints {
            shown
                .events
                .push((elapsed, format!("watch hints {:?} -> {:?}", hints, h.hints)));
            hints = h.hints;
        }
        if h.listings != listings {
            shown.events.push((
                elapsed,
                format!("listings {:?} -> {:?}", listings, h.listings),
            ));
            listings = h.listings;
        }
        for progress in &h.operations[operations..] {
            shown
                .events
                .push((elapsed, format!("operation {:?}", progress.state)));
        }
        operations = h.operations.len();
        for (index, pane) in [PaneId::Left, PaneId::Right].into_iter().enumerate() {
            let value = space(h, pane);
            if value != last[index] {
                last[index] = value;
                shown
                    .timeline
                    .push((elapsed, pane, value as f64 / GIB, actual(h) as f64 / GIB));
            }
            let time = if index == 0 {
                &mut shown.left
            } else {
                &mut shown.right
            };
            if time.is_none() && reached(value, before[index]) {
                *time = Some(elapsed);
            }
        }
        let complete = (shown.row.is_some() || name.is_none())
            && shown.left.is_some()
            && shown.right.is_some();
        if complete || elapsed > limit {
            return shown;
        }
        h.idle(Duration::from_millis(2));
    }
}

/// Creates the file to delete and waits until both panels show the space it took.
fn prepare(h: &mut native::Harness, path: &Path) -> [u64; 2] {
    let before = [space(h, PaneId::Left), space(h, PaneId::Right)];
    let start = Instant::now();
    write_file(path, FILE_SIZE);
    let taken = observe(
        h,
        start,
        None,
        before,
        -(FILE_SIZE as i64),
        Duration::from_secs(20),
    );
    if taken.left.is_none() || taken.right.is_none() {
        eprintln!("  the panels did not show the new file's space: {taken:?}");
    }
    // Past every debounce, hint cooldown and read throttle.
    run(h, Duration::from_secs(2));
    [space(h, PaneId::Left), space(h, PaneId::Right)]
}

fn millis(value: Option<Duration>) -> String {
    value.map_or_else(
        || "never".into(),
        |value| format!("{} ms", value.as_millis()),
    )
}

fn report(scenario: &str, round: usize, shown: &Shown) {
    eprintln!(
        "DELETE LATENCY {scenario} #{round}: row {}, left free space {}, right free space {}",
        millis(shown.row),
        millis(shown.left),
        millis(shown.right)
    );
    let mut lines: Vec<(Duration, String)> = shown
        .timeline
        .iter()
        .map(|(elapsed, pane, value, actual)| {
            (
                *elapsed,
                format!("{pane:?} shows {value:.1} GiB (volume reports {actual:.2} GiB)"),
            )
        })
        .chain(shown.events.iter().cloned())
        .collect();
    lines.sort_by_key(|(elapsed, _)| *elapsed);
    for (elapsed, line) in lines {
        eprintln!("    {:>5} ms {line}", elapsed.as_millis());
    }
}

/// Measurement, not a pass/fail test: prints when each panel showed a deletion.
///
/// The left panel shows the folder of the deleted file, the right panel another
/// folder on the same volume. Each file is 2 GiB, so the space it frees
/// outweighs other programs' writes to the volume meanwhile.
#[test]
#[ignore = "measurement; writes several GiB"]
fn delete_latency_benchmark() {
    let rounds = std::env::var("MIDNIGHT_DELETE_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2);
    let mut h = native::Harness::watched(0);
    h.until(|h| {
        shown_space(&h.ui.window.get_left_free_space()).is_some()
            && shown_space(&h.ui.window.get_right_free_space()).is_some()
    });
    let elsewhere = h.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let limit = Duration::from_secs(12);
    for round in 1..=rounds {
        // Another program removes a file of the shown folder, which was idle.
        let name = format!("external-{round}.bin");
        let path = h.left.join(&name);
        let before = prepare(&mut h, &path);
        let start = Instant::now();
        let removal = remove_elsewhere(&path);
        let mut shown = observe(&mut h, start, Some(&name), before, FILE_SIZE as i64, limit);
        shown.removed(start, removal);
        report("rm, idle folder", round, &shown);

        // Another program removes a file 300 ms after another change of the folder.
        let name = format!("external-after-change-{round}.bin");
        let path = h.left.join(&name);
        let before = prepare(&mut h, &path);
        fs::write(h.left.join(format!("touched-{round}.txt")), b"touched").unwrap();
        run(&mut h, Duration::from_millis(300));
        let start = Instant::now();
        let removal = remove_elsewhere(&path);
        let mut shown = observe(&mut h, start, Some(&name), before, FILE_SIZE as i64, limit);
        shown.removed(start, removal);
        report("rm, 300 ms after a change", round, &shown);

        // Another program removes a file of a folder that no panel shows.
        let path = elsewhere.join(format!("elsewhere-{round}.bin"));
        let before = prepare(&mut h, &path);
        let start = Instant::now();
        let removal = remove_elsewhere(&path);
        let mut shown = observe(&mut h, start, None, before, FILE_SIZE as i64, limit);
        shown.removed(start, removal);
        report("rm, folder not shown", round, &shown);

        // The application deletes a file of the shown folder (Shift+F8, Enter).
        let name = format!("application-{round}.bin");
        let path = h.left.join(&name);
        let before = prepare(&mut h, &path);
        h.focus(PaneId::Left, &name, false);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::KeyPressed {
                text: Key::Shift.into(),
            });
        ui::key(&h.ui.window, Key::F8);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::KeyReleased {
                text: Key::Shift.into(),
            });
        h.until(|h| !h.ui.window.get_operation_token().is_empty());
        let start = Instant::now();
        input::key(&h.dialog(), Key::Return);
        let shown = observe(&mut h, start, Some(&name), before, FILE_SIZE as i64, limit);
        report("application Shift+F8", round, &shown);
        assert!(!path.exists());
    }
}
