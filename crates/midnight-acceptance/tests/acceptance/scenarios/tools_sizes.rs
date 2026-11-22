//! Folder sizes (Ctrl+Shift+S) on marked folders: links/ (links inside not followed),
//! readonly/ and many/, also with a filename search typed meanwhile; with nothing marked,
//! during a filename search, every folder (as without one); directory links listed in links/
//! themselves; a folder with an unreadable subfolder (a partial size "≈ N" whose warning
//! says it is a partial total); and cancelling a running scan (an estimate or "—", said
//! to be unfinished).
use crate::{
    Context,
    app::{App, Config},
    input, panel, tools_support as tools,
};
use midnight_core::PaneId;
use midnight_ui::FileRow;
use slint::platform::{Key, WindowAdapter};
use std::{fs, path::Path, time::Duration};

/// Logical bytes of regular files below `path`; links are not followed.
fn disk_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        0
    } else if metadata.is_dir() {
        fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| disk_size(&entry.path()))
            .sum()
    } else {
        metadata.len()
    }
}

fn row(app: &App, name: &str) -> Result<FileRow, String> {
    app.rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| format!("no {name:?} row; rows {:?}", app.names(PaneId::Left)))
}

/// A finished, exact size in bytes.
fn exact(row: &FileRow) -> Option<u64> {
    if row.size_state == 0 && !row.size_running && !row.size.contains('≈') {
        tools::number(&row.size)
    } else {
        None
    }
}

/// Focus the left list with nothing marked, so a scan measures every folder.
fn unmarked_focus(app: &mut App) -> Result<(), String> {
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    let marked = app
        .rows(PaneId::Left)
        .iter()
        .filter(|row| row.selected)
        .count();
    if marked != 0 {
        return Err(format!("{marked} rows stay marked after Ctrl+D"));
    }
    Ok(())
}

/// The name under the left panel's cursor.
fn focused_name(app: &App) -> Option<String> {
    usize::try_from(app.cursor(PaneId::Left))
        .ok()
        .and_then(|row| app.names(PaneId::Left).get(row).cloned())
}

fn measuring(app: &App) -> bool {
    app.window.get_left_status().contains("Measuring folders")
}

/// Press Ctrl+Shift+S and wait until the scan started and finished.
fn measure(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    input::chord(app.slint(), &[Key::Control, Key::Shift], "s");
    app.until_for(what, Duration::from_secs(180), |app| {
        !measuring(app) && done(app)
    });
}

