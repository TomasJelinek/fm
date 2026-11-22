//! Options › General: every check box, the disk I/O picker and Clear command history,
//! each with its visible effect; Cancel discards, OK keeps, a restart on the same
//! settings shows them again (a cleared, unremembered history recalls nothing, while the
//! seeded history of the first start is recalled), Reset to defaults restores them,
//! Escape discards a preview and Enter saves. Options' page list by keyboard: Shift+Tab
//! reaches the list from OK, Space and Enter choose the focused page, Up and Down walk the
//! pages in their displayed order (the focus follows), the last page shown reopens in the
//! same session and a restart opens General; Enter on a focused combo box opens its list
//! and neither confirms nor saves Options.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, PAGES, SettingsStore},
    panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{
    Bookmark, BookmarkId, CommandHistoryEntry, DiskIoConcurrency, PaneId, ShellFlavor, SizeDisplay,
    UserCommand, UserCommandId, UserCommandInvocation, UserCommandLaunch,
};
use slint::platform::Key;
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const BIG: &str = "alpine-big.bin";
const BIG_BYTES: usize = 2_345_678;
const SEARCH_FILTER: &str = "Show only filename search matches";
const SIZES_IN_BYTES: &str = "Always show file sizes in bytes";
const VERIFY_COPIES: &str = "Verify entire copied file with SHA-256 (recommended)";
const REMEMBER_HISTORY: &str = "Remember command history after restart";
const QUICK_VERIFICATION: &str = "Quick verification: size + first/middle/last 64 KiB";
const DISK_IO: &str = "Disk I/O concurrency";
/// The command line's history picker: enabled exactly when there is history to recall.
const HISTORY_PICKER: &str = "Executed command history";
/// The newest command of the seeded history, which Alt+Up recalls first: a saved history
/// lists the newest command first.
const NEWEST_COMMAND: &str = "echo first";
/// How long Alt+Up is tried: a hang guard for a recall (it shows within a frame), the
/// observation window for none.
const RECALL_WINDOW: Duration = Duration::from_secs(2);
/// A hang guard for a filename search to be applied.
const SEARCH_LIMIT: Duration = Duration::from_secs(5);

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

/// The name under the left panel's cursor.
fn cursor_name(app: &App) -> Option<String> {
    usize::try_from(app.cursor(PaneId::Left))
        .ok()
        .and_then(|row| app.names(PaneId::Left).get(row).cloned())
}

/// Type a prefix into the focused panel and return the names it shows once the search
/// is applied (its query shown, nothing pending, the cursor on a match) and the rows are
/// what `expected` takes, or when [`SEARCH_LIMIT`] has passed (the caller then reports
/// the rows); Escape ends the search, and the full listing must come back.
fn search(
    app: &mut App,
    prefix: &str,
    expected: impl Fn(&[String]) -> bool,
) -> Result<(Vec<String>, Option<String>), String> {
    panel::focus_list(app, PaneId::Left)?;
    let before = app.names(PaneId::Left);
    input::type_text(app.slint(), prefix);
    let wanted = prefix.to_lowercase();
    let is_applied = |app: &App| {
        app.window.get_left_search_active()
            && app.window.get_left_search_query().to_lowercase() == wanted
            && !app.window.get_left_search_pending()
            && cursor_name(app).is_some_and(|name| name.to_lowercase().starts_with(&wanted))
    };
    options::poll(app, SEARCH_LIMIT, |app| {
        is_applied(app) && expected(&app.names(PaneId::Left))
    });
    let applied = is_applied(app);
    let names = app.names(PaneId::Left);
    let cursor = cursor_name(app);
    if !applied {
        return Err(format!(
            "typing {prefix:?} did not apply a filename search within {SEARCH_LIMIT:?}: active {}, query {:?}, pending {}, cursor {cursor:?}, rows {names:?}",
            app.window.get_left_search_active(),
            app.window.get_left_search_query(),
            app.window.get_left_search_pending()
        ));
    }
    input::key(app.slint(), Key::Escape);
    if !options::poll(app, SEARCH_LIMIT, |app| {
        !app.window.get_left_search_active() && app.names(PaneId::Left) == before
    }) {
        return Err(format!(
            "Escape did not end the search on {prefix:?}: active {}, rows {:?} instead of {before:?}",
            app.window.get_left_search_active(),
            app.names(PaneId::Left)
        ));
    }
    Ok((names, cursor))
}

