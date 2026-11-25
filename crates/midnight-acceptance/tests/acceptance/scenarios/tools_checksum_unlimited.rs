//! Checksums without size limits, and the results following the run.
//!
//! 20,000 files in one folder and a chain of 70 nested folders with a file on each level,
//! marked together with Include subfolders and every algorithm chosen: the calculation
//! completes with 20,070 rows and says nothing of a limit; Ctrl+A counts 20,070 rows for
//! Copy, and Ctrl+Shift+C copies the 20,070 full rows whole, several MiB, more than the
//! 4 MiB the clipboard once took; the saved manifest has 20,070 lines and verifies.
//!
//! While a calculation runs and nothing is selected, the list follows the row being
//! calculated (a run of 200 sparse files read with all five algorithms, slow enough to
//! watch): across five samples, each after the run moved on by a screenful, the boundary
//! between calculated and waiting rows is in view and the list has moved on. A click on a
//! row stops the following (the list stays where it is while the run goes on); Ctrl+D
//! (nothing selected) resumes it; a scroll by the user pauses it again.
use crate::{
    Context, Scenario,
    app::{App, Config},
    input, inspect, panel,
    scenarios::tools_checksums::{
        RESULTS, close_menu, copy_action, copy_caption_of, open, open_menu, progress, row_point,
        wait_progress,
    },
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(40 * 60);

/// This module registers its own scenario (see `scenarios/mod.rs`, `OWNED`).
pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "tools_checksum_unlimited",
        isolation: crate::Isolation::Parallel,
        area: "checksums",
        summary: "Checksums without size limits: 20,000 files and a 70-level chain calculated in full with every algorithm (20,070 rows, no limit named), Ctrl+A counting every row for Copy, Ctrl+Shift+C copying all 20,070 full rows (over 4 MiB), the manifest saved (20,070 lines) and verified; the list following the row being calculated, stopped by a selection, resumed by Ctrl+D, paused by a scroll.",
        run: unlimited,
        timeout: TIMEOUT,
    }]
}
const FILES: usize = 20_000;
const LEVELS: usize = 70;
const TOTAL: u64 = (FILES + LEVELS) as u64;

/// How many files the running calculation has done ("Calculating… · K of N files · …").
fn done(line: &str) -> Option<u64> {
    let part = line.split(" · ").nth(1)?;
    tools::number(part.split(" of ").next()?)
}

/// The indices of the whole rows in view, after two frames (the list follows the
/// calculated row from a change handler, which a frame runs).
fn in_view(window: &Tool) -> Vec<usize> {
    let _ = crate::app::render(window);
    tools::list_rows(window, RESULTS)
        .into_iter()
        .filter(|row| row.whole)
        .map(|row| row.index)
        .collect()
}

/// Wait until the run has done at least `count` files; returns the count, or None when it
/// ended first. While it lists its files the status line names no count yet.
fn wait_done(app: &mut App, window: &Tool, count: u64) -> Option<u64> {
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        app.settle();
        let line = progress(window);
        if ["Completed", "Incomplete", "Cancelled", "Failed", "Error"]
            .iter()
            .any(|end| line.starts_with(end))
        {
            return None;
        }
        if line.starts_with("Calculating")
            && let Some(now) = done(&line)
            && now >= count
        {
            return Some(now);
        }
        if Instant::now() >= deadline {
            return None;
        }
        app.idle(Duration::from_millis(5));
    }
}

/// Wait for the status line to start with `prefix`: a run over 20,070 files takes minutes on
/// a busy Windows disk, so the hang guard is long.
fn wait_long(app: &mut App, window: &Tool, what: &str, prefix: &str) -> String {
    let watched = window.clone();
    let wanted = prefix.to_owned();
    app.until_for(what, Duration::from_secs(1200), move |_| {
        progress(&watched).starts_with(&wanted)
    });
    progress(window)
}

fn tree(root: &Path) -> Result<(), String> {
    let flat = root.join("flat");
    fs::create_dir_all(&flat).map_err(|error| error.to_string())?;
    for index in 0..FILES {
        fs::write(
            flat.join(format!("f{index:05}.txt")),
            format!("file {index}\n"),
        )
        .map_err(|error| format!("flat/f{index:05}.txt: {error}"))?;
    }
    let mut level = root.join("chain");
    for depth in 1..=LEVELS {
        level = level.join(format!("{depth:02}"));
        fs::create_dir_all(&level).map_err(|error| format!("{}: {error}", level.display()))?;
        fs::write(level.join("n.txt"), format!("level {depth}\n"))
            .map_err(|error| format!("{}: {error}", level.display()))?;
    }
    Ok(())
}

