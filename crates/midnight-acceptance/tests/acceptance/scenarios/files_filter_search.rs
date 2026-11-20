//! Show hidden files and the panel filter, driven by keys and clicks: Ctrl+H, the
//! panel's View menu and F10 hide and show dot-names (the cursor moves to a shown
//! row, marks on hidden entries are dropped, Ctrl+A and the status line follow,
//! an inline tree leaves them out); Options › General saves the choice across a
//! restart. Then the panel filter (Masks, the hidden-name exclusion, Expression,
//! Show all) and filename search.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, io_error},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{Model, platform::Key};
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const SHOW_HIDDEN: &str = "Show hidden files";
const TREE_FOLDER: &str = "folder with spaces";
const TREE_SECRET: &str = ".secret";
const TREE_SHOWN: &str = "sub";

fn config(folder: &Path, work: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, work);
    config.settings = Some(store.clone());
    config
}

/// Apply a panel filter through Ctrl+F12 with `mode` and `text`; wait for the panel.
fn apply_filter(app: &mut App, mode: &str, text: &str) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter dialog", |app| {
        app.window.get_panel_filter_view().open
    });
    support::choose(app, "Filter mode", mode, 3)?;
    input::fill(app.slint(), "File filter expression", text)?;
    input::click_button(app.slint(), "OK")?;
    app.until("the filter to apply", |app| {
        !app.window.get_panel_filter_view().open
    });
    app.idle(Duration::from_millis(200));
    Ok(())
}

/// Top-level names of the left panel, sorted, without `..`.
fn listed(app: &App) -> Vec<String> {
    let mut names: Vec<String> = app
        .rows(LEFT)
        .iter()
        .filter(|row| !row.parent_entry && row.tree_depth == 0)
        .map(|row| row.name.to_string())
        .collect();
    names.sort();
    names
}

#[track_caller]
fn wait_listing(app: &mut App, wanted: &[String]) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if listed(app) == wanted {
            return true;
        }
        app.idle(Duration::from_millis(20));
    }
    false
}

fn visible_names(all: &[String]) -> Vec<String> {
    all.iter()
        .filter(|name| !name.starts_with('.'))
        .cloned()
        .collect()
}

fn hidden_count(app: &App) -> Option<usize> {
    let status = app.window.get_left_status().to_string();
    let words: Vec<&str> = status.split_whitespace().collect();
    words
        .windows(2)
        .find(|pair| pair[1] == "hidden")
        .and_then(|pair| pair[0].parse().ok())
}

/// The "N hidden" field of the left panel's status line, as shown.
fn footer_hidden(app: &App) -> Option<String> {
    let details = app.window.get_left_details();
    (0..details.row_count())
        .filter_map(|row| details.row_data(row))
        .find(|field| field.label == "Hidden")
        .map(|field| field.value.to_string())
}

fn depth_of(app: &App, name: &str) -> Option<i32> {
    app.rows(LEFT)
        .iter()
        .find(|row| row.name == name)
        .map(|row| row.tree_depth)
}

/// Click the expander of `name` in the left panel.
fn toggle_branch(app: &mut App, name: &str) -> Result<(), String> {
    panel::wait_input(app);
    let wanted = name.to_owned();
    app.until(&format!("{name}'s expander"), move |app| {
        app.rows(LEFT).iter().any(|row| {
            row.name == wanted && (row.tree_expanded || matches!(row.tree_disclosure, 1 | 3))
        })
    });
    let row = app
        .rows(LEFT)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| format!("no {name} row"))?;
    let verb = if row.tree_expanded {
        "Collapse"
    } else if row.tree_disclosure == 1 {
        "Discover"
    } else {
        "Expand"
    };
    let label = format!("{verb} folders in {name}");
    let left_edge = app.window.get_left_list_left() + app.window.get_left_list_width();
    let expander = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(text, _)| *text == label)
        .map(|(_, item)| item)
        .find(|item| inspect::center(item).x < left_edge)
        .ok_or_else(|| format!("no {label:?} expander in the left panel"))?;
    input::click_at(app.slint(), inspect::center(&expander));
    app.settle();
    Ok(())
}

