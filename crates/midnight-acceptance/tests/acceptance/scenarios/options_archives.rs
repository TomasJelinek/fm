//! Options › Archives: custom extension chips per archive format (add, case folding,
//! remove, invalid input, built-in conflicts), a built-in association check box,
//! Cancel, OK, persistence across a restart, and Escape in the extension field.
//!
//! Browsing a file with a custom extension needs the installed native filesystem
//! plugin, which this harness does not stage; the setting, its display and storage
//! are verified.
use crate::{
    Context,
    app::{App, Config},
    input,
    options_support::{self as options, SettingsStore},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::ArchiveFormat;
use slint::platform::Key;
use std::{collections::BTreeMap, fs, path::Path, sync::Arc, time::Duration};

const FIELD: &str = "Custom archive extension";
const ADD: &str = "Add custom archive extension";
const JAR: &str = ".jar · Java application";

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

/// The custom extension chips shown for the selected format.
fn chips(app: &App) -> Vec<String> {
    options::labels(app, AccessibleRole::ListItem)
        .into_iter()
        .filter(|label| {
            label.starts_with('.')
                && options::find_any(app, AccessibleRole::Button, &format!("Remove {label}"))
                    .is_some()
        })
        .collect()
}

/// Type an extension into the (opened) field and press Enter; returns the error shown.
fn submit(app: &mut App, text: &str) -> Result<String, String> {
    if options::find_any(app, AccessibleRole::TextInput, FIELD).is_none() {
        options::press(app, ADD)?;
        app.settle();
    }
    let field = options::reveal(app, AccessibleRole::TextInput, FIELD)?;
    options::click_item(app, &field)?;
    input::replace_text(app.slint(), text);
    input::key(app.slint(), Key::Return);
    app.idle(Duration::from_millis(60));
    Ok(error(app))
}

/// The extension field's error line (empty when none).
fn error(app: &App) -> String {
    options::texts(app)
        .into_iter()
        .find(|text| {
            text.starts_with("Enter an extension")
                || text.starts_with("Use a literal extension")
                || text.contains("is built into")
                || text.contains("already")
                || text.starts_with("An archive extension")
        })
        .unwrap_or_default()
}

pub fn archives(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("archives");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    fs::copy(
        context.fixture.path("archives").join("bundle.zip"),
        folder.join("backup.mybak"),
    )
    .or_else(|_| fs::write(folder.join("backup.mybak"), b"not an archive").map(|_| 0))
    .map_err(|error| error.to_string())?;
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    options::open(&mut app)?;
    options::page(&mut app, "Archives")?;
    options::choose(&mut app, "Archive format", "ZIP")?;
    let availability = options::texts_with(&app, "Decoder");
    if !chips(&app).is_empty() {
        return Err(format!("unexpected custom extensions {:?}", chips(&app)));
    }

    // A literal suffix, case-folded.
    let message = submit(&mut app, "MyBak")?;
    if !message.is_empty() || chips(&app) != [".mybak"] {
        return Err(format!(
            "adding MyBak shows chips {:?} and error {message:?}",
            chips(&app)
        ));
    }
    // Invalid input: each is refused with a reason and adds nothing.
    for (text, reason) in [
        ("bad ext", "Use a literal extension"),
        ("*.zip", "Use a literal extension"),
        ("dir/name", "Use a literal extension"),
        ("a..b", "Enter an extension"),
        ("mybak", "already"),
    ] {
        let message = submit(&mut app, text)?;
        if !message.contains(reason) {
            return Err(format!(
                "{text:?} was answered with {message:?}, expected {reason:?}; chips {:?}",
                chips(&app)
            ));
        }
        if chips(&app) != [".mybak"] {
            return Err(format!("{text:?} changed the chips to {:?}", chips(&app)));
        }
    }
    options::review_shot(
        &mut app,
        context,
        "archives-invalid",
        "Options › Archives refusing an extension",
    );
    // A built-in suffix points at its existing check box.
    let message = submit(&mut app, "zip")?;
    if !message.contains("is built into ZIP") {
        return Err(format!("adding zip answered {message:?}"));
    }
    options::press(&mut app, "Show built-in checkbox")?;
    if options::value(
        &app,
        AccessibleRole::TextInput,
        "Search archive associations",
    )
    .as_deref()
        != Some("zip")
        || options::checked(&app, ".zip · ZIP archive").is_none()
    {
        return Err("Show built-in checkbox did not show the .zip check box".into());
    }
    // A built-in association: .jar is off by default; turn it on.
    options::fill(&mut app, "Search archive associations", "jar")?;
    options::set_checked(&mut app, JAR, true)?;

    // Another format keeps its own list.
    options::choose(&mut app, "Archive format", "7-Zip")?;
    if !chips(&app).is_empty() {
        return Err(format!("7-Zip shows ZIP's chips {:?}", chips(&app)));
    }
    if !submit(&mut app, "pack7")?.is_empty() || chips(&app) != [".pack7"] {
        return Err(format!("adding pack7 to 7-Zip shows {:?}", chips(&app)));
    }
    // Remove: add a temporary suffix and remove it with its chip's button.
    if !submit(&mut app, "temp-ext")?.is_empty() {
        return Err("adding temp-ext failed".into());
    }
    options::press(&mut app, "Remove .temp-ext")?;
    if chips(&app) != [".pack7"] {
        return Err(format!("removing .temp-ext left {:?}", chips(&app)));
    }
    options::review_shot(
        &mut app,
        context,
        "archives-7zip",
        "Options › Archives: a 7-Zip custom extension",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save the archive settings".into());
    }
    let expected: BTreeMap<String, ArchiveFormat> = [
        ("mybak".to_owned(), ArchiveFormat::Zip),
        ("pack7".to_owned(), ArchiveFormat::SevenZip),
    ]
    .into();
    let saved = store.current();
    if saved.archive_custom_extensions != expected {
        return Err(format!(
            "saved custom extensions {:?}",
            saved.archive_custom_extensions
        ));
    }
    if saved.archive_association_overrides.get("jar") != Some(&true) {
        return Err(format!(
            "saved association overrides {:?}",
            saved.archive_association_overrides
        ));
    }

    // Cancel discards a removal.
    options::open(&mut app)?;
    options::page(&mut app, "Archives")?;
    options::choose(&mut app, "Archive format", "ZIP")?;
    options::press(&mut app, "Remove .mybak")?;
    if !chips(&app).is_empty() {
        return Err("Remove .mybak did not remove the chip".into());
    }
    options::cancel(&mut app)?;
    if store.current().archive_custom_extensions != expected {
        return Err("Cancel did not keep .mybak".into());
    }

    // Restart: both formats show their saved chips.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    options::open(&mut app)?;
    options::page(&mut app, "Archives")?;
    options::choose(&mut app, "Archive format", "ZIP")?;
    if chips(&app) != [".mybak"] {
        return Err(format!("after a restart ZIP shows {:?}", chips(&app)));
    }
    options::fill(&mut app, "Search archive associations", "jar")?;
    if options::checked(&app, JAR) != Some(true) {
        return Err("after a restart .jar is not checked".into());
    }
    options::choose(&mut app, "Archive format", "7-Zip")?;
    if chips(&app) != [".pack7"] {
        return Err(format!("after a restart 7-Zip shows {:?}", chips(&app)));
    }
    context.note(format!(
        "Archives: MyBak stored as .mybak (ZIP), pack7 (7-Zip); spaces, wildcards, paths, empty parts and duplicates refused with reasons; zip pointed to its built-in check box; .jar association enabled; Cancel kept a removed chip; a restart showed all. Availability: {availability:?}. Browsing backup.mybak needs the native filesystem plugin (not staged here)"
    ));

    // Last, because it ends the Options session: the field's hint says Escape cancels
    // the entry, which must not close Options and discard its other changes.
    options::choose(&mut app, "Archive format", "ZIP")?;
    options::press(&mut app, ADD)?;
    input::type_text(app.slint(), "draft");
    app.settle();
    let typed = options::value(&app, AccessibleRole::TextInput, FIELD);
    if typed.as_deref() != Some("draft") {
        return Err(format!(
            "the opened extension field holds {typed:?}, not the typed text"
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.idle(Duration::from_millis(100));
    if !app.window.get_options_open() {
        return Err("finding OPT-ESC: Escape in Options › Archives › Custom archive extension (whose hint says \"Press Enter to add it, or Escape to cancel\") closes the whole Options dialog and discards its unsaved changes instead of cancelling the entry".into());
    }
    if options::find_any(&app, AccessibleRole::TextInput, FIELD).is_some() {
        return Err("Escape left the extension field open".into());
    }
    options::cancel(&mut app)?;
    app.shutdown()
}
