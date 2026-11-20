//! Folder tabs beyond the basics in `files_tabs`: a middle click closes a tab (never
//! the last one), a double click after the tabs opens one at the panel's folder,
//! Ctrl+Shift+digit switches to a numbered tab (a missing number does nothing), many
//! tabs scroll with arrows that keep the active tab in view, a panel stops at 32 tabs,
//! the right panel's tabs are its own, Ctrl+U moves the tabs with their panel, F7,
//! Shift+F4 and F2 in a second tab act on that tab's folder in both panels, and tabs
//! with their active one come back after a restart.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect,
    options_support::SettingsStore,
    panel,
    scenarios::panels_support::{self as ps, LEFT, RIGHT},
};
use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};
use midnight_core::PaneId;
use slint::{LogicalPosition, Model, platform::Key};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn tabs(app: &App, pane: PaneId) -> Vec<(String, bool)> {
    match pane {
        PaneId::Left => app.window.get_left_tabs(),
        PaneId::Right => app.window.get_right_tabs(),
    }
    .iter()
    .map(|tab| (tab.path.to_string(), tab.active))
    .collect()
}

fn count(app: &App, pane: PaneId) -> usize {
    tabs(app, pane).len()
}

fn active(app: &App, pane: PaneId) -> Option<usize> {
    tabs(app, pane).iter().position(|(_, active)| *active)
}

/// The tab buttons of `pane` ("label\npath"), in strip order, scrolled into view or
/// not. The scenario keeps the two panels' tab folders apart, so a tab's path tells
/// its panel.
fn tab_buttons(app: &App, pane: PaneId) -> Vec<(String, ItemRc)> {
    let paths: Vec<String> = tabs(app, pane).into_iter().map(|(path, _)| path).collect();
    let mut found = Vec::new();
    for root in inspect::roots(app.slint()).iter().take(1) {
        inspect::walk(root, &mut |item| {
            // Tabs scrolled out of the strip count too (`is_visible` is geometric).
            if item.accessible_role() == AccessibleRole::Button
                && let Some(label) = inspect::label(item)
                && !label.starts_with("Close tab: ")
                && let Some((_, path)) = label.split_once('\n')
                && paths.iter().any(|tab| tab == path)
            {
                found.push((label, item.clone()));
            }
        });
    }
    found
}

/// Whether each tab of the left strip is wholly in view, in strip order.
fn wholly_shown(app: &App) -> Vec<bool> {
    tab_buttons(app, LEFT)
        .iter()
        .map(|(_, item)| {
            let full = inspect::rect(item);
            inspect::visible_rect(item).is_some_and(|shown| shown.2 + 1.0 >= full.2)
        })
        .collect()
}

fn center(item: &ItemRc) -> LogicalPosition {
    let (x, y, w, h) = inspect::visible_rect(item).unwrap_or_else(|| inspect::rect(item));
    LogicalPosition::new(x + w / 2.0, y + h / 2.0)
}

fn wait_count(app: &mut App, pane: PaneId, wanted: usize) -> bool {
    ps::wait(app, Duration::from_secs(5), |app| {
        count(app, pane) == wanted
    })
}

fn new_tab(app: &mut App, pane: PaneId) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    let before = count(app, pane);
    input::chord(app.slint(), &[Key::Control], "t");
    if !wait_count(app, pane, before + 1) {
        return Err(format!("Ctrl+T in the {pane:?} panel opened no tab"));
    }
    Ok(())
}

fn go(app: &mut App, pane: PaneId, path: &Path) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &path.to_string_lossy());
    input::key(app.slint(), Key::Return);
    panel::wait_location(app, pane, path);
    Ok(())
}

pub fn tabs_more(context: &mut Context) -> Result<(), String> {
    let base = support::scratch(context, "tabs")?;
    let (first, second, third) = (base.join("first"), base.join("second"), base.join("third"));
    for folder in [&first, &second, &third] {
        ps::folder(folder)?;
        ps::write(&folder.join("note.txt"), "note")?;
    }
    let right = base.join("right");
    ps::folder(&right)?;
    let store = SettingsStore::new();
    let mut config = Config::new(&first, &right);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let mut failures = Vec::new();

    pointer_gestures(&mut app, context, &first, &second, &mut failures)?;
    numbered(&mut app, &first, &second, &third, &mut failures)?;
    second_tab_operations(&mut app, &first, &second, &right, &mut failures)?;
    swap(&mut app, context, &mut failures)?;
    overflow(&mut app, context, &mut failures)?;
    // The tabs, their folders and the active one come back after a restart that
    // restores the saved session (no explicit folders, as a plain start).
    let saved = tabs(&app, LEFT);
    let session = store.current().tabs[0].clone();
    context.note(format!(
        "saved left tab session {:?} (active {})",
        session.paths, session.active
    ));
    app.shutdown()?;
    drop(app);
    let mut pending = support::Pending::default();
    let restored = if RESTORE_TABS_READY {
        restored_tabs(context, &store, &first, &right, &saved)
    } else {
        Err("not run until the harness's Config::restore_tabs start is confirmed".into())
    };
    pending.check(
        "tabs and the active tab restored after a restart",
        "harness: a Config::restore_tabs start is not verified yet",
        restored,
    );
    support::report_with(context, &failures, &pending)
}