fn ctrl_h(app: &mut App) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "h");
    app.settle();
    Ok(())
}

/// Hide and show dot-names with every control; returns the app after a restart that
/// starts with them hidden by the saved Options choice.
fn hidden_files(
    mut app: App,
    context: &mut Context,
    folder: &Path,
    all: &[String],
    store: &Arc<SettingsStore>,
    failures: &mut Vec<String>,
) -> Result<App, String> {
    let shown = visible_names(all);
    if !listed(&app).contains(&".hidden".to_owned())
        || !listed(&app).contains(&".hidden-folder".to_owned())
    {
        failures.push(format!(
            "dot-names are not listed by default: {:?}",
            listed(&app)
        ));
    }
    if let Some(count) = hidden_count(&app) {
        failures.push(format!(
            "the status reports {count} hidden while they are shown"
        ));
    }

    // Cursor and a mark on .hidden, another mark on a shown file.
    panel::select_row(&mut app, LEFT, ".hidden")?;
    input::key(app.slint(), Key::Insert);
    panel::select_row(&mut app, LEFT, "UPPER.TXT")?;
    input::key(app.slint(), Key::Insert);
    panel::select_row(&mut app, LEFT, ".hidden")?;
    app.until("both marks", |app| {
        support::selected_names(app, LEFT).len() == 2
    });
    let order = app.names(LEFT);
    let position = order.iter().position(|name| name == ".hidden").unwrap_or(0);
    let following = order
        .iter()
        .skip(position + 1)
        .find(|name| !name.starts_with('.'))
        .cloned();

    // Ctrl+H hides them.
    ctrl_h(&mut app)?;
    if !wait_listing(&mut app, &shown) {
        failures.push(format!(
            "Ctrl+H lists {:?}, expected {shown:?}",
            listed(&app)
        ));
    }
    app.until("the hidden count", |app| hidden_count(app).is_some());
    if hidden_count(&app) != Some(2) || footer_hidden(&app).as_deref() != Some("2 hidden") {
        failures.push(format!(
            "the status says {:?} and the status line {:?} after Ctrl+H",
            app.window.get_left_status(),
            footer_hidden(&app)
        ));
    }
    let cursor = support::cursor_name(&app, LEFT);
    if cursor.is_none() || cursor != following {
        failures.push(format!(
            "the cursor on .hidden moved to {cursor:?}, expected the next shown row {following:?}"
        ));
    }
    app.until("the hidden mark to drop", |app| {
        support::selected_names(app, LEFT) == ["UPPER.TXT"]
    });
    app.screenshot(context, "hidden-hidden", "Ctrl+H: dot-names are not shown");

    // Ctrl+A selects only shown files.
    input::chord(app.slint(), &[Key::Control], "a");
    let files: Vec<String> = shown
        .iter()
        .filter(|name| folder.join(name).is_file())
        .cloned()
        .collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut selected = Vec::new();
    while Instant::now() < deadline {
        selected = support::selected_names(&app, LEFT);
        selected.sort();
        if selected == files {
            break;
        }
        app.idle(Duration::from_millis(20));
    }
    if selected != files {
        failures.push(format!(
            "Ctrl+A with hidden files hidden selected {selected:?}"
        ));
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // An inline tree leaves a hidden child out and lists it once shown again.
    toggle_branch(&mut app, TREE_FOLDER)?;
    // Branches list folders: a shown and a hidden subfolder.
    app.until("the expanded branch", |app| {
        depth_of(app, TREE_SHOWN) == Some(1)
    });
    if depth_of(&app, TREE_SECRET).is_some() {
        failures.push("the expanded tree lists the hidden .secret".into());
    }
    app.screenshot(
        context,
        "hidden-tree",
        "Expanded folder without its dot-name",
    );

    // The View menu shows them again; the dropped mark stays dropped.
    input::click(
        app.slint(),
        AccessibleRole::Combobox,
        "Left panel view mode",
    )?;
    app.settle();
    input::click_button(app.slint(), SHOW_HIDDEN)?;
    app.until("the hidden child in the tree", |app| {
        depth_of(app, TREE_SECRET) == Some(1)
    });
    app.until(".hidden again", |app| {
        app.names(LEFT).contains(&".hidden".to_owned())
    });
    if hidden_count(&app).is_some() || footer_hidden(&app).is_some() {
        failures.push("the status still reports hidden entries after showing them".into());
    }
    if support::selected_names(&app, LEFT).contains(&".hidden".to_owned()) {
        failures.push("showing hidden files marked .hidden again".into());
    }
    toggle_branch(&mut app, TREE_FOLDER)?;
    app.until("the collapsed branch", |app| {
        depth_of(app, TREE_SHOWN).is_none()
    });

    // F10 hides them again.
    panel::focus_list(&mut app, LEFT)?;
    input::key(app.slint(), Key::F10);
    app.until("the main menu", |app| inspect::roots(app.slint()).len() > 1);
    support::click_scrolled(app.slint(), AccessibleRole::Button, "Hide hidden files")?;
    if !wait_listing(&mut app, &shown) {
        failures.push(format!("F10 Hide hidden files lists {:?}", listed(&app)));
    }
    support::dismiss_all(&mut app);
    ctrl_h(&mut app)?;
    if !wait_listing(&mut app, all) {
        failures.push(format!("a second Ctrl+H lists {:?}", listed(&app)));
    }

    // Options › General saves the choice; a restart starts with it.
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    if options::checked(&app, SHOW_HIDDEN) != Some(true) {
        failures.push(format!(
            "Options shows {SHOW_HIDDEN} as {:?}",
            options::checked(&app, SHOW_HIDDEN)
        ));
    }
    options::set_checked(&mut app, SHOW_HIDDEN, false)?;
    if !options::ok(&mut app, store)? {
        failures.push("OK did not save Show hidden files".into());
    }
    if store.current().show_hidden_files {
        failures.push("the saved settings still show hidden files".into());
    }
    if !wait_listing(&mut app, &shown) {
        failures.push(format!("after OK the panel lists {:?}", listed(&app)));
    }
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(folder, &context.work, store));
    if !wait_listing(&mut app, &shown) {
        failures.push(format!(
            "after a restart with hidden files hidden the panel lists {:?}",
            listed(&app)
        ));
    }
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    if options::checked(&app, SHOW_HIDDEN) != Some(false) {
        failures.push(format!(
            "after a restart Options shows {SHOW_HIDDEN} as {:?}",
            options::checked(&app, SHOW_HIDDEN)
        ));
    }
    options::set_checked(&mut app, SHOW_HIDDEN, true)?;
    options::ok(&mut app, store)?;
    if !wait_listing(&mut app, all) {
        failures.push(format!("showing them again lists {:?}", listed(&app)));
    }
    context.note(
        "hidden files: Ctrl+H (status 2 hidden, cursor to the next shown row, mark dropped, Ctrl+A shown files only, tree child left out), View menu, F10, Options saved across a restart",
    );
    Ok(app)
}

