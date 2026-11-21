//! Find's results and what they act on: every column sorted in both directions against
//! known orders (folders first and missing extensions last under Extension, equal keys
//! by path), the Name order of mixed-case and accented names equal to the panel's;
//! keyboard selection in the results; Go to file by Enter, double click and button, for
//! files and folders, and Open on a folder; View, F3 and Alt+F3 capture the selected
//! file for the viewer (folders cannot be viewed or edited); Edit and F4 run the
//! configured editor on the result; Open reaches the native launcher; a result deleted
//! or changed after the search is refused with its reason, the panel unmoved; Stop keeps
//! the results found so far, sorted, and a new search replaces them.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::SettingsStore,
    panel,
    scenarios::find_support::{
        self as find, BASE, Findings, Launched, Query, Row, all_rows, ordered, record_launches,
        search, select_result, status,
    },
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{EditorPreferences, PaneId};
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter},
};
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

struct Setup {
    act: PathBuf,
    collate: PathBuf,
    stop: PathBuf,
    edited: PathBuf,
}

fn setup(work: &Path) -> Result<Setup, String> {
    let act = work.join("act");
    let day = |month: u64| {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_640_995_200 + month * 2_678_400)
    };
    for (path, size, month) in [
        ("alpha.txt", 10, 0),
        ("gamma.md", 20, 1),
        ("beta.log", 300, 2),
        ("sub/epsilon.txt", 4000, 3),
        ("delta", 5, 4),
        ("sub/zeta.rs", 1, 5),
    ] {
        let data: Vec<u8> = path.bytes().cycle().take(size).collect();
        tools::write_at(&act.join(path), &data, day(month))?;
    }
    fs::create_dir_all(act.join("zfolder.d")).map_err(|error| error.to_string())?;
    let collate = work.join("collate");
    let now = SystemTime::now();
    for name in [
        "apple.txt",
        "Banana.txt",
        "cherry.txt",
        "Äpfel.txt",
        "_under.txt",
        "10-ten.txt",
        "9-nine.txt",
        "Zebra.txt",
        "éclair.txt",
    ] {
        tools::write_at(&collate.join(name), name.as_bytes(), now)?;
    }
    let stop = work.join("stop");
    for index in 0..20 {
        tools::write_at(
            &stop.join("early").join(format!("e{index:02}.txt")),
            b"a needle in a file\n",
            now,
        )?;
    }
    fs::create_dir_all(stop.join("late")).map_err(|error| error.to_string())?;
    for name in ["huge-1.txt", "huge-2.txt"] {
        tools::sparse(&stop.join("late").join(name), 1 << 30)?;
    }
    let edited = work.join("edited");
    fs::create_dir_all(&edited).map_err(|error| error.to_string())?;
    Ok(Setup {
        act,
        collate,
        stop,
        edited,
    })
}

pub fn result_actions(context: &mut Context) -> Result<(), String> {
    let setup = setup(&context.work)?;
    let cp = crate::tool::program(context, "cp")?;
    let edited = setup.edited.display().to_string();
    let store = SettingsStore::seeded(move |preferences| {
        preferences.editor = EditorPreferences {
            executable: Some(cp),
            arguments: vec!["{file}".into(), edited],
            rules: Vec::new(),
        };
    });
    let mut config = Config::new(&setup.act, &setup.collate);
    config.settings = Some(store);
    config.editor = true;
    let mut app = App::start(config);
    let launched = record_launches(&app);
    let mut findings = Findings::default();
    findings.check("sorting", sorting(&mut app, context, &setup));
    find::reset(&mut app);
    findings.check("collation", collation(&mut app, &setup));
    find::reset(&mut app);
    findings.check(
        "Go to file and the result keys",
        go_to(&mut app, context, &setup),
    );
    find::reset(&mut app);
    findings.check(
        "View, Edit and Open",
        view_edit_open(&mut app, context, &setup, &launched),
    );
    find::reset(&mut app);
    findings.check(
        "a new search after scrolling",
        scrolled_list(&mut app, context, &setup),
    );
    find::reset(&mut app);
    findings.check("stale results", stale(&mut app, &setup, &launched));
    find::reset(&mut app);
    findings.check("Stop", stop(&mut app, context, &setup));
    find::reset(&mut app);
    app.shutdown()?;
    context.note("Every column sorted both ways (folders first, missing extensions last, ties by path); Name order equals the panel's; Home/End/Up/Down/Page Down in the results; Go to file by Enter, double click and button, for files and folders, Open on a folder; View/F3/Alt+F3 captures and Edit/F4 runs the editor on the result; Open reaches the launcher; deleted and changed results refused; Stop keeps 20 results, sorted");
    findings.into_result()
}