pub fn folder_sizes_tools(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut config = Config::new(&root, &context.work);
    config.size_bytes = true;
    let mut app = App::start(config);
    let pane = PaneId::Left;

    // Marked folders only: links (its links are not followed), readonly and many. A
    // filename search typed meanwhile ("tex", on text/) does not change what is measured:
    // Ctrl+Shift+S ends the search and measures the marks.
    let marked = ["links", "readonly", "many"];
    for name in marked {
        panel::select_row(&mut app, pane, name)?;
        input::key(app.slint(), Key::Insert);
    }
    input::type_text(app.slint(), "tex");
    app.until("the filename search to find text", |app| {
        app.window.get_left_search_active() && focused_name(app).as_deref() == Some("text")
    });
    measure(&mut app, "the marked folder sizes", |app| {
        marked
            .iter()
            .all(|name| row(app, name).ok().as_ref().and_then(exact).is_some())
    });
    let mut problems = Vec::new();
    for name in marked {
        let shown = row(&app, name)?;
        let expected = disk_size(&root.join(name));
        if exact(&shown) != Some(expected) {
            problems.push(format!("{name}: shown {:?}, disk {expected}", shown.size));
        }
    }
    if disk_size(&root.join("links")) != 0 {
        problems.push("the links folder's own files are not zero bytes".into());
    }
    // Unmarked folders were not measured.
    for name in ["text", "deep", "sizes"] {
        let shown = row(&app, name)?;
        if exact(&shown).is_some() {
            problems.push(format!("unmarked {name} was measured: {:?}", shown.size));
        }
    }
    if app.window.get_left_search_active() {
        problems.push("the filename search stays open after Ctrl+Shift+S".into());
    }
    app.screenshot(
        context,
        "marked-folders",
        "Ctrl+Shift+S on marked links/, readonly/ and many/",
    );
    if !problems.is_empty() {
        return Err(format!("marked folder sizes: {problems:#?}"));
    }
    // With nothing marked, Ctrl+Shift+S during a filename search ("dee", on deep/) ends
    // the search and measures as it does without one: every folder ("Measure selected
    // folders, or all folders when none are selected"), not only the focused match.
    unmarked_focus(&mut app)?;
    input::type_text(app.slint(), "dee");
    app.until("the filename search to find deep", |app| {
        app.window.get_left_search_active() && focused_name(app).as_deref() == Some("deep")
    });
    let all = ["deep", "text", "sizes"];
    measure(&mut app, "every folder's size", |app| {
        all.iter()
            .all(|name| row(app, name).ok().as_ref().and_then(exact).is_some())
    });
    for name in all {
        let shown = row(&app, name)?;
        if exact(&shown) != Some(disk_size(&root.join(name))) {
            return Err(format!(
                "Ctrl+Shift+S during a search with nothing marked: {name}/ shows {:?}, the disk {}",
                shown.size,
                disk_size(&root.join(name))
            ));
        }
    }
    if app.window.get_left_search_active() {
        return Err("the filename search stays open after Ctrl+Shift+S".into());
    }
    // Inside links/: a marked folder link is measured as its target; the link to its
    // own folder measures that folder without following the links in it. Windows has
    // no symbolic links in the fixture (they need a privilege): there the same folder
    // links are directory junctions in a work folder, opened with Ctrl+G.
    unmarked_focus(&mut app)?;
    #[cfg(windows)]
    {
        let links = context.work.join("links");
        fs::create_dir(&links).map_err(|error| format!("{}: {error}", links.display()))?;
        crate::fixtures::junction(
            &links.join("to-folder"),
            &root.join("deep").join("level-01"),
        )?;
        crate::fixtures::junction(&links.join("to-self"), &links)?;
        input::chord(app.slint(), &[Key::Control], "g");
        input::replace_text(app.slint(), &links.display().to_string());
        input::key(app.slint(), Key::Return);
        panel::wait_location(&mut app, pane, &links);
        context.note("inside links/: directory junctions in a work folder (Windows)");
    }
    #[cfg(not(windows))]
    panel::enter(&mut app, pane, "links")?;
    for name in ["to-folder", "to-self"] {
        panel::select_row(&mut app, pane, name)?;
        input::key(app.slint(), Key::Insert);
    }
    measure(&mut app, "the sizes inside links/", |app| {
        ["to-folder", "to-self"].iter().all(|name| {
            row(app, name)
                .ok()
                .as_ref()
                .is_some_and(|row| exact(row).is_some() || row.size_state != 0)
        })
    });
    let to_folder = row(&app, "to-folder")?;
    let to_self = row(&app, "to-self")?;
    let target = disk_size(&root.join("deep/level-01"));
    context.note(format!(
        "inside links/: to-folder {:?} (target holds {target} bytes), to-self {:?}, dangling {:?}, to-file {:?}",
        to_folder.size,
        to_self.size,
        row(&app, "dangling").map(|row| row.size.to_string()).unwrap_or_default(),
        row(&app, "to-file").map(|row| row.size.to_string()).unwrap_or_default()
    ));
    if exact(&to_folder) != Some(target) || exact(&to_self) != Some(0) {
        return Err(format!(
            "inside links/: to-folder shows {:?} (expected {target}), to-self {:?} (expected 0)",
            to_folder.size, to_self.size
        ));
    }
    app.screenshot(
        context,
        "inside-links",
        "Folder links measured inside links/",
    );
    app.shutdown()?;

    // An unreadable subfolder: a partial size with a warning.
    let partial = context.work.join("sizes-warning");
    fs::create_dir_all(partial.join("mixed/locked")).map_err(|error| error.to_string())?;
    fs::write(partial.join("mixed/readable.bin"), [7_u8; 100])
        .map_err(|error| error.to_string())?;
    fs::write(partial.join("mixed/locked/hidden.bin"), [7_u8; 50])
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(partial.join("clean")).map_err(|error| error.to_string())?;
    fs::write(partial.join("clean/file.bin"), [1_u8; 64]).map_err(|error| error.to_string())?;
    tools::set_mode(&partial.join("mixed/locked"), 0o000)?;
    let mut config = Config::new(&partial, &context.work);
    config.size_bytes = true;
    let mut app = App::start(config);
    unmarked_focus(&mut app)?;
    measure(&mut app, "the sizes with an unreadable folder", |app| {
        ["mixed", "clean"].iter().all(|name| {
            row(app, name)
                .ok()
                .is_some_and(|row| !row.size_running && row.size_state != 1)
        })
    });
    let mixed = row(&app, "mixed")?;
    let clean = row(&app, "clean")?;
    app.screenshot(
        context,
        "unreadable",
        "A folder with an unreadable subfolder",
    );
    tools::set_mode(&partial.join("mixed/locked"), 0o755)?;
    if exact(&clean) != Some(64) {
        return Err(format!("clean/ shows {:?}", clean.size));
    }
    if mixed.size_state != 3 || !mixed.size.contains('≈') || tools::number(&mixed.size) != Some(100)
    {
        return Err(format!(
            "mixed/ with an unreadable subfolder shows {:?} in state {}",
            mixed.size, mixed.size_state
        ));
    }
    let main = app.main.clone();
    if !tools::has(
        main.window(),
        i_slint_core::items::AccessibleRole::Button,
        "Folder size warning",
    ) {
        return Err("no Folder size warning button for the partial size".into());
    }
    // The warning says what the number is: a finished scan with warnings, a partial total.
    if !mixed
        .size_hint
        .starts_with("Completed with warnings; this is a partial total.")
    {
        return Err(format!(
            "the partial size's warning reads {:?}",
            mixed.size_hint
        ));
    }
    app.shutdown()?;

    // Cancel: one folder of 100,000 empty files keeps the scan busy long enough to
    // cancel it (top-level folders are measured concurrently, one folder's tree in
    // order); two small folders beside it.
    let busy = context.work.join("sizes-cancel");
    for folder in 0..100 {
        let path = busy.join("bulk").join(format!("part-{folder:03}"));
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        for file in 0..1000 {
            fs::write(path.join(format!("{file:04}")), b"").map_err(|error| error.to_string())?;
        }
    }
    for name in ["small-a", "small-b"] {
        fs::create_dir_all(busy.join(name)).map_err(|error| error.to_string())?;
        fs::write(busy.join(name).join("file"), b"").map_err(|error| error.to_string())?;
    }
    let mut config = Config::new(&busy, &context.work);
    config.size_bytes = true;
    let mut app = App::start(config);
    unmarked_focus(&mut app)?;
    input::chord(app.slint(), &[Key::Control, Key::Shift], "s");
    app.until("the scan to run", measuring);
    input::chord(app.slint(), &[Key::Control, Key::Shift], "s");
    app.until("the scan to stop", |app| !measuring(app));
    let status = app.window.get_desktop_status().to_string();
    let rows: Vec<FileRow> = app
        .rows(pane)
        .into_iter()
        .filter(|row| row.directory && !row.parent_entry)
        .collect();
    let partial_rows = rows.iter().filter(|row| row.size_state == 2).count();
    let running = rows.iter().filter(|row| row.size_running).count();
    let bulk = row(&app, "bulk")?;
    app.screenshot(
        context,
        "cancelled",
        "Ctrl+Shift+S again cancelled the scan",
    );
    // A cancelled folder shows what was counted as an estimate ("≈ N"), or "—" when
    // nothing was, and says it is unfinished.
    if !status.contains("cancelled")
        || bulk.size_state != 2
        || exact(&bulk).is_some()
        || !(bulk.size.starts_with("≈ ") || bulk.size == "—")
        || !bulk.size_hint.starts_with("Unfinished or cancelled")
        || running != 0
    {
        return Err(format!(
            "after cancelling: status {status:?}, bulk shows {:?} in state {} (hint {:?}), {partial_rows} partial and {running} running of {} folders",
            bulk.size,
            bulk.size_state,
            bulk.size_hint,
            rows.len()
        ));
    }
    // Measuring again completes every folder.
    measure(&mut app, "the full scan", |app| {
        app.rows(PaneId::Left)
            .iter()
            .filter(|row| row.directory && !row.parent_entry)
            .all(|row| exact(row) == Some(0))
    });
    context.note(format!(
        "links/ 0 B (not followed), readonly/ and many/ exact; unreadable subfolder → partial with warning; cancel: {status:?}, {partial_rows} of {} folders partial",
        rows.len()
    ));
    app.shutdown()
}
