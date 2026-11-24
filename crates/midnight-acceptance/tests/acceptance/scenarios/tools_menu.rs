//! Help (F1), Drive information (Ctrl+F1) and the main menu (F10): every menu item is
//! clicked and its result checked (a dialog, window, popup or effect), then
//! cancelled or undone. Nothing is ever confirmed; Exit comes last. An action with
//! several chords lists them all, joined with " / ", and its tooltip names them.
use crate::{
    Context,
    app::{self, App, Config},
    input, inspect,
    launches::Launch,
    panel,
    scenarios::transfers::snapshot,
    tools_support::{self as tools},
};
use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};
use midnight_core::{PaneId, TerminalMode};
use midnight_ui::{FileListPresentation, HotPathsPresentation, PanelChoices};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowAdapter},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const SIZE: (u32, u32) = (900, 640);

/// The terminal button's Ctrl+click in the Tools category, named as the platform names it.
const ELEVATED_TERMINAL: &str = if cfg!(windows) {
    "Open terminal as administrator"
} else {
    "Open root terminal (sudo)"
};

/// Nothing open on top of the panels.
fn quiet(app: &App) -> bool {
    !app.window.get_embedded_modal_open()
        && !tools::popup_open(app.slint())
        && app.window.get_operation_token().is_empty()
        && app.window.get_copy_draft_token().is_empty()
        && app.window.get_metadata_token().is_empty()
        && app.window.get_volume_popup_pane() < 0
        && !app.window.get_options_open()
        && !app.window.get_about_open()
        && tools::shown_notice(app).is_none()
        && app.tool_windows().is_empty()
}

/// Poll `condition` for up to `limit`; an error names `what` and the visible state.
fn wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} did not happen; notice {:?}, tool windows {:?}, modal {}, popup {}",
                tools::shown_notice(app),
                tools::tool_titles(app),
                app.window.get_embedded_modal_open(),
                tools::popup_open(app.slint())
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

/// Close whatever is open with Escape (tool windows first) until the panels are free.
/// A tool window that ignores Escape (ACC-TOOLS-06) is closed with its Close button.
fn dismiss(app: &mut App, what: &str) -> Result<(), String> {
    for attempt in 0..4 {
        if quiet(app) {
            return Ok(());
        }
        for window in app.tool_windows() {
            // A window the harness never sized has no visible focus item for keys.
            app::layout_tool(&window, SIZE.0, SIZE.1);
            if attempt < 2 {
                input::key(window.window(), Key::Escape);
            } else {
                tools::click_last(window.window(), "Close")?;
            }
        }
        if !app.tool_windows().is_empty() {
            app.idle(Duration::from_millis(100));
            continue;
        }
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(100));
    }
    wait(app, &format!("{what} to close"), quiet)
}

const CATEGORIES: [&str; 6] = [
    "All actions",
    "Files",
    "Selection",
    "Panels and navigation",
    "Tools",
    "Application",
];

/// Every main menu item by category, in display order.
const MENU: [(&str, &[&str]); 5] = [
    (
        "Files",
        &[
            "Open",
            "View",
            "View as plain text",
            "Edit",
            "Copy to…",
            "Move to…",
            "Rename…",
            "Create folder…",
            "Create empty file…",
            "Move to Trash…",
            "Delete permanently…",
            "Copy to clipboard",
            "Cut to clipboard",
            "Paste files",
            "Attributes, times and case…",
            "Change name case…",
            "System properties",
        ],
    ),
    (
        "Selection",
        &[
            "Select files and folders",
            "Select files only",
            "Deselect files and folders",
            "Invert files only",
            "Invert files and folders",
            "Select by mask…",
            "Deselect by mask…",
            "Invert, folders as in Select by mask",
        ],
    ),
    (
        "Panels and navigation",
        &[
            "Refresh active folder",
            "Parent folder",
            "Filesystem root",
            "Back",
            "Forward",
            "Enter path…",
            "As other panel",
            "Swap panels",
            "Enlarge / restore active panel",
            "Filter panel…",
            "Hide hidden files",
            "Brief view",
            "Detailed view",
            "Thumbnail view",
            "Sort by name",
            "Sort by extension",
            "Sort by size",
            "Sort by modified time",
            "New tab",
            "Close tab",
            "Add Hot Path",
            "Manage Hot Paths…",
            "Left panel drives and locations",
            "Right panel drives and locations",
        ],
    ),
    (
        "Tools",
        &[
            "Find files…",
            "Checksums…",
            "Make File List…",
            "Compare files…",
            "Compare directories…",
            "Clear comparison marks",
            "Calculate / cancel folder sizes",
            "Drive information",
            "Command line",
            ELEVATED_TERMINAL,
            "Configure Custom Commands…",
            "Connect network drive…",
            "Disconnect network drive…",
        ],
    ),
    (
        "Application",
        &[
            "Options",
            "Jobs",
            "Keyboard help",
            "About Midnight Explorer",
            "Exit",
        ],
    ),
];