/// Each row's Extension, Size and Modified columns agree with the disk: the name's
/// extension, "<DIR>" or the byte count, the modification time present.
fn columns_match(rows: &[Row]) -> Result<(), String> {
    for row in rows {
        let path = row.path();
        let metadata =
            fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = if metadata.is_dir() {
            "<DIR>".to_owned()
        } else {
            format!("{} B", metadata.len())
        };
        if row.extension != extension
            || crate::tools_support::number(&row.size) != crate::tools_support::number(&size)
            || row.size.ends_with(" B") != size.ends_with(" B")
            || row.modified.trim().is_empty()
        {
            return Err(format!(
                "the row of {} shows extension {:?}, size {:?}, modified {:?}; the disk has {extension:?} and {size:?}",
                row.name, row.extension, row.size, row.modified
            ));
        }
    }
    Ok(())
}

/// Wait until the results show `wanted` (paths relative to `root`) in this order.
fn wait_order(
    app: &mut App,
    window: &Tool,
    root: &Path,
    wanted: &[&str],
    what: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let rows = all_rows(app, window)?;
        let shown = ordered(&rows, root);
        if shown == wanted {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("{what}: {shown:?}, expected {wanted:?}"));
        }
        app.idle(Duration::from_millis(50));
    }
}

/// Click a header and wait for the order and the header's arrow.
fn sort_by(
    app: &mut App,
    window: &Tool,
    root: &Path,
    column: &str,
    arrow: &str,
    wanted: &[&str],
) -> Result<(), String> {
    input::click_button(window.window(), &format!("Sort by {column}"))?;
    wait_order(app, window, root, wanted, &format!("{column} {arrow}"))?;
    let header = format!("{column} {arrow}");
    if !inspect::visible_texts(window.window()).contains(&header) {
        return Err(format!("the {column} header does not show {arrow}"));
    }
    Ok(())
}

