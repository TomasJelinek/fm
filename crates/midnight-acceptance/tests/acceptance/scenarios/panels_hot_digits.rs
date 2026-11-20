//! Numbered Hot Paths with the physical number keys the native key hook delivers:
//! a plain digit, Shift+digit and Ctrl+digit open Hot Path N in the focused panel, 0 is
//! number 10, Ctrl+Alt+digit and Alt+Shift+digit open it in the other panel, an
//! unassigned number is swallowed (no navigation, no search) and a digit typed during a
//! filename search stays text; Ctrl+Shift+0 makes the current folder number 10. The
//! toolbar's star adds the current folder under the first free number, the Hot Paths
//! menu's Add path adds one beyond ten, and the Hot Paths menu and the drive menu open
//! saved folders.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect,
    options_support::SettingsStore,
    panel,
    scenarios::panels_support::{self as ps, LEFT, RIGHT},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{Bookmark, BookmarkId, PaneId};
use slint::{Model, platform::Key};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const SLOTS: [u8; 9] = [1, 2, 3, 4, 5, 7, 8, 9, 10];

fn hot(base: &Path, slot: u8) -> PathBuf {
    base.join(format!("hp-{slot}"))
}

fn reached(app: &mut App, pane: PaneId, path: &Path) -> bool {
    let wanted = path.to_path_buf();
    ps::wait(app, Duration::from_secs(5), |app| {
        app.location(pane).as_ref() == Some(&wanted) && !app.loading(pane)
    })
}

fn slot_of(app: &App, path: &Path) -> Option<i32> {
    app.window
        .get_bookmarks()
        .iter()
        .find(|row| Path::new(row.path.as_str()) == path)
        .map(|row| row.slot)
}

fn go(app: &mut App, path: &Path) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &path.to_string_lossy());
    input::key(app.slint(), Key::Return);
    panel::wait_location(app, LEFT, path);
    Ok(())
}