fn category(label: &str) -> Result<&'static str, String> {
    // The hidden-files item names the change it would make.
    let label = if label == "Show hidden files" {
        "Hide hidden files"
    } else {
        label
    };
    MENU.iter()
        .find(|(_, items)| items.contains(&label))
        .map(|(category, _)| *category)
        .ok_or_else(|| format!("{label:?} is not a known menu item"))
}

/// Open the main menu (F10) showing `category`.
fn open_menu(app: &mut App, category: &str) -> Result<(), String> {
    dismiss(app, "the previous step")?;
    panel::focus_list(app, PaneId::Left)?;
    input::key(app.slint(), Key::F10);
    wait(app, "the main menu", |app| tools::popup_open(app.slint()))?;
    let main = app.main.clone();
    tools::choose_combo(app, main.window(), "Menu category", &CATEGORIES, category)
}

/// The first item of the open menu: it anchors scrolling of the menu's list.
fn menu_anchor(app: &App) -> Result<ItemRc, String> {
    let mut anchor = None;
    if let Some(menu) = inspect::roots(app.slint()).last() {
        inspect::walk(menu, &mut |item| {
            if anchor.is_none()
                && item.accessible_role() == AccessibleRole::Button
                && item.is_visible()
                && inspect::label(item).is_some_and(|label| label != "Close menu")
            {
                anchor = Some(item.clone());
            }
        });
    }
    anchor.ok_or_else(|| "the main menu shows no item".to_owned())
}

/// The labels of the open menu's items, scrolled through from top to bottom (items
/// outside the list's viewport are not visible to the item tree).
fn menu_items(app: &mut App) -> Result<Vec<String>, String> {
    let anchor = menu_anchor(app)?;
    let main = app.main.clone();
    Ok(tools::scrolled_labels(
        app,
        main.window(),
        &anchor,
        AccessibleRole::Button,
        |label| label != "Close menu",
    ))
}

/// F10, the item's category, and a click on the item, which must be enabled. The
/// category keeps the list short; the list is scrolled to the item.
fn menu(app: &mut App, label: &str) -> Result<(), String> {
    open_menu(app, category(label)?)?;
    let main = app.main.clone();
    let anchor = menu_anchor(app)?;
    let item = tools::scroll_to(app, main.window(), &anchor, AccessibleRole::Button, label)?;
    if !tools::enabled(&item) {
        return Err(format!("the menu item {label:?} is disabled"));
    }
    input::click(main.window(), AccessibleRole::Button, label)?;
    app.settle();
    Ok(())
}

