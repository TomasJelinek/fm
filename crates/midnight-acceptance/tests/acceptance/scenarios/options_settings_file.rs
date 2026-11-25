//! Options' Export and Import of the configuration file, through the scripted file
//! chooser and the Engine's own settings-file workers:
//!
//! - Export writes what OK would save now ("Exported settings to <path>."), a preview
//!   included; exporting to an existing file asks Replace or Keep: Keep leaves the file
//!   byte for byte, Replace writes the new settings and keeps the replaced file for
//!   recovery.
//! - Import previews the file's settings at once ("Imported <name>. … Cancel keeps your
//!   current settings."): Cancel restores the current ones and saves nothing; OK saves
//!   them and applies the file's Hot Paths (a digit opens the imported one).
//! - Refused files leave the settings unchanged with the reason: not JSON, no settings
//!   version, a newer version.
//! - A file written on the other operating system: its Hot Path and its editor are kept
//!   inactive and named; the Hot Path's number says why it does not open, the Hot Paths
//!   manager lists and removes it, and General lists the editor, which Remove and OK drop.
//!
//! Registered here, not in the registry's list: this module owns its entries.
use crate::{
    Context, Scenario,
    app::{App, Config},
    options_support::{self as options, SettingsStore},
    panel,
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::{Bookmark, BookmarkId, PaneId, PreferenceTheme, SizeDisplay};
use midnight_ui::HotPathsPresentation;
use slint::ComponentHandle;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const EXPORT: &str = "Export configuration…";
const IMPORT: &str = "Import configuration…";
const SIZES_IN_BYTES: &str = "Always show file sizes in bytes";
/// A hang guard for a settings-file transfer and its message.
const TRANSFER: Duration = Duration::from_secs(20);

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "options_settings_file",
        isolation: crate::Isolation::Parallel,
        area: "options",
        summary: "Options › Export and Import configuration: the exported file, Replace and Keep for an existing one, the import preview, Cancel and OK (Hot Paths applied), refused files (not JSON, no version, newer), a file from the other operating system (its Hot Path and editor kept inactive, listed and removed).",
        run: settings_file,
        timeout: Duration::from_secs(8 * 60),
    }]
}

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

fn bookmark(id: u64, name: &str, path: &Path, slot: u8) -> Bookmark {
    Bookmark {
        id: BookmarkId(id),
        name: name.into(),
        path: path.to_path_buf(),
        archive: None,
        network: None,
        slot: Some(slot),
    }
}

/// The settings-file line above the Options footer: (text, is an error), if shown.
fn feedback(app: &App) -> Option<(String, bool)> {
    for (label, error) in [
        ("Settings file status", false),
        ("Settings file error", true),
    ] {
        if let Some(text) = options::find_any(app, AccessibleRole::Text, label)
            .and_then(|item| item.accessible_string_property(AccessibleStringProperty::Value))
            .map(|value| value.to_string())
            .filter(|value| !value.is_empty())
        {
            return Some((text, error));
        }
    }
    None
}

/// Wait for the settings-file line to satisfy `wanted`; an error quotes what it shows.
fn wait_feedback(
    app: &mut App,
    what: &str,
    wanted: impl Fn(&str, bool) -> bool,
) -> Result<String, String> {
    if options::poll(app, TRANSFER, |app| {
        feedback(app).is_some_and(|(text, error)| wanted(&text, error))
    }) {
        Ok(feedback(app).map(|(text, _)| text).unwrap_or_default())
    } else {
        Err(format!(
            "{what}: the settings-file line shows {:?}",
            feedback(app)
        ))
    }
}