/// Alt+Up in the command editor, pressed until it recalls a command or for
/// [`RECALL_WINDOW`]: the recalled command, if any. A recalled draft is emptied again
/// and Escape returns to the file list.
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
    if !app.window.get_command_text().is_empty() {
        return Err(format!(
            "the command editor starts with the draft {:?}",
            app.window.get_command_text()
        ));
    }
    let deadline = Instant::now() + RECALL_WINDOW;
    let mut text = String::new();
    while text.is_empty() && Instant::now() < deadline {
        input::chord(app.slint(), &[Key::Alt], Key::UpArrow);
        app.idle(Duration::from_millis(20));
        text = app.window.get_command_text().to_string();
    }
    if !text.is_empty() {
        input::replace_text(app.slint(), "");
        app.settle();
    }
    input::key(app.slint(), Key::Escape);
    app.until("Escape to return to the file list", |app| {
        !app.window.get_command_focused()
    });
    Ok((!text.is_empty()).then_some(text))
}

/// The title of a topic identity.
fn title(topic: i32) -> &'static str {
    PAGES
        .iter()
        .find(|(_, id)| *id == topic)
        .map_or("?", |(name, _)| name)
}

/// Whether `label` names an Options page.
fn is_page(label: &str) -> bool {
    PAGES.iter().any(|(name, _)| *name == label)
}

/// The focused topic row, if a topic row has the focus.
fn focused_page(app: &App) -> Option<String> {
    options::focus(app)
        .filter(|(role, label)| *role == AccessibleRole::Button && is_page(label))
        .map(|(_, label)| label)
}

/// Press `key` on the focused topic row and expect page `topic` shown with its row
/// focused and checked (Up and Down move the selection and the focus together).
fn step_page(app: &mut App, key: Key, topic: i32, failures: &mut Vec<String>) {
    input::key(app.slint(), key);
    let reached = options::poll(app, Duration::from_secs(2), |app| {
        app.window.get_options_topic_index() == topic
            && focused_page(app).as_deref() == Some(title(topic))
    });
    if !reached {
        failures.push(format!(
            "{key:?} on the page list should show {} (topic {topic}) with its row focused; topic {}, focus {:?}",
            title(topic),
            app.window.get_options_topic_index(),
            options::focus(app)
        ));
    }
    let checked = options::find_any(app, AccessibleRole::Button, title(topic)).and_then(|row| {
        row.accessible_string_property(
            i_slint_core::accessibility::AccessibleStringProperty::Checked,
        )
    });
    if checked.as_deref() != Some("true") {
        failures.push(format!(
            "the {} row is not marked as the shown page (checked {checked:?})",
            title(topic)
        ));
    }
}