/// Every chord of `label`'s action in one text beside the item, joined with " / " in the
/// order Options › Keyboard lists them, and the item's tooltip naming the item and those
/// chords ("<label> (<chords>)"; a disabled item's tooltip adds a line saying why).
fn joined_chords(app: &mut App, label: &str, chords: &str) -> Result<(), String> {
    open_menu(app, category(label)?)?;
    let main = app.main.clone();
    let anchor = menu_anchor(app)?;
    let item = tools::scroll_to(app, main.window(), &anchor, AccessibleRole::Button, label)?;
    let texts = tools::item_texts(&item);
    let tooltip = format!("{label} ({chords})");
    let tips = tools::hover_tooltip(app, main.window(), &item);
    if !texts.iter().any(|text| text == chords)
        || !tips
            .iter()
            .any(|tip| tip.lines().next() == Some(tooltip.as_str()))
    {
        return Err(format!(
            "the menu item {label:?} shows {texts:?} with the tooltip {tips:?}; expected the keys {chords:?} and the tooltip {tooltip:?}"
        ));
    }
    Ok(())
}

fn selected(app: &App) -> BTreeSet<String> {
    app.rows(PaneId::Left)
        .iter()
        .filter(|row| row.selected && !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

fn at(app: &App, path: &Path) -> bool {
    app.location(PaneId::Left).as_deref() == Some(path) && !app.loading(PaneId::Left)
}

/// What an item without a harness service did: windows, notices, dialogs; then closed.
fn observe(app: &mut App, label: &str) -> Result<String, String> {
    let status = app.window.get_desktop_status().to_string();
    app.idle(Duration::from_millis(400));
    let mut seen = Vec::new();
    for title in tools::tool_titles(app) {
        seen.push(format!("window {title:?}"));
    }
    if let Some((title, body)) = tools::shown_notice(app) {
        seen.push(format!(
            "notice {title:?}: {:?}",
            body.lines().next().unwrap_or_default()
        ));
    }
    if app.window.get_embedded_modal_open() || tools::popup_open(app.slint()) {
        seen.push("a dialog".into());
    }
    let now = app.window.get_desktop_status().to_string();
    if now != status && !now.is_empty() {
        seen.push(format!("status {now:?}"));
    }
    dismiss(app, label)?;
    Ok(if seen.is_empty() {
        format!("{label}: no visible effect")
    } else {
        format!("{label}: {}", seen.join(", "))
    })
}

pub fn help_and_drive_info(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, &root));
    panel::focus_list(&mut app, PaneId::Left)?;

    // F1: the keyboard help window lists the tools' shortcuts.
    let window = tools::open_tool(&mut app, "the Help window", &[], Key::F1, SIZE);
    if tools::title(&window) != "Keyboard help · Midnight Explorer" {
        return Err(format!("F1 opened {:?}", tools::title(&window)));
    }
    let texts = inspect::visible_texts(window.window());
    let all = texts.join("\n");
    if !texts
        .iter()
        .any(|text| text == "Midnight Explorer keyboard help")
    {
        return Err(format!("the help shows {texts:?}"));
    }
    let mut missing = Vec::new();
    for keys in [
        "Alt+F7",
        "Ctrl+F8",
        "Ctrl+F10",
        "Ctrl+Shift+S",
        "Ctrl+F1",
        "F10",
    ] {
        if !all.contains(keys) {
            missing.push(keys);
        }
    }
    if !missing.is_empty() {
        return Err(format!("the keyboard help does not mention {missing:?}"));
    }
    app.screenshot_window(context, &window, "help", "Keyboard help (F1)");
    // Reported at the end, after the remaining checks ran.
    let outdated = all.contains("Native size scans follow directory links");
    tools::click_last(window.window(), "Close")?;
    let watched = window.clone();
    app.until("the help window to close", move |_| {
        !watched.window().is_visible()
    });
    // Escape closes it too.
    panel::focus_list(&mut app, PaneId::Left)?;
    let window = tools::open_tool(&mut app, "the Help window again", &[], Key::F1, SIZE);
    tools::close_tool(&mut app, &window, "the Help window");

    // Ctrl+F1: drive information for the panel's folder, filled in after a moment.
    panel::focus_list(&mut app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], Key::F1);
    wait(&mut app, "the Drive information notice", |app| {
        tools::shown_notice(app).is_some_and(|(title, _)| title == "Drive information")
    })?;
    wait(&mut app, "the volume details", |app| {
        tools::shown_notice(app).is_some_and(|(_, body)| body.contains("Total:"))
    })?;
    let (_, body) = tools::shown_notice(&app).unwrap_or_default();
    let folder = format!("Folder: {}", root.display());
    let mut absent = Vec::new();
    for wanted in [
        folder.as_str(),
        "Volume / mount:",
        "Filesystem:",
        "Total:",
        "Free on filesystem:",
        "Available to you:",
        "Read-only filesystem:",
    ] {
        if !body.contains(wanted) {
            absent.push(wanted);
        }
    }
    let total = body
        .lines()
        .find_map(|line| line.strip_prefix("Total: "))
        .and_then(|rest| rest.split(" bytes").next())
        .and_then(tools::number);
    if !absent.is_empty() || total.is_none_or(|bytes| bytes == 0) {
        return Err(format!(
            "drive information lacks {absent:?} or a total ({total:?}): {body:?}"
        ));
    }
    let main = app.main.clone();
    if !inspect::visible_texts(main.window())
        .iter()
        .any(|text| text.contains("Total:"))
    {
        return Err("the drive information is not shown".into());
    }
    app.screenshot(context, "drive-info", "Drive information (Ctrl+F1)");
    tools::click_shown(main.window(), "Close")?;
    wait(&mut app, "the drive information to close", quiet)?;
    context.note(format!(
        "help lists the tools' shortcuts and closes with Close and Escape; drive information: total {} bytes",
        total.unwrap_or_default()
    ));
    app.shutdown()?;
    if outdated {
        return Err("finding ACC-TOOLS-03: the keyboard help says \"Native size scans follow directory links and junctions\", but links inside a measured folder are skipped (folder_sizes, tools_folder_sizes; docs/developer/TREE_PANEL.md)".into());
    }
    Ok(())
}

