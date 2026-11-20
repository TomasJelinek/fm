//! Find's search folders and windows: several roots added by typing (Enter and OK),
//! Subfolders on and off across them, a listed folder added again (ticked, not
//! duplicated), relative and parent-step paths refused with the entry kept open, a
//! missing folder reported while the others are searched, Remove, the 16-folder limit;
//! Find from the right panel (its folder, and Go to file in that panel), Ctrl+F, the
//! toolbar button and the main menu; at most four Find windows; the window is modeless
//! and keeps its draft; windows search at once, each with its own results, a search run
//! again finds the same, and with two searches running a third waits; Escape closes the
//! Add entry, then the help, then the window; Close during a running search.
use crate::{
    Context,
    app::{App, Config},
    input, inspect, panel,
    scenarios::find_support::{
        self as find, BASE, Findings, Query, open_find, open_on, relative, search, set,
    },
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::{Key, WindowAdapter};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const ABSOLUTE: &str = "Find roots must be absolute native paths without NUL or parent steps.";
const SIXTEEN: &str = "Find requires 1 to 16 absolute native directory roots.";

struct Tree {
    roots: PathBuf,
    a: PathBuf,
    b: PathBuf,
    c: PathBuf,
}

fn tree(work: &Path) -> Result<Tree, String> {
    let roots = work.join("roots");
    let now = SystemTime::now();
    for name in ["a", "b", "c"] {
        tools::write_at(
            &roots.join(name).join(format!("{name}1.txt")),
            name.as_bytes(),
            now,
        )?;
        tools::write_at(
            &roots.join(name).join("deep").join(format!("{name}2.txt")),
            name.as_bytes(),
            now,
        )?;
    }
    for index in 0..16 {
        let name = format!("r{index:02}");
        tools::write_at(
            &roots.join("extra").join(&name).join(format!("{name}.txt")),
            name.as_bytes(),
            now,
        )?;
    }
    std::fs::create_dir_all(roots.join("slow")).map_err(|error| error.to_string())?;
    tools::sparse(&roots.join("slow/huge.txt"), 1 << 30)?;
    Ok(Tree {
        a: roots.join("a"),
        b: roots.join("b"),
        c: roots.join("c"),
        roots,
    })
}

fn txt(app: &mut App, window: &Tool, subfolders: bool) -> Result<(String, Vec<find::Row>), String> {
    search(
        app,
        window,
        Query {
            masks: "*.txt",
            subfolders,
            ..BASE
        },
    )
}

pub fn roots_and_windows(context: &mut Context) -> Result<(), String> {
    let tree = tree(&context.work)?;
    let mut app = App::start(Config::new(&tree.a, &tree.b));
    let mut findings = Findings::default();
    findings.check("several roots", several_roots(&mut app, context, &tree));
    find::reset(&mut app);
    findings.check("the right panel", right_panel(&mut app, &tree));
    find::reset(&mut app);
    findings.check("ways to open Find", openers(&mut app, &tree));
    find::reset(&mut app);
    findings.check("four windows", four_windows(&mut app, context));
    find::reset(&mut app);
    findings.check(
        "two searches at once",
        two_windows(&mut app, context, &tree),
    );
    find::reset(&mut app);
    findings.check("modeless window", modeless(&mut app, &tree));
    find::reset(&mut app);
    findings.check("Escape order and Close", escape_and_close(&mut app, &tree));
    find::reset(&mut app);
    app.shutdown()?;
    context.note("Several roots (typed with Enter and OK, ticked again, refused, missing, removed, 16-root limit) with Subfolders on and off; Find from the right panel with Go to file there; Ctrl+F, toolbar and main menu; four windows at most; a modeless window keeps its draft; Escape order; Close during a search");
    findings.into_result()
}

fn several_roots(app: &mut App, context: &mut Context, tree: &Tree) -> Result<(), String> {
    let window = open_find(app)?;
    let w = window.window();
    let picker = tools::control(w, AccessibleRole::Combobox, "Search folders")?;
    if tools::value(&picker) != tree.a.display().to_string() {
        return Err(format!(
            "Search folders starts at {:?}, not the panel's folder",
            tools::value(&picker)
        ));
    }
    find::add_root(app, &window, &tree.b, false)?;
    find::add_root(app, &window, &tree.c, true)?;
    if find::folders_label(&window) != "Search folders (3)" {
        return Err(format!(
            "after two Adds the label reads {:?}",
            find::folders_label(&window)
        ));
    }
    if tools::has(w, AccessibleRole::TextInput, "Folder to add") {
        return Err("an accepted Add left its entry open".into());
    }
    let (text, rows) = txt(app, &window, true)?;
    let all = set(&[
        "a/a1.txt",
        "a/deep/a2.txt",
        "b/b1.txt",
        "b/deep/b2.txt",
        "c/c1.txt",
        "c/deep/c2.txt",
    ]);
    if !text.starts_with("Completed") || relative(&rows, &tree.roots) != all {
        return Err(format!(
            "three roots with Subfolders: {text:?}, {:?}",
            relative(&rows, &tree.roots)
        ));
    }
    app.screenshot_window(context, &window, "three-roots", "*.txt in three folders");
    let (text, rows) = txt(app, &window, false)?;
    if !text.starts_with("Completed")
        || relative(&rows, &tree.roots) != set(&["a/a1.txt", "b/b1.txt", "c/c1.txt"])
    {
        return Err(format!(
            "three roots without Subfolders: {text:?}, {:?}",
            relative(&rows, &tree.roots)
        ));
    }
    // A listed folder added again is ticked, not listed twice.
    find::tick_root(app, &window, &tree.b, false)?;
    if find::folders_label(&window) != "Search folders (2 of 3)" {
        return Err(format!(
            "after unticking b: {:?}",
            find::folders_label(&window)
        ));
    }
    find::add_root(app, &window, &tree.b, false)?;
    let listed = find::root_boxes(app, &window)?;
    let wanted: Vec<(String, bool)> = [&tree.a, &tree.b, &tree.c]
        .iter()
        .map(|path| (path.display().to_string(), true))
        .collect();
    if listed != wanted || find::folders_label(&window) != "Search folders (3)" {
        return Err(format!(
            "adding a listed folder again: {listed:?}, label {:?}",
            find::folders_label(&window)
        ));
    }
    // Refused paths keep the entry open with the reason.
    for typed in [
        PathBuf::from("relative/folder"),
        tree.a.join("..").join("b"),
    ] {
        find::add_root(app, &window, &typed, false)?;
        let error = find::error_line(&window);
        if error.as_deref() != Some(ABSOLUTE)
            || !tools::has(w, AccessibleRole::TextInput, "Folder to add")
        {
            return Err(format!(
                "adding {} shows {error:?} (entry open: {})",
                typed.display(),
                tools::has(w, AccessibleRole::TextInput, "Folder to add")
            ));
        }
    }
    app.screenshot_window(
        context,
        &window,
        "relative-refused",
        "A path with a parent step is refused; the entry stays open",
    );
    input::key(w, Key::Escape);
    app.settle();
    if tools::has(w, AccessibleRole::TextInput, "Folder to add") || !w.is_visible() {
        return Err("Escape did not close only the Add entry".into());
    }
    // A missing folder: the others are still searched, the search is Incomplete and
    // the details name it.
    let missing = tree.roots.join("missing");
    find::add_root(app, &window, &missing, false)?;
    let (text, rows) = txt(app, &window, true)?;
    if !text.starts_with("Incomplete") || relative(&rows, &tree.roots) != all {
        return Err(format!(
            "with a missing folder: {text:?}, {:?}",
            relative(&rows, &tree.roots)
        ));
    }
    let texts = find::details(app, &window)?;
    if !find::mentions(&texts, &missing.display().to_string()) {
        return Err(format!(
            "the details do not name the missing folder: {texts:?}"
        ));
    }
    find::remove_root(app, &window, &missing)?;
    if find::folders_label(&window) != "Search folders (3)" {
        return Err(format!("Remove left {:?}", find::folders_label(&window)));
    }
    // Sixteen folders at most.
    let extra: Vec<PathBuf> = (0..14)
        .map(|index| tree.roots.join("extra").join(format!("r{index:02}")))
        .collect();
    for path in &extra[..13] {
        find::add_root(app, &window, path, false)?;
    }
    if find::folders_label(&window) != "Search folders (16)" {
        return Err(format!("16 folders: {:?}", find::folders_label(&window)));
    }
    find::add_root(app, &window, &extra[13], false)?;
    if find::error_line(&window).as_deref() != Some(SIXTEEN)
        || find::folders_label(&window) != "Search folders (16)"
    {
        return Err(format!(
            "a 17th folder: error {:?}, label {:?}",
            find::error_line(&window),
            find::folders_label(&window)
        ));
    }
    input::key(w, Key::Escape);
    app.settle();
    let (text, rows) = txt(app, &window, true)?;
    let mut expected = all.clone();
    for index in 0..13 {
        expected.insert(format!("extra/r{index:02}/r{index:02}.txt"));
    }
    if !text.starts_with("Completed") || relative(&rows, &tree.roots) != expected {
        return Err(format!("sixteen folders: {text:?}, {} rows", rows.len()));
    }
    app.screenshot_window(
        context,
        &window,
        "sixteen-roots",
        "*.txt in sixteen folders",
    );
    find::close(app, &window);
    Ok(())
}

/// Find from the right panel searches its folder; Go to file moves that panel.
fn right_panel(app: &mut App, tree: &Tree) -> Result<(), String> {
    let window = open_on(app, PaneId::Right)?;
    let picker = tools::control(window.window(), AccessibleRole::Combobox, "Search folders")?;
    if tools::value(&picker) != tree.b.display().to_string() {
        return Err(format!(
            "Find from the right panel starts at {:?}",
            tools::value(&picker)
        ));
    }
    let (text, rows) = txt(app, &window, true)?;
    if !text.starts_with("Completed")
        || relative(&rows, &tree.roots) != set(&["b/b1.txt", "b/deep/b2.txt"])
    {
        return Err(format!(
            "*.txt from the right panel: {text:?}, {:?}",
            relative(&rows, &tree.roots)
        ));
    }
    find::select_result(app, &window, "b2.txt", None)?;
    tools::click_last(window.window(), "Go to file")?;
    find::wait_cursor(app, PaneId::Right, &tree.b.join("deep"), "b2.txt");
    if app.location(PaneId::Left).as_deref() != Some(tree.a.as_path()) {
        return Err(format!(
            "Go to file from the right panel's Find moved the left panel to {:?}",
            app.location(PaneId::Left)
        ));
    }
    find::close(app, &window);
    panel::focus_list(app, PaneId::Right)?;
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, PaneId::Right, &tree.b);
    Ok(())
}