/// Options' page list and pickers by keyboard, from OK (where Options puts the focus):
/// see the module comment. Leaves Options closed on a page other than General.
fn navigation(app: &mut App, store: &SettingsStore, context: &mut Context) -> Result<(), String> {
    let mut failures = Vec::new();
    let saves = store.saves();

    // Enter on a focused combo box opens its list; it neither confirms nor saves.
    options::open(app)?;
    options::page(app, "General")?;
    if focused_page(app).is_none() {
        // A click on a topic row gives that row the keyboard focus, as Tab would.
        let row = options::reveal(app, AccessibleRole::Button, "General")?;
        options::click_item(app, &row)?;
    }
    options::tab_until(app, false, 40, DISK_IO, &|role, label| {
        role == AccessibleRole::Combobox && label == DISK_IO
    })?;
    let shown = options::value(app, AccessibleRole::Combobox, DISK_IO);
    let popups = inspect::roots(app.slint()).len();
    input::key(app.slint(), Key::Return);
    let opened = options::poll(app, Duration::from_secs(2), |app| {
        inspect::roots(app.slint()).len() > popups
    });
    if !opened {
        failures.push(format!(
            "Enter on the focused {DISK_IO:?} combo box opened no list"
        ));
    }
    if !app.window.get_options_open() || store.saves() != saves {
        return Err(format!(
            "Enter on the focused {DISK_IO:?} combo box closed Options ({}) or saved ({} saves)",
            !app.window.get_options_open(),
            store.saves() - saves
        ));
    }
    if opened {
        input::key(app.slint(), Key::Escape);
        if !options::poll(app, Duration::from_secs(2), |app| {
            inspect::roots(app.slint()).len() == popups
        }) {
            failures.push(format!("Escape left the {DISK_IO:?} list open"));
        }
        if !app.window.get_options_open() {
            return Err(format!(
                "Escape in the open {DISK_IO:?} list closed Options as well"
            ));
        }
    }
    let after = options::value(app, AccessibleRole::Combobox, DISK_IO);
    if after != shown {
        failures.push(format!(
            "opening and closing the {DISK_IO:?} list with Enter and Escape changed it from {shown:?} to {after:?}"
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.until("Escape to close Options", |app| {
        !app.window.get_options_open()
    });

    // Reopened (on General, the page shown last), the focus is on OK; Shift+Tab walks
    // back through the page into the list and reaches its last row first.
    options::open(app)?;
    if app.window.get_options_topic_index() != 0 {
        failures.push(format!(
            "Options reopened on topic {} instead of General, the page shown last",
            app.window.get_options_topic_index()
        ));
    }
    if options::focus(app) != Some((AccessibleRole::Button, "OK".to_owned())) {
        failures.push(format!(
            "Options opened with the focus on {:?}, not on OK",
            options::focus(app)
        ));
    }
    let visited = options::tab_until(app, true, 60, "a row of the page list", &|role, label| {
        role == AccessibleRole::Button && is_page(label)
    })?;
    let last = PAGES[PAGES.len() - 1].0;
    if focused_page(app).as_deref() != Some(last) {
        failures.push(format!(
            "Shift+Tab from OK should reach the list at its last row, {last}; it went {visited:?}"
        ));
    }
    if app.window.get_options_topic_index() != 0 {
        failures.push("moving the focus into the page list changed the page".to_owned());
    }
    // Space chooses the focused row's page; Shift+Tab moves to the row above without
    // choosing it; Enter chooses that one and keeps Options open.
    let space_topic = PAGES[PAGES.len() - 1].1;
    input::key(app.slint(), Key::Space);
    if !options::poll(app, Duration::from_secs(2), |app| {
        app.window.get_options_topic_index() == space_topic
    }) {
        failures.push(format!(
            "Space on the focused {last} row shows topic {} instead",
            app.window.get_options_topic_index()
        ));
    }
    input::chord(app.slint(), &[Key::Shift], Key::Tab);
    app.settle();
    let (above, enter_topic) = PAGES[PAGES.len() - 2];
    if focused_page(app).as_deref() != Some(above)
        || app.window.get_options_topic_index() != space_topic
    {
        failures.push(format!(
            "Shift+Tab from the {last} row should focus the {above} row without choosing it; focus {:?}, topic {}",
            options::focus(app),
            app.window.get_options_topic_index()
        ));
    }
    input::key(app.slint(), Key::Return);
    if !options::poll(app, Duration::from_secs(2), |app| {
        app.window.get_options_topic_index() == enter_topic
    }) {
        failures.push(format!(
            "Enter on the focused {above} row shows topic {} instead",
            app.window.get_options_topic_index()
        ));
    }
    if !app.window.get_options_open() || store.saves() != saves {
        return Err(format!(
            "Enter on a page row closed Options ({}) or saved ({} saves) instead of choosing the page",
            !app.window.get_options_open(),
            store.saves() - saves
        ));
    }
    // Up walks to General in the displayed order and stops there; Down walks every
    // page down to the last and stops there.
    let position = PAGES.len() - 2;
    for index in (0..position).rev() {
        step_page(app, Key::UpArrow, PAGES[index].1, &mut failures);
    }
    step_page(app, Key::UpArrow, PAGES[0].1, &mut failures);
    let order: Vec<i32> = PAGES.iter().skip(1).map(|(_, topic)| *topic).collect();
    if order != [1, 8, 6, 7, 4, 10, 9, 3, 5] {
        failures.push(format!("the expected page order changed: {order:?}"));
    }
    for topic in order {
        step_page(app, Key::DownArrow, topic, &mut failures);
    }
    step_page(app, Key::DownArrow, space_topic, &mut failures);
    if !app.window.get_options_open() || store.saves() != saves {
        return Err("walking the page list closed or saved Options".to_owned());
    }
    // Recorded, not judged: Up on a row that has the focus but is not the shown page
    // (Shift+Tab from the shown last page to the one above it).
    input::chord(app.slint(), &[Key::Shift], Key::Tab);
    app.settle();
    let focused = focused_page(app);
    input::key(app.slint(), Key::UpArrow);
    app.settle();
    context.note(format!(
        "Options page list: Up on the focused {focused:?} row while {last} is shown shows {} with the focus on {:?}",
        title(app.window.get_options_topic_index()),
        focused_page(app)
    ));
    let shown_last = app.window.get_options_topic_index();
    if shown_last == 0 {
        return Err("the navigation must leave a page other than General shown".to_owned());
    }

    // Escape closes Options without saving; it reopens on the page shown last.
    input::key(app.slint(), Key::Escape);
    app.until("Escape to close Options", |app| {
        !app.window.get_options_open()
    });
    options::open(app)?;
    if app.window.get_options_topic_index() != shown_last {
        failures.push(format!(
            "Options reopened on topic {} instead of {} ({shown_last}), the page shown last",
            app.window.get_options_topic_index(),
            title(shown_last)
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.until("Escape to close Options", |app| {
        !app.window.get_options_open()
    });
    if store.saves() != saves {
        failures.push(format!(
            "navigating Options saved {} times",
            store.saves() - saves
        ));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("Options navigation:\n{}", failures.join("\n")))
    }
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
    // Seeded too: a Hot Path and a Custom Command, which Reset to defaults must keep.
    let hot = folder.clone();
    let never_run = folder.join("never-run");
    let store = SettingsStore::seeded(move |preferences| {
        preferences.command_history = history(&["echo first", "echo second"]);
        preferences.bookmarks = vec![Bookmark {
            id: BookmarkId(1),
            name: "General".into(),
            path: hot,
            archive: None,
            network: None,
            slot: Some(2),
        }];
        preferences.user_commands = Arc::new(vec![UserCommand {
            id: UserCommandId::new("kept").expect("a valid command id"),
            name: "Kept command".into(),
            enabled: true,
            toolbar_label: None,
            icon: None,
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            // Never run here: this scenario installs no launcher.
            invocation: UserCommandInvocation::Program {
                executable: never_run,
                arguments: Vec::new(),
            },
        }]);
    });
    let mut app = App::start(config(&folder, &store));

    // The first start loads the seeded history as a restart loads remembered history:
    // the history picker is enabled and Alt+Up recalls the newest command. This is the
    // positive control of the "nothing recalled" check after the second restart.
    if options::enabled_state(&app, AccessibleRole::Combobox, HISTORY_PICKER) != Some(true) {
        return Err(format!(
            "with a remembered history the {HISTORY_PICKER:?} picker is {:?} instead of enabled",
            options::enabled_state(&app, AccessibleRole::Combobox, HISTORY_PICKER)
        ));
    }
    let recalled = recall(&mut app)?;
    if recalled.as_deref() != Some(NEWEST_COMMAND) {
        return Err(format!(
            "with a remembered history Alt+Up recalls {recalled:?} instead of {NEWEST_COMMAND:?}"
        ));
    }

    // Defaults: readable sizes; typing jumps to a match without hiding other rows.
    if !shows_readable(size_of(&app, BIG)) {
        return Err(format!(
            "{BIG} does not show a readable size by default: {:?}",
            size_of(&app, BIG)
        ));
    }
    let every_row = app.names(PaneId::Left);
    let (names, cursor) = search(&mut app, "ga", |names| names == every_row.as_slice())?;
    if names != every_row || cursor.as_deref() != Some("gamma.log") {
        return Err(format!(
            "with the search filter off, typing \"ga\" should keep every row and put the cursor on gamma.log; rows {names:?}, cursor {cursor:?}"
        ));
    }

    // Cancel discards a previewed change. Options first opens on General.
    options::open(&mut app)?;
    if app.window.get_options_topic_index() != 0 {
        return Err(format!(
            "Options first opened on topic {} instead of General",
            app.window.get_options_topic_index()
        ));
    }
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
    let only_matches = |names: &[String]| {
        !names
            .iter()
            .any(|name| name == "beta.txt" || name == "gamma.log")
            && names.iter().any(|name| name == "alpha.txt")
            && names.iter().any(|name| name == BIG)
    };
    let (names, _) = search(&mut app, "al", only_matches)?;
    if !only_matches(&names) {
        return Err(format!(
            "with the search filter on, typing \"al\" should show only alpha.txt and {BIG}; it shows {names:?}"
        ));
    }
    app.screenshot(
        context,
        "general-effects",
        "After OK: exact byte sizes in the panel",
    );

    // The page list and pickers by keyboard; leaves Options last shown on another page
    // than General.
    navigation(&mut app, &store, context)?;

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
    // The history was cleared and is not remembered: the picker is disabled and Alt+Up
    // recalls nothing within the window in which the seeded history was recalled.
    if options::enabled_state(&app, AccessibleRole::Combobox, HISTORY_PICKER) != Some(false) {
        return Err(format!(
            "after clearing and a restart the {HISTORY_PICKER:?} picker is {:?} instead of disabled",
            options::enabled_state(&app, AccessibleRole::Combobox, HISTORY_PICKER)
        ));
    }
    if let Some(recalled) = recall(&mut app)? {
        return Err(format!(
            "after clearing and a restart Alt+Up recalls {recalled:?} in the command editor"
        ));
    }
    // No setting stores the page: a restart opens Options on General again, although
    // another page was shown last before it.
    options::open(&mut app)?;
    if app.window.get_options_topic_index() != 0 {
        return Err(format!(
            "after a restart Options opened on topic {} instead of General",
            app.window.get_options_topic_index()
        ));
    }
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
    if saved.bookmarks.len() != 1
        || saved.bookmarks[0].slot != Some(2)
        || saved.bookmarks[0].path != folder
        || saved.user_commands.len() != 1
        || saved.user_commands[0].name != "Kept command"
    {
        return Err(format!(
            "Reset to defaults did not keep the Hot Path and the Custom Command: {:?}, {:?}",
            saved.bookmarks, saved.user_commands
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
    context.note("General: the seeded history was recalled (Alt+Up gave the newest command, the history picker enabled); Cancel discarded a previewed change; OK saved all six General settings (search filter, byte sizes, quick copy verification, disk I/O 2, history off, history cleared) with visible effects; a restart showed them, disabled the history picker and recalled nothing within 2 s of Alt+Up; Reset to defaults restored them and kept the tabs; Escape discarded a preview, Enter saved");
    context.note("Options navigation: Options first opened on General with the focus on OK; Enter on the focused Disk I/O combo box opened its list without closing or saving Options and Escape closed only the list; Shift+Tab from OK reached the page list at Plugins; Space and Enter chose the focused rows; Up walked to General and Down to Plugins in the displayed order with the focus following, both stopping at the ends; the page shown last reopened in the session and a restart opened General");
    app.shutdown()
}