pub fn main_menu(context: &mut Context) -> Result<(), String> {
    let left = context.work.join("menu");
    let right = context.work.join("menu-right");
    for (path, data) in [
        ("alpha.txt", &b"alpha\n"[..]),
        ("beta.txt", b"beta beta\n"),
        ("gamma.dat", b"gamma gamma gamma\n"),
        ("sub/inner.txt", b"inner\n"),
    ] {
        let full = left.join(path);
        fs::create_dir_all(full.parent().unwrap_or(&left)).map_err(|error| error.to_string())?;
        fs::write(&full, data).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&right).map_err(|error| error.to_string())?;
    fs::write(right.join("other.txt"), b"other\n").map_err(|error| error.to_string())?;
    let parent: PathBuf = context.work.clone();
    let disk_before = snapshot(&left)?;
    let mut app = App::start(Config::new(&left, &right));
    let pane = PaneId::Left;
    let mut log: Vec<String> = Vec::new();

    // Each category lists exactly its items, in order; together they are the menu.
    panel::focus_list(&mut app, pane)?;
    input::key(app.slint(), Key::F10);
    wait(&mut app, "the main menu", |app| {
        tools::popup_open(app.slint())
    })?;
    app.screenshot(context, "main-menu", "Main menu (F10)");
    let all = menu_items(&mut app)?;
    let listed: Vec<String> = MENU
        .iter()
        .flat_map(|(_, items)| items.iter().map(|item| (*item).to_owned()))
        .collect();
    if all != listed {
        return Err(format!(
            "the main menu lists {all:?}; the scenario covers {listed:?}"
        ));
    }
    for (name, items) in MENU {
        open_menu(&mut app, name)?;
        let shown = menu_items(&mut app)?;
        if shown
            != items
                .iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        {
            return Err(format!("the {name} category lists {shown:?}"));
        }
        if name == "Tools" {
            app.screenshot(context, "main-menu-tools", "Main menu, Tools category");
        }
    }
    // An action with several chords lists them all, joined, as Options › Keyboard does.
    for (label, chords) in [
        ("Delete permanently…", "Shift+Delete / Shift+F8"),
        (
            "Calculate / cancel folder sizes",
            "Ctrl+Shift+S / Ctrl+Shift+F10",
        ),
    ] {
        joined_chords(&mut app, label, chords)?;
    }
    dismiss(&mut app, "the main menu")?;

    // Files.
    panel::select_row(&mut app, pane, "sub")?;
    menu(&mut app, "Open")?;
    let sub = left.join("sub");
    wait(&mut app, "Open to enter sub", |app| at(app, &sub))?;
    panel::leave(&mut app, pane)?;
    panel::select_row(&mut app, pane, "alpha.txt")?;
    for label in ["View", "View as plain text", "Edit"] {
        menu(&mut app, label)?;
        log.push(observe(&mut app, label)?);
    }
    menu(&mut app, "Copy to…")?;
    wait(&mut app, "the Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty() && !app.window.get_copy_draft_move()
    })?;
    menu(&mut app, "Move to…")?;
    wait(&mut app, "the Move dialog", |app| {
        !app.window.get_copy_draft_token().is_empty() && app.window.get_copy_draft_move()
    })?;
    for label in [
        "Rename…",
        "Create folder…",
        "Create empty file…",
        "Move to Trash…",
    ] {
        menu(&mut app, label)?;
        wait(&mut app, &format!("the {label} dialog"), |app| {
            !app.window.get_operation_token().is_empty()
        })?;
        log.push(format!(
            "{label}: dialog {:?}",
            app.window.get_operation_title().to_string()
        ));
    }
    menu(&mut app, "Delete permanently…")?;
    wait(&mut app, "the permanent delete confirmation", |app| {
        !app.window.get_operation_token().is_empty() && app.window.get_operation_dangerous()
    })?;
    for label in ["Copy to clipboard", "Cut to clipboard", "Paste files"] {
        menu(&mut app, label)?;
        log.push(observe(&mut app, label)?);
    }
    menu(&mut app, "Attributes, times and case…")?;
    wait(&mut app, "the Attributes dialog", |app| {
        !app.window.get_metadata_token().is_empty()
    })?;
    menu(&mut app, "Change name case…")?;
    wait(&mut app, "the name case dialog", |app| {
        !app.window.get_metadata_token().is_empty()
    })?;
    menu(&mut app, "System properties")?;
    log.push(observe(&mut app, "System properties")?);
    dismiss(&mut app, "the Files items")?;
    if snapshot(&left)? != disk_before {
        return Err("cancelling the Files items changed the folder on disk".into());
    }

    // Selection.
    let steps: [(&str, &[&str]); 6] = [
        (
            "Select files and folders",
            &["alpha.txt", "beta.txt", "gamma.dat", "sub"],
        ),
        ("Deselect files and folders", &[]),
        ("Select files only", &["alpha.txt", "beta.txt", "gamma.dat"]),
        ("Invert files only", &[]),
        (
            "Invert files and folders",
            &["alpha.txt", "beta.txt", "gamma.dat", "sub"],
        ),
        ("Deselect files and folders", &[]),
    ];
    for (label, wanted) in steps {
        menu(&mut app, label)?;
        let wanted = names(wanted);
        wait(&mut app, &format!("{label} → {wanted:?}"), |app| {
            selected(app) == wanted
        })?;
    }
    // Select and Deselect by mask open their dialog (cancelled); the keypad's
    // invert inverts files only while Include folders is not chosen.
    for (label, select) in [("Select by mask…", true), ("Deselect by mask…", false)] {
        menu(&mut app, label)?;
        wait(&mut app, &format!("the {label} dialog"), |app| {
            let choices = app.window.global::<PanelChoices>();
            !choices.get_mask_token().is_empty() && choices.get_mask_select() == select
        })?;
        dismiss(&mut app, label)?;
    }
    menu(&mut app, "Invert, folders as in Select by mask")?;
    let files = names(&["alpha.txt", "beta.txt", "gamma.dat"]);
    wait(&mut app, "the invert to mark the files", |app| {
        selected(app) == files
    })?;
    menu(&mut app, "Deselect files and folders")?;
    wait(&mut app, "the selection to clear", |app| {
        selected(app).is_empty()
    })?;

    // Panels and navigation.
    let listings = app.listings[0];
    menu(&mut app, "Refresh active folder")?;
    wait(&mut app, "the refreshed listing", |app| {
        app.listings[0] > listings
    })?;
    for (label, target) in [
        ("Parent folder", &parent),
        ("Back", &left),
        ("Forward", &parent),
        ("Back", &left),
    ] {
        menu(&mut app, label)?;
        wait(
            &mut app,
            &format!("{label} to show {}", target.display()),
            |app| at(app, target),
        )?;
    }
    let root = crate::files_support::filesystem_root(&left);
    menu(&mut app, "Filesystem root")?;
    wait(&mut app, "the filesystem root", |app| at(app, &root))?;
    menu(&mut app, "Back")?;
    wait(&mut app, "Back from the root", |app| at(app, &left))?;
    menu(&mut app, "Enter path…")?;
    wait(&mut app, "the path editor", |app| {
        tools::has(app.slint(), AccessibleRole::TextInput, "Folder path")
    })?;
    input::key(app.slint(), Key::Escape);
    app.settle();
    menu(&mut app, "As other panel")?;
    wait(&mut app, "the other panel's folder", |app| at(app, &right))?;
    menu(&mut app, "Back")?;
    wait(&mut app, "Back to the folder", |app| at(app, &left))?;
    for (label, read) in [
        (
            "Swap panels",
            (|app: &App| app.window.get_panels_swapped()) as fn(&App) -> bool,
        ),
        ("Enlarge / restore active panel", |app: &App| {
            app.window.get_panel_zoomed()
        }),
    ] {
        let before = read(&app);
        menu(&mut app, label)?;
        wait(&mut app, &format!("{label} to toggle"), |app| {
            read(app) != before
        })?;
        menu(&mut app, label)?;
        wait(&mut app, &format!("{label} to toggle back"), |app| {
            read(app) == before
        })?;
    }
    menu(&mut app, "Filter panel…")?;
    wait(&mut app, "the panel filter", |app| {
        app.window.get_panel_filter_view().open
    })?;
    // The hidden-files item is named for the change it makes; it saves the choice.
    let shown = |app: &App| app.window.global::<PanelChoices>().get_show_hidden_files();
    menu(&mut app, "Hide hidden files")?;
    wait(&mut app, "hidden files to be hidden", |app| !shown(app))?;
    menu(&mut app, "Show hidden files")?;
    wait(&mut app, "hidden files to be shown again", shown)?;
    for (label, mode) in [
        ("Brief view", 1),
        ("Thumbnail view", 2),
        ("Detailed view", 0),
    ] {
        menu(&mut app, label)?;
        wait(&mut app, &format!("{label} (mode {mode})"), |app| {
            app.window.get_left_view_mode() == mode
        })?;
    }
    let mut sorts = Vec::new();
    for label in [
        "Sort by extension",
        "Sort by size",
        "Sort by modified time",
        "Sort by name",
    ] {
        let before = (app.window.get_left_sort(), app.window.get_left_descending());
        menu(&mut app, label)?;
        wait(&mut app, &format!("{label} to change the order"), |app| {
            (app.window.get_left_sort(), app.window.get_left_descending()) != before
        })?;
        sorts.push(app.window.get_left_sort());
    }
    let unique: BTreeSet<i32> = sorts.iter().copied().collect();
    if unique.len() != 4 || app.window.get_left_descending() {
        return Err(format!(
            "the four sort items gave {sorts:?} (descending {})",
            app.window.get_left_descending()
        ));
    }
    let tabs = app.window.get_left_tabs().row_count();
    menu(&mut app, "New tab")?;
    wait(&mut app, "a new tab", |app| {
        app.window.get_left_tabs().row_count() == tabs + 1
    })?;
    menu(&mut app, "Close tab")?;
    wait(&mut app, "the tab to close", |app| {
        app.window.get_left_tabs().row_count() == tabs
    })?;
    let bookmarks = app.window.get_bookmarks().row_count();
    menu(&mut app, "Add Hot Path")?;
    wait(&mut app, "a new Hot Path", |app| {
        app.window.get_bookmarks().row_count() == bookmarks + 1
    })?;
    menu(&mut app, "Manage Hot Paths…")?;
    wait(&mut app, "Manage Hot Paths", |app| {
        !app.window
            .global::<HotPathsPresentation>()
            .get_token()
            .is_empty()
    })?;
    for (label, pane_index) in [
        ("Left panel drives and locations", 0),
        ("Right panel drives and locations", 1),
    ] {
        menu(&mut app, label)?;
        wait(&mut app, label, |app| {
            app.window.get_volume_popup_pane() == pane_index
        })?;
    }

    // Tools, with the cursor on a file (Make File List and Compare files need one).
    dismiss(&mut app, "the drives popup")?;
    panel::select_row(&mut app, pane, "alpha.txt")?;
    for (label, title) in [
        ("Find files…", "Find files · Midnight Explorer"),
        ("Checksums…", "Checksums · Midnight Explorer"),
    ] {
        menu(&mut app, label)?;
        wait(&mut app, &format!("the {title} window"), |app| {
            tools::tool_titled(app, title).is_some()
        })?;
    }
    menu(&mut app, "Make File List…")?;
    wait(&mut app, "Make File List", |app| {
        app.window.global::<FileListPresentation>().get_active()
    })?;
    menu(&mut app, "Compare files…")?;
    log.push(observe(&mut app, "Compare files…")?);
    // Clear comparison marks is unavailable until a comparison shows its marks.
    if menu(&mut app, "Clear comparison marks").is_ok() {
        return Err("Clear comparison marks is enabled with no comparison shown".into());
    }
    menu(&mut app, "Compare directories…")?;
    wait(&mut app, "Compare directories", |app| {
        app.window.get_comparison_view().open
    })?;
    // Compare with subfolders (the dialog's button), which marks rows with badges:
    // menu/ and menu-right/ share no name, so every row gets one.
    let main = app.main.clone();
    tools::set_check(&mut app, main.window(), "Compare subfolders", true)?;
    let compare = tools::control(main.window(), AccessibleRole::Button, "Compare")?;
    input::click_at(main.window(), inspect::center(&compare));
    let badges = |app: &App| -> usize {
        [PaneId::Left, PaneId::Right]
            .into_iter()
            .flat_map(|pane| app.rows(pane))
            .filter(|row| row.compare_mark != 0)
            .count()
    };
    wait(&mut app, "the comparison's badges", |app| {
        !app.window.get_comparison_view().open && badges(app) > 0
    })?;
    let compared = selected(&app);
    menu(&mut app, "Clear comparison marks")?;
    wait(&mut app, "Clear comparison marks", |app| {
        badges(app) == 0 && app.window.get_left_watch_status().is_empty()
    })?;
    if selected(&app) != compared {
        return Err(format!(
            "Clear comparison marks changed the selection from {compared:?} to {:?}",
            selected(&app)
        ));
    }
    log.push(format!(
        "Clear comparison marks: badges gone, the selection {compared:?} stays"
    ));
    // The comparison selected the one-sided files: folder sizes below act on the folders.
    panel::focus_list(&mut app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    wait(&mut app, "Ctrl+D to clear the selection", |app| {
        selected(app).is_empty()
    })?;
    menu(&mut app, "Calculate / cancel folder sizes")?;
    wait(&mut app, "the folder size of sub", |app| {
        app.rows(PaneId::Left).iter().any(|row| {
            row.name == "sub"
                && row.size_state == 0
                && !row.size_running
                && row.size.chars().any(|c| c.is_ascii_digit())
        })
    })?;
    menu(&mut app, "Drive information")?;
    wait(&mut app, "Drive information", |app| {
        tools::shown_notice(app).is_some_and(|(title, _)| title == "Drive information")
    })?;
    menu(&mut app, "Command line")?;
    wait(&mut app, "the command line to take focus", |app| {
        app.window.get_command_focused()
    })?;
    panel::focus_list(&mut app, pane)?;
    if app.window.get_command_focused() {
        return Err("clicking the panel did not leave the command line".into());
    }
    menu(&mut app, "Configure Custom Commands…")?;
    wait(&mut app, "Options › Custom Commands", |app| {
        app.window.get_options_open() && app.window.get_options_topic_index() == 9
    })?;
    for label in ["Connect network drive…", "Disconnect network drive…"] {
        open_menu(&mut app, "Tools")?;
        let main = app.main.clone();
        let anchor = menu_anchor(&app)?;
        let item = tools::scroll_to(
            &mut app,
            main.window(),
            &anchor,
            AccessibleRole::Button,
            label,
        )?;
        if tools::enabled(&item) {
            input::click(main.window(), AccessibleRole::Button, label)?;
            app.settle();
            log.push(observe(&mut app, label)?);
        } else {
            log.push(format!("{label}: disabled (network-available is false)"));
            dismiss(&mut app, "the main menu")?;
        }
    }

    // Application.
    menu(&mut app, "Options")?;
    wait(&mut app, "Options", |app| app.window.get_options_open())?;
    for (label, title) in [
        ("Jobs", "Jobs"),
        ("Keyboard help", "Keyboard help · Midnight Explorer"),
    ] {
        menu(&mut app, label)?;
        wait(&mut app, &format!("the {title} window"), |app| {
            tools::tool_titled(app, title).is_some()
        })?;
    }
    menu(&mut app, "About Midnight Explorer")?;
    wait(&mut app, "About", |app| app.window.get_about_open())?;
    dismiss(&mut app, "About")?;
    if snapshot(&left)? != disk_before {
        return Err("the menu sweep changed the folder on disk".into());
    }
    menu(&mut app, "Exit")?;
    wait(&mut app, "Exit to hide the main window", |app| {
        !app.slint().is_visible()
    })?;
    for line in &log {
        context.note(line.clone());
    }
    context.note("every main menu item opened its dialog, window or popup (cancelled with Escape) or had its effect (undone); Exit hid the main window");
    app.shutdown()?;
    // The elevated terminal runs in an application of its own, with the desktop stand-in
    // that records terminals instead of starting them.
    context.note(elevated_terminal(&left, &right)?);
    Ok(())
}