fn sorting(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    let window = find::open_find(app)?;
    let root = &setup.act;
    let (text, rows) = search(app, &window, BASE)?;
    if !text.starts_with("Completed") {
        return Err(format!("*.* ended {text:?}"));
    }
    columns_match(&rows)?;
    let names_ascending = [
        "alpha.txt",
        "beta.log",
        "delta",
        "sub/epsilon.txt",
        "gamma.md",
        "sub",
        "sub/zeta.rs",
        "zfolder.d",
    ];
    wait_order(app, &window, root, &names_ascending, "Name ascending")?;
    if !inspect::visible_texts(window.window())
        .iter()
        .any(|text| text == "Name ↑")
    {
        return Err("the Name header shows no ascending arrow".into());
    }
    let mut descending = names_ascending;
    descending.reverse();
    sort_by(app, &window, root, "Name", "↓", &descending)?;
    sort_by(
        app,
        &window,
        root,
        "Extension",
        "↑",
        &[
            "zfolder.d",
            "sub",
            "beta.log",
            "gamma.md",
            "sub/zeta.rs",
            "alpha.txt",
            "sub/epsilon.txt",
            "delta",
        ],
    )?;
    sort_by(
        app,
        &window,
        root,
        "Extension",
        "↓",
        &[
            "zfolder.d",
            "sub",
            "alpha.txt",
            "sub/epsilon.txt",
            "sub/zeta.rs",
            "gamma.md",
            "beta.log",
            "delta",
        ],
    )?;
    app.screenshot_window(
        context,
        &window,
        "extension-descending",
        "Extension descending: folders first, the extensionless file last",
    );
    sort_by(
        app,
        &window,
        root,
        "Path",
        "↑",
        &[
            "alpha.txt",
            "beta.log",
            "delta",
            "gamma.md",
            "sub",
            "zfolder.d",
            "sub/epsilon.txt",
            "sub/zeta.rs",
        ],
    )?;
    sort_by(
        app,
        &window,
        root,
        "Path",
        "↓",
        &[
            "sub/epsilon.txt",
            "sub/zeta.rs",
            "alpha.txt",
            "beta.log",
            "delta",
            "gamma.md",
            "sub",
            "zfolder.d",
        ],
    )?;
    // Files only for sizes and times; the search keeps the window's order (Path ↓).
    let (text, _) = search(
        app,
        &window,
        Query {
            mode: "Expression",
            masks: "kind = file",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") {
        return Err(format!("kind = file ended {text:?}"));
    }
    wait_order(
        app,
        &window,
        root,
        &[
            "sub/epsilon.txt",
            "sub/zeta.rs",
            "alpha.txt",
            "beta.log",
            "delta",
            "gamma.md",
        ],
        "a new search in the window's order (Path descending)",
    )?;
    let by_size = [
        "sub/zeta.rs",
        "delta",
        "alpha.txt",
        "gamma.md",
        "beta.log",
        "sub/epsilon.txt",
    ];
    sort_by(app, &window, root, "Size", "↑", &by_size)?;
    let mut reversed = by_size;
    reversed.reverse();
    sort_by(app, &window, root, "Size", "↓", &reversed)?;
    let by_time = [
        "alpha.txt",
        "gamma.md",
        "beta.log",
        "sub/epsilon.txt",
        "delta",
        "sub/zeta.rs",
    ];
    sort_by(app, &window, root, "Modified", "↑", &by_time)?;
    let mut reversed = by_time;
    reversed.reverse();
    sort_by(app, &window, root, "Modified", "↓", &reversed)?;
    find::close(app, &window);
    Ok(())
}

/// Name order uses the panels' collation: the results of the right panel's folder in
/// the same order as that panel lists them.
fn collation(app: &mut App, setup: &Setup) -> Result<(), String> {
    let window = find::open_on(app, PaneId::Right)?;
    let (text, _) = search(
        app,
        &window,
        Query {
            masks: "*.txt",
            ..BASE
        },
    )?;
    let panel: Vec<String> = app
        .names(PaneId::Right)
        .into_iter()
        .filter(|name| name != "..")
        .collect();
    let wanted: Vec<&str> = panel.iter().map(String::as_str).collect();
    let sorted = find::wait_names(app, &window, &wanted, "Name order as in the panel");
    find::close(app, &window);
    if !text.starts_with("Completed") {
        return Err(format!("*.txt over the collation folder ended {text:?}"));
    }
    let found = sorted?;
    if setup.collate.read_dir().map(|entries| entries.count()).ok() != Some(found.len()) {
        return Err(format!("{} names for the collation folder", found.len()));
    }
    Ok(())
}

fn go_to(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    let window = find::open_find(app)?;
    let w = window.window();
    let (text, rows) = search(app, &window, BASE)?;
    if !text.starts_with("Completed") || rows.len() != 8 {
        return Err(format!("*.* ended {text:?} with {} rows", rows.len()));
    }
    let rows = find::wait_names(
        app,
        &window,
        &[
            "alpha.txt",
            "beta.log",
            "delta",
            "epsilon.txt",
            "gamma.md",
            "sub",
            "zeta.rs",
            "zfolder.d",
        ],
        "Name ascending",
    )?;
    let names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    // Keys move the selection.
    select_result(app, &window, "delta", None)?;
    for (key, wanted, what) in [
        (Key::Home, names.first(), "Home"),
        (Key::End, names.last(), "End"),
        (Key::UpArrow, names.get(names.len() - 2), "Up"),
        (Key::Home, names.first(), "Home again"),
        (Key::DownArrow, names.get(1), "Down"),
        (Key::PageDown, names.last(), "Page Down past the end"),
    ] {
        input::key(w, key);
        app.settle();
        if find::selected_name(&window).as_ref() != wanted {
            return Err(format!(
                "{what} selected {:?}, expected {wanted:?}",
                find::selected_name(&window)
            ));
        }
    }
    // Enter: Go to file of the selected file.
    select_result(app, &window, "epsilon.txt", None)?;
    input::key(w, Key::Return);
    find::wait_cursor(app, PaneId::Left, &setup.act.join("sub"), "epsilon.txt");
    if !w.is_visible() {
        return Err("Go to file closed the Find window".into());
    }
    // A double click.
    let (_, (x, y, width, height)) =
        find::reveal_row(app, &window, |row: &Row| row.name == "gamma.md")?;
    input::double_click_at(
        w,
        LogicalPosition::new(x + width.min(160.0) / 2.0, y + height / 2.0),
    );
    find::wait_cursor(app, PaneId::Left, &setup.act, "gamma.md");
    // The button on a folder; Open on a folder goes there too.
    select_result(app, &window, "zfolder.d", None)?;
    for (button, enabled) in [("View", false), ("Edit", false), ("Open", true)] {
        if find::button_enabled(&window, button)? != enabled {
            return Err(format!(
                "with a folder selected, {button} is enabled={}",
                !enabled
            ));
        }
    }
    tools::click_last(w, "Go to file")?;
    find::wait_cursor(app, PaneId::Left, &setup.act, "zfolder.d");
    panel::focus_list(app, PaneId::Left)?;
    panel::enter(app, PaneId::Left, "sub")?;
    select_result(app, &window, "sub", None)?;
    tools::click_last(w, "Open")?;
    find::wait_cursor(app, PaneId::Left, &setup.act, "sub");
    app.screenshot(
        context,
        "goto-folder",
        "Open on a folder result: Go to file",
    );
    find::close(app, &window);
    Ok(())
}

fn view_edit_open(
    app: &mut App,
    context: &mut Context,
    setup: &Setup,
    launched: &Rc<RefCell<Vec<Launched>>>,
) -> Result<(), String> {
    let window = find::open_find(app)?;
    let w = window.window();
    let (text, _) = search(app, &window, BASE)?;
    if !text.starts_with("Completed") {
        return Err(format!("*.* ended {text:?}"));
    }
    select_result(app, &window, "alpha.txt", None)?;
    for button in ["View", "Edit", "Open"] {
        if !find::button_enabled(&window, button)? {
            return Err(format!("with a file selected, {button} is disabled"));
        }
    }
    for (button, hint) in [
        ("View", "View internally (F3); plain text (Alt+F3)."),
        ("Edit", "Edit with the configured external editor (F4)."),
    ] {
        let item = tools::control(w, AccessibleRole::Button, button)?;
        let tips = tools::hover_tooltip(app, w, &item);
        if !tips.iter().any(|tip| tip == hint) {
            return Err(format!("the {button} tooltip shows {tips:?}"));
        }
    }
    // View, F3 and Alt+F3 capture the selected file for the viewer.
    let expect = |launch: Launched, path: &Path, basic: bool, how: &str| {
        if launch.path != path || launch.basic != basic || !launch.captured {
            return Err(format!(
                "{how} asked for {launch:?}, expected {} (plain text {basic})",
                path.display()
            ));
        }
        Ok(())
    };
    let before = launched.borrow().len();
    tools::click_last(w, "View")?;
    let launch = find::wait_launch(app, launched, before, "View")?;
    expect(launch, &setup.act.join("alpha.txt"), false, "View")?;
    select_result(app, &window, "beta.log", None)?;
    let before = launched.borrow().len();
    input::key(w, Key::F3);
    let launch = find::wait_launch(app, launched, before, "F3")?;
    expect(launch, &setup.act.join("beta.log"), false, "F3")?;
    select_result(app, &window, "gamma.md", None)?;
    let before = launched.borrow().len();
    input::chord(w, &[Key::Alt], Key::F3);
    let launch = find::wait_launch(app, launched, before, "Alt+F3")?;
    expect(launch, &setup.act.join("gamma.md"), true, "Alt+F3")?;
    // A folder never reaches the viewer: F3 on it goes to the folder instead.
    select_result(app, &window, "zfolder.d", None)?;
    let before = launched.borrow().len();
    input::key(w, Key::F3);
    find::wait_cursor(app, PaneId::Left, &setup.act, "zfolder.d");
    app.idle(Duration::from_millis(300));
    if launched.borrow().len() != before {
        return Err("F3 on a folder result asked for a viewer".into());
    }

    // Edit and F4 run the configured editor (cp into edited/) on the result.
    for (name, folder, how) in [
        ("delta", setup.act.clone(), "Edit"),
        ("zeta.rs", setup.act.join("sub"), "F4"),
    ] {
        select_result(app, &window, name, Some(folder.as_path()))?;
        if how == "Edit" {
            tools::click_last(w, "Edit")?;
        } else {
            input::key(w, Key::F4);
        }
        let copy = setup.edited.join(name);
        crate::options_support::wait_file(app, &copy, &format!("{how} on {name}"))?;
        let expected = fs::read(folder.join(name)).map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while fs::read(&copy).ok().as_ref() != Some(&expected) {
            if Instant::now() >= deadline {
                return Err(format!("{how} on {name} copied other bytes"));
            }
            app.idle(Duration::from_millis(20));
        }
    }
    app.until("the editor status", |app| {
        app.window.get_desktop_status() == "External editor started"
    });

    // Open reaches the native launcher, which this harness does not install.
    select_result(app, &window, "alpha.txt", None)?;
    tools::click_last(w, "Open")?;
    let (title, body) = find::wait_notice(app, "Open on a file")?;
    app.screenshot(
        context,
        "open-unavailable",
        "Open: no desktop launcher here",
    );
    find::dismiss_notice(app);
    if title != "Desktop action failed" || !body.contains("unavailable") {
        return Err(format!("Open on a file reported {title:?}: {body:?}"));
    }
    find::close(app, &window);
    Ok(())
}

/// A new, shorter result list (ten rows, then one) after scrolling a long one to its end
/// shows its rows: the list must not stay scrolled past its new content.
fn scrolled_list(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    let many = context.fixture.path("many");
    let window = find::open_find(app)?;
    find::add_root(app, &window, &many, false)?;
    find::tick_root(app, &window, &setup.act, false)?;
    let mut problems = Vec::new();
    for (masks, shot) in [
        ("item-0000?.*", "after-scrolling-ten"),
        ("item-00001.*", "after-scrolling-one"),
    ] {
        let (text, rows) = search(app, &window, BASE)?;
        if !text.starts_with("Completed") || rows.len() < 100 {
            return Err(format!(
                "*.* over many/ ended {text:?} with {} rows",
                rows.len()
            ));
        }
        find::wheel(app, &window, -1.0e7)?;
        find::fill(app, &window, Query { masks, ..BASE })?;
        let text = find::run(app, &window)?;
        app.idle(Duration::from_millis(400));
        let shown = find::visible_rows(&window).len();
        let total = find::result_count(&window);
        app.screenshot_window(
            context,
            &window,
            shot,
            &format!(
                "{masks} after scrolling {} results: {shown} of {total} rows shown",
                rows.len()
            ),
        );
        if !text.starts_with("Completed") || total == 0 || shown != total {
            problems.push(format!(
                "finding FIND-RESULTS-SCROLL: after scrolling {} results to the end, {masks} ({text}) shows {shown} of its {total} rows: the list stays scrolled past its new content",
                rows.len()
            ));
        }
    }
    find::close(app, &window);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// A result deleted or changed after the search is refused with its reason; the panel
/// does not move and no viewer or editor runs.
fn stale(
    app: &mut App,
    setup: &Setup,
    launched: &Rc<RefCell<Vec<Launched>>>,
) -> Result<(), String> {
    let window = find::open_find(app)?;
    let w = window.window();
    let (text, _) = search(app, &window, BASE)?;
    if !text.starts_with("Completed") {
        return Err(format!("*.* ended {text:?}"));
    }
    let location = app.location(PaneId::Left);
    fs::remove_file(setup.act.join("beta.log")).map_err(|error| error.to_string())?;
    select_result(app, &window, "beta.log", None)?;
    tools::click_last(w, "Go to file")?;
    let error = find::wait_error(app, &window, "Go to file of a deleted result", |text| {
        text.contains("beta.log")
    })?;
    app.idle(Duration::from_millis(200));
    if app.location(PaneId::Left) != location {
        return Err(format!(
            "Go to file of a deleted result moved the panel to {:?} ({error})",
            app.location(PaneId::Left)
        ));
    }
    let before = launched.borrow().len();
    tools::click_last(w, "View")?;
    find::wait_error(app, &window, "View of a deleted result", |text| {
        text.contains("beta.log")
    })?;
    // A changed file: its revision differs from the search's.
    let alpha = setup.act.join("alpha.txt");
    fs::write(&alpha, b"changed after the search\n").map_err(|error| error.to_string())?;
    let _ = fs::remove_file(setup.edited.join("alpha.txt"));
    select_result(app, &window, "alpha.txt", None)?;
    tools::click_last(w, "Edit")?;
    let error = find::wait_error(app, &window, "Edit of a changed result", |text| {
        text.contains("alpha.txt") || text.contains("changed")
    })?;
    tools::click_last(w, "View")?;
    find::wait_error(app, &window, "View of a changed result", |text| {
        text.contains("changed") || text.contains("alpha.txt")
    })?;
    app.idle(Duration::from_millis(500));
    if setup.edited.join("alpha.txt").exists() {
        return Err(format!("the editor ran on a changed result ({error})"));
    }
    if launched.borrow().len() != before {
        return Err("a viewer was asked for a deleted or changed result".into());
    }
    find::close(app, &window);
    Ok(())
}

/// Stop keeps what was found, sorted; a new search replaces it.
fn stop(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    let window = find::open_find(app)?;
    let w = window.window();
    find::add_root(app, &window, &setup.stop.join("early"), false)?;
    find::add_root(app, &window, &setup.stop.join("late"), false)?;
    find::tick_root(app, &window, &setup.act, false)?;
    find::fill(
        app,
        &window,
        Query {
            content: Some("needle"),
            ..BASE
        },
    )?;
    if find::button_enabled(&window, "Feed to panel")? {
        return Err("Feed to panel is enabled before a search".into());
    }
    // While it runs, the window names the folder it searches, here the one with the long
    // reads; the name goes with the search.
    let late = setup.stop.join("late").display().to_string();
    let idle_lines = naming(&window, &late);
    tools::click_last(w, "Find")?;
    // The first folder's 20 matches are found at once; the second folder's 1 GiB files
    // keep the search reading (matches are published with the next progress sample,
    // which a long file read delays, or at the end).
    let watched = window.clone();
    app.until("the content search to run", move |_| {
        status(&watched).starts_with("Searching")
    });
    app.idle(Duration::from_millis(600));
    let running = status(&window);
    if !running.starts_with("Searching") {
        return Err(format!(
            "the search over two 1 GiB files ended before Stop: {running:?}"
        ));
    }
    if find::button_enabled(&window, "Feed to panel")? {
        return Err("Feed to panel is enabled while the search runs".into());
    }
    let mut problems = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while naming(&window, &late) <= idle_lines {
        if Instant::now() >= deadline || !status(&window).starts_with("Searching") {
            problems.push(format!(
                "while reading the files of {late:?} the window does not name that folder: texts {:?}",
                inspect::visible_texts(w)
            ));
            break;
        }
        app.idle(Duration::from_millis(50));
    }
    let stopped = Instant::now();
    tools::click_last(w, "Stop")?;
    let watched = window.clone();
    app.until("Stop to end the search", move |_| {
        status(&watched).starts_with("Cancelled")
            && inspect::find(watched.window(), AccessibleRole::Button, "Find").is_some()
    });
    let took = stopped.elapsed();
    let text = status(&window);
    let early: Vec<String> = (0..20).map(|index| format!("e{index:02}.txt")).collect();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let rows = all_rows(app, &window)?;
        let names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
        if names == early {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "after Stop the results are {names:?} ({text}), expected e00…e19 in order"
            ));
        }
        app.idle(Duration::from_millis(50));
    }
    if tools::count_before(&text, " found") != Some(20)
        || !find::button_enabled(&window, "Feed to panel")?
    {
        return Err(format!(
            "after Stop: {text:?}, Feed to panel enabled {:?}",
            find::button_enabled(&window, "Feed to panel")
        ));
    }
    if naming(&window, &late) != idle_lines {
        problems.push(format!(
            "after Stop the window still names the searched folder {late:?}"
        ));
    }
    app.screenshot_window(context, &window, "stopped", "Stop kept the 20 results");
    context.note(format!(
        "Stop: {running:?} → {text:?} in {} ms",
        took.as_millis()
    ));
    let (text, rows) = search(
        app,
        &window,
        Query {
            masks: "e0*.txt",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") || rows.len() != 10 {
        return Err(format!(
            "a new search after Stop: {text:?}, {} rows",
            rows.len()
        ));
    }
    let wanted: Vec<&str> = early[..10].iter().map(String::as_str).collect();
    find::wait_names(app, &window, &wanted, "a new search after Stop")?;
    find::close(app, &window);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// How many shown texts name `folder` as the one being read ("Scanning: <folder>").
fn naming(window: &Tool, folder: &str) -> usize {
    inspect::labels(window.window(), AccessibleRole::Text)
        .iter()
        .filter(|label| label.strip_prefix("Scanning: ") == Some(folder))
        .count()
}