/// Whether the harness's `Config::restore_tabs` start (a start without explicit
/// folders, as the application restores its session) is known to complete.
const RESTORE_TABS_READY: bool = false;

/// A restart on the saved settings shows the saved tabs, the active one active.
fn restored_tabs(
    context: &mut Context,
    store: &std::sync::Arc<SettingsStore>,
    first: &Path,
    right: &Path,
    saved: &[(String, bool)],
) -> Result<(), String> {
    let mut config = Config::new(first, right);
    config.settings = Some(store.clone());
    config.restore_tabs = true;
    let mut app = App::start(config);
    let target = saved.to_vec();
    let mut problems = Vec::new();
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        tabs(app, LEFT) == target
    }) {
        problems.push(format!(
            "after a restart the left tabs are {:?}, expected {saved:?}",
            tabs(&app, LEFT)
        ));
    }
    let active_path = saved
        .iter()
        .find(|(_, active)| *active)
        .map(|(path, _)| PathBuf::from(path));
    if app.location(LEFT) != active_path {
        problems.push(format!(
            "after a restart the left panel shows {:?}, expected its active tab {active_path:?}",
            app.location(LEFT)
        ));
    }
    app.screenshot(context, "restored-tabs", "Tabs restored after a restart");
    app.shutdown()?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Middle click closes a tab but not the last; a double click after the tabs opens one.
fn pointer_gestures(
    app: &mut App,
    context: &mut Context,
    first: &Path,
    second: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // The only tab: a middle click does nothing, its close button is disabled.
    let only = tab_buttons(app, LEFT);
    if let Some((_, item)) = only.first() {
        ps::middle_click(app.slint(), center(item));
        app.idle(Duration::from_millis(200));
        if count(app, LEFT) != 1 {
            failures.push("a middle click closed the last tab".into());
        }
    } else {
        failures.push("no tab button in the left panel".into());
    }
    new_tab(app, LEFT)?;
    go(app, LEFT, second)?;
    let buttons = tab_buttons(app, LEFT);
    let second_label = format!("second\n{}", second.display());
    match buttons.iter().find(|(label, _)| *label == second_label) {
        None => failures.push(format!(
            "no tab labelled {second_label:?}; tabs {:?}",
            buttons.iter().map(|(label, _)| label).collect::<Vec<_>>()
        )),
        Some((_, item)) => {
            ps::middle_click(app.slint(), center(item));
            if !wait_count(app, LEFT, 1) {
                failures.push("a middle click did not close the second tab".into());
            }
            panel::wait_location(app, LEFT, first);
        }
    }
    // A double click on the empty strip after the last tab opens a tab at the folder.
    let buttons = tab_buttons(app, LEFT);
    match buttons.last() {
        None => failures.push("no tab to find the strip's empty end".into()),
        Some((_, item)) => {
            let (x, y, w, h) = inspect::rect(item);
            let spot = LogicalPosition::new(x + w + 40.0, y + h / 2.0);
            ps::double_click(app, spot);
            if !wait_count(app, LEFT, 2) {
                failures.push(format!(
                    "a double click after the tabs opened no tab ({} tab(s))",
                    count(app, LEFT)
                ));
            } else {
                let paths = tabs(app, LEFT);
                if paths.iter().any(|(path, _)| Path::new(path) != first) {
                    failures.push(format!(
                        "the double-clicked tab is not at the panel's folder: {paths:?}"
                    ));
                }
                app.screenshot(
                    context,
                    "double-click-tab",
                    "A tab opened by double-clicking the strip",
                );
                panel::focus_list(app, LEFT)?;
                input::chord(app.slint(), &[Key::Control], "w");
                wait_count(app, LEFT, 1);
            }
        }
    }
    Ok(())
}