/// The Tools category's elevated terminal (the terminal button's Ctrl+click), with the
/// desktop stand-in: an elevated terminal of the selected shell in the active panel's
/// folder is asked for and recorded, never started (no consent prompt, no sudo).
fn elevated_terminal(left: &Path, right: &Path) -> Result<String, String> {
    let mut app = App::start(Config {
        desktop: true,
        ..Config::new(left, right)
    });
    let result = menu(&mut app, ELEVATED_TERMINAL).and_then(|()| {
        wait(&mut app, "the elevated terminal to be recorded", |app| {
            !app.launches.all().is_empty()
        })?;
        let folder = app.location(PaneId::Left);
        match app.launches.all().as_slice() {
            [Launch::Terminal { request, .. }]
                if request.mode == TerminalMode::Elevated
                    && request.command.is_empty()
                    && Some(&request.working_directory) == folder.as_ref() =>
            {
                Ok(format!(
                    "{ELEVATED_TERMINAL}: an elevated {:?} terminal in the left panel's folder was recorded, not started",
                    request.shell
                ))
            }
            other => Err(format!(
                "{ELEVATED_TERMINAL}: launches {other:?}, the left panel in {folder:?}; expected one elevated terminal there"
            )),
        }
    });
    let dismissed = dismiss(&mut app, "the elevated terminal");
    let shutdown = app.shutdown();
    let note = result?;
    dismissed?;
    shutdown?;
    Ok(note)
}
