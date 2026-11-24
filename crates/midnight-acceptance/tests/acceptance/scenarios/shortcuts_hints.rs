//! What the windows say about the shortcuts, compared with the catalog: F1 keyboard help
//! lists every action with all of its keys ("Unassigned" without one) and its context;
//! the F10 main menu and the panel's View and Sort menus show every key of each item's
//! action in binding order, as F1 lists them ("Ctrl+= / Num + / Ctrl++" beside Select by
//! mask, "Delete / F8" beside Move to Trash, nothing beside Options); after Create folder
//! is rebound and Sort by size gets a second key in Options
//! › Keyboard, the menus, the help and the footer show the new keys at once, F7's cell
//! is unassigned, and the new key opens the dialog.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
    tools_support::{self as tools},
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::{PaneId, ShortcutChord, shortcut_catalog};
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    collections::BTreeMap,
    fs,
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const HELP: &str = "Keyboard help · Midnight Explorer";
const NEW_KEY: &str = "Ctrl+Alt+K";
/// A second key for Sort by size.
const SIZE_KEY: &str = "Ctrl+Alt+Z";

/// The main menu's items and their actions (main-menu.slint), in display order.
const MENU: [(&str, &str); 66] = [
    ("Open", "panel.open"),
    ("View", "files.view"),
    ("View as plain text", "files.view-basic"),
    ("Edit", "files.edit"),
    ("Copy to…", "files.copy-to"),
    ("Move to…", "files.move-to"),
    ("Rename…", "files.rename"),
    ("Create folder…", "files.create-directory"),
    ("Create empty file…", "files.create-file"),
    ("Move to Trash…", "files.trash"),
    ("Delete permanently…", "files.delete"),
    ("Copy to clipboard", "files.copy"),
    ("Cut to clipboard", "files.cut"),
    ("Paste files", "files.paste"),
    ("Attributes, times and case…", "files.attributes"),
    ("Change name case…", "files.case"),
    ("System properties", "files.properties"),
    ("Select files and folders", "selection.all"),
    ("Select files only", "selection.files"),
    ("Deselect files and folders", "selection.clear"),
    ("Invert files only", "selection.invert"),
    ("Invert files and folders", "selection.invert-all"),
    ("Select by mask…", "selection.mask-select"),
    ("Deselect by mask…", "selection.mask-deselect"),
    (
        "Invert, folders as in Select by mask",
        "selection.mask-invert",
    ),
    ("Refresh active folder", "panel.refresh"),
    ("Parent folder", "panel.parent"),
    ("Filesystem root", "panel.root"),
    ("Back", "panel.back"),
    ("Forward", "panel.forward"),
    ("Enter path…", "panel.path"),
    ("As other panel", "panel.as-other"),
    ("Swap panels", "panel.swap"),
    ("Enlarge / restore active panel", "panel.zoom"),
    ("Filter panel…", "panel.filter"),
    ("Hide hidden files", "panel.hidden-files"),
    ("Brief view", "view.brief"),
    ("Detailed view", "view.detailed"),
    ("Thumbnail view", "view.thumbnails"),
    ("Sort by name", "sort.name"),
    ("Sort by extension", "sort.extension"),
    ("Sort by size", "sort.size"),
    ("Sort by modified time", "sort.modified"),
    ("New tab", "tab.new"),
    ("Close tab", "tab.close"),
    ("Add Hot Path", "bookmark.add"),
    ("Manage Hot Paths…", "bookmark.manage"),
    ("Left panel drives and locations", "drive.left"),
    ("Right panel drives and locations", "drive.right"),
    ("Find files…", "files.find"),
    ("Checksums…", "files.checksums"),
    ("Make File List…", "files.make-list"),
    ("Compare files…", "files.compare"),
    ("Compare directories…", "panel.compare"),
    ("Clear comparison marks", "panel.compare-clear"),
    ("Calculate / cancel folder sizes", "panel.folder-sizes"),
    ("Drive information", "panel.drive-info"),
    ("Command line", "command.focus"),
    ("Configure Custom Commands…", "app.custom-commands"),
    ("Connect network drive…", "network.connect"),
    ("Disconnect network drive…", "network.disconnect"),
    ("Options", "app.options"),
    ("Jobs", "app.jobs"),
    ("Keyboard help", "app.help"),
    ("About Midnight Explorer", "app.about"),
    ("Exit", "app.exit"),
];

