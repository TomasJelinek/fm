//! Feed to panel in depth: the results become a list over the panel that opened the
//! Find, in the Find window's order, with its title and status; the cursor keys,
//! Insert, Space, the bulk selection keys (each with its configured meaning, as in a
//! panel), keypad *, Ctrl+click and Shift+click, Alt+Up/Down between marks; Enter and a
//! double click go to a file or open a folder; Backspace, Back to folder, Alt+Left,
//! Ctrl+Backspace and Ctrl+G leave the list (the folder underneath unchanged), Tab keeps
//! it; F3, Alt+F3 and F4 act on the focused file and explain a folder; F5 copies, F6
//! moves (the list keeps the old row), F8 only asks, Shift+F8 deletes a marked folder
//! with the result inside it; Alt+Enter summarizes the marks; every action the list
//! refuses (keys, keypad, toolbar, function bar and main menu) explains itself and
//! leaves the folder underneath untouched; a Find from the right panel feeds that one.
//! The Find window's selection arrives as the list's marks (Ctrl+C and Ctrl+X put them on
//! the clipboard, F5's dialog reads "Copy 3 files (11 B) to:"), and the list's cursor starts on
//! the Find's cursor result, on its first row when the Find window had none.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as files, TransferSpec},
    input,
    options_support::SettingsStore,
    panel,
    scenarios::find_support::{
        self as find, BASE, Findings, Launched, Query, all_rows, feed_cursor, feed_marked,
        feed_names, feed_rows, feed_view, record_launches, search,
    },
    tools_support as tools,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{
    EditorPreferences, PaneId, UserCommand, UserCommandId, UserCommandInvocation, UserCommandLaunch,
};
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter},
};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

const LEFT: PaneId = PaneId::Left;
const REFUSED: &str = "Not available in search results";
/// The custom command on the toolbar (its button is labelled with its name).
const COMMAND: &str = "Feed marker";
const COMMAND_ID: &str = "feed-marker";
/// The results of `*.txt;sub` sorted by Name descending, and ascending.
const ORDER: [&str; 6] = ["sub", "g.txt", "f.txt", "d.txt", "b.txt", "a.txt"];
const ASCENDING: [&str; 6] = ["a.txt", "b.txt", "d.txt", "f.txt", "g.txt", "sub"];

struct Setup {
    root: PathBuf,
    target: PathBuf,
    edited: PathBuf,
}

fn setup(work: &Path) -> Result<Setup, String> {
    let root = work.join("feed");
    let now = SystemTime::now();
    for (path, data) in [
        ("docs/a.txt", "a"),
        ("docs/b.txt", "bb"),
        ("docs/c.log", "ccc"),
        ("src/d.txt", "dddd"),
        ("src/sub/g.txt", "ggggggg"),
        ("f.txt", "fffff"),
    ] {
        tools::write_at(&root.join(path), data.as_bytes(), now)?;
    }
    let target = work.join("feed-target");
    let edited = work.join("edited");
    for folder in [&target, &edited] {
        fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    Ok(Setup {
        root,
        target,
        edited,
    })
}

/// Search `*.txt;sub` from `pane` (Name descending when asked), feed the results to
/// that panel and wait for the list; returns the Find window's order.
fn feed(
    app: &mut App,
    pane: PaneId,
    ascending: &[&str],
    descending: bool,
) -> Result<Vec<String>, String> {
    let window = find::open_on(app, pane)?;
    let (text, rows) = search(
        app,
        &window,
        Query {
            masks: "*.txt;sub",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") || rows.len() != ascending.len() {
        let w = window.window();
        let state = format!(
            "masks {:?}, Subfolders {:?}, {}, folder {:?}, panel at {:?}",
            tools::control(w, AccessibleRole::TextInput, "File name masks")
                .map(|item| tools::value(&item)),
            tools::is_checked(w, "Subfolders"),
            find::folders_label(&window),
            tools::control(w, AccessibleRole::Combobox, "Search folders")
                .map(|item| tools::value(&item)),
            app.location(pane)
        );
        return Err(format!(
            "*.txt;sub ended {text:?} with {} rows {:?}; {state}",
            rows.len(),
            rows.iter().map(|row| row.path()).collect::<Vec<_>>()
        ));
    }
    let rows = find::wait_names(app, &window, ascending, "Name ascending")?;
    let mut order: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    if descending {
        input::click_button(window.window(), "Sort by Name")?;
        let mut wanted = order.clone();
        wanted.reverse();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let names: Vec<String> = all_rows(app, &window)?
                .iter()
                .map(|row| row.name.clone())
                .collect();
            if names == wanted {
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!("Name descending shows {names:?}"));
            }
            app.idle(Duration::from_millis(50));
        }
        order = wanted;
    }
    tools::click_last(window.window(), "Feed to panel")?;
    app.until("the results list to open", move |app| {
        feed_view(app, pane).open
    });
    let watched = window.clone();
    app.until("Feed to panel to close the Find window", move |_| {
        !watched.window().is_visible()
    });
    panel::wait_input(app);
    Ok(order)
}

/// The list as `list` leaves it (Name descending): fed again when an earlier check
/// failed before feeding or left it, so one failure does not hide the later checks.
fn ensure_fed(app: &mut App) -> Result<(), String> {
    if !feed_view(app, LEFT).open || feed_names(app, LEFT) != ORDER {
        if feed_view(app, LEFT).open {
            key(app, &[], Key::Backspace);
        }
        feed(app, LEFT, &ASCENDING, true)?;
    }
    Ok(())
}