/// Ctrl+Shift+digit as the native key hook delivers it.
fn numbered(
    app: &mut App,
    first: &Path,
    second: &Path,
    third: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    new_tab(app, LEFT)?;
    go(app, LEFT, second)?;
    new_tab(app, LEFT)?;
    go(app, LEFT, third)?;
    for (digit, index, path) in [
        ("Digit1", 0, first),
        ("Digit3", 2, third),
        ("Digit2", 1, second),
    ] {
        panel::focus_list(app, LEFT)?;
        if !ps::physical(app, digit, true, false, true) {
            failures.push(format!("Ctrl+Shift+{digit} was not taken"));
        }
        if !ps::wait(app, Duration::from_secs(5), |app| {
            active(app, LEFT) == Some(index) && app.location(LEFT).as_deref() == Some(path)
        }) {
            failures.push(format!(
                "Ctrl+Shift+{digit}: active tab {:?} at {:?}, expected tab {} at {}",
                active(app, LEFT),
                app.location(LEFT),
                index + 1,
                path.display()
            ));
        }
    }
    panel::focus_list(app, LEFT)?;
    ps::physical(app, "Digit9", true, false, true);
    app.idle(Duration::from_millis(200));
    if active(app, LEFT) != Some(1) || count(app, LEFT) != 3 {
        failures.push(format!(
            "Ctrl+Shift+9 without a ninth tab changed the tabs: {:?}",
            tabs(app, LEFT)
        ));
    }
    if let Err(error) = ps::stays_idle(app, Duration::from_millis(300)) {
        failures.push(format!("Ctrl+Shift+9 without a ninth tab: {error}"));
        support::dismiss_all(app);
    }
    Ok(())
}

/// F7, Shift+F4 and F2 in a second tab act on that tab's folder, in both panels.
fn second_tab_operations(
    app: &mut App,
    first: &Path,
    second: &Path,
    right: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // Left: tab 2 (second/) is active.
    let start = support::named_operation(app, &[], Key::F7, "made in second")?;
    let progress = support::wait_operation(app, start);
    if let Err(error) = support::operation_ok(&progress, "F7 in the second tab") {
        failures.push(error);
    }
    if !second.join("made in second").is_dir() || first.join("made in second").exists() {
        failures.push("F7 in the second tab did not create the folder in that tab's folder".into());
    }
    support::wait_row(app, LEFT, "made in second", true);
    panel::select_row(app, LEFT, "made in second")?;
    let start = support::named_operation(app, &[], Key::F2, "renamed in second")?;
    let progress = support::wait_operation(app, start);
    if let Err(error) = support::operation_ok(&progress, "F2 in the second tab") {
        failures.push(error);
    }
    if !second.join("renamed in second").is_dir() {
        failures.push("F2 in the second tab did not rename on disk".into());
    }
    // Back to tab 1: its folder and listing are its own.
    panel::focus_list(app, LEFT)?;
    ps::physical(app, "Digit1", true, false, true);
    panel::wait_location(app, LEFT, first);
    if ps::shown(app, LEFT)
        .iter()
        .any(|name| name.contains("second"))
    {
        failures.push(format!(
            "the first tab lists the second tab's entries: {:?}",
            ps::shown(app, LEFT)
        ));
    }
    let start = support::named_operation(app, &[Key::Shift], Key::F4, "made in first.txt")?;
    let progress = support::wait_operation(app, start);
    if let Err(error) = support::operation_ok(&progress, "Shift+F4 in the first tab") {
        failures.push(error);
    }
    if !first.join("made in first.txt").is_file() || second.join("made in first.txt").exists() {
        failures.push("Shift+F4 in the first tab did not create the file in its folder".into());
    }
    // Right panel: its own second tab, F7 there; the left tabs are untouched.
    let left_tabs = tabs(app, LEFT);
    new_tab(app, RIGHT)?;
    let inner = right.join("inner");
    ps::folder(&inner)?;
    go(app, RIGHT, &inner)?;
    let start = support::named_operation(app, &[], Key::F7, "made in right tab")?;
    let progress = support::wait_operation(app, start);
    if let Err(error) = support::operation_ok(&progress, "F7 in the right panel's second tab") {
        failures.push(error);
    }
    if !inner.join("made in right tab").is_dir() {
        failures.push("F7 in the right panel's second tab created nothing in its folder".into());
    }
    if tabs(app, LEFT) != left_tabs {
        failures.push("opening a right panel tab changed the left tabs".into());
    }
    panel::focus_list(app, RIGHT)?;
    input::chord(app.slint(), &[Key::Control], "w");
    wait_count(app, RIGHT, 1);
    // Leave the left panel on tab 2.
    panel::focus_list(app, LEFT)?;
    ps::physical(app, "Digit2", true, false, true);
    panel::wait_location(app, LEFT, second);
    Ok(())
}