/// Keys the main menu must show, written out (the documented spellings): every key of
/// the item's action, in binding order.
const ANCHORS: [(&str, &str); 9] = [
    ("Select by mask…", "Ctrl+= / Num + / Ctrl++"),
    ("Deselect by mask…", "Ctrl+- / Num -"),
    ("Invert, folders as in Select by mask", "Num *"),
    ("Create folder…", "F7"),
    ("Move to Trash…", "Delete / F8"),
    ("Brief view", "Alt+2"),
    ("Find files…", "Ctrl+F / Alt+F7"),
    (
        "Calculate / cancel folder sizes",
        "Ctrl+Shift+S / Ctrl+Shift+F10",
    ),
    ("Options", ""),
];

/// How the windows spell a key: modifiers in the order Ctrl, Alt, Shift, Meta; digits
/// without "Digit"; punctuation as its character; keypad keys as "Num +".
fn shown(key: &str) -> Result<String, String> {
    let chord = ShortcutChord::parse(key).map_err(|error| format!("{key}: {error}"))?;
    let name = match chord.key.as_str() {
        "Period" => ".".to_owned(),
        "Comma" => ",".to_owned(),
        "Plus" => "+".to_owned(),
        "Minus" => "-".to_owned(),
        "Equal" => "=".to_owned(),
        "BracketLeft" => "[".to_owned(),
        "BracketRight" => "]".to_owned(),
        "Backslash" => "\\".to_owned(),
        "Slash" => "/".to_owned(),
        "Semicolon" => ";".to_owned(),
        "Quote" => "'".to_owned(),
        "Backquote" => "`".to_owned(),
        "NumpadAdd" => "Num +".to_owned(),
        "NumpadSubtract" => "Num -".to_owned(),
        "NumpadMultiply" => "Num *".to_owned(),
        other => other.strip_prefix("Digit").unwrap_or(other).to_owned(),
    };
    let mut parts: Vec<&str> = Vec::new();
    for (on, modifier) in [
        (chord.control, "Ctrl"),
        (chord.alt, "Alt"),
        (chord.shift, "Shift"),
        (chord.meta, "Meta"),
    ] {
        if on {
            parts.push(modifier);
        }
    }
    parts.push(&name);
    Ok(parts.join("+"))
}

/// What a menu shows for `keys`: every key in binding order, as F1 lists them, joined
/// with " / "; nothing for an unassigned action.
fn all_keys(keys: &[String]) -> Result<String, String> {
    Ok(keys
        .iter()
        .map(|key| shown(key))
        .collect::<Result<Vec<_>, _>>()?
        .join(" / "))
}

fn wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("{what} did not happen"));
        }
        app.idle(Duration::from_millis(10));
    }
}

/// F1's help text: the shortcut lines of every action.
fn help_text(app: &mut App) -> Result<String, String> {
    panel::focus_list(app, LEFT)?;
    let window = tools::open_tool(app, "the Help window", &[], Key::F1, (900, 640));
    if tools::title(&window) != HELP {
        return Err(format!("F1 opened {:?}", tools::title(&window)));
    }
    let text = inspect::visible_texts(window.window()).join("\n");
    tools::close_tool(app, &window, "the Help window");
    Ok(text)
}