/// Close what a check left open and put the left panel back in the tree's root, so each
/// check starts the same whatever the one before it did.
fn between(app: &mut App, root: &Path) -> Result<(), String> {
    find::reset(app);
    if app.location(LEFT).as_deref() == Some(root) {
        return Ok(());
    }
    if feed_view(app, LEFT).open {
        panel::focus_list(app, LEFT)?;
        input::key(app.slint(), Key::Backspace);
        app.until("the fed list to close", |app| !feed_view(app, LEFT).open);
    }
    if app.location(LEFT).as_deref() != Some(root) {
        panel::focus_list(app, LEFT)?;
        input::chord(app.slint(), &[Key::Control], "g");
        input::replace_text(app.slint(), &root.display().to_string());
        input::key(app.slint(), Key::Return);
        panel::wait_location(app, LEFT, root);
    }
    Ok(())
}

fn key(app: &mut App, modifiers: &[Key], text: impl Into<slint::SharedString>) {
    input::chord(app.slint(), modifiers, text);
    app.settle();
}

/// Mark exactly `wanted` with Home, Insert and Down.
fn set_marks(app: &mut App, wanted: &[&str]) -> Result<(), String> {
    key(app, &[], Key::Home);
    for row in feed_rows(app, LEFT) {
        let name = row.name.to_string();
        let mark = row.selected != wanted.contains(&name.as_str());
        key(app, &[], if mark { Key::Insert } else { Key::DownArrow });
    }
    let marked = feed_marked(app, LEFT);
    if marked != find::set(wanted) {
        return Err(format!("marking {wanted:?} marked {marked:?}"));
    }
    key(app, &[], Key::Home);
    Ok(())
}

/// Make Select by mask remember `folders` for Include folders: Select files (Num +) in
/// the right panel with a mask that marks nothing there, then back to the fed list.
fn mask_folders(app: &mut App, folders: bool) -> Result<(), String> {
    let focused = |app: &App| app.window.get_focused_pane();
    key(app, &[], Key::Tab);
    if focused(app) != PaneId::Right.index() as i32 {
        return Err(format!(
            "Tab focused pane {}, not the right one",
            focused(app)
        ));
    }
    if !find::keypad(app, "NumpadAdd") {
        return Err("Num + was not handled in the right panel".into());
    }
    super::files_selection::apply_mask(app, "*.no-such-extension", folders)?;
    key(app, &[], Key::Tab);
    // A fed list takes the keyboard in place of its panel's file list.
    let feed = app.window.get_feed_focused_pane();
    if feed != LEFT.index() as i32 || !feed_view(app, LEFT).open {
        return Err(format!(
            "Tab back focused pane {} (fed list {feed}); the left panel's results open: {}",
            focused(app),
            feed_view(app, LEFT).open
        ));
    }
    Ok(())
}

pub fn feed_panel(context: &mut Context) -> Result<(), String> {
    let setup = setup(&context.work)?;
    let cp = crate::tool::program(context, "cp")?;
    let touch = crate::tool::program(context, "touch")?;
    let edited = setup.edited.display().to_string();
    let marker = setup
        .edited
        .join("custom-command-ran")
        .display()
        .to_string();
    let store = SettingsStore::seeded(move |preferences| {
        preferences.editor = EditorPreferences {
            executable: Some(cp),
            arguments: vec!["{file}".into(), edited],
            rules: Vec::new(),
        };
        // A harmless custom command on the toolbar: it would touch a marker file.
        preferences.user_commands = std::sync::Arc::new(vec![UserCommand {
            output: Default::default(),
            id: UserCommandId::new(COMMAND_ID).expect("a valid command ID"),
            name: COMMAND.into(),
            enabled: true,
            toolbar_label: Some(COMMAND.into()),
            icon: None,
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable: touch,
                arguments: vec![marker],
            },
        }]);
    });
    let mut config = Config::new(&setup.root, &setup.target);
    config.settings = Some(store);
    config.editor = true;
    config.user_commands = true;
    // Ctrl+C and Ctrl+X in the list reach the in-process clipboard.
    config.clipboard = true;
    let mut app = App::start(config);
    let launched = record_launches(&app);
    let mut findings = Findings::default();
    findings.check("the list", list(&mut app, context, &setup));
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check(
        "the Find selection carried to the list",
        selection_carried(&mut app, context, &setup),
    );
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check("marks", marks(&mut app));
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check("refusals", refusals(&mut app, context, &setup));
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check(
        "view, edit and properties",
        view_edit(&mut app, &setup, &launched),
    );
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check("leaving the list", leaving(&mut app, &setup));
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check("going to results", going(&mut app, &setup));
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check("file operations", operations(&mut app, context, &setup));
    findings.check("putting the panel back", between(&mut app, &setup.root));
    findings.check("the right panel", right_panel(&mut app, &setup));
    findings.check("putting the panel back", between(&mut app, &setup.root));
    app.shutdown()?;
    context.note("Feed to panel: order, title and status; keys, marks, bulk keys, keypad *, Ctrl/Shift+click, Alt+Up/Down; Enter, double click; Backspace, Back to folder, Alt+Left, Ctrl+Backspace, Ctrl+G, Tab; F3, Alt+F3, F4, Alt+Enter; F5, F6, F8 asked, Shift+F8; refusals by key, keypad, toolbar, function bar and menu; the right panel");
    findings.into_result()
}

