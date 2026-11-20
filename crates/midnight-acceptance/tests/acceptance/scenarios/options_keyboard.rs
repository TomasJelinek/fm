//! Options › Keyboard: rebind Create folder from F7 to another key; the new key works
//! and the old one does nothing; an occupied key asks before replacing (declined, and
//! accepted then cancelled with Options); persistence across a restart; Restore
//! command brings the default back. The capture field records Tab and Escape; Ctrl+Space
//! and a fifth shortcut are refused with a reason.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::PaneId;
use slint::{SharedString, platform::Key};
use std::{fs, path::Path, sync::Arc, time::Duration};

const ACTION: &str = "files.create-directory";
const LABEL: &str = "Create folder";
const NEW_KEY: &str = "Ctrl+Alt+K";
const ALIASES: &str = "Shortcuts for selected command";
const CAPTURE: &str = "Press shortcut keys";

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

/// Press a key in the focused left list; return whether the Create directory dialog
/// opened (it is closed again with Escape).
fn opens_create_folder(
    app: &mut App,
    modifiers: &[Key],
    key: impl Into<SharedString>,
) -> Result<bool, String> {
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), modifiers, key);
    for _ in 0..20 {
        app.idle(Duration::from_millis(25));
        if !app.window.get_operation_token().is_empty() {
            let title = inspect::visible_texts(app.slint());
            input::key(app.slint(), Key::Escape);
            app.until("the dialog to close", |app| {
                app.window.get_operation_token().is_empty()
            });
            if !title.iter().any(|text| text.contains("Create")) {
                return Err(format!("the key opened another dialog: {title:?}"));
            }
            return Ok(true);
        }
        if !app.window.get_copy_draft_token().is_empty() {
            input::key(app.slint(), Key::Escape);
            app.until("the Copy dialog to close", |app| {
                app.window.get_copy_draft_token().is_empty()
            });
            return Ok(false);
        }
    }
    Ok(false)
}

/// Whether F5 opens the Copy dialog (closed again with Escape).
fn f5_opens_copy(app: &mut App) -> Result<bool, String> {
    panel::select_row(app, PaneId::Left, "item.txt")?;
    input::key(app.slint(), Key::F5);
    for _ in 0..20 {
        app.idle(Duration::from_millis(25));
        if !app.window.get_copy_draft_token().is_empty() {
            input::key(app.slint(), Key::Escape);
            app.until("the Copy dialog to close", |app| {
                app.window.get_copy_draft_token().is_empty()
            });
            return Ok(true);
        }
        if !app.window.get_operation_token().is_empty() {
            input::key(app.slint(), Key::Escape);
            app.until("the dialog to close", |app| {
                app.window.get_operation_token().is_empty()
            });
            return Ok(false);
        }
    }
    Ok(false)
}

/// Show the Create folder entry of the keyboard editor and select it.
fn select_entry(app: &mut App) -> Result<(), String> {
    options::page(app, "Keyboard")?;
    options::fill(app, "Search commands", LABEL)?;
    let entry = options::reveal(app, AccessibleRole::ListItem, LABEL)?;
    let id = entry
        .accessible_string_property(AccessibleStringProperty::Value)
        .map(|value| value.to_string());
    if id.as_deref() != Some(ACTION) {
        return Err(format!("{LABEL:?} is the action {id:?}, not {ACTION}"));
    }
    options::click_item(app, &entry)?;
    Ok(())
}

fn capture(
    app: &mut App,
    modifiers: &[Key],
    key: impl Into<SharedString>,
    shown: &str,
) -> Result<(), String> {
    let field = options::reveal(app, AccessibleRole::TextInput, CAPTURE)?;
    options::click_item(app, &field)?;
    input::chord(app.slint(), modifiers, key);
    app.idle(Duration::from_millis(60));
    let candidate = options::value(app, AccessibleRole::TextInput, CAPTURE);
    if candidate.as_deref() != Some(shown) {
        return Err(format!("pressing {shown} captured {candidate:?}"));
    }
    Ok(())
}