/// Ctrl+F, the toolbar's Find button and F10's Find files… each open a Find window on
/// the active panel's folder.
fn openers(app: &mut App, tree: &Tree) -> Result<(), String> {
    let check = |app: &mut App, how: &str, before: usize| -> Result<(), String> {
        let window = tools::wait_tool(app, &format!("{how} to open Find"), before, find::SIZE);
        if tools::title(&window) != "Find files · Midnight Explorer" {
            return Err(format!("{how} opened {:?}", tools::title(&window)));
        }
        let picker = tools::control(window.window(), AccessibleRole::Combobox, "Search folders")?;
        if tools::value(&picker) != tree.a.display().to_string() {
            return Err(format!(
                "{how}: Search folders shows {:?}",
                tools::value(&picker)
            ));
        }
        find::close(app, &window);
        Ok(())
    };
    panel::focus_list(app, PaneId::Left)?;
    let before = app.tool_windows().len();
    input::chord(app.slint(), &[Key::Control], "f");
    check(app, "Ctrl+F", before)?;
    panel::focus_list(app, PaneId::Left)?;
    let buttons = find::main_buttons(app, "Find files and text");
    let (_, button) = buttons
        .first()
        .ok_or("the toolbar has no Find files and text button")?;
    let before = app.tool_windows().len();
    input::click_at(app.slint(), inspect::center(button));
    check(app, "the toolbar's Find button", before)?;
    panel::focus_list(app, PaneId::Left)?;
    let before = app.tool_windows().len();
    find::menu(app, "Tools", "Find files…")?;
    check(app, "F10 Find files…", before)
}