pub fn hot_digits(context: &mut Context) -> Result<(), String> {
    let base = support::scratch(context, "hot")?;
    for slot in SLOTS {
        ps::folder(&hot(&base, slot))?;
    }
    ps::write(&hot(&base, 3).join("h1.txt"), "h")?;
    for name in ["extra", "star-folder", "menu-folder", "right-start"] {
        ps::folder(&base.join(name))?;
    }
    let bookmarks: Vec<Bookmark> = SLOTS
        .iter()
        .map(|slot| Bookmark {
            id: BookmarkId(u64::from(*slot)),
            name: format!("hp-{slot}"),
            path: hot(&base, *slot),
            archive: None,
            network: None,
            slot: Some(*slot),
        })
        .collect();
    let store = SettingsStore::seeded(move |preferences| preferences.bookmarks = bookmarks);
    let mut config = Config::new(&base, base.join("right-start"));
    config.settings = Some(store);
    let mut app = App::start(config);
    let mut failures = Vec::new();

    // Plain digits, Shift+digit and Ctrl+digit in the focused (left) panel.
    for (key, control, shift, slot) in [
        ("Digit1", false, false, 1),
        ("Digit0", false, false, 10),
        ("Digit2", false, true, 2),
        ("Digit3", true, false, 3),
    ] {
        panel::focus_list(&mut app, LEFT)?;
        let taken = ps::physical(&mut app, key, control, false, shift);
        let target = hot(&base, slot);
        if !taken || !reached(&mut app, LEFT, &target) {
            failures.push(format!(
                "{}{}{key}: taken {taken}, left panel at {:?}, expected Hot Path {slot} {}",
                if control { "Ctrl+" } else { "" },
                if shift { "Shift+" } else { "" },
                app.location(LEFT),
                target.display()
            ));
        }
    }
    // Ctrl+Alt+digit and Alt+Shift+digit: the other panel.
    for (key, control, shift, slot) in [("Digit4", true, false, 4), ("Digit5", false, true, 5)] {
        panel::focus_list(&mut app, LEFT)?;
        let taken = ps::physical(&mut app, key, control, true, shift);
        let target = hot(&base, slot);
        if !taken || !reached(&mut app, RIGHT, &target) {
            failures.push(format!(
                "{}Alt+{}{key}: taken {taken}, right panel at {:?}, expected Hot Path {slot}",
                if control { "Ctrl+" } else { "" },
                if shift { "Shift+" } else { "" },
                app.location(RIGHT)
            ));
        }
        if app.location(LEFT).as_deref() != Some(hot(&base, 3).as_path()) {
            failures.push(format!(
                "opening Hot Path {slot} in the other panel moved the left panel to {:?}",
                app.location(LEFT)
            ));
        }
    }
    // An unassigned number is swallowed: no navigation, no search.
    panel::focus_list(&mut app, LEFT)?;
    let taken = ps::physical(&mut app, "Digit6", false, false, false);
    if !taken {
        // The native hook would then deliver the character.
        input::key(app.slint(), "6");
        app.settle();
    }
    if !taken
        || app.location(LEFT).as_deref() != Some(hot(&base, 3).as_path())
        || app.window.get_left_search_active()
    {
        failures.push(format!(
            "the unassigned 6: taken {taken}, left panel at {:?}, search {:?}",
            app.location(LEFT),
            app.window.get_left_search_query()
        ));
    }
    // During a filename search a digit stays text.
    input::type_text(app.slint(), "h");
    app.settle();
    let taken = ps::physical(&mut app, "Digit1", false, false, false);
    if !taken {
        input::key(app.slint(), "1");
        app.settle();
    }
    if taken
        || app.window.get_left_search_query() != "h1"
        || app.location(LEFT).as_deref() != Some(hot(&base, 3).as_path())
    {
        failures.push(format!(
            "1 during the search \"h\": taken as a shortcut {taken}, query {:?}, left panel at {:?}",
            app.window.get_left_search_query(),
            app.location(LEFT)
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.settle();
    // Ctrl+Shift+0 makes the current folder number 10.
    let extra = base.join("extra");
    go(&mut app, &extra)?;
    ps::physical(&mut app, "Digit0", true, false, true);
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        slot_of(app, &extra) == Some(10)
    }) {
        failures.push(format!(
            "Ctrl+Shift+0 in extra/: its Hot Path number is {:?}",
            slot_of(&app, &extra)
        ));
    }
    go(&mut app, &base)?;
    ps::physical(&mut app, "Digit0", false, false, false);
    if !reached(&mut app, LEFT, &extra) {
        failures.push(format!(
            "0 after Ctrl+Shift+0 went to {:?}",
            app.location(LEFT)
        ));
    }
    // The star adds the current folder under the first free number (6).
    let star = base.join("star-folder");
    go(&mut app, &star)?;
    match ps::click_starting(&mut app, "Add current path to Hot Paths") {
        Err(error) => failures.push(format!("the Hot Paths star: {error}")),
        Ok(_) => {
            if !ps::wait(&mut app, Duration::from_secs(5), |app| {
                slot_of(app, &star) == Some(6)
            }) {
                failures.push(format!(
                    "the star saved star-folder/ as number {:?}, expected the first free number 6",
                    slot_of(&app, &star)
                ));
            }
        }
    }
    go(&mut app, &base)?;
    ps::physical(&mut app, "Digit6", false, false, false);
    if !reached(&mut app, LEFT, &star) {
        failures.push(format!("6 after the star went to {:?}", app.location(LEFT)));
    }
    // The menu's Add path: no number is free, it is saved without one.
    let menu = base.join("menu-folder");
    go(&mut app, &menu)?;
    let added = (|| -> Result<(), String> {
        input::click(app.slint(), AccessibleRole::Combobox, "Hot Paths")?;
        app.settle();
        input::click_button(app.slint(), "Add path")?;
        app.settle();
        Ok(())
    })();
    if let Err(error) = added {
        failures.push(format!("Hot Paths ▾ Add path: {error}"));
        support::dismiss_all(&mut app);
    } else if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        slot_of(app, &menu).is_some_and(|slot| slot <= 0)
    }) {
        failures.push(format!(
            "Add path with every number taken saved menu-folder/ as {:?}, expected no number",
            slot_of(&app, &menu)
        ));
    }
    // The Hot Paths menu and the drive menu open saved folders.
    go(&mut app, &base)?;
    let nine = hot(&base, 9).to_string_lossy().into_owned();
    match open_choice(&mut app, "Hot Paths", &nine) {
        Err(error) => failures.push(format!("Hot Paths ▾ {nine}: {error}")),
        Ok(label) => {
            if !reached(&mut app, LEFT, &hot(&base, 9)) {
                failures.push(format!(
                    "Hot Paths ▾ {label:?} went to {:?}",
                    app.location(LEFT)
                ));
            }
        }
    }
    let eight = hot(&base, 8).to_string_lossy().into_owned();
    match open_choice(&mut app, "Drive or volume", &eight) {
        Err(error) => failures.push(format!("the drive menu's Hot Path {eight}: {error}")),
        Ok(label) => {
            if !reached(&mut app, LEFT, &hot(&base, 8)) {
                failures.push(format!(
                    "the drive menu's {label:?} went to {:?}",
                    app.location(LEFT)
                ));
            }
        }
    }
    app.shutdown()?;
    support::report(&failures)
}

/// Open the left panel's (or toolbar's) menu `combo` and choose the entry whose label
/// ends with `path`; that label.
fn open_choice(app: &mut App, combo: &str, path: &str) -> Result<String, String> {
    panel::focus_list(app, LEFT)?;
    let edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    let item = inspect::find_all(app.slint(), AccessibleRole::Combobox, combo)
        .into_iter()
        .find(|item| inspect::center(item).x < edge || combo == "Hot Paths")
        .ok_or_else(|| format!("no {combo:?} menu"))?;
    input::click_at(app.slint(), inspect::center(&item));
    app.settle();
    let labels: Vec<String> = ps::popup_buttons(app)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    let label = labels
        .iter()
        .find(|label| label.ends_with(path))
        .cloned()
        .ok_or_else(|| format!("the menu lists no {path}: {labels:?}"))?;
    ps::click_popup_button(app, &label)?;
    Ok(label)
}