/// Ctrl+U moves the tabs with their panel.
fn swap(app: &mut App, context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let before = tabs(app, LEFT);
    let middle = app.width as f32 / 2.0;
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "u");
    app.settle();
    let after = tabs(app, LEFT);
    let positions: Vec<f32> = tab_buttons(app, LEFT)
        .iter()
        .map(|(_, item)| inspect::center(item).x)
        .collect();
    if !app.window.get_panels_swapped() || after != before {
        failures.push(format!(
            "Ctrl+U: swapped {}, the left panel's tabs {before:?} → {after:?}",
            app.window.get_panels_swapped()
        ));
    }
    if positions.len() != before.len() || positions.iter().any(|x| *x < middle) {
        failures.push(format!(
            "after Ctrl+U the left panel's {} tabs are not drawn on the right half (x {positions:?})",
            before.len()
        ));
    }
    app.screenshot(
        context,
        "swapped-tabs",
        "Ctrl+U: the three tabs moved with their panel",
    );
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "u");
    app.settle();
    if app.window.get_panels_swapped() {
        failures.push("the second Ctrl+U did not swap back".into());
    }
    Ok(())
}

/// The scroll arrows agree with what the strip shows: Scroll left is enabled exactly
/// when the first tab is not wholly in view, Scroll right when the last is not; the
/// active tab is wholly in view.
fn check_strip(app: &App, what: &str, failures: &mut Vec<String>) {
    let shown = wholly_shown(app);
    let arrow = |label: &str| {
        inspect::find(app.slint(), AccessibleRole::Button, label)
            .map(|item| crate::tools_support::enabled(&item))
    };
    let (left, right) = (arrow("Scroll tabs left"), arrow("Scroll tabs right"));
    let first = shown.first().copied().unwrap_or(false);
    let last = shown.last().copied().unwrap_or(false);
    if left != Some(!first) || right != Some(!last) {
        failures.push(format!(
            "{what}: Scroll tabs left enabled {left:?} (first tab wholly shown {first}), Scroll tabs right enabled {right:?} (last tab wholly shown {last})"
        ));
    }
    let index = active(app, LEFT).unwrap_or(usize::MAX);
    if !shown.get(index).copied().unwrap_or(false) {
        failures.push(format!(
            "{what}: the active tab {} of {} is not wholly in view",
            index + 1,
            shown.len()
        ));
    }
}

/// Many tabs: scroll arrows keep the active tab in view; the panel stops at 32.
fn overflow(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let start = count(app, LEFT);
    while count(app, LEFT) < 14 {
        new_tab(app, LEFT)?;
    }
    if inspect::find(app.slint(), AccessibleRole::Button, "Scroll tabs left").is_none() {
        failures.push(format!(
            "14 tabs show no scroll arrows; buttons {:?}",
            inspect::labels(app.slint(), AccessibleRole::Button)
                .into_iter()
                .filter(|label| label.contains("tab"))
                .collect::<Vec<_>>()
        ));
    } else {
        check_strip(app, "14 tabs, the newest active", failures);
        app.screenshot(context, "many-tabs", "Fourteen tabs with scroll arrows");
        // Each enabled arrow scrolls; the arrows follow.
        for label in ["Scroll tabs left", "Scroll tabs right"] {
            let Some(item) = inspect::find(app.slint(), AccessibleRole::Button, label) else {
                continue;
            };
            if !crate::tools_support::enabled(&item) {
                continue;
            }
            let before = wholly_shown(app);
            input::click_at(app.slint(), center(&item));
            app.settle();
            if wholly_shown(app) == before {
                failures.push(format!("{label} did not scroll the tabs"));
            }
        }
        // Switching to tab 1 and to the last tab reveals each.
        for digit in ["Digit1", "Digit9"] {
            panel::focus_list(app, LEFT)?;
            ps::physical(app, digit, true, false, true);
            app.settle();
            check_strip(app, &format!("after Ctrl+Shift+{digit}"), failures);
        }
    }
    // Up to 32: then the + button is disabled and Ctrl+T adds nothing.
    while count(app, LEFT) < 32 {
        new_tab(app, LEFT)?;
    }
    let plus = inspect::find(
        app.slint(),
        AccessibleRole::Button,
        "A panel can have up to 32 folder tabs.",
    );
    if !plus
        .as_ref()
        .is_some_and(|item| !crate::tools_support::enabled(item))
    {
        failures.push("at 32 tabs the + button is not disabled with its limit explained".into());
    }
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "t");
    app.idle(Duration::from_millis(200));
    if count(app, LEFT) != 32 {
        failures.push(format!("Ctrl+T at 32 tabs made {}", count(app, LEFT)));
    }
    support::dismiss_all(app);
    // Back to as many tabs as before; Ctrl+W closes the active one each time.
    while count(app, LEFT) > start {
        panel::focus_list(app, LEFT)?;
        let before = count(app, LEFT);
        input::chord(app.slint(), &[Key::Control], "w");
        if !wait_count(app, LEFT, before - 1) {
            failures.push(format!("Ctrl+W at {before} tabs closed nothing"));
            break;
        }
    }
    // The second tab active for the restart.
    panel::focus_list(app, LEFT)?;
    ps::physical(app, "Digit2", true, false, true);
    app.settle();
    Ok(())
}