/// Four Find windows at most: a fifth is refused with a notice until one closes.
fn four_windows(app: &mut App, context: &mut Context) -> Result<(), String> {
    let mut windows = Vec::new();
    for _ in 0..4 {
        windows.push(open_find(app)?);
    }
    input::chord(app.slint(), &[Key::Alt], Key::F7);
    let notice = find::wait_notice(app, "a fifth Find window")?;
    app.screenshot(context, "fifth-find", "A fifth Find window is refused");
    find::dismiss_notice(app);
    if notice
        != (
            "Find files".to_owned(),
            "Close one of the 4 open Find windows first.".to_owned(),
        )
        || app.tool_windows().len() != 4
    {
        return Err(format!(
            "a fifth Alt+F7: notice {notice:?}, {} windows",
            app.tool_windows().len()
        ));
    }
    let first = windows.remove(0);
    find::close(app, &first);
    windows.push(open_find(app)?);
    for window in &windows {
        find::close(app, window);
    }
    Ok(())
}

/// Start a content search for "needle" over the 1 GiB file in its own window.
fn long_search(app: &mut App, tree: &Tree) -> Result<Tool, String> {
    let window = open_find(app)?;
    find::add_root(app, &window, &tree.roots.join("slow"), false)?;
    find::tick_root(app, &window, &tree.a, false)?;
    find::fill(
        app,
        &window,
        Query {
            content: Some("needle"),
            ..BASE
        },
    )?;
    tools::click_last(window.window(), "Find")?;
    let watched = window.clone();
    app.until("the long search to run", move |_| {
        find::status(&watched).starts_with("Searching")
    });
    Ok(window)
}