/// The fed list: the Find window's order, its title and status, and the cursor keys.
fn list(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    let order = feed(app, LEFT, &ASCENDING, true)?;
    if order != ORDER {
        return Err(format!("the Find window's order was {order:?}"));
    }
    if feed_names(app, LEFT) != ORDER {
        return Err(format!(
            "the list shows {:?}, the Find window {ORDER:?}",
            feed_names(app, LEFT)
        ));
    }
    let view = feed_view(app, LEFT);
    let title = format!("Search results (6) · {}", setup.root.display());
    let status = "0 of 6 selected · Enter: go to file · Backspace: back to folder";
    if view.title.as_str() != title || view.status.as_str() != status {
        return Err(format!(
            "the list is titled {:?} with status {:?}",
            view.title, view.status
        ));
    }
    let folders: Vec<String> = feed_rows(app, LEFT)
        .iter()
        .map(|row| row.folder.to_string())
        .collect();
    // The folders as the platform spells them (`\` between names on Windows).
    let expected: Vec<String> = ["src", "src/sub", "", "src", "docs", "docs"]
        .iter()
        .map(|folder| {
            folder
                .split('/')
                .filter(|name| !name.is_empty())
                .fold(setup.root.clone(), |path, name| path.join(name))
                .display()
                .to_string()
        })
        .collect();
    if folders != expected {
        return Err(format!("the Folder column shows {folders:?}"));
    }
    if !feed_rows(app, LEFT)[0].directory || feed_rows(app, LEFT)[0].size.as_str() != "<DIR>" {
        return Err("the folder result is not shown as a folder".into());
    }
    app.screenshot(
        context,
        "fed",
        "Six results fed to the left panel, Name descending",
    );
    // No result was chosen in the Find window, so the list's cursor starts on its first
    // row (the Find's cursor result when there is one: `selection_carried`).
    if feed_cursor(app, LEFT).as_deref() != Some(ORDER[0]) {
        return Err(format!(
            "with no result chosen in the Find window, the list's cursor starts on {:?}, not its first row {}",
            feed_cursor(app, LEFT),
            ORDER[0]
        ));
    }
    for (keys, wanted) in [
        (Key::DownArrow, "g.txt"),
        (Key::End, "a.txt"),
        (Key::UpArrow, "b.txt"),
        (Key::Home, "sub"),
        (Key::PageDown, "a.txt"),
        (Key::PageUp, "sub"),
    ] {
        key(app, &[], keys);
        if feed_cursor(app, LEFT).as_deref() != Some(wanted) {
            return Err(format!(
                "{keys:?} put the cursor on {:?}, expected {wanted}",
                feed_cursor(app, LEFT)
            ));
        }
    }
    // Alt+Right (forward) and Ctrl+R (refresh) do nothing in the list.
    for (modifiers, text, how) in [
        (
            &[Key::Alt][..],
            slint::SharedString::from(Key::RightArrow),
            "Alt+Right",
        ),
        (&[Key::Control][..], "r".into(), "Ctrl+R"),
    ] {
        key(app, modifiers, text);
        app.idle(Duration::from_millis(150));
        if !feed_view(app, LEFT).open
            || feed_names(app, LEFT) != ORDER
            || tools::shown_notice(app).is_some()
        {
            return Err(format!(
                "{how} in the list: open {}, rows {:?}, notice {:?}",
                feed_view(app, LEFT).open,
                feed_names(app, LEFT),
                tools::shown_notice(app)
            ));
        }
    }
    Ok(())
}

