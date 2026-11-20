//! Options › General: every check box, the disk I/O picker and Clear command history,
//! each with its visible effect; Cancel discards, OK keeps, a restart on the same
//! settings shows them again (a cleared, unremembered history recalls nothing), Reset
//! to defaults restores them, Escape discards a preview and Enter saves.
use crate::{
    Context,
    app::{App, Config},
    input,
    options_support::{self as options, SettingsStore},
    panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CommandHistoryEntry, DiskIoConcurrency, PaneId, ShellFlavor, SizeDisplay};
use slint::platform::Key;
use std::{fs, path::Path, sync::Arc, time::Duration};

const BIG: &str = "alpine-big.bin";
const BIG_BYTES: usize = 2_345_678;
const SEARCH_FILTER: &str = "Show only filename search matches";
const SIZES_IN_BYTES: &str = "Always show file sizes in bytes";
const VERIFY_COPIES: &str = "Verify entire copied file with SHA-256 (recommended)";
const REMEMBER_HISTORY: &str = "Remember command history after restart";
const QUICK_VERIFICATION: &str = "Quick verification: size + first/middle/last 64 KiB";

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

/// The size column of `name` in the left panel.
fn size_of(app: &App, name: &str) -> Option<String> {
    app.rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == name)
        .map(|row| row.size.to_string())
}

/// Exact bytes: only the digits of BIG_BYTES with grouping, no unit.
fn shows_bytes(size: Option<String>) -> bool {
    size.is_some_and(|size| {
        !size.chars().any(|c| c.is_alphabetic())
            && size
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
                == BIG_BYTES.to_string()
    })
}

/// A readable size: a number with a unit.
fn shows_readable(size: Option<String>) -> bool {
    size.is_some_and(|size| size.contains("MiB") || size.contains("MB"))
}

/// Type a prefix into the focused panel and return the names it then shows; Escape
/// ends the search.
fn search(app: &mut App, prefix: &str) -> Result<(Vec<String>, Option<String>), String> {
    panel::focus_list(app, PaneId::Left)?;
    input::type_text(app.slint(), prefix);
    app.idle(Duration::from_millis(150));
    let names = app.names(PaneId::Left);
    let cursor = usize::try_from(app.cursor(PaneId::Left))
        .ok()
        .and_then(|row| names.get(row).cloned());
    input::key(app.slint(), Key::Escape);
    app.idle(Duration::from_millis(150));
    Ok((names, cursor))
}

/// Alt+Up in the command editor: the recalled command, if any (Escape returns to the
/// file list).
fn recall(app: &mut App) -> Result<Option<String>, String> {
    panel::focus_list(app, PaneId::Left)?;
    let window = &app.window;
    let position = slint::LogicalPosition::new(
        window.get_command_input_left() + window.get_command_input_width() / 2.0,
        window.get_command_input_top() + window.get_command_input_height() / 2.0,
    );
    input::click_at(app.slint(), position);
    app.until("the command editor to take focus", |app| {
        app.window.get_command_focused()
    });
    input::chord(app.slint(), &[Key::Alt], Key::UpArrow);
    app.idle(Duration::from_millis(150));
    let text = app.window.get_command_text().to_string();
    input::key(app.slint(), Key::Escape);
    app.idle(Duration::from_millis(100));
    Ok((!text.is_empty()).then_some(text))
}

fn history(commands: &[&str]) -> Vec<CommandHistoryEntry> {
    commands
        .iter()
        .map(|command| CommandHistoryEntry {
            command: (*command).to_owned(),
            shell: ShellFlavor::Posix,
        })
        .collect()
}