/// Windows search at once: a long content search in one, a name search (run twice with
/// the kept draft) in another from the other panel; each keeps its own results, Go to
/// file moves its own panel, Stop ends only its own search. At most two searches walk
/// at once: with two long searches running, a third waits until one stops.
fn two_windows(app: &mut App, context: &mut Context, tree: &Tree) -> Result<(), String> {
    let slow = long_search(app, tree)?;
    let quick = open_on(app, PaneId::Right)?;
    let expected = set(&["b/b1.txt", "b/deep/b2.txt"]);
    let (text, rows) = txt(app, &quick, true)?;
    if !text.starts_with("Completed") || relative(&rows, &tree.roots) != expected {
        return Err(format!(
            "the second window's search: {text:?}, {:?}",
            relative(&rows, &tree.roots)
        ));
    }
    let again = find::run(app, &quick)?;
    let rows = find::all_rows(app, &quick)?;
    if !again.starts_with("Completed") || relative(&rows, &tree.roots) != expected {
        return Err(format!(
            "running the same search again: {again:?}, {:?}",
            relative(&rows, &tree.roots)
        ));
    }
    let running = find::status(&slow);
    if !running.starts_with("Searching") {
        return Err(format!(
            "the first window's search ended while the second searched: {running:?}"
        ));
    }
    // A second long search: both workers are busy, so a third search waits.
    let slow2 = long_search(app, tree)?;
    let waiting = open_on(app, PaneId::Right)?;
    find::fill(
        app,
        &waiting,
        Query {
            masks: "*.txt",
            ..BASE
        },
    )?;
    tools::click_last(waiting.window(), "Find")?;
    app.idle(Duration::from_millis(1000));
    let queued = find::status(&waiting);
    app.screenshot_window(
        context,
        &waiting,
        "queued-search",
        "A third search waits while two run",
    );
    if !queued.starts_with("Searching") || find::result_count(&waiting) != 0 {
        return Err(format!(
            "with two long searches running, a third reads {queued:?} with {} rows",
            find::result_count(&waiting)
        ));
    }
    tools::click_last(slow.window(), "Stop")?;
    let watched = waiting.clone();
    app.until(
        "the waiting search to run once a worker is free",
        move |_| find::status(&watched).starts_with("Completed"),
    );
    let rows = find::all_rows(app, &waiting)?;
    if relative(&rows, &tree.roots) != expected {
        return Err(format!(
            "the third search found {:?}",
            relative(&rows, &tree.roots)
        ));
    }
    find::select_result(app, &quick, "b1.txt", None)?;
    tools::click_last(quick.window(), "Go to file")?;
    find::wait_cursor(app, PaneId::Right, &tree.b, "b1.txt");
    tools::click_last(slow2.window(), "Stop")?;
    for window in [&slow, &slow2] {
        let watched = window.clone();
        app.until("the long searches to stop", move |_| {
            find::status(&watched).starts_with("Cancelled")
        });
    }
    let rows = find::all_rows(app, &quick)?;
    if relative(&rows, &tree.roots) != expected || !find::status(&quick).starts_with("Completed") {
        return Err("Stop in one window changed another window's results".into());
    }
    for window in [&slow, &quick, &slow2, &waiting] {
        find::close(app, window);
    }
    Ok(())
}