fn marks(app: &mut App) -> Result<(), String> {
    ensure_fed(app)?;
    let mut problems: Vec<String> = Vec::new();
    let files = ["g.txt", "f.txt", "d.txt", "b.txt", "a.txt"];
    // Insert marks and moves; Space marks in place; the status counts and sums.
    key(app, &[], Key::Home);
    key(app, &[], Key::Insert);
    key(app, &[], " ");
    let status = feed_view(app, LEFT).status.to_string();
    if feed_marked(app, LEFT) != find::set(&["sub", "g.txt"])
        || feed_cursor(app, LEFT).as_deref() != Some("g.txt")
        || !status.starts_with("2 of 6 selected · ")
        || status.starts_with("2 of 6 selected · Enter")
    {
        return Err(format!(
            "Insert then Space: marks {:?}, cursor {:?}, status {status:?}",
            feed_marked(app, LEFT),
            feed_cursor(app, LEFT)
        ));
    }
    // The bulk selection keys, each from a known state.
    let mut bulk = |app: &mut App,
                    from: &[&str],
                    modifiers: &[Key],
                    text: &str,
                    wanted: &[&str],
                    what: &str|
     -> Result<BTreeSet<String>, String> {
        set_marks(app, from)?;
        key(app, modifiers, text);
        let marked = feed_marked(app, LEFT);
        if marked != find::set(wanted) {
            problems.push(format!(
                "{what} from {from:?} marked {marked:?}, expected {wanted:?}"
            ));
        }
        Ok(marked)
    };
    bulk(
        app,
        &[],
        &[Key::Control, Key::Shift],
        "a",
        &ORDER,
        "Ctrl+Shift+A (select files and folders)",
    )?;
    bulk(
        app,
        &[],
        &[Key::Control],
        "a",
        &files,
        "Ctrl+A (select files only)",
    )?;
    bulk(
        app,
        &["sub"],
        &[Key::Control],
        "a",
        &files,
        "finding FIND-FEED-SELECT-FILES: Ctrl+A (select files only, which in a panel replaces the selection with the files)",
    )?;
    bulk(app, &ORDER, &[Key::Control], "d", &[], "Ctrl+D (deselect)")?;
    let inverted = bulk(
        app,
        &["sub", "a.txt"],
        &[Key::Control],
        "i",
        &["sub", "g.txt", "f.txt", "d.txt", "b.txt"],
        "finding FIND-FEED-INVERT: Ctrl+I (invert files only, which in a panel keeps folder marks)",
    )?;
    bulk(
        app,
        &["sub", "a.txt"],
        &[Key::Control, Key::Shift],
        "i",
        &["g.txt", "f.txt", "d.txt", "b.txt"],
        "Ctrl+Shift+I (invert files and folders)",
    )?;
    // Keypad * inverts the marks as in a panel: like Ctrl+I while Select by mask leaves
    // folders out (the default) ...
    set_marks(app, &["sub", "a.txt"])?;
    if !find::keypad(app, "NumpadMultiply") {
        problems.push("keypad * was not handled by the list".into());
    }
    if feed_marked(app, LEFT) != inverted {
        problems.push(format!(
            "keypad * marked {:?}, Ctrl+I {inverted:?}",
            feed_marked(app, LEFT)
        ));
    }
    // ... and folders too once Select by mask last had Include folders (chosen in the
    // right panel's Select files, Num +, with a mask that marks nothing there).
    match mask_folders(app, true) {
        Err(error) => problems.push(format!(
            "Select by mask with Include folders in the right panel: {error}"
        )),
        Ok(()) => {
            set_marks(app, &["sub", "a.txt"])?;
            if !find::keypad(app, "NumpadMultiply") {
                problems.push("keypad * was not handled by the list".into());
            }
            let wanted = find::set(&["g.txt", "f.txt", "d.txt", "b.txt"]);
            if feed_marked(app, LEFT) != wanted {
                problems.push(format!(
                    "after Select by mask with Include folders, keypad * marked {:?}, not \
                     {wanted:?} (folders inverted too, as in a panel)",
                    feed_marked(app, LEFT)
                ));
            }
        }
    }
    if let Err(error) = mask_folders(app, false) {
        problems.push(format!(
            "Select by mask without Include folders in the right panel: {error}"
        ));
    }
    // Ctrl+click toggles a row; Shift+click marks the range from the cursor.
    set_marks(app, &[])?;
    let rect = |app: &App, name: &str| {
        find::feed_row_rect(app, name)
            .map(|(x, y, w, h)| LogicalPosition::new(x + w.min(120.0) / 2.0, y + h / 2.0))
    };
    let d = rect(app, "d.txt").ok_or("the d.txt row is not shown")?;
    input::click_with(app.slint(), &[Key::Control], d);
    app.settle();
    let a = rect(app, "a.txt").ok_or("the a.txt row is not shown")?;
    input::click_with(app.slint(), &[Key::Shift], a);
    app.settle();
    if feed_marked(app, LEFT) != find::set(&["d.txt", "b.txt", "a.txt"])
        || feed_cursor(app, LEFT).as_deref() != Some("a.txt")
    {
        problems.push(format!(
            "Ctrl+click d.txt then Shift+click a.txt: marks {:?}, cursor {:?}",
            feed_marked(app, LEFT),
            feed_cursor(app, LEFT)
        ));
    }
    // Alt+Down and Alt+Up go to the next and previous marked row.
    set_marks(app, &["g.txt", "b.txt"])?;
    for (arrow, wanted) in [
        (Key::DownArrow, "g.txt"),
        (Key::DownArrow, "b.txt"),
        (Key::UpArrow, "g.txt"),
    ] {
        key(app, &[Key::Alt], arrow);
        if feed_cursor(app, LEFT).as_deref() != Some(wanted) {
            problems.push(format!(
                "Alt+{arrow:?} put the cursor on {:?}, expected {wanted}",
                feed_cursor(app, LEFT)
            ));
        }
    }
    set_marks(app, &[])?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// The refusal notice of `label` after `how`; the list stays open and nothing else is.
fn expect_refusal(app: &mut App, problems: &mut Vec<String>, how: &str, label: &str) {
    let notice = find::wait_notice(app, how);
    find::dismiss_notice(app);
    let wanted = (
        REFUSED.to_owned(),
        format!(
            "{label} is not available in a list of search results. Leave the list with Backspace to use it in the folder."
        ),
    );
    match notice {
        Ok(notice) if notice == wanted => {}
        Ok(notice) => problems.push(format!("{how}: {notice:?}")),
        Err(error) => problems.push(error),
    }
    if !feed_view(app, LEFT).open {
        problems.push(format!("{how} closed the list"));
    }
    if !files::main_idle(app) {
        problems.push(format!("{how} left something open"));
        files::dismiss_all(app);
    }
}

/// Every refused action names itself in a notice and changes nothing.
fn refusals(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    ensure_fed(app)?;
    let before = files::tree_facts(&setup.root)?;
    let sorting = "Sorting (sort the results in the Find window before feeding them)";
    let attributes = "Attributes, times and case";
    let mut problems = Vec::new();
    let mut disabled = Vec::new();
    let chords: [(&[Key], slint::SharedString, &str, &str); 18] = [
        (&[], Key::F2.into(), "F2", "Rename"),
        (&[], Key::F7.into(), "F7", "Create folder"),
        (&[Key::Shift], Key::F4.into(), "Shift+F4", "New file"),
        (&[Key::Control], "v".into(), "Ctrl+V", "Paste"),
        (&[Key::Control], Key::F3.into(), "Ctrl+F3", sorting),
        (&[Key::Control], Key::F6.into(), "Ctrl+F6", sorting),
        (
            &[Key::Control, Key::Shift],
            "s".into(),
            "Ctrl+Shift+S",
            "Folder sizes",
        ),
        (&[Key::Control], Key::F2.into(), "Ctrl+F2", attributes),
        (&[Key::Control], Key::F7.into(), "Ctrl+F7", attributes),
        (
            &[Key::Control, Key::Shift],
            "c".into(),
            "Ctrl+Shift+C",
            "Compare files",
        ),
        (&[Key::Control], Key::F8.into(), "Ctrl+F8", "Checksums"),
        (&[Key::Control], "m".into(), "Ctrl+M", "Make file list"),
        (
            &[Key::Control],
            Key::F10.into(),
            "Ctrl+F10",
            "Compare directories",
        ),
        (
            &[Key::Control],
            Key::F12.into(),
            "Ctrl+F12",
            "The panel filter",
        ),
        (&[Key::Control], "h".into(), "Ctrl+H", "Show hidden files"),
        (&[Key::Control], "=".into(), "Ctrl+=", "Select by mask"),
        (&[Key::Control], "-".into(), "Ctrl+−", "Deselect by mask"),
        (
            &[Key::Control],
            Key::Return.into(),
            "Ctrl+Enter",
            "Inserting the focused name",
        ),
    ];
    for (modifiers, text, how, label) in chords {
        key(app, modifiers, text);
        expect_refusal(app, &mut problems, how, label);
    }
    for (name, label) in [
        ("NumpadAdd", "Select by mask"),
        ("NumpadSubtract", "Deselect by mask"),
    ] {
        find::keypad(app, name);
        expect_refusal(app, &mut problems, name, label);
    }
    // The toolbar, the function bar and the main menu refuse the same way.
    for (prefix, label) in [
        ("New empty file", "New file"),
        ("Attributes, times and case", attributes),
        ("F7", "Create folder"),
    ] {
        let buttons = find::main_buttons(app, prefix);
        match buttons.first() {
            // A disabled button refuses too: it cannot act on the folder under the list.
            Some((_, button)) if !tools::enabled(button) => {
                disabled.push(prefix);
            }
            Some((_, button)) => {
                input::click_at(app.slint(), crate::inspect::center(button));
                app.settle();
                expect_refusal(app, &mut problems, &format!("the button {prefix:?}"), label);
            }
            None => problems.push(format!("no button {prefix:?}")),
        }
    }
    // A custom command from the toolbar: refused, and it does not run.
    match find::main_buttons(app, COMMAND).first() {
        Some((_, button)) => {
            input::click_at(app.slint(), crate::inspect::center(button));
            app.settle();
            expect_refusal(
                app,
                &mut problems,
                "the custom command's button",
                "A custom command",
            );
            app.idle(Duration::from_millis(300));
            if setup.edited.join("custom-command-ran").exists() {
                problems.push("the refused custom command ran".into());
            }
        }
        None => problems.push(format!("no toolbar button {COMMAND:?}")),
    }
    for (category, item, label) in [
        ("Files", "Create folder…", "Create folder"),
        ("Panels and navigation", "Sort by name", sorting),
        ("Tools", "Checksums…", "Checksums"),
    ] {
        match find::menu(app, category, item) {
            Ok(()) => expect_refusal(app, &mut problems, &format!("F10 {item}"), label),
            Err(error) => problems.push(format!("F10 {item}: {error}")),
        }
    }
    app.screenshot(context, "refused", "The list explains an action it refuses");
    if !disabled.is_empty() {
        context.note(format!(
            "Under the fed list these toolbar buttons are disabled rather than explained: {disabled:?}"
        ));
    }
    if files::tree_facts(&setup.root)? != before {
        problems.push("a refused action changed the folder under the list".into());
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// F3, Alt+F3 and F4 act on the focused file; a focused folder is explained;
/// Alt+Enter summarizes the marks.
fn view_edit(
    app: &mut App,
    setup: &Setup,
    launched: &Rc<RefCell<Vec<Launched>>>,
) -> Result<(), String> {
    ensure_fed(app)?;
    let paths: Vec<PathBuf> = ORDER
        .iter()
        .map(|name| match *name {
            "sub" => setup.root.join("src/sub"),
            "g.txt" => setup.root.join("src/sub/g.txt"),
            "f.txt" => setup.root.join("f.txt"),
            "d.txt" => setup.root.join("src/d.txt"),
            other => setup.root.join("docs").join(other),
        })
        .collect();
    key(app, &[], Key::Home);
    key(app, &[], Key::DownArrow);
    key(app, &[], Key::DownArrow);
    for (modifiers, basic, how) in [(&[][..], false, "F3"), (&[Key::Alt][..], true, "Alt+F3")] {
        let before = launched.borrow().len();
        key(app, modifiers, Key::F3);
        let launch = find::wait_launch(app, launched, before, how)?;
        if launch.path != setup.root.join("f.txt")
            || launch.basic != basic
            || launch.captured
            || launch.entries != paths
        {
            return Err(format!("{how} on f.txt asked for {launch:?}"));
        }
    }
    // F4: the configured editor (cp into edited/) on the focused file.
    key(app, &[], Key::F4);
    crate::options_support::wait_file(app, &setup.edited.join("f.txt"), "F4 on f.txt")?;
    // A folder cannot be viewed or edited.
    key(app, &[], Key::Home);
    for (keys, title, body) in [
        (
            Key::F3,
            "View",
            "Focus a file in the search results to view it.",
        ),
        (
            Key::F4,
            "Edit",
            "Focus a file in the search results to edit it.",
        ),
    ] {
        let before = launched.borrow().len();
        key(app, &[], keys);
        let notice = find::wait_notice(app, &format!("{keys:?} on a folder"))?;
        find::dismiss_notice(app);
        if notice != (title.to_owned(), body.to_owned()) || launched.borrow().len() != before {
            return Err(format!("{keys:?} on the folder result: {notice:?}"));
        }
    }
    // Alt+Enter: the marked results' properties, not a refusal.
    set_marks(app, &["d.txt", "b.txt"])?;
    key(app, &[Key::Alt], Key::Return);
    let (title, body) = find::wait_notice(app, "Alt+Enter on two marked results")?;
    find::dismiss_notice(app);
    if title == REFUSED || !(body.contains('2') || title.contains('2')) {
        return Err(format!("Alt+Enter showed {title:?}: {body:?}"));
    }
    set_marks(app, &[])?;
    Ok(())
}

/// The panel's folder under the list, as the disk has it.
fn shows_folder(app: &App, folder: &Path) -> Result<(), String> {
    if feed_view(app, LEFT).open {
        return Err("the list is still open".into());
    }
    if app.location(LEFT).as_deref() != Some(folder) {
        return Err(format!("the panel shows {:?}", app.location(LEFT)));
    }
    let shown: BTreeSet<String> = app
        .names(LEFT)
        .into_iter()
        .filter(|name| name != "..")
        .collect();
    let disk: BTreeSet<String> = panel::disk_names(folder).into_iter().collect();
    if shown != disk {
        return Err(format!("the panel lists {shown:?}, the disk {disk:?}"));
    }
    Ok(())
}

fn leaving(app: &mut App, setup: &Setup) -> Result<(), String> {
    ensure_fed(app)?;
    let root = setup.root.clone();
    // Tab moves to the other panel and back; the list stays.
    key(app, &[], Key::Tab);
    if !feed_view(app, LEFT).open {
        return Err("Tab closed the list".into());
    }
    key(app, &[], Key::Tab);
    for (modifiers, text, how) in [
        (
            &[][..],
            slint::SharedString::from(Key::Backspace),
            "Backspace",
        ),
        (&[Key::Alt][..], Key::LeftArrow.into(), "Alt+Left"),
        (&[Key::Control][..], Key::Backspace.into(), "Ctrl+Backspace"),
        (&[Key::Shift][..], Key::Backspace.into(), "Shift+Backspace"),
    ] {
        if !feed_view(app, LEFT).open {
            feed(app, LEFT, &ASCENDING, false)?;
        }
        key(app, modifiers, text);
        let wanted = root.clone();
        app.until(&format!("{how} to leave the list"), move |app| {
            !feed_view(app, LEFT).open && app.location(LEFT).as_deref() == Some(wanted.as_path())
        });
        shows_folder(app, &root).map_err(|error| format!("after {how}: {error}"))?;
    }
    feed(app, LEFT, &ASCENDING, false)?;
    input::click_button(app.slint(), "Back to folder")?;
    let wanted = root.clone();
    app.until("Back to folder", move |app| {
        !feed_view(app, LEFT).open && app.location(LEFT).as_deref() == Some(wanted.as_path())
    });
    shows_folder(app, &root).map_err(|error| format!("after Back to folder: {error}"))?;
    // Any navigation of the panel leaves the list: a typed path.
    feed(app, LEFT, &ASCENDING, false)?;
    let docs = root.join("docs");
    key(app, &[Key::Control], "g");
    input::replace_text(app.slint(), &docs.display().to_string());
    key(app, &[], Key::Return);
    panel::wait_location(app, LEFT, &docs);
    shows_folder(app, &docs).map_err(|error| format!("after Ctrl+G: {error}"))?;
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, LEFT, &root);
    Ok(())
}

/// Enter and a double click go to a file (cursor on it) or open a folder.
fn going(app: &mut App, setup: &Setup) -> Result<(), String> {
    let root = &setup.root;
    feed(app, LEFT, &ASCENDING, false)?;
    let index = feed_names(app, LEFT)
        .iter()
        .position(|name| name == "d.txt")
        .ok_or("d.txt is not listed")?;
    key(app, &[], Key::Home);
    for _ in 0..index {
        key(app, &[], Key::DownArrow);
    }
    key(app, &[], Key::Return);
    find::wait_cursor(app, LEFT, &root.join("src"), "d.txt");
    if feed_view(app, LEFT).open {
        return Err("Enter on a file kept the list".into());
    }
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, LEFT, root);
    feed(app, LEFT, &ASCENDING, false)?;
    let index = feed_names(app, LEFT)
        .iter()
        .position(|name| name == "sub")
        .ok_or("sub is not listed")?;
    key(app, &[], Key::Home);
    for _ in 0..index {
        key(app, &[], Key::DownArrow);
    }
    key(app, &[], Key::Return);
    panel::wait_location(app, LEFT, &root.join("src/sub"));
    if feed_view(app, LEFT).open {
        return Err("Enter on a folder kept the list".into());
    }
    panel::focus_list(app, LEFT)?;
    let back = root.clone();
    input::chord(app.slint(), &[Key::Control], "g");
    input::replace_text(app.slint(), &back.display().to_string());
    input::key(app.slint(), Key::Return);
    panel::wait_location(app, LEFT, &back);
    // Ctrl+PageDown (enter folder) goes to the focused result as Enter does.
    feed(app, LEFT, &ASCENDING, false)?;
    let index = feed_names(app, LEFT)
        .iter()
        .position(|name| name == "b.txt")
        .ok_or("b.txt is not listed")?;
    key(app, &[], Key::Home);
    for _ in 0..index {
        key(app, &[], Key::DownArrow);
    }
    key(app, &[Key::Control], Key::PageDown);
    find::wait_cursor(app, LEFT, &root.join("docs"), "b.txt");
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, LEFT, root);
    feed(app, LEFT, &ASCENDING, false)?;
    let (x, y, w, h) = find::feed_row_rect(app, "a.txt").ok_or("the a.txt row is not shown")?;
    input::double_click_at(
        app.slint(),
        LogicalPosition::new(x + w.min(120.0) / 2.0, y + h / 2.0),
    );
    find::wait_cursor(app, LEFT, &root.join("docs"), "a.txt");
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, LEFT, root);
    Ok(())
}