fn unlimited(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("unlimited");
    tree(&root)?;
    let mut config = Config::new(&root, &context.work);
    config.clipboard = true;
    let mut app = App::start(config);
    for name in ["chain", "flat"] {
        panel::select_row(&mut app, PaneId::Left, name)?;
        input::key(app.slint(), Key::Insert);
    }
    let window = open(&mut app);
    let w = window.window();
    open_menu(&mut app, &window)?;
    tools::set_check(&mut app, w, "Include subfolders", true)?;
    // Every algorithm: the full rows of all 20,070 rows then make a copy well over the
    // 4 MiB the clipboard was once limited to.
    for label in ["SHA-512", "MD5", "SHA-1", "CRC32"] {
        tools::set_check(&mut app, w, label, true)?;
    }
    close_menu(&mut app, &window)?;
    let started = Instant::now();
    tools::click_last(w, "Calculate")?;
    let mut problems = Vec::new();
    let line = wait_long(
        &mut app,
        &window,
        "the calculation of 20,070 files",
        "Completed",
    );
    let took = started.elapsed();
    let texts = inspect::visible_texts(w);
    if tools::count_before(&line, " calculated") != Some(TOTAL) || !line.starts_with("Completed · ")
    {
        problems.push(format!(
            "the calculation ended {line:?}, expected Completed · 20,070 calculated"
        ));
    }
    // Paths are left out: the run's folders are named after this scenario.
    if let Some(limit) = texts
        .iter()
        .filter(|text| !text.contains('/') && !text.contains('\\'))
        .find(|text| text.to_lowercase().contains("limit"))
    {
        problems.push(format!("the window speaks of a limit: {limit:?}"));
    }
    app.screenshot_window(
        context,
        &window,
        "completed",
        &format!("20,070 files: {line}"),
    );
    // Ctrl+A counts every row for Copy.
    let first = tools::list_rows(&window, RESULTS)
        .into_iter()
        .find(|row| row.whole)
        .ok_or("no result row in view")?;
    input::click_at(w, row_point(&first));
    app.settle();
    input::chord(w, &[Key::Control], "a");
    app.settle();
    let copy = copy_action(&mut app, &window)?;
    if copy_caption_of(&copy.label, "SHA-256") != Some(TOTAL) || !copy.enabled {
        problems.push(format!(
            "after Ctrl+A the footer offers {:?} (enabled {})",
            copy.label, copy.enabled
        ));
    }
    // Ctrl+Shift+C: the full rows of every row, far over the former 4 MiB clipboard cap,
    // copied whole (the path and five checksums per row, a blank line between rows).
    let before = tools::clipboard_writes(&app);
    input::chord(w, &[Key::Control, Key::Shift], "c");
    match tools::wait_copied_within(
        &mut app,
        before,
        "Ctrl+Shift+C on 20,070 rows",
        Duration::from_secs(300),
    ) {
        Ok(text) => {
            let blocks: Vec<&str> = text.split("\n\n").collect();
            let first = root.join("flat").join("f00000.txt").display().to_string();
            let digest = crate::encode::hex(&crate::encode::sha256(b"file 0\n"));
            let found = blocks
                .iter()
                .find(|block| block.lines().next() == Some(first.as_str()));
            let whole = blocks.iter().all(|block| block.lines().count() == 6);
            if text.len() <= 4 << 20
                || blocks.len() as u64 != TOTAL
                || !whole
                || !found.is_some_and(|block| block.contains(&format!("SHA-256: {digest}")))
            {
                problems.push(format!(
                    "Ctrl+Shift+C on every row copied {} bytes in {} blocks (every block a path and five checksums: {whole}); the block of {first} {:?}",
                    text.len(),
                    blocks.len(),
                    found.map(|block| block.lines().take(2).collect::<Vec<_>>())
                ));
            }
        }
        Err(error) => problems.push(error),
    }
    // The manifest: every row, and it verifies.
    let saves = app.manifests.len();
    tools::click_last(w, "Save manifest…")?;
    app.until_for(
        "the manifest of 20,070 files",
        Duration::from_secs(600),
        move |app| app.manifests.len() > saves,
    );
    if let Some(Err(error)) = app.manifests.last() {
        return Err(format!("saving the manifest failed: {error}"));
    }
    let manifest = root.join("checksums.sha256");
    let text = fs::read_to_string(&manifest)
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    let lines = text.lines().filter(|line| !line.trim().is_empty()).count() as u64;
    let deepest = format!(
        " *chain/{}/n.txt",
        (1..=LEVELS)
            .map(|depth| format!("{depth:02}"))
            .collect::<Vec<_>>()
            .join("/")
    );
    if lines != TOTAL || !text.contains(&deepest) || !text.contains(" *flat/f19999.txt") {
        problems.push(format!(
            "checksums.sha256 has {lines} lines (expected {TOTAL}), the 70th level's file listed {}, the last flat file {}",
            text.contains(&deepest),
            text.contains(" *flat/f19999.txt")
        ));
    }
    tools::click_last(w, "Verify")?;
    app.settle();
    tools::click_last(w, "Verify")?;
    let verified = wait_long(
        &mut app,
        &window,
        "the manifest's verification",
        "Completed",
    );
    let wanted = "Completed · 20,070 verified · 0 mismatched · 0 missing";
    if tools::count_before(&verified, " verified") != Some(TOTAL)
        || tools::count_before(&verified, " mismatched") != Some(0)
        || tools::count_before(&verified, " missing") != Some(0)
    {
        problems.push(format!(
            "verifying the manifest ended {verified:?}, expected {wanted:?}"
        ));
    }
    tools::close_tool(&mut app, &window, "the Checksums window");
    context.note(format!(
        "{TOTAL} files calculated in {:.0} s: {line:?}; verified: {verified:?}",
        took.as_secs_f64()
    ));
    app.shutdown()?;
    drop(app);
    let _ = fs::remove_dir_all(&root);
    match following(context) {
        Ok(note) => context.note(note),
        Err(error) => problems.push(format!("following the run: {error}")),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// The list following the run needs a run that lasts: 200 sparse files of 32 MiB (no disk
/// space), read with all five algorithms, take a moment each, while the rows of 20,000
/// small files finish faster than the window can be sampled.
const SLOW_FILES: usize = 200;

fn following(context: &mut Context) -> Result<String, String> {
    let folder = context.work.join("follow");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    for index in 0..SLOW_FILES {
        tools::sparse(&folder.join(format!("s{index:03}.bin")), 32 << 20)?;
    }
    let mut app = App::start(Config::new(&folder, &context.work));
    let result = (|| -> Result<String, String> {
        panel::focus_list(&mut app, PaneId::Left)?;
        // Every file of the folder.
        input::chord(app.slint(), &[Key::Control], "a");
        app.settle();
        let window = open(&mut app);
        let w = window.window();
        open_menu(&mut app, &window)?;
        for label in ["SHA-512", "MD5", "SHA-1", "CRC32"] {
            tools::set_check(&mut app, w, label, true)?;
        }
        close_menu(&mut app, &window)?;
        tools::click_last(w, "Calculate")?;
        let note = follow(&mut app, context, &window)?;
        // The rest is not needed: Stop, and the window closes.
        if tools::has(w, AccessibleRole::Button, "Stop") {
            tools::click_last(w, "Stop")?;
            wait_progress(&mut app, &window, "the slow run to stop", "Cancelled");
        }
        tools::close_tool(&mut app, &window, "the Checksums window");
        Ok(note)
    })();
    app.shutdown()?;
    drop(app);
    let _ = fs::remove_dir_all(&folder);
    result
}
/// Whether the rows in view hold both calculated rows and rows still waiting or being
/// read: the row being calculated is in view.
fn boundary_in_view(window: &Tool) -> bool {
    let rows: Vec<tools::ListRow> = tools::list_rows(window, RESULTS)
        .into_iter()
        .filter(|row| row.whole)
        .collect();
    let calculated = rows.iter().any(|row| row.label.contains(" · Calculated"));
    let pending = rows
        .iter()
        .any(|row| row.label.contains(" · Waiting") || row.label.contains(" · Reading"));
    calculated && pending
}

/// The statuses of the rows in view, for failure messages.
fn statuses(window: &Tool) -> String {
    tools::list_rows(window, RESULTS)
        .into_iter()
        .filter(|row| row.whole)
        .map(|row| {
            let status = row.label.split(" · ").nth(3).unwrap_or_default().to_owned();
            format!("{}: {status}", row.index)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// While the run goes on: the row being calculated stays in view across five samples/// with nothing selected; a click stops the following, Ctrl+D resumes it, a scroll by the
/// user pauses it.
fn follow(app: &mut App, context: &mut Context, window: &Tool) -> Result<String, String> {
    let w = window.window();
    // The listed files show as rows first: a screenful of them gives the page size.
    let deadline = Instant::now() + Duration::from_secs(300);
    while in_view(window).len() < 4 {
        if Instant::now() >= deadline {
            return Err(format!(
                "no screenful of rows while the run lists its files ({:?})",
                progress(window)
            ));
        }
        app.idle(Duration::from_millis(10));
    }
    let rows = in_view(window).len() as u64;
    let page = |_: &Tool| rows;
    // Past the first screenful first.
    let mut at = wait_done(app, window, page(window) * 2)
        .ok_or("the run ended before the list had to move")?;
    let mut samples = Vec::new();
    let mut last_first = 0;
    for sample in 0..5 {
        let shown = in_view(window);
        let line = progress(window);
        let now = done(&line).unwrap_or(at);
        // The row being calculated is in view: the boundary between the calculated rows
        // and those still waiting (the progress line counts in steps of the rows' chunks,
        // so the rows themselves say where the run is); and the list has moved on.
        let first = shown.first().copied().unwrap_or_default();
        if !boundary_in_view(window) || first <= last_first {
            return Err(format!(
                "sample {}: {now} files done, the rows in view are {:?}..{:?} ({})",
                sample + 1,
                shown.first(),
                shown.last(),
                statuses(window)
            ));
        }
        last_first = first;
        samples.push((now, first));
        at = wait_done(app, window, now + page(window))
            .ok_or_else(|| format!("the run ended after {} samples", sample + 1))?;
    }
    app.screenshot_window(
        context,
        window,
        "following",
        "The list follows the row being calculated",
    );
    // A click selects a row: the list stays put while the run goes on.
    let rows: Vec<tools::ListRow> = tools::list_rows(window, RESULTS)
        .into_iter()
        .filter(|row| row.whole)
        .collect();
    let target = rows
        .get(rows.len() / 2)
        .ok_or("no row in view to click")?
        .clone();
    input::click_at(w, row_point(&target));
    app.settle();
    let held = in_view(window);
    let now = done(&progress(window)).unwrap_or(at);
    wait_done(app, window, now + page(window))
        .ok_or("the run ended before the selection's check")?;
    let after_click = in_view(window);
    if after_click != held
        || !tools::selected_names(window, RESULTS).contains(&target.name().to_owned())
    {
        return Err(format!(
            "with {} selected the list moved from {:?}..{:?} to {:?}..{:?}",
            target.name(),
            held.first(),
            held.last(),
            after_click.first(),
            after_click.last()
        ));
    }
    // Ctrl+D: nothing selected, the list follows again.
    input::chord(w, &[Key::Control], "d");
    app.settle();
    let now = done(&progress(window)).unwrap_or(at);
    let resumed = wait_done(app, window, now + page(window))
        .ok_or("the run ended before the following could resume")?;
    let shown = in_view(window);
    let line = progress(window);
    let current = done(&line).unwrap_or(resumed);
    if !boundary_in_view(window) || shown.first() <= after_click.first() {
        return Err(format!(
            "after Ctrl+D the list does not follow: {current} done, rows {:?}..{:?} in view ({})",
            shown.first(),
            shown.last(),
            statuses(window)
        ));
    }
    // A scroll by the user pauses the following: the list stays where the wheel left it.
    let list = tools::control(w, AccessibleRole::List, RESULTS)?;
    let middle = inspect::center(&list);
    for _ in 0..5 {
        w.dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(middle.x, middle.y),
            delta_x: 0.0,
            delta_y: 400.0,
        });
        app.settle();
    }
    let scrolled = in_view(window);
    let now = done(&progress(window)).unwrap_or(current);
    wait_done(app, window, now + page(window)).ok_or("the run ended before the scroll's check")?;
    let after_scroll = in_view(window);
    if after_scroll != scrolled {
        return Err(format!(
            "after a scroll up the list moved on from {:?}..{:?} to {:?}..{:?}",
            scrolled.first(),
            scrolled.last(),
            after_scroll.first(),
            after_scroll.last()
        ));
    }
    Ok(format!(
        "following: samples (done, first row in view) {samples:?}; a click held rows {:?}..{:?}; Ctrl+D resumed; a scroll paused at {:?}..{:?}",
        held.first(),
        held.last(),
        scrolled.first(),
        scrolled.last()
    ))
}