/// Press Export or Import with the chooser's answer queued first, so that no file is
/// ever chosen outside this scenario's folder; the press must reach the chooser once.
fn choose(app: &mut App, button: &str, path: &Path) -> Result<(), String> {
    let requests = app.picker.requests().len();
    app.picker.answer(Some(path.to_path_buf()));
    options::press(app, button)?;
    if !options::poll(app, TRANSFER, |app| app.picker.requests().len() > requests) {
        return Err(format!("{button} did not ask the file chooser"));
    }
    let asked = app.picker.requests().len() - requests;
    if asked != 1 {
        return Err(format!("{button} asked the file chooser {asked} times"));
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// A path as a JSON string literal's content.
fn json_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// The settings document with its `"version"` member replaced by `version` (or
/// removed).
fn with_version(text: &str, version: Option<&str>) -> Result<String, String> {
    let start = text
        .find("\"version\":")
        .ok_or("the exported settings have no \"version\" member")?;
    let end = text[start..]
        .find(',')
        .map(|offset| start + offset + 1)
        .ok_or("the exported settings' version member does not end")?;
    Ok(match version {
        Some(version) => format!("{}\"version\": {version},{}", &text[..start], &text[end..]),
        None => {
            // Drop the member and the whitespace before the next one.
            let rest = text[end..].trim_start();
            format!("{}{rest}", &text[..start])
        }
    })
}

/// An absolute folder path as the other operating system writes one.
fn other_system_path(name: &str) -> String {
    if cfg!(windows) {
        format!("/home/tester/{name}")
    } else {
        format!("C:\\\\Users\\\\tester\\\\{name}")
    }
}

/// The folder the left panel shows.
fn left(app: &App) -> Option<PathBuf> {
    app.location(PaneId::Left)
}

/// The Hot Paths manager is open.
fn hot_paths_open(app: &App) -> bool {
    !app.window
        .global::<HotPathsPresentation>()
        .get_token()
        .is_empty()
}

fn settings_file(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("settings-file");
    let panel_folder = root.join("panel");
    let hot = root.join("hot-current");
    let imported_hot = root.join("hot-imported");
    let files = root.join("files");
    for folder in [&panel_folder, &hot, &imported_hot, &files] {
        fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    write(&panel_folder.join("item.txt"), "item\n")?;

    // The file to import later: exported by another profile with the Light theme, a Hot
    // Path 4 to `hot-imported` and a fallback editor (a harmless stand-in, never run).
    let editor = crate::tool::program(context, "true")?;
    let other_store = {
        let imported_hot = imported_hot.clone();
        let editor = editor.clone();
        SettingsStore::seeded(move |preferences| {
            preferences.dark_theme = PreferenceTheme::PlumSmoke;
            preferences.light_theme = PreferenceTheme::Light;
            preferences.bookmarks = vec![bookmark(1, "Imported", &imported_hot, 4)];
            preferences.editor.executable = Some(editor);
            preferences.editor.arguments = vec!["{file}".into()];
        })
    };
    let light = files.join("light.json");
    {
        let mut app = App::start(config(&panel_folder, &other_store));
        options::open(&mut app)?;
        choose(&mut app, EXPORT, &light)?;
        wait_feedback(&mut app, "exporting the Light profile", |text, error| {
            !error && text.starts_with("Exported settings to")
        })?;
        options::cancel(&mut app)?;
        app.shutdown()?;
    }
    let light_text = read(&light)?;
    if !light_text.contains(&json_path(&imported_hot)) {
        return Err(format!(
            "the exported Light profile does not name its Hot Path {}",
            imported_hot.display()
        ));
    }

    // This profile: the Dark theme and a Hot Path 3 to `hot-current`.
    let store = {
        let hot = hot.clone();
        SettingsStore::seeded(move |preferences| {
            preferences.dark_theme = PreferenceTheme::Dark;
            preferences.bookmarks = vec![bookmark(1, "Current", &hot, 3)];
        })
    };
    let mut app = App::start(config(&panel_folder, &store));
    let dark = app.window.get_theme_index();
    let saves = store.saves();

    // Export: what OK would save now, a previewed change included.
    let exported = files.join("settings.json");
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    options::set_checked(&mut app, SIZES_IN_BYTES, true)?;
    choose(&mut app, EXPORT, &exported)?;
    let message = wait_feedback(&mut app, "Export", |text, error| {
        !error && text.starts_with("Exported settings to")
    })?;
    let first_text = read(&exported)?;
    if !message.contains("settings.json") || !first_text.contains(&json_path(&hot)) {
        return Err(format!(
            "Export said {message:?} and wrote {} bytes without the Hot Path {}",
            first_text.len(),
            hot.display()
        ));
    }
    if !first_text.contains("\"size_display\": \"bytes\"") {
        return Err(format!(
            "the export does not hold the previewed byte sizes: {}",
            first_text
                .lines()
                .filter(|line| line.contains("size"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "settings-exported",
        "Options after Export configuration",
    );

    // An existing destination asks: Keep leaves it alone.
    choose(&mut app, EXPORT, &exported)?;
    wait_feedback(&mut app, "Export onto an existing file", |text, error| {
        !error && text == "settings.json already exists. Replace it?"
    })?;
    options::review_shot(
        &mut app,
        context,
        "settings-replace-question",
        "Export onto an existing file: Replace or Keep",
    );
    options::press(&mut app, "Keep")?;
    wait_feedback(&mut app, "Keep", |text, _| {
        text == "settings.json was kept. Settings were not exported."
    })?;
    if read(&exported)? != first_text {
        return Err("Keep changed the existing settings file".into());
    }
    // Replace writes the new settings and keeps the replaced file for recovery.
    options::set_checked(&mut app, SIZES_IN_BYTES, false)?;
    choose(&mut app, EXPORT, &exported)?;
    wait_feedback(&mut app, "the second export onto the file", |text, _| {
        text.ends_with("Replace it?")
    })?;
    options::press(&mut app, "Replace")?;
    let replaced = wait_feedback(&mut app, "Replace", |text, error| {
        !error && text.starts_with("Exported settings to")
    })?;
    let second_text = read(&exported)?;
    if second_text == first_text || !second_text.contains("\"size_display\": \"readable\"") {
        return Err("Replace did not write the current settings (readable sizes)".into());
    }
    if let Some(start) = replaced.find("The replaced file was kept as ") {
        let recovery = replaced[start + "The replaced file was kept as ".len()..]
            .trim_end_matches('.')
            .to_owned();
        if read(Path::new(&recovery))? != first_text {
            return Err(format!(
                "the recovery copy {recovery} does not hold the replaced settings"
            ));
        }
    } else {
        context.note(format!(
            "Replace kept no recovery copy (the filesystem may not support it): {replaced:?}"
        ));
    }
    options::cancel(&mut app)?;
    if store.saves() != saves {
        return Err("exporting saved the settings".into());
    }

    // Import: a preview of the Light profile; Cancel keeps the Dark one.
    options::open(&mut app)?;
    choose(&mut app, IMPORT, &light)?;
    let preview = wait_feedback(&mut app, "Import", |text, error| {
        !error && text.starts_with("Imported light.json.")
    })?;
    if !preview.contains("Cancel keeps your current settings") {
        return Err(format!("the import preview says {preview:?}"));
    }
    if !options::poll(&mut app, Duration::from_secs(5), move |app| {
        app.window.get_theme_index() == PreferenceTheme::PlumSmoke.index()
    }) {
        return Err("the import did not preview Plum Smoke for the dark host".into());
    }
    options::review_shot(
        &mut app,
        context,
        "settings-import-preview",
        "Import configuration: the Light profile previewed",
    );
    options::cancel(&mut app)?;
    if !options::poll(&mut app, Duration::from_secs(5), move |app| {
        app.window.get_theme_index() == dark
    }) {
        return Err("Cancel did not restore the Dark theme".into());
    }
    let current = store.current();
    if store.saves() != saves
        || current.dark_theme != PreferenceTheme::Dark
        || current.light_theme != PreferenceTheme::DEFAULT_LIGHT
    {
        return Err("Cancel after the import saved settings".into());
    }

    // Import and OK save both appearances and Hot Path 4. Digit 4 opens it.
    options::open(&mut app)?;
    choose(&mut app, IMPORT, &light)?;
    wait_feedback(&mut app, "the second import", |text, error| {
        !error && text.starts_with("Imported light.json.")
    })?;
    if !options::ok(&mut app, &store)? {
        return Err("OK after the import saved nothing".into());
    }
    let saved = store.current();
    if saved.dark_theme != PreferenceTheme::PlumSmoke
        || saved.light_theme != PreferenceTheme::Light
        || !saved
            .bookmarks
            .iter()
            .any(|mark| mark.path == imported_hot && mark.slot == Some(4))
    {
        return Err(format!(
            "OK after the import saved dark {:?}, light {:?} and Hot Paths {:?}",
            saved.dark_theme, saved.light_theme, saved.bookmarks
        ));
    }
    if saved.size_display != SizeDisplay::Readable {
        return Err("the import kept the cancelled byte sizes".into());
    }
    // The number row's 4, as the native key hook delivers it.
    panel::focus_list(&mut app, PaneId::Left)?;
    if !crate::scenarios::panels_support::physical(&mut app, "Digit4", false, false, false) {
        return Err("no shortcut took the number row's 4 (Hot Path 4)".into());
    }
    let wanted = imported_hot.clone();
    if !options::poll(&mut app, Duration::from_secs(10), move |app| {
        left(app).as_deref() == Some(wanted.as_path())
    }) {
        return Err(format!(
            "digit 4 opened {:?}, not the imported Hot Path {}",
            left(&app),
            imported_hot.display()
        ));
    }

    // Refused files change nothing and say why.
    let saves = store.saves();
    let refusals = [
        (
            "not-json.json",
            "theme = 'dark'\n".to_owned(),
            "not a JSON settings document",
        ),
        (
            "no-version.json",
            with_version(&second_text, None)?,
            "has no settings version",
        ),
        (
            "newer.json",
            with_version(&second_text, Some("999"))?,
            "settings version 999 from a newer Midnight Explorer",
        ),
    ];
    options::open(&mut app)?;
    for (name, text, reason) in &refusals {
        let path = files.join(name);
        write(&path, text)?;
        choose(&mut app, IMPORT, &path)?;
        let message = wait_feedback(&mut app, name, |text, error| {
            error && text.starts_with(&format!("Could not import {name}:"))
        })?;
        if !message.contains(reason) || !message.ends_with("Your settings are unchanged.") {
            return Err(format!(
                "importing {name} says {message:?}, without {reason:?}"
            ));
        }
    }
    options::review_shot(
        &mut app,
        context,
        "settings-import-refused",
        "Import configuration: a refused file",
    );
    options::cancel(&mut app)?;
    let current = store.current();
    if store.saves() != saves
        || current.dark_theme != PreferenceTheme::PlumSmoke
        || current.light_theme != PreferenceTheme::Light
    {
        return Err("a refused import changed the settings".into());
    }

    // A file from the other operating system: its Hot Path and its editor stay inactive.
    // The Hot Path is listed in the Hot Paths manager with a note, its number says why
    // it does not open, and it can be removed there; the editor is listed on General and
    // removed there.
    let foreign = files.join("other-system.json");
    let other_hot = other_system_path("projects");
    let other_editor = other_system_path("editor");
    let foreign_text = light_text
        .replace(&json_path(&imported_hot), &other_hot)
        .replace(&json_path(&editor), &other_editor);
    if !foreign_text.contains(&other_hot) || !foreign_text.contains(&other_editor) {
        return Err("the Light profile's Hot Path and editor could not be rewritten".into());
    }
    write(&foreign, &foreign_text)?;
    options::open(&mut app)?;
    choose(&mut app, IMPORT, &foreign)?;
    let system = if cfg!(windows) { "Linux" } else { "Windows" };
    let message = wait_feedback(&mut app, "the other system's file", |text, error| {
        !error && text.starts_with("Imported other-system.json.")
    })?;
    if !message.contains(&format!(
        "Kept inactive because they use {system} paths: 1 hot path and the editor."
    )) {
        return Err(format!(
            "the other system's file was imported with {message:?}"
        ));
    }
    if !options::ok(&mut app, &store)? {
        return Err("OK after the other system's file saved nothing".into());
    }
    let saved = store.current();
    if saved.foreign_hot_paths.len() != 1
        || saved.foreign_settings.len() != 1
        || saved.bookmarks.iter().any(|mark| mark.slot == Some(4))
    {
        return Err(format!(
            "the other system's file was saved with the inactive Hot Paths {:?}, the inactive settings {:?} and the Hot Paths {:?}",
            saved.foreign_hot_paths, saved.foreign_settings, saved.bookmarks
        ));
    }
    // Its number does not open the folder of another system: a notice says why.
    panel::focus_list(&mut app, PaneId::Left)?;
    let before = left(&app);
    crate::scenarios::panels_support::physical(&mut app, "Digit4", false, false, false);
    let (title, body) = options::wait_notice(&mut app, "the inactive Hot Path's number")?;
    if left(&app) != before || !format!("{title} {body}").contains("projects") {
        return Err(format!(
            "the inactive Hot Path's number moved the panel to {:?} or reported {title:?}: {body:?}",
            left(&app)
        ));
    }
    options::dismiss_notice(&mut app)?;
    // The Hot Paths manager lists it with a note, and removes it.
    panel::focus_list(&mut app, PaneId::Left)?;
    options::pick(&mut app, "Hot Paths", "Manage Hot Paths…")?;
    app.until("the Hot Paths manager", hot_paths_open);
    let shown = other_hot.replace("\\\\", "\\");
    let remove = options::labels(&app, AccessibleRole::Button)
        .into_iter()
        .find(|label| label.starts_with("Remove Hot Path: ") && label.contains(&shown))
        .ok_or_else(|| {
            format!(
                "the Hot Paths manager lists no inactive {shown}; buttons {:?}",
                options::labels(&app, AccessibleRole::Button)
            )
        })?;
    options::review_shot(
        &mut app,
        context,
        "settings-kept-hot-path",
        "Hot Paths: a Hot Path kept inactive from the other system",
    );
    options::press(&mut app, &remove)?;
    options::press(&mut app, "Close")?;
    app.until("the Hot Paths manager to close", |app| !hot_paths_open(app));
    let watched = store.clone();
    if !options::poll(&mut app, Duration::from_secs(5), move |_| {
        watched.current().foreign_hot_paths.is_empty()
    }) {
        return Err(format!(
            "removing the inactive Hot Path left {:?} saved",
            store.current().foreign_hot_paths
        ));
    }
    // General lists the inactive editor; Remove, then OK, drops it.
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    let removes: Vec<String> = options::labels(&app, AccessibleRole::Button)
        .into_iter()
        .filter(|label| label.starts_with("Remove "))
        .collect();
    let shown_editor = other_editor.replace("\\\\", "\\");
    let kept_rows = options::texts_with(&app, &shown_editor).len();
    if removes.len() != 1 || kept_rows == 0 {
        return Err(format!(
            "General lists the inactive settings {removes:?} ({kept_rows} rows naming {shown_editor})"
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "settings-kept-inactive",
        "Options › General: an editor kept from the other system",
    );
    options::press(&mut app, &removes[0])?;
    if !options::poll(&mut app, Duration::from_secs(3), |app| {
        !options::labels(app, AccessibleRole::Button)
            .iter()
            .any(|label| label.starts_with("Remove "))
    }) {
        return Err(format!("{} left the inactive editor listed", removes[0]));
    }
    if !options::ok(&mut app, &store)? || !store.current().foreign_settings.is_empty() {
        return Err(format!(
            "OK after removing the inactive editor saved {:?}",
            store.current().foreign_settings
        ));
    }
    context.note(format!(
        "Settings file: Export wrote the previewed byte sizes and the Hot Path ({message:?} for the other system's file); Keep left the existing file byte for byte; Replace wrote the current settings and kept the replaced file for recovery; Import previewed Plum Smoke on a dark host, Cancel restored Dark and saved nothing, OK saved both Plum Smoke and Light and Hot Path 4, which digit 4 opened; not JSON, no version and a newer version were refused with their reasons; a file from the other system kept its Hot Path and its editor inactive: the Hot Path's number reported why it does not open, the Hot Paths manager listed and removed it, General listed the editor and Remove then OK dropped it"
    ));
    app.shutdown()
}