/// F5 copies the marked results, F6 moves the focused one (its row stays), F8 only
/// asks, Shift+F8 deletes a marked folder and the result inside it.
fn operations(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    feed(app, LEFT, &ASCENDING, true)?;
    set_marks(app, &["a.txt", "b.txt"])?;
    let run = files::transfer(app, context, TransferSpec::copy())?;
    files::completed(&run, "F5 of two marked results")?;
    for (name, data) in [("a.txt", "a"), ("b.txt", "bb")] {
        if fs::read(setup.target.join(name)).ok().as_deref() != Some(data.as_bytes()) {
            return Err(format!("F5 did not copy {name}"));
        }
    }
    // F6 of the focused (unmarked) result.
    set_marks(app, &[])?;
    let index = feed_names(app, LEFT)
        .iter()
        .position(|name| name == "f.txt")
        .ok_or("f.txt is not listed")?;
    for _ in 0..index {
        key(app, &[], Key::DownArrow);
    }
    let run = files::transfer(app, context, TransferSpec::moving())?;
    files::completed(&run, "F6 of the focused result")?;
    if setup.root.join("f.txt").exists()
        || fs::read(setup.target.join("f.txt")).ok().as_deref() != Some(&b"fffff"[..])
    {
        return Err("F6 did not move f.txt".into());
    }
    if !feed_names(app, LEFT).contains(&"f.txt".to_owned()) {
        return Err("after F6 the list no longer shows the moved result".into());
    }
    // F8 asks for the Trash; Escape keeps the file.
    key(app, &[], Key::Home);
    key(app, &[], Key::End);
    key(app, &[], Key::F8);
    files::wait_operation_dialog(app, "the Trash confirmation");
    if app.window.get_operation_dangerous() {
        return Err("F8 asked for a permanent delete".into());
    }
    key(app, &[], Key::Escape);
    app.until("the confirmation to close", |app| {
        app.window.get_operation_token().is_empty()
    });
    if !setup.root.join("docs/a.txt").exists() {
        return Err("a cancelled F8 removed docs/a.txt".into());
    }
    // Delete asks for the Trash too, Shift+Delete for a permanent delete; Escape.
    for (modifiers, dangerous, how) in [
        (&[][..], false, "Delete"),
        (&[Key::Shift][..], true, "Shift+Delete"),
    ] {
        key(app, modifiers, Key::Delete);
        files::wait_operation_dialog(app, &format!("the {how} confirmation"));
        if app.window.get_operation_dangerous() != dangerous {
            return Err(format!("{how} asked for the wrong delete"));
        }
        key(app, &[], Key::Escape);
        app.until("the confirmation to close", |app| {
            app.window.get_operation_token().is_empty()
        });
    }
    if !setup.root.join("docs/a.txt").exists() {
        return Err("a cancelled Delete removed docs/a.txt".into());
    }
    // Shift+F8 of a marked folder takes the result inside it along.
    set_marks(app, &["sub"])?;
    let progress = files::delete_permanently(app, context, Some("feed-delete-folder"))?;
    files::operation_ok(&progress, "Shift+F8 of a marked folder result")?;
    if setup.root.join("src/sub").exists() {
        return Err("Shift+F8 left src/sub".into());
    }
    app.until("the deleted rows to leave the list", |app| {
        let names = feed_names(app, LEFT);
        !names.contains(&"sub".to_owned()) && !names.contains(&"g.txt".to_owned())
    });
    let left: Vec<String> = feed_names(app, LEFT);
    if left != ["f.txt", "d.txt", "b.txt", "a.txt"] {
        return Err(format!("after Shift+F8 the list shows {left:?}"));
    }
    key(app, &[], Key::Backspace);
    panel::wait_location(app, LEFT, &setup.root);
    Ok(())
}