/// Every main menu item's (label, shown key), scrolled through from the top.
fn menu_keys(app: &mut App) -> Result<BTreeMap<String, String>, String> {
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::F10);
    wait(app, "the main menu", |app| tools::popup_open(app.slint()))?;
    let main = app.main.clone();
    let anchor = inspect::roots(main.window())
        .last()
        .and_then(|menu| {
            let mut found = None;
            inspect::walk(menu, &mut |item| {
                if found.is_none()
                    && item.accessible_role() == AccessibleRole::Button
                    && item.is_visible()
                    && inspect::label(item).is_some_and(|label| label != "Close menu")
                {
                    found = Some(item.clone());
                }
            });
            found
        })
        .ok_or("the main menu shows no item")?;
    let clip = inspect::clip_rect(&anchor).unwrap_or_else(|| inspect::rect(&anchor));
    let position = LogicalPosition::new(clip.0 + clip.2 / 2.0, clip.1 + clip.3 / 2.0);
    let scroll = |app: &mut App, delta: f32| {
        main.window().dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: delta,
        });
        app.settle();
        inspect::settle_tree(main.window());
    };
    for _ in 0..20 {
        scroll(app, 300.0);
    }
    let mut keys = BTreeMap::new();
    let mut quiet = 0;
    while quiet < 3 && keys.len() < 200 {
        let before = keys.len();
        if let Some(menu) = inspect::roots(main.window()).last() {
            inspect::walk(menu, &mut |item| {
                if item.accessible_role() == AccessibleRole::Button
                    && item.is_visible()
                    && let Some(label) = inspect::label(item)
                    && label != "Close menu"
                {
                    // The chords follow the label: one text beside it, or under it one per
                    // line when they do not fit beside it.
                    let texts = tools::item_texts(item);
                    let key = match texts.iter().position(|text| *text == label) {
                        Some(at) => texts[at + 1..].join(" / "),
                        None => texts.last().cloned().unwrap_or_default(),
                    };
                    keys.entry(label).or_insert(key);
                }
            });
        }
        quiet = if keys.len() == before { quiet + 1 } else { 0 };
        scroll(app, -60.0);
    }
    input::key(app.slint(), Key::Escape);
    wait(app, "the main menu to close", |app| {
        !tools::popup_open(app.slint())
    })?;
    Ok(keys)
}

/// The left panel's View and Sort menus (panel-view-controls.slint): each row's label
/// and the action whose keys it shows. The Sort menu's "Reverse order" shows the keys
/// of the current sort column (Name here).
const PANEL_MENUS: [(&str, &[(&str, &str)]); 2] = [
    (
        "Left panel view mode",
        &[
            ("Detailed", "view.detailed"),
            ("Brief", "view.brief"),
            ("Thumbnails", "view.thumbnails"),
            ("Hide hidden files", "panel.hidden-files"),
        ],
    ),
    (
        "Left panel sort",
        &[
            ("Name", "sort.name"),
            ("Size", "sort.size"),
            ("Modified", "sort.modified"),
            ("Extension", "sort.extension"),
            ("Reverse order", "sort.name"),
        ],
    ),
];

/// Every row of the panel menu labelled `menu`: (label, the key text beside it), read
/// from its open list (opened with a click, closed with Escape).
fn panel_menu_keys(app: &mut App, menu: &str) -> Result<Vec<(String, String)>, String> {
    panel::focus_list(app, LEFT)?;
    let popups = inspect::roots(app.slint()).len();
    input::click(app.slint(), AccessibleRole::Combobox, menu)?;
    wait(app, &format!("the {menu:?} list"), move |app| {
        inspect::roots(app.slint()).len() > popups
    })?;
    let mut rows: Vec<(f32, String, String)> = Vec::new();
    if let Some(list) = inspect::roots(app.slint()).last() {
        inspect::walk(list, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && item.is_visible()
                && let Some(label) = inspect::label(item)
            {
                let key = tools::item_texts(item)
                    .into_iter()
                    .rev()
                    .find(|text| *text != label && text != "✓")
                    .unwrap_or_default();
                rows.push((inspect::rect(item).1, label, key));
            }
        });
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    input::key(app.slint(), Key::Escape);
    wait(app, &format!("the {menu:?} list to close"), move |app| {
        inspect::roots(app.slint()).len() == popups
    })?;
    Ok(rows
        .into_iter()
        .map(|(_, label, key)| (label, key))
        .collect())
}