/// The text below "Shortcut currently used by".
/// The text shown directly below "Shortcut currently used by".
fn used_by(app: &App) -> String {
    let Some(caption) = options::find_any(app, AccessibleRole::Text, "Shortcut currently used by")
    else {
        return String::new();
    };
    let (x, y, _, height) = inspect::rect(&caption);
    inspect::controls(app.slint(), AccessibleRole::Text)
        .into_iter()
        .map(|(text, item)| (inspect::rect(&item), text))
        .filter(|((left, top, _, _), _)| *top >= y + height - 1.0 && (left - x).abs() < 4.0)
        .min_by(|a, b| a.0.1.total_cmp(&b.0.1))
        .map(|(_, text)| text)
        .unwrap_or_default()
}

fn replacement_open(app: &App) -> bool {
    inspect::find(app.slint(), AccessibleRole::Groupbox, "Replace shortcut").is_some()
}

pub fn keyboard(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("keyboard");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    fs::write(folder.join("item.txt"), "item\n").map_err(|error| error.to_string())?;
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    if !opens_create_folder(&mut app, &[], Key::F7)? {
        return Err("F7 does not open Create folder by default".into());
    }

    // The capture field records Tab and Escape instead of running them; a key reserved
    // for the input method and a fifth shortcut are refused with a reason. Cancel then
    // discards the whole session.
    options::open(&mut app)?;
    select_entry(&mut app)?;
    for (key, shown) in [(Key::Tab, "Tab"), (Key::Escape, "Escape")] {
        let field = options::reveal(&mut app, AccessibleRole::TextInput, CAPTURE)?;
        options::click_item(&mut app, &field)?;
        input::key(app.slint(), key);
        app.idle(Duration::from_millis(80));
        let candidate = options::value(&app, AccessibleRole::TextInput, CAPTURE);
        if candidate.as_deref() != Some(shown) || !app.window.get_options_open() {
            return Err(format!(
                "{shown} in the capture field captured {candidate:?} (Options open: {})",
                app.window.get_options_open()
            ));
        }
    }
    capture(&mut app, &[Key::Control], Key::Space, "Ctrl+Space")?;
    if options::is_enabled(&app, AccessibleRole::Button, "Assign") {
        options::press(&mut app, "Assign")?;
    }
    if options::texts_with(&app, "reserved").is_empty()
        || options::value(&app, AccessibleRole::Combobox, ALIASES).as_deref() != Some("F7")
    {
        return Err(format!(
            "Ctrl+Space was not refused as reserved: shortcuts {:?}, texts {:?}",
            options::value(&app, AccessibleRole::Combobox, ALIASES),
            options::texts_with(&app, "Ctrl+Space")
        ));
    }
    for (index, letter) in ["m", "n", "o", "q"].into_iter().enumerate() {
        let shown = format!("Ctrl+Alt+{}", letter.to_uppercase());
        capture(&mut app, &[Key::Control, Key::Alt], letter, &shown)?;
        if index < 3 {
            options::press(&mut app, "Assign")?;
            if options::texts_with(&app, "At most four").len() > 0 {
                return Err(format!("the shortcut number {} was refused", index + 2));
            }
        } else {
            if options::is_enabled(&app, AccessibleRole::Button, "Assign") {
                options::press(&mut app, "Assign")?;
            }
            if options::texts_with(&app, "At most four").is_empty() {
                return Err(format!(
                    "a fifth shortcut for {LABEL} was not refused; texts: {:?}",
                    options::texts_with(&app, "Ctrl+Alt")
                ));
            }
        }
    }
    let saves = store.saves();
    options::cancel(&mut app)?;
    if store.saves() != saves || !store.current().shortcuts.is_empty() {
        return Err("Cancel kept the shortcuts assigned in the editor".into());
    }
    if !opens_create_folder(&mut app, &[], Key::F7)? {
        return Err("after the cancelled session F7 does not open Create folder".into());
    }

    // An occupied key: the editor names its owner and asks before replacing; decline.
    options::open(&mut app)?;
    select_entry(&mut app)?;
    let aliases = options::value(&app, AccessibleRole::Combobox, ALIASES);
    if aliases.as_deref() != Some("F7") {
        return Err(format!("{LABEL} shows the shortcuts {aliases:?}, not F7"));
    }
    capture(&mut app, &[], Key::F5, "F5")?;
    let owner = used_by(&app);
    if owner.is_empty() || owner == "—" || owner == "Unassigned" {
        return Err(format!("F5 is not reported as used (used by {owner:?})"));
    }
    options::press(&mut app, "Assign")?;
    app.until("the Replace shortcut question", replacement_open);
    options::review_shot(
        &mut app,
        context,
        "keyboard-conflict",
        "Keyboard: F5 is taken, replace?",
    );
    options::press(&mut app, "Cancel")?;
    app.until("the question to close", |app| !replacement_open(app));
    if options::value(&app, AccessibleRole::Combobox, ALIASES).as_deref() != Some("F7") {
        return Err("declining the replacement changed the shortcuts".into());
    }

    // Rebind: remove F7, assign a free key, OK.
    options::press(&mut app, "Remove")?;
    let aliases = options::value(&app, AccessibleRole::Combobox, ALIASES);
    if aliases.as_deref().is_some_and(|value| value.contains("F7")) {
        return Err(format!("Remove left {aliases:?}"));
    }
    capture(&mut app, &[Key::Control, Key::Alt], "k", NEW_KEY)?;
    if used_by(&app) != "Unassigned" {
        return Err(format!(
            "{NEW_KEY} is reported as used by {:?}",
            used_by(&app)
        ));
    }
    options::press(&mut app, "Assign")?;
    let aliases = options::value(&app, AccessibleRole::Combobox, ALIASES);
    if aliases.as_deref() != Some(NEW_KEY) {
        return Err(format!("after Assign {LABEL} has {aliases:?}"));
    }
    options::review_shot(
        &mut app,
        context,
        "keyboard-rebound",
        "Keyboard: Create folder rebound",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save the shortcut".into());
    }
    let saved = store.current().shortcuts;
    if !saved
        .iter()
        .any(|binding| binding.action == ACTION && binding.keys == [NEW_KEY])
    {
        return Err(format!("the saved shortcuts are {saved:?}"));
    }
    if opens_create_folder(&mut app, &[], Key::F7)? {
        return Err("F7 still opens Create folder after the rebinding".into());
    }
    if !opens_create_folder(&mut app, &[Key::Control, Key::Alt], "k")? {
        return Err(format!("{NEW_KEY} does not open Create folder"));
    }

    // Accept a replacement, then Cancel Options: nothing changes.
    options::open(&mut app)?;
    select_entry(&mut app)?;
    capture(&mut app, &[], Key::F5, "F5")?;
    options::press(&mut app, "Assign")?;
    app.until("the Replace shortcut question", replacement_open);
    options::press(&mut app, "Replace")?;
    app.until("the question to close", |app| !replacement_open(app));
    let staged = options::value(&app, AccessibleRole::Combobox, ALIASES);
    if staged
        .as_deref()
        .is_none_or(|value| value != NEW_KEY && value != "F5")
    {
        return Err(format!("after Replace {LABEL} shows {staged:?}"));
    }
    options::cancel(&mut app)?;
    if store.current().shortcuts != saved {
        return Err("Cancel saved the replaced shortcut".into());
    }
    if !f5_opens_copy(&mut app)? {
        return Err("after Cancel F5 no longer opens Copy".into());
    }

    // Restart: the saved binding is still in effect.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    if opens_create_folder(&mut app, &[], Key::F7)? {
        return Err("after a restart F7 opens Create folder again".into());
    }
    if !opens_create_folder(&mut app, &[Key::Control, Key::Alt], "k")? {
        return Err(format!(
            "after a restart {NEW_KEY} does not open Create folder"
        ));
    }

    // Restore command: back to F7.
    options::open(&mut app)?;
    select_entry(&mut app)?;
    options::press(&mut app, "Restore command")?;
    let aliases = options::value(&app, AccessibleRole::Combobox, ALIASES);
    if aliases.as_deref() != Some("F7") {
        return Err(format!("Restore command left {aliases:?}"));
    }
    options::ok(&mut app, &store)?;
    if store
        .current()
        .shortcuts
        .iter()
        .any(|binding| binding.action == ACTION)
    {
        return Err("Restore command kept an override for Create folder".into());
    }
    if !opens_create_folder(&mut app, &[], Key::F7)? {
        return Err("after Restore command F7 does not open Create folder".into());
    }
    context.note(format!(
        "Keyboard: F5 reported as used by {owner:?}; replacement declined, then accepted and cancelled with Options; Create folder rebound F7 → {NEW_KEY}: the new key opens it, F7 does not, also after a restart; Restore command brought F7 back"
    ));
    app.shutdown()
}