pub fn filter_and_search(context: &mut Context) -> Result<(), String> {
    let root = support::scratch(context, "filter")?;
    support::stage(context, &["names"], &root)?;
    let folder = root.join("names");
    for name in [TREE_SECRET, TREE_SHOWN] {
        let subfolder = folder.join(TREE_FOLDER).join(name);
        fs::create_dir(&subfolder).map_err(|error| io_error(&subfolder, error))?;
    }
    let all = support::names_on_disk(&folder);
    let store = SettingsStore::new();
    let app = App::start(config(&folder, &context.work, &store));
    let mut failures = Vec::new();
    let mut app = hidden_files(app, context, &folder, &all, &store, &mut failures)?;

    // Masks: *.txt;*.TXT keeps those files and every folder.
    apply_filter(&mut app, "Masks", "*.txt;*.TXT")?;
    let folders: Vec<String> = all
        .iter()
        .filter(|name| folder.join(name).is_dir())
        .cloned()
        .collect();
    let mut expected: Vec<String> = all
        .iter()
        .filter(|name| {
            folder.join(name).is_file() && (name.ends_with(".txt") || name.ends_with(".TXT"))
        })
        .cloned()
        .chain(folders.iter().cloned())
        .collect();
    expected.sort();
    if !wait_listing(&mut app, &expected) {
        failures.push(format!(
            "Masks *.txt;*.TXT lists {:?}, expected {expected:?}",
            listed(&app)
        ));
    }
    if app.window.get_left_filter_label().is_empty() {
        failures.push("an applied filter shows no indicator".into());
    }
    app.screenshot(context, "filter-masks", "Masks *.txt;*.TXT");

    // Masks with an exclusion: every file but the dot-names.
    apply_filter(&mut app, "Masks", "*.*|.*")?;
    let mut expected: Vec<String> = all
        .iter()
        .filter(|name| folder.join(name).is_dir() || !name.starts_with('.'))
        .cloned()
        .collect();
    expected.sort();
    if !wait_listing(&mut app, &expected) {
        failures.push(format!(
            "Masks *.*|.* lists {:?}, expected {expected:?}",
            listed(&app)
        ));
    }

    // Expression: hide dot-names, folders included.
    apply_filter(&mut app, "Expression", "not name like \".*\"")?;
    let expected = visible_names(&all);
    if !wait_listing(&mut app, &expected) {
        failures.push(format!(
            "Expression not name like \".*\" lists {:?}, expected {expected:?}",
            listed(&app)
        ));
    }
    app.screenshot(context, "filter-expression", "Expression hiding dot-names");

    // Show all removes the filter.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter dialog", |app| {
        app.window.get_panel_filter_view().open
    });
    input::click_button(app.slint(), "Show all")?;
    app.until("the filter dialog to close", |app| {
        !app.window.get_panel_filter_view().open
    });
    if !wait_listing(&mut app, &all) {
        failures.push(format!("Show all lists {:?}", listed(&app)));
    }

    // Filename search: a typed prefix moves to the first match, Down to the next,
    // Escape restores the listing.
    panel::focus_list(&mut app, LEFT)?;
    input::key(app.slint(), Key::Home);
    input::type_text(app.slint(), "lo");
    app.settle();
    let matches: Vec<String> = app
        .names(LEFT)
        .into_iter()
        .filter(|name| name.to_lowercase().starts_with("lo"))
        .collect();
    if !app.window.get_left_search_active() || app.window.get_left_search_query() != "lo" {
        failures.push(format!(
            "typing \"lo\" did not start a search (active {}, query {:?})",
            app.window.get_left_search_active(),
            app.window.get_left_search_query()
        ));
    }
    let first = support::cursor_name(&app, LEFT);
    if first.as_ref() != matches.first() {
        failures.push(format!(
            "search \"lo\" put the cursor on {first:?}, matches {matches:?}"
        ));
    }
    app.screenshot(context, "search-lo", "Filename search \"lo\"");
    input::key(app.slint(), Key::DownArrow);
    app.settle();
    if support::cursor_name(&app, LEFT).as_ref() != matches.get(1) {
        failures.push(format!(
            "Down during search went to {:?}, matches {matches:?}",
            support::cursor_name(&app, LEFT)
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.settle();
    if app.window.get_left_search_active() {
        failures.push("Escape did not leave the search".into());
    }
    if listed(&app) != all {
        failures.push("leaving the search did not restore the listing".into());
    }
    context.note(format!(
        "filter: Masks *.txt;*.TXT, Masks *.*|.*, Expression not name like \".*\", Show all; search \"lo\" matched {matches:?}"
    ));
    app.shutdown()?;
    support::report(&failures)
}