/// The panel menus against `keys_of(action)`: every row shows all of its action's keys,
/// as F1 lists them ("Ctrl+F3 / …"), or nothing for an unassigned action.
fn compare_panel_menus(
    app: &mut App,
    keys_of: &dyn Fn(&str) -> Vec<String>,
    when: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    for (menu, rows) in PANEL_MENUS {
        let shown_rows = panel_menu_keys(app, menu)?;
        // The hidden-files row names what it will do: Hide or Show.
        let labels: Vec<&str> = shown_rows
            .iter()
            .map(|(label, _)| match label.as_str() {
                "Show hidden files" => "Hide hidden files",
                other => other,
            })
            .collect();
        let wanted_labels: Vec<&str> = rows.iter().map(|(label, _)| *label).collect();
        if labels != wanted_labels {
            failures.push(format!(
                "{when}: the {menu:?} list shows {labels:?} instead of {wanted_labels:?}"
            ));
            continue;
        }
        for ((label, key), (_, action)) in shown_rows.iter().zip(rows.iter()) {
            let wanted = all_keys(&keys_of(action))?;
            if *key != wanted {
                failures.push(format!(
                    "{when}: the {menu:?} list shows {key:?} beside {label:?}, expected {wanted:?}"
                ));
            }
        }
    }
    Ok(())
}

/// The footer cell for an unmodified function key: (label, enabled).
fn footer_cell(app: &App, key: &str) -> Option<(String, bool)> {
    let prefix = format!("{key} ");
    inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, _)| label == key || label.starts_with(&prefix))
        .map(|(label, item)| (label, tools::enabled(&item)))
}

/// Check the help, the menu and the footer against `keys_of(action)`.
fn compare(
    app: &mut App,
    keys_of: &dyn Fn(&str) -> Vec<String>,
    when: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let help = help_text(app)?;
    let mut missing = Vec::new();
    for definition in shortcut_catalog() {
        let keys = keys_of(&definition.id);
        let listed = if keys.is_empty() {
            "Unassigned".to_owned()
        } else {
            keys.iter()
                .map(|key| shown(key))
                .collect::<Result<Vec<_>, _>>()?
                .join(" / ")
        };
        let line = format!("{} — {listed}\n", definition.label);
        if !help.contains(&line) {
            missing.push(line.trim_end().to_owned());
        }
    }
    if !missing.is_empty() {
        failures.push(format!(
            "{when}: F1 help lacks {} of {} shortcut lines: {missing:?}",
            missing.len(),
            shortcut_catalog().len()
        ));
    }
    let menu = menu_keys(app)?;
    for (label, action) in MENU {
        let label = if action == "panel.hidden-files" && !menu.contains_key(label) {
            "Show hidden files"
        } else {
            label
        };
        let Some(key) = menu.get(label) else {
            failures.push(format!("{when}: the main menu has no {label:?}"));
            continue;
        };
        let wanted = all_keys(&keys_of(action))?;
        if *key != wanted {
            failures.push(format!(
                "{when}: the main menu shows {key:?} beside {label:?}, expected {wanted:?}"
            ));
        }
    }
    Ok(())
}

fn defaults(action: &str) -> Vec<String> {
    shortcut_catalog()
        .iter()
        .find(|definition| definition.id == action)
        .map(|definition| definition.defaults.clone())
        .unwrap_or_default()
}

fn opens_create(
    app: &mut App,
    modifiers: &[Key],
    key: impl Into<slint::SharedString>,
) -> Result<bool, String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), modifiers, key);
    for _ in 0..20 {
        app.idle(Duration::from_millis(25));
        if !app.window.get_operation_token().is_empty() {
            let title = app.window.get_operation_title().to_string();
            input::key(app.slint(), Key::Escape);
            wait(app, "the dialog to close", |app| {
                app.window.get_operation_token().is_empty()
            })?;
            return Ok(title.contains("Create"));
        }
    }
    Ok(false)
}