/// Leave the left panel's fed list with Backspace, when one is open, and wait until the
/// folder is back. The list takes the keys only while it has the keyboard (after a dialog
/// closed it may not): a click on its first row gives it back first.
fn leave_list(app: &mut App) -> Result<(), String> {
    if !feed_view(app, LEFT).open {
        return Ok(());
    }
    if app.window.get_feed_focused_pane() != LEFT.index() as i32
        && let Some(name) = feed_names(app, LEFT).first().cloned()
        && let Some((x, y, w, h)) = find::feed_row_rect(app, &name)
    {
        input::click_at(
            app.slint(),
            LogicalPosition::new(x + w.min(120.0) / 2.0, y + h / 2.0),
        );
        app.settle();
    }
    key(app, &[], Key::Backspace);
    app.until("the fed list to close", |app| !feed_view(app, LEFT).open);
    Ok(())
}

/// The Find window's selection arrives as the list's marks: three results selected
/// with Ctrl+click, the cursor on the last; Feed to panel lists all six with exactly
/// those marked and the list's cursor on the Find's cursor result; Ctrl+C and Ctrl+X put
/// the marked files on the clipboard to copy and to move; F5 names "3 selected search
/// results" (cancelled). A selection that only follows the cursor marks nothing, and the
/// list's cursor is still on that result.
fn selection_carried(app: &mut App, context: &mut Context, setup: &Setup) -> Result<(), String> {
    // The list a check before left open (Find opens from the panel's own file list).
    leave_list(app)?;
    let window = find::open_on(app, LEFT)?;
    let (text, _) = search(
        app,
        &window,
        Query {
            masks: "*.txt;sub",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") {
        return Err(format!("*.txt;sub ended {text:?}"));
    }
    find::wait_names(app, &window, &ASCENDING, "Name ascending")?;
    let w = window.window();
    find::select_result(app, &window, "b.txt", None)?;
    for name in ["d.txt", "f.txt"] {
        let (_, (x, y, width, height)) = find::reveal_row(app, &window, |row| row.name == name)?;
        input::click_with(
            w,
            &[Key::Control],
            LogicalPosition::new(x + width.min(160.0) / 2.0, y + height / 2.0),
        );
        app.settle();
    }
    let selected = tools::selected_names(&window, "Find results");
    let status = find::status(&window);
    // b.txt, d.txt and f.txt hold 2 + 4 + 5 bytes.
    if selected != ["b.txt", "d.txt", "f.txt"] || !status.ends_with(" · 3 selected · 11 B") {
        return Err(format!(
            "Ctrl+click selected {selected:?} with the status {status:?}"
        ));
    }
    tools::click_last(w, "Feed to panel")?;
    app.until("the results list to open", |app| feed_view(app, LEFT).open);
    panel::wait_input(app);
    let marked = feed_marked(app, LEFT);
    let cursor = feed_cursor(app, LEFT);
    let view = feed_view(app, LEFT);
    if feed_names(app, LEFT) != ASCENDING
        || marked != find::set(&["b.txt", "d.txt", "f.txt"])
        || cursor.as_deref() != Some("f.txt")
        || !view.status.starts_with("3 of 6 selected · ")
    {
        return Err(format!(
            "fed with three results selected: rows {:?}, marks {marked:?}, cursor {cursor:?}, status {:?}",
            feed_names(app, LEFT),
            view.status
        ));
    }
    app.screenshot(
        context,
        "fed-selection",
        "The Find selection arrives marked",
    );
    let wanted: BTreeSet<PathBuf> = [
        setup.root.join("docs/b.txt"),
        setup.root.join("src/d.txt"),
        setup.root.join("f.txt"),
    ]
    .into_iter()
    .collect();
    for (letter, intent, how) in [
        ("c", midnight_core::FileClipboardIntent::Copy, "Ctrl+C"),
        ("x", midnight_core::FileClipboardIntent::Move, "Ctrl+X"),
    ] {
        let before = tools::clipboard_writes(app);
        key(app, &[Key::Control], letter);
        let files = tools::wait_copied_files(app, before, &format!("{how} in the fed list"))?;
        let paths: BTreeSet<PathBuf> = files.paths.iter().cloned().collect();
        if paths != wanted || files.intent != intent {
            return Err(format!(
                "{how} in the fed list put {:?} ({:?}) on the clipboard, expected {wanted:?} ({intent:?})",
                files.paths, files.intent
            ));
        }
    }
    key(app, &[], Key::F5);
    app.until("the Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    let texts = crate::inspect::visible_texts(app.slint());
    let source = app.window.get_copy_draft_source().to_string();
    key(app, &[], Key::Escape);
    app.until("the Copy dialog to close", |app| {
        app.window.get_copy_draft_token().is_empty()
    });
    // The dialog's first line counts the marked results and their bytes (b, d and f:
    // 2 + 4 + 5); where they come from is its tooltip.
    if !texts.iter().any(|text| text == "Copy 3 files (11 B) to:") || source != "Search results" {
        return Err(format!(
            "F5 on three marked results: the Copy dialog shows {texts:?} with the source {source:?}; expected \"Copy 3 files (11 B) to:\" from \"Search results\""
        ));
    }
    if !files::main_idle(app) {
        return Err("Escape in the Copy dialog left something open".into());
    }
    leave_list(app)?;
    // A selection that only follows the cursor (a plain click) marks nothing; the list's
    // cursor is on that result.
    let window = find::open_on(app, LEFT)?;
    let (text, _) = search(
        app,
        &window,
        Query {
            masks: "*.txt;sub",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") {
        return Err(format!("*.txt;sub ended {text:?} the second time"));
    }
    find::wait_names(app, &window, &ASCENDING, "Name ascending")?;
    find::select_result(app, &window, "g.txt", None)?;
    tools::click_last(window.window(), "Feed to panel")?;
    app.until("the results list to open", |app| feed_view(app, LEFT).open);
    panel::wait_input(app);
    let marked = feed_marked(app, LEFT);
    let cursor = feed_cursor(app, LEFT);
    leave_list(app)?;
    if !marked.is_empty() || cursor.as_deref() != Some("g.txt") {
        return Err(format!(
            "fed after a plain click on g.txt: marks {marked:?}, cursor {cursor:?} (expected none marked, the cursor on g.txt)"
        ));
    }
    Ok(())
}

/// A Find opened in the right panel feeds the right panel.
fn right_panel(app: &mut App, setup: &Setup) -> Result<(), String> {
    // The file operations leave a.txt and b.txt (F5) and f.txt (F6) in the target; write
    // the ones they did not, so this check does not depend on theirs.
    for (name, data) in [("a.txt", "a"), ("b.txt", "bb"), ("f.txt", "fffff")] {
        let path = setup.target.join(name);
        if !path.exists() {
            tools::write_at(&path, data.as_bytes(), SystemTime::now())?;
        }
    }
    feed(app, PaneId::Right, &["a.txt", "b.txt", "f.txt"], false)?;
    let names: BTreeSet<String> = feed_names(app, PaneId::Right).into_iter().collect();
    if names != find::set(&["a.txt", "b.txt", "f.txt"]) || feed_view(app, LEFT).open {
        return Err(format!(
            "the right panel's list shows {names:?} (left list open: {})",
            feed_view(app, LEFT).open
        ));
    }
    let view = feed_view(app, PaneId::Right);
    if !view
        .title
        .starts_with(&format!("Search results (3) · {}", setup.target.display()))
    {
        return Err(format!("the right list is titled {:?}", view.title));
    }
    key(app, &[], Key::Backspace);
    let target = setup.target.clone();
    app.until("the right list to close", move |app| {
        !feed_view(app, PaneId::Right).open
            && app.location(PaneId::Right).as_deref() == Some(target.as_path())
    });
    Ok(())
}