/// The panels stay usable while Find is open, and the form keeps its draft.
fn modeless(app: &mut App, tree: &Tree) -> Result<(), String> {
    let window = open_find(app)?;
    input::fill(window.window(), "File name masks", "kept-draft*.txt")?;
    app.settle();
    panel::enter(app, PaneId::Left, "deep")?;
    panel::leave(app, PaneId::Left)?;
    let masks = tools::control(
        window.window(),
        AccessibleRole::TextInput,
        "File name masks",
    )?;
    if !window.window().is_visible() || tools::value(&masks) != "kept-draft*.txt" {
        return Err(format!(
            "after using the panel the Find window is visible={} with masks {:?}",
            window.window().is_visible(),
            tools::value(&masks)
        ));
    }
    if app.location(PaneId::Left).as_deref() != Some(tree.a.as_path()) {
        return Err("the left panel did not return to its folder".into());
    }
    find::close(app, &window);
    Ok(())
}

/// Escape closes the Add entry, then the help, then the window; Close ends a running
/// search with its window.
fn escape_and_close(app: &mut App, tree: &Tree) -> Result<(), String> {
    let window = open_find(app)?;
    let w = window.window();
    input::click_button(w, "Add")?;
    app.settle();
    input::key(w, Key::Escape);
    app.settle();
    if tools::has(w, AccessibleRole::TextInput, "Folder to add") || !w.is_visible() {
        return Err("Escape did not close the Add entry first".into());
    }
    input::key(w, Key::F1);
    app.settle();
    if !inspect::visible_texts(w)
        .iter()
        .any(|text| text == "Masks syntax")
    {
        return Err("F1 showed no help".into());
    }
    input::key(w, Key::Escape);
    app.settle();
    if inspect::visible_texts(w)
        .iter()
        .any(|text| text == "Masks syntax")
        || !w.is_visible()
    {
        return Err("Escape did not close the help first".into());
    }
    input::key(w, Key::Escape);
    let watched = window.clone();
    app.until("Escape to close the window", move |_| {
        !watched.window().is_visible()
    });

    // Close while a content search reads a 1 GiB file.
    let window = open_find(app)?;
    let w = window.window();
    find::add_root(app, &window, &tree.roots.join("slow"), false)?;
    find::fill(
        app,
        &window,
        Query {
            content: Some("needle"),
            ..BASE
        },
    )?;
    tools::click_last(w, "Find")?;
    let watched = window.clone();
    app.until("the content search to run", move |_| {
        find::status(&watched).starts_with("Searching")
    });
    tools::click_last(w, "Close")?;
    let watched = window.clone();
    app.until("Close to close the searching window", move |_| {
        !watched.window().is_visible()
    });
    // The session is gone: four new windows still fit.
    let mut windows = Vec::new();
    for _ in 0..4 {
        windows.push(open_find(app)?);
    }
    for window in &windows {
        find::close(app, window);
    }
    app.idle(Duration::from_millis(100));
    let left: BTreeSet<String> = app.names(PaneId::Left).into_iter().collect();
    if !left.contains("a1.txt") {
        return Err(format!("the left panel lists {left:?}"));
    }
    Ok(())
}