pub fn hints(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("hints");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    fs::write(folder.join("item.txt"), "item\n").map_err(|error| error.to_string())?;
    let store = SettingsStore::new();
    let mut config = Config::new(&folder, &folder);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let mut failures: Vec<String> = Vec::new();

    // The anchors, then every item and every help line against the catalog.
    let menu = menu_keys(&mut app)?;
    for (label, wanted) in ANCHORS {
        if menu.get(label).map(String::as_str) != Some(wanted) {
            failures.push(format!(
                "the main menu shows {:?} beside {label:?}, expected {wanted:?}",
                menu.get(label)
            ));
        }
    }
    compare(&mut app, &defaults, "defaults", &mut failures)?;
    compare_panel_menus(&mut app, &defaults, "defaults", &mut failures)?;
    let cell = footer_cell(&app, "F7");
    if !cell
        .as_ref()
        .is_some_and(|(label, enabled)| label.starts_with("F7 ") && *enabled)
    {
        failures.push(format!("the footer's F7 cell is {cell:?}"));
    }

    // Rebind Create folder from F7 to Ctrl+Alt+K in Options › Keyboard.
    options::open(&mut app)?;
    options::page(&mut app, "Keyboard")?;
    options::fill(&mut app, "Search commands", "Create folder")?;
    let entry = options::reveal(&mut app, AccessibleRole::ListItem, "Create folder")?;
    options::click_item(&mut app, &entry)?;
    options::press(&mut app, "Remove")?;
    let field = options::reveal(&mut app, AccessibleRole::TextInput, "Press shortcut keys")?;
    options::click_item(&mut app, &field)?;
    input::chord(app.slint(), &[Key::Control, Key::Alt], "k");
    app.idle(Duration::from_millis(60));
    options::press(&mut app, "Assign")?;
    let aliases = options::value(
        &app,
        AccessibleRole::Combobox,
        "Shortcuts for selected command",
    );
    if aliases.as_deref() != Some(NEW_KEY) {
        return Err(format!("after Assign Create folder has {aliases:?}"));
    }
    // Sort by size gets a second key, which its panel menu row must list too.
    options::fill(&mut app, "Search commands", "Sort by size")?;
    let entry = options::reveal(&mut app, AccessibleRole::ListItem, "Sort by size")?;
    options::click_item(&mut app, &entry)?;
    let field = options::reveal(&mut app, AccessibleRole::TextInput, "Press shortcut keys")?;
    options::click_item(&mut app, &field)?;
    input::chord(app.slint(), &[Key::Control, Key::Alt], "z");
    app.idle(Duration::from_millis(60));
    let captured = options::value(&app, AccessibleRole::TextInput, "Press shortcut keys");
    if captured.as_deref() != Some(SIZE_KEY) {
        return Err(format!("pressing {SIZE_KEY} captured {captured:?}"));
    }
    options::press(&mut app, "Assign")?;
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save the rebinding".into());
    }
    let rebound = |action: &str| match action {
        "files.create-directory" => vec![NEW_KEY.to_owned()],
        "sort.size" => {
            let mut keys = defaults(action);
            keys.push(SIZE_KEY.to_owned());
            keys
        }
        _ => defaults(action),
    };
    compare(
        &mut app,
        &rebound,
        "after rebinding Create folder",
        &mut failures,
    )?;
    compare_panel_menus(
        &mut app,
        &rebound,
        "after adding a key to Sort by size",
        &mut failures,
    )?;
    match footer_cell(&app, "F7") {
        Some((label, enabled)) if !enabled && !label.contains("Create") => {}
        other => failures.push(format!(
            "after rebinding Create folder the footer's F7 cell is {other:?}, expected an unassigned, disabled F7"
        )),
    }
    let entry = inspect::find(app.slint(), AccessibleRole::Button, "F7 — unavailable");
    if entry.is_none_or(|item| {
        item.accessible_string_property(AccessibleStringProperty::Enabled)
            .is_some_and(|value| value == "true")
    }) {
        failures.push("the footer does not mark F7 unavailable after the rebinding".into());
    }
    if !opens_create(&mut app, &[Key::Control, Key::Alt], "k")? {
        failures.push(format!("{NEW_KEY} does not open Create folder"));
    }
    if opens_create(&mut app, &[], Key::F7)? {
        failures.push("F7 still opens Create folder".into());
    }
    app.screenshot(
        context,
        "rebound-footer",
        "The footer after Create folder moved to Ctrl+Alt+K",
    );
    app.shutdown()?;
    if failures.is_empty() {
        context.note(format!("F1 help, the F10 menu and the left panel's View and Sort menus matched the catalog before and after rebinding Create folder to {NEW_KEY} and adding {SIZE_KEY} to Sort by size (the Sort menu listing both keys); the footer followed"));
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