pub fn general(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("general");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    for name in ["alpha.txt", "beta.txt", "gamma.log"] {
        fs::write(folder.join(name), name).map_err(|error| error.to_string())?;
    }
    fs::write(folder.join(BIG), vec![7_u8; BIG_BYTES]).map_err(|error| error.to_string())?;
    let store = SettingsStore::seeded(|preferences| {
        preferences.command_history = history(&["echo first", "echo second"]);
    });
    let mut app = App::start(config(&folder, &store));

    // Defaults: readable sizes; typing jumps to a match without hiding other rows.
    if !shows_readable(size_of(&app, BIG)) {
        return Err(format!(
            "{BIG} does not show a readable size by default: {:?}",
            size_of(&app, BIG)
        ));
    }
    let (names, cursor) = search(&mut app, "ga")?;
    if !names.iter().any(|name| name == "beta.txt") || cursor.as_deref() != Some("gamma.log") {
        return Err(format!(
            "with the search filter off, typing \"ga\" should keep every row and put the cursor on gamma.log; rows {names:?}, cursor {cursor:?}"
        ));
    }

    // Cancel discards a previewed change.
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    options::review_shot(
        &mut app,
        context,
        "general-defaults",
        "Options › General, defaults",
    );
    options::set_checked(&mut app, SIZES_IN_BYTES, true)?;
    app.until("the panel to preview exact byte sizes", |app| {
        shows_bytes(size_of(app, BIG))
    });
    let saves = store.saves();
    options::cancel(&mut app)?;
    app.until("Cancel to restore readable sizes", |app| {
        shows_readable(size_of(app, BIG))
    });
    if store.saves() != saves || store.current().size_display != SizeDisplay::Readable {
        return Err(format!(
            "Cancel saved settings: {} saves, size display {:?}",
            store.saves() - saves,
            store.current().size_display
        ));
    }

    // Change every General option, then OK.
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    if options::checked(&app, SIZES_IN_BYTES) != Some(false) {
        return Err("Options reopened with the cancelled byte-size choice".into());
    }
    options::set_checked(&mut app, SEARCH_FILTER, true)?;
    options::set_checked(&mut app, SIZES_IN_BYTES, true)?;
    options::set_checked(&mut app, VERIFY_COPIES, false)?;
    if options::texts_with(&app, QUICK_VERIFICATION).is_empty() {
        return Err("turning full copy verification off shows no quick-verification note".into());
    }
    options::choose(&mut app, "Disk I/O concurrency", "Manual: 2")?;
    options::set_checked(&mut app, REMEMBER_HISTORY, false)?;
    if !options::is_enabled(&app, AccessibleRole::Button, "Clear command history") {
        return Err("Clear command history is disabled although history exists".into());
    }
    options::press(&mut app, "Clear command history")?;
    if options::is_enabled(&app, AccessibleRole::Button, "Clear command history") {
        return Err("Clear command history stays enabled after clearing".into());
    }
    options::review_shot(
        &mut app,
        context,
        "general-changed",
        "Options › General, every option changed",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK closed Options without saving the changed settings".into());
    }
    let saved = store.current();
    let expected = [
        ("quick search filter", saved.quick_search_filter, true),
        (
            "sizes in bytes",
            saved.size_display == SizeDisplay::Bytes,
            true,
        ),
        (
            "full copy verification",
            saved.copy_checksum_verification,
            false,
        ),
        (
            "disk I/O concurrency 2",
            saved.disk_io_concurrency == DiskIoConcurrency::Manual(2),
            true,
        ),
        (
            "remember command history",
            saved.remember_command_history,
            false,
        ),
        (
            "command history empty",
            saved.command_history.is_empty(),
            true,
        ),
    ];
    let wrong: Vec<_> = expected
        .iter()
        .filter(|(_, actual, wanted)| actual != wanted)
        .map(|(what, actual, _)| format!("{what}={actual}"))
        .collect();
    if !wrong.is_empty() {
        return Err(format!("OK saved unexpected General settings: {wrong:?}"));
    }

    // Their visible effect: exact sizes, and typing hides non-matching rows.
    if !shows_bytes(size_of(&app, BIG)) {
        return Err(format!(
            "after OK {BIG} shows {:?}, not exact bytes",
            size_of(&app, BIG)
        ));
    }
    let (names, _) = search(&mut app, "al")?;
    if names
        .iter()
        .any(|name| name == "beta.txt" || name == "gamma.log")
        || !names.iter().any(|name| name == "alpha.txt")
        || !names.iter().any(|name| name == BIG)
    {
        return Err(format!(
            "with the search filter on, typing \"al\" should show only alpha.txt and {BIG}; it shows {names:?}"
        ));
    }
    app.until("Escape to end the filtered search", |app| {
        app.names(PaneId::Left)
            .iter()
            .any(|name| name == "beta.txt")
    });
    app.screenshot(
        context,
        "general-effects",
        "After OK: exact byte sizes in the panel",
    );

    // Restart on the saved settings.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    if !shows_bytes(size_of(&app, BIG)) {
        return Err(format!(
            "after a restart {BIG} shows {:?}, not exact bytes",
            size_of(&app, BIG)
        ));
    }
    // The history was cleared and is not remembered: Alt+Up recalls nothing.
    if let Some(recalled) = recall(&mut app)? {
        return Err(format!(
            "after clearing and a restart Alt+Up recalls {recalled:?} in the command editor"
        ));
    }
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    let shown = [
        (
            SEARCH_FILTER,
            options::checked(&app, SEARCH_FILTER),
            Some(true),
        ),
        (
            SIZES_IN_BYTES,
            options::checked(&app, SIZES_IN_BYTES),
            Some(true),
        ),
        (
            VERIFY_COPIES,
            options::checked(&app, VERIFY_COPIES),
            Some(false),
        ),
        (
            REMEMBER_HISTORY,
            options::checked(&app, REMEMBER_HISTORY),
            Some(false),
        ),
    ];
    for (label, actual, wanted) in shown {
        if actual != wanted {
            return Err(format!(
                "after a restart {label:?} shows {actual:?} instead of {wanted:?}"
            ));
        }
    }
    let disk = options::value(&app, AccessibleRole::Combobox, "Disk I/O concurrency");
    if disk.as_deref() != Some("Manual: 2") {
        return Err(format!(
            "after a restart Disk I/O concurrency shows {disk:?}"
        ));
    }
    if options::is_enabled(&app, AccessibleRole::Button, "Clear command history") {
        return Err("after a restart Clear command history is enabled for an empty history".into());
    }
    options::review_shot(
        &mut app,
        context,
        "general-restarted",
        "Options › General after a restart",
    );

    // Reset to defaults, then OK: every General option returns to its default.
    options::press(&mut app, "Reset to defaults")?;
    let defaults = [
        (SEARCH_FILTER, Some(false)),
        (SIZES_IN_BYTES, Some(false)),
        (VERIFY_COPIES, Some(true)),
        (REMEMBER_HISTORY, Some(true)),
    ];
    for (label, wanted) in defaults {
        let actual = options::checked(&app, label);
        if actual != wanted {
            return Err(format!(
                "after Reset to defaults {label:?} shows {actual:?} instead of {wanted:?}"
            ));
        }
    }
    let disk = options::value(&app, AccessibleRole::Combobox, "Disk I/O concurrency");
    if disk.as_deref() != Some("Automatic (4)") {
        return Err(format!(
            "after Reset to defaults Disk I/O concurrency shows {disk:?}"
        ));
    }
    if !options::ok(&mut app, &store)? {
        return Err("OK after Reset to defaults saved nothing".into());
    }
    let saved = store.current();
    if saved.quick_search_filter
        || saved.size_display != SizeDisplay::Readable
        || !saved.copy_checksum_verification
        || saved.disk_io_concurrency != DiskIoConcurrency::Automatic
        || !saved.remember_command_history
    {
        return Err(format!(
            "Reset to defaults saved filter={} sizes={:?} verify={} disk={:?} remember={}",
            saved.quick_search_filter,
            saved.size_display,
            saved.copy_checksum_verification,
            saved.disk_io_concurrency,
            saved.remember_command_history
        ));
    }
    if saved.tabs[0].paths.first() != Some(&folder) {
        return Err(format!(
            "Reset to defaults lost the panel tabs: {:?}",
            saved.tabs[0].paths
        ));
    }
    app.until("readable sizes after the reset", |app| {
        shows_readable(size_of(app, BIG))
    });
    // Escape discards a previewed change like Cancel; Enter confirms like OK.
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    options::set_checked(&mut app, SIZES_IN_BYTES, true)?;
    app.until("the panel to preview exact byte sizes", |app| {
        shows_bytes(size_of(app, BIG))
    });
    let saves = store.saves();
    input::key(app.slint(), Key::Escape);
    app.until("Escape to close Options", |app| {
        !app.window.get_options_open()
    });
    app.until("Escape to restore readable sizes", |app| {
        shows_readable(size_of(app, BIG))
    });
    if store.saves() != saves || store.current().size_display != SizeDisplay::Readable {
        return Err("Escape saved the previewed byte sizes".into());
    }
    options::open(&mut app)?;
    input::key(app.slint(), Key::Return);
    app.until("Enter to close Options", |app| {
        !app.window.get_options_open()
    });
    if store.saves() != saves + 1 {
        return Err(format!(
            "Enter closed Options with {} saves instead of one",
            store.saves() - saves
        ));
    }
    context.note("General: Cancel discarded a previewed change; OK saved all six General settings (search filter, byte sizes, quick copy verification, disk I/O 2, history off, history cleared) with visible effects; a restart showed them and recalled no cleared history; Reset to defaults restored them and kept the tabs; Escape discarded a preview, Enter saved");
    app.shutdown()
}
