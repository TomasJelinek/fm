//! View modes, sorting and the panel layout, with marks and the cursor on files that
//! every change must keep: sorting by Ctrl+F3–F6, by clicking each column header (both
//! directions; the header that shows the arrow and its accessible state) and from the
//! drive bar's sort menu (every column and Reverse order) gives exactly the expected
//! order with folders first and extensionless files last, the same marks and the cursor
//! on the same file, scrolled into view; the right panel sorts on its own. Detailed,
//! Brief and Thumbnails from the View menu and Alt+3/2/5 keep cursor and marks, and the
//! arrow keys move by column in Brief and by cell and grid row in Thumbnails. The
//! splitter is dragged (clamped at its minimum, names still shown whole), centered by
//! a double click, moved and centered with its keys and restored after a restart;
//! Ctrl+F11 enlarges the active panel, Tab moves the enlargement, Ctrl+F11 restores; a
//! small window shows one panel and a rail. Header sorting and the splitter work the
//! same at touch density.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect,
    options_support::SettingsStore,
    panel,
    scenarios::panels_support::{self as ps, LEFT, RIGHT},
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
};
use midnight_core::{PaneId, UiDensity};
use slint::{LogicalPosition, platform::Key};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

const FILES: usize = 40;
const MARKED: [&str; 3] = ["n05.e35", "n17.e39", "n33.e31"];
const CURSOR: &str = "n29.e03";
const NO_EXTENSION: &str = "zz-noext";

/// One test file: name, extension rank, size and modified minute (each a permutation,
/// so every column orders the files completely).
struct Spec {
    name: String,
    extension: Option<String>,
    size: usize,
    minute: u64,
}

fn specs() -> Vec<Spec> {
    let mut specs: Vec<Spec> = (0..FILES)
        .map(|index| {
            let extension = format!("e{:02}", (index * 7) % FILES);
            Spec {
                name: format!("n{index:02}.{extension}"),
                extension: Some(extension),
                size: (index * 13) % FILES + 1,
                minute: ((index * 17) % FILES) as u64,
            }
        })
        .collect();
    specs.push(Spec {
        name: NO_EXTENSION.into(),
        extension: None,
        size: FILES + 1,
        minute: FILES as u64,
    });
    specs
}

/// The expected file order for a sort column (0 name, 1 size, 2 modified, 3 extension)
/// and direction; extensionless files stay last when sorting by extension.
fn expected(specs: &[Spec], column: i32, descending: bool) -> Vec<String> {
    let mut order: Vec<&Spec> = specs.iter().collect();
    match column {
        1 => order.sort_by_key(|spec| spec.size),
        2 => order.sort_by_key(|spec| spec.minute),
        3 => order.sort_by_key(|spec| (spec.extension.is_none(), spec.extension.clone())),
        _ => order.sort_by_key(|spec| spec.name.clone()),
    }
    if descending {
        if column == 3 {
            let (with, without): (Vec<&Spec>, Vec<&Spec>) =
                order.into_iter().partition(|spec| spec.extension.is_some());
            order = with.into_iter().rev().chain(without).collect();
        } else {
            order.reverse();
        }
    }
    order.into_iter().map(|spec| spec.name.clone()).collect()
}

fn files(app: &App, pane: PaneId) -> Vec<String> {
    app.rows(pane)
        .iter()
        .filter(|row| !row.directory && !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect()
}

fn folders_first(app: &App, pane: PaneId) -> bool {
    let kinds: Vec<bool> = app
        .rows(pane)
        .iter()
        .filter(|row| !row.parent_entry)
        .map(|row| row.directory)
        .collect();
    kinds.windows(2).all(|pair| pair[0] || !pair[1])
}

fn sort_state(app: &App, pane: PaneId) -> (i32, bool) {
    match pane {
        PaneId::Left => (app.window.get_left_sort(), app.window.get_left_descending()),
        PaneId::Right => (
            app.window.get_right_sort(),
            app.window.get_right_descending(),
        ),
    }
}

fn column_name(column: i32) -> &'static str {
    match column {
        1 => "Size",
        2 => "Modified",
        3 => "Extension",
        _ => "Name",
    }
}

/// The cursor row lies inside the rows the list shows: wholly inside the viewport in
/// Detailed, among the rows the virtualized grid shows in Brief and Thumbnails.
fn cursor_in_view(app: &App) -> bool {
    let cursor = app.cursor(LEFT);
    if cursor < 0 {
        return false;
    }
    if app.window.get_left_view_mode() != 0 {
        return cursor >= app.window.get_left_visible_row_start()
            && cursor < app.window.get_left_visible_row_end();
    }
    let top = app.window.get_left_list_top();
    let height = app.window.get_left_list_height();
    let row = app.window.get_left_view_row_height();
    let y = top + app.window.get_left_list_content_y() + cursor as f32 * row;
    y >= top - 0.5 && y + row <= top + height + 0.5
}

/// What a sort request on `column` should give: the current column reverses, another
/// column starts ascending.
fn after_request(app: &App, column: i32) -> (i32, bool) {
    let (current, descending) = sort_state(app, LEFT);
    if current == column {
        (column, !descending)
    } else {
        (column, false)
    }
}

/// After a sort or view change: the order the reported sort state promises, folders
/// first, the same marks and the cursor on the same file in view.
fn check_state(
    app: &mut App,
    specs: &[Spec],
    what: &str,
    expect_order: bool,
    failures: &mut Vec<String>,
) {
    let (column, descending) = sort_state(app, LEFT);
    let wanted = expected(specs, column, descending);
    if expect_order {
        let target = wanted.clone();
        if !ps::wait(app, Duration::from_secs(10), |app| {
            files(app, LEFT) == target
        }) {
            failures.push(format!(
                "{what} (sorted by {} {}): files {:?}, expected {wanted:?}",
                column_name(column),
                if descending {
                    "descending"
                } else {
                    "ascending"
                },
                files(app, LEFT)
            ));
        }
        if !folders_first(app, LEFT) {
            failures.push(format!("{what}: folders are not listed first"));
        }
    }
    let marks = ps::marked(app, LEFT);
    if marks != ps::sorted(&MARKED) {
        failures.push(format!("{what}: marks {marks:?}, expected {MARKED:?}"));
    }
    if ps::cursor(app, LEFT).as_deref() != Some(CURSOR) {
        failures.push(format!(
            "{what}: the cursor is on {:?}, expected it to stay on {CURSOR}",
            ps::cursor(app, LEFT)
        ));
    } else if !ps::wait(app, Duration::from_secs(2), cursor_in_view) {
        failures.push(format!(
            "{what}: the cursor row {} is scrolled out of view (content y {})",
            app.cursor(LEFT),
            app.window.get_left_list_content_y()
        ));
    }
}

/// The left panel's header button `label`.
fn heading(app: &App, label: &str) -> Option<ItemRc> {
    let edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    inspect::find_all(app.slint(), AccessibleRole::Button, label)
        .into_iter()
        .find(|item| inspect::center(item).x < edge)
}

fn heading_value(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Value)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// Each header tells whether it sorts and in which direction.
fn check_headings(app: &App, what: &str, failures: &mut Vec<String>) {
    let (column, descending) = sort_state(app, LEFT);
    let sorted = if descending {
        "Descending"
    } else {
        "Ascending"
    };
    for (label, own) in [
        ("Sort by Name", 0),
        ("Sort by Extension", 3),
        ("Sort by Size", 1),
        ("Sort by Modified", 2),
        ("Sort by Modified time", 2),
    ] {
        let Some(item) = heading(app, label) else {
            failures.push(format!("{what}: no {label:?} header in the left panel"));
            continue;
        };
        let wanted = if own == column { sorted } else { "Not sorted" };
        let found = heading_value(&item);
        if found != wanted {
            failures.push(format!(
                "{what}: the {label:?} header says {found:?}, expected {wanted:?}"
            ));
        }
    }
}

fn click_item(app: &mut App, item: &ItemRc) {
    let (x, y, w, h) = inspect::visible_rect(item).unwrap_or_else(|| inspect::rect(item));
    input::click_at(app.slint(), LogicalPosition::new(x + w / 2.0, y + h / 2.0));
    app.settle();
}

/// Choose `item` in the left panel's sort menu (or View menu).
fn menu(app: &mut App, menu: &str, item: &str) -> Result<(), String> {
    panel::wait_input(app);
    input::click(app.slint(), AccessibleRole::Combobox, menu)?;
    app.settle();
    input::click_button(app.slint(), item)?;
    app.settle();
    Ok(())
}

pub fn view_sort(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "view-sort")?;
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let specs = specs();
    for spec in &specs {
        crate::tools_support::write_at(
            &folder.join(&spec.name),
            &vec![b'x'; spec.size],
            base + Duration::from_secs(spec.minute * 60),
        )?;
    }
    for name in ["dir-a", "dir-b"] {
        ps::folder(&folder.join(name))?;
    }
    let other = support::scratch(context, "view-sort-right")?;
    for (name, size) in [("big.bin", 300), ("mid.bin", 200), ("small.bin", 100)] {
        ps::write(&other.join(name), vec![0_u8; size])?;
    }
    let store = SettingsStore::new();
    let mut config = Config::new(&folder, &other);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let mut failures = Vec::new();

    for name in MARKED {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    panel::select_row(&mut app, LEFT, CURSOR)?;
    check_state(&mut app, &specs, "at the start", true, &mut failures);

    sorting(&mut app, context, &specs, &mut failures)?;
    views(&mut app, context, &specs, &mut failures)?;
    let split = splitter(&mut app, context, &mut failures)?;
    app.shutdown()?;
    drop(app);
    restored_split(&store, &folder, &other, split, &mut failures)?;
    touch(context, &folder, &other, &mut failures)?;
    support::report(&failures)
}

fn sorting(
    app: &mut App,
    context: &mut Context,
    specs: &[Spec],
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // Ctrl+F3–F6, each twice: a new column starts ascending, the repeat reverses.
    for (key, column) in [(Key::F3, 0), (Key::F4, 3), (Key::F5, 2), (Key::F6, 1)] {
        for press in 0..2 {
            panel::focus_list(app, LEFT)?;
            let wanted = after_request(app, column);
            input::chord(app.slint(), &[Key::Control], key);
            app.settle();
            let state = sort_state(app, LEFT);
            if state != wanted {
                failures.push(format!(
                    "Ctrl+{key:?} press {}: sort state {state:?}, expected {wanted:?}",
                    press + 1
                ));
            }
            check_state(
                app,
                specs,
                &format!("Ctrl+{key:?} press {}", press + 1),
                true,
                failures,
            );
        }
    }
    // Header clicks.
    for (label, column) in [
        ("Sort by Size", 1),
        ("Sort by Modified", 2),
        ("Sort by Extension", 3),
        ("Sort by Name", 0),
    ] {
        for press in 0..2 {
            let Some(item) = heading(app, label) else {
                failures.push(format!("no {label:?} header"));
                break;
            };
            let wanted = after_request(app, column);
            click_item(app, &item);
            let state = sort_state(app, LEFT);
            if state != wanted {
                failures.push(format!(
                    "{label} click {}: sort state {state:?}, expected {wanted:?}",
                    press + 1
                ));
            }
            let what = format!("{label} click {}", press + 1);
            check_state(app, specs, &what, true, failures);
            check_headings(app, &what, failures);
        }
        if column == 2 {
            // The Time header sorts the same column: it reverses it once more.
            if let Some(item) = heading(app, "Sort by Modified time") {
                let wanted = after_request(app, 2);
                click_item(app, &item);
                if sort_state(app, LEFT) != wanted {
                    failures.push(format!(
                        "the Time header after the Modified header: {:?}, expected {wanted:?}",
                        sort_state(app, LEFT)
                    ));
                }
                check_state(app, specs, "Time header click", true, failures);
                check_headings(app, "Time header click", failures);
            } else {
                failures.push("no Sort by Modified time header".into());
            }
        }
    }
    app.screenshot(
        context,
        "sorted-by-header",
        "Name header clicked twice: descending",
    );
    // The drive bar's sort menu: every column, then Reverse order.
    for (item, wanted) in [
        ("Size", (1, false)),
        ("Reverse order", (1, true)),
        ("Extension", (3, false)),
        ("Modified", (2, false)),
        ("Reverse order", (2, true)),
        ("Name", (0, false)),
    ] {
        if let Err(error) = menu(app, "Left panel sort", item) {
            failures.push(format!("sort menu {item}: {error}"));
            support::dismiss_all(app);
            continue;
        }
        if sort_state(app, LEFT) != wanted {
            failures.push(format!(
                "sort menu {item}: sort state {:?}, expected {wanted:?}",
                sort_state(app, LEFT)
            ));
        }
        let shown =
            support::accessible_value(app.slint(), AccessibleRole::Combobox, "Left panel sort");
        if shown.as_deref() != Some(column_name(wanted.0)) {
            failures.push(format!(
                "sort menu {item}: the menu shows {shown:?}, expected {:?}",
                column_name(wanted.0)
            ));
        }
        check_state(app, specs, &format!("sort menu {item}"), true, failures);
    }
    // The right panel sorts on its own.
    let left = sort_state(app, LEFT);
    panel::focus_list(app, RIGHT)?;
    input::chord(app.slint(), &[Key::Control], Key::F6);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        files(app, RIGHT) == ["small.bin", "mid.bin", "big.bin"]
    }) {
        failures.push(format!(
            "Ctrl+F6 in the right panel lists {:?}",
            files(app, RIGHT)
        ));
    }
    if sort_state(app, LEFT) != left {
        failures.push("sorting the right panel changed the left panel's sort".into());
    }
    panel::focus_list(app, LEFT)?;
    Ok(())
}

fn view_mode(app: &App) -> i32 {
    app.window.get_left_view_mode()
}

fn views(
    app: &mut App,
    context: &mut Context,
    specs: &[Spec],
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // The View menu.
    for (item, mode) in [("Brief", 1), ("Thumbnails", 2), ("Detailed", 0)] {
        if let Err(error) = menu(app, "Left panel view mode", item) {
            failures.push(format!("View menu {item}: {error}"));
            support::dismiss_all(app);
            continue;
        }
        if !ps::wait(app, Duration::from_secs(5), |app| view_mode(app) == mode) {
            failures.push(format!("View menu {item}: view mode {}", view_mode(app)));
        }
        check_state(app, specs, &format!("View menu {item}"), false, failures);
        match mode {
            1 => {
                if app.window.get_left_view_columns() < 2 {
                    failures.push(format!(
                        "Brief shows {} column(s)",
                        app.window.get_left_view_columns()
                    ));
                }
                app.screenshot(context, "brief", "Brief view, cursor and marks kept");
                arrows_brief(app, failures)?;
            }
            2 => {
                if app.window.get_left_view_row_height() < 80.0 {
                    failures.push(format!(
                        "Thumbnails rows are {} px tall",
                        app.window.get_left_view_row_height()
                    ));
                }
                app.screenshot(
                    context,
                    "thumbnails",
                    "Thumbnails view, cursor and marks kept",
                );
                arrows_thumbnails(app, failures)?;
            }
            _ => {}
        }
    }
    // Alt+3/2/5 as the native key hook delivers them.
    for (digit, mode) in [("Digit2", 1), ("Digit5", 2), ("Digit3", 0)] {
        panel::focus_list(app, LEFT)?;
        if !ps::physical(app, digit, false, true, false) {
            failures.push(format!("Alt+{digit} was not taken in the file list"));
        }
        if !ps::wait(app, Duration::from_secs(5), |app| view_mode(app) == mode) {
            failures.push(format!("Alt+{digit}: view mode {}", view_mode(app)));
        }
        check_state(app, specs, &format!("Alt+{digit}"), false, failures);
    }
    Ok(())
}

/// Brief: from the first row, Right moves one column (the rows of a column) further
/// and Left back; the cursor then returns to its file.
fn arrows_brief(app: &mut App, failures: &mut Vec<String>) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    let rows = app.window.get_left_view_rows();
    input::key(app.slint(), Key::Home);
    app.settle();
    let start = app.cursor(LEFT);
    input::key(app.slint(), Key::RightArrow);
    app.settle();
    let right = app.cursor(LEFT);
    if start != 0 || right != rows {
        failures.push(format!(
            "Right in Brief moved the cursor from row {start} to {right}, expected 0 → {rows} ({rows} rows per column)"
        ));
    }
    input::key(app.slint(), Key::LeftArrow);
    app.settle();
    if app.cursor(LEFT) != start {
        failures.push(format!(
            "Left in Brief went to row {}, expected back to {start}",
            app.cursor(LEFT)
        ));
    }
    panel::select_row(app, LEFT, CURSOR)?;
    Ok(())
}

/// Thumbnails: Right/Left one cell, Down/Up one grid row.
fn arrows_thumbnails(app: &mut App, failures: &mut Vec<String>) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    let columns = app.window.get_left_view_columns();
    let start = app.cursor(LEFT);
    for (key, delta) in [
        (Key::RightArrow, 1),
        (Key::LeftArrow, -1),
        (Key::DownArrow, columns),
        (Key::UpArrow, -columns),
    ] {
        let before = app.cursor(LEFT);
        input::key(app.slint(), key);
        app.settle();
        if app.cursor(LEFT) != before + delta {
            failures.push(format!(
                "{key:?} in Thumbnails ({columns} per row) moved the cursor {before} → {}, expected {}",
                app.cursor(LEFT),
                before + delta
            ));
        }
    }
    if app.cursor(LEFT) != start {
        failures.push("the Thumbnails arrow round trip did not return to the start".into());
    }
    Ok(())
}

fn splitter_item(app: &App) -> Option<ItemRc> {
    inspect::find(app.slint(), AccessibleRole::Button, "Panel splitter")
}

fn widths(app: &App) -> (f32, f32) {
    (
        app.window.get_left_pane_width(),
        app.window.get_right_pane_width(),
    )
}

/// The visible file-name texts of the left list (found by their accessible label, the
/// raw name): how many there are and those cut short (elided).
fn elided_names(app: &App) -> (usize, Vec<String>) {
    let left = app.window.get_left_list_left();
    let right = left + app.window.get_left_list_width();
    let names: Vec<String> = ps::shown(app, LEFT);
    let mut checked = 0;
    let mut cut = Vec::new();
    for root in inspect::roots(app.slint()).iter().take(1) {
        inspect::walk(root, &mut |item| {
            let is_text =
                item.downcast::<SimpleText>().is_some() || item.downcast::<ComplexText>().is_some();
            let Some(label) = inspect::label(item) else {
                return;
            };
            let (x, _, width, _) = inspect::rect(item);
            if is_text
                && names.contains(&label)
                && x >= left
                && x < right
                && item.is_visible()
                && let Some((_, _, shown, _)) = inspect::visible_rect(item)
            {
                checked += 1;
                let needed = preferred(item);
                if needed > shown.min(width) + 1.0 {
                    cut.push(format!("{label} ({shown:.0} of {needed:.0} px)"));
                }
            }
        });
    }
    (checked, cut)
}

/// A text item's preferred width.
pub fn preferred(item: &ItemRc) -> f32 {
    use i_slint_core::{items::Item, layout::Orientation};
    let Some(adapter) = item.window_adapter() else {
        return 0.0;
    };
    if let Some(text) = item.downcast::<SimpleText>() {
        return text
            .as_pin_ref()
            .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
            .preferred;
    }
    if let Some(text) = item.downcast::<ComplexText>() {
        return text
            .as_pin_ref()
            .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
            .preferred;
    }
    0.0
}

/// The splitter by pointer and keys; the dragged ratio and left width it ends with.
fn splitter(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
) -> Result<(f32, f32), String> {
    let item = splitter_item(app).ok_or("no Panel splitter")?;
    let (left_before, right_before) = widths(app);
    let center = inspect::center(&item);
    // Drag 200 px to the left.
    input::drag(
        app.slint(),
        center,
        LogicalPosition::new(center.x - 200.0, center.y),
        10,
    );
    app.settle();
    let (left, right) = widths(app);
    if (left - (left_before - 200.0)).abs() > 2.0 || (right - (right_before + 200.0)).abs() > 2.0 {
        failures.push(format!(
            "dragging the splitter 200 px left: widths {left_before}/{right_before} → {left}/{right}"
        ));
    }
    let saved_ratio = app.window.get_panel_ratio();
    // Far past the minimum: the left panel keeps its minimum width and its names.
    let item = splitter_item(app).ok_or("no Panel splitter")?;
    let center = inspect::center(&item);
    input::drag(
        app.slint(),
        center,
        LogicalPosition::new(20.0, center.y),
        10,
    );
    app.settle();
    let (narrow, _) = widths(app);
    if narrow < 280.0 {
        failures.push(format!(
            "dragging the splitter to the window edge left the panel {narrow} px wide (minimum 286)"
        ));
    }
    let (checked, cut) = elided_names(app);
    if checked < 10 {
        failures.push(format!(
            "at the narrowest split only {checked} file-name texts are visible in the left list"
        ));
    }
    if !cut.is_empty() {
        failures.push(format!(
            "at the narrowest split ({narrow} px) file names are cut: {:?}",
            cut.iter().take(6).collect::<Vec<_>>()
        ));
    }
    app.screenshot(context, "narrow-split", "The splitter at its minimum");
    // Double click centers.
    let item = splitter_item(app).ok_or("no Panel splitter")?;
    let center = inspect::center(&item);
    ps::double_click(app, center);
    app.settle();
    let (left, right) = widths(app);
    if (left - right).abs() > 2.0 || (app.window.get_panel_ratio() - 0.5).abs() > 0.001 {
        failures.push(format!(
            "double-clicking the splitter left the panels {left}/{right} (ratio {})",
            app.window.get_panel_ratio()
        ));
    }
    // Its keys: Left 12 px, Right back, Enter centers.
    let item = splitter_item(app).ok_or("no Panel splitter")?;
    app.idle(Duration::from_millis(650));
    input::click_at(app.slint(), inspect::center(&item));
    app.settle();
    for (key, wanted) in [
        (Key::LeftArrow, left - 12.0),
        (Key::LeftArrow, left - 24.0),
        (Key::RightArrow, left - 12.0),
    ] {
        input::key(app.slint(), key);
        app.settle();
        if (widths(app).0 - wanted).abs() > 1.5 {
            failures.push(format!(
                "splitter {key:?}: the left panel is {} px, expected {wanted}",
                widths(app).0
            ));
        }
    }
    input::key(app.slint(), Key::Return);
    app.settle();
    if (widths(app).0 - left).abs() > 1.5 {
        failures.push(format!(
            "Enter on the splitter: the left panel is {} px, expected the centered {left}",
            widths(app).0
        ));
    }
    // A dragged ratio survives a restart.
    let item = splitter_item(app).ok_or("no Panel splitter")?;
    let center = inspect::center(&item);
    input::drag(
        app.slint(),
        center,
        LogicalPosition::new(center.x + 150.0, center.y),
        10,
    );
    app.settle();
    let dragged = app.window.get_panel_ratio();
    let dragged_width = widths(app).0;
    context.note(format!(
        "splitter ratios: after 200 px left {saved_ratio:.4}, after 150 px right of center {dragged:.4}"
    ));
    zoom(app, failures)?;
    compact(app, context, failures)?;
    Ok((dragged, dragged_width))
}

/// After a restart on the same settings the panels keep the dragged split.
fn restored_split(
    store: &Arc<SettingsStore>,
    folder: &Path,
    other: &Path,
    (dragged, dragged_width): (f32, f32),
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let mut config = Config::new(folder, other);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let ratio = app.window.get_panel_ratio();
    if (ratio - dragged).abs() > 0.002 || (widths(&app).0 - dragged_width).abs() > 2.0 {
        failures.push(format!(
            "after a restart the split ratio is {ratio} ({} px), expected the dragged {dragged} ({dragged_width} px)",
            widths(&app).0
        ));
    }
    app.shutdown()
}

/// Ctrl+F11 enlarges the active panel; Tab moves the enlargement; Ctrl+F11 restores.
fn zoom(app: &mut App, failures: &mut Vec<String>) -> Result<(), String> {
    let before = widths(app);
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F11);
    app.settle();
    let (left, right) = widths(app);
    if !app.window.get_panel_zoomed() || right > 1.0 || left < before.0 + before.1 - 2.0 {
        failures.push(format!(
            "Ctrl+F11: zoomed {}, widths {left}/{right} (before {before:?})",
            app.window.get_panel_zoomed()
        ));
    }
    input::key(app.slint(), Key::Tab);
    app.settle();
    let (left, right) = widths(app);
    if app.window.get_active_pane() != 1 || left > 1.0 || right < before.0 + before.1 - 2.0 {
        failures.push(format!(
            "Tab while enlarged: active pane {}, widths {left}/{right}: the enlargement did not move to the right panel",
            app.window.get_active_pane()
        ));
    }
    input::key(app.slint(), Key::Tab);
    app.settle();
    input::chord(app.slint(), &[Key::Control], Key::F11);
    app.settle();
    let after = widths(app);
    if app.window.get_panel_zoomed()
        || (after.0 - before.0).abs() > 1.5
        || (after.1 - before.1).abs() > 1.5
    {
        failures.push(format!(
            "the second Ctrl+F11: zoomed {}, widths {after:?}, expected the split {before:?}",
            app.window.get_panel_zoomed()
        ));
    }
    Ok(())
}

/// A small window shows the active panel and a rail that activates the other one.
fn compact(app: &mut App, context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let before = widths(app);
    app.resize(600, 480);
    app.settle();
    let rail = inspect::find(app.slint(), AccessibleRole::Button, "Activate right panel");
    match rail {
        None => failures.push(format!(
            "a 600×480 window shows no rail for the right panel; buttons {:?}",
            inspect::labels(app.slint(), AccessibleRole::Button)
                .into_iter()
                .filter(|label| label.contains("panel"))
                .collect::<Vec<_>>()
        )),
        Some(rail) => {
            app.screenshot(context, "compact", "A 600×480 window: one panel and a rail");
            click_item(app, &rail);
            if app.window.get_active_pane() != 1 {
                failures.push("clicking the rail did not activate the right panel".into());
            }
            if inspect::find(app.slint(), AccessibleRole::Button, "Activate left panel").is_none() {
                failures.push(
                    "with the right panel active the rail does not offer the left one".into(),
                );
            }
        }
    }
    app.resize(1340, 820);
    app.settle();
    panel::focus_list(app, LEFT)?;
    let after = widths(app);
    if (after.0 - before.0).abs() > 1.5 || (after.1 - before.1).abs() > 1.5 {
        failures.push(format!(
            "back at 1340×820 the panels are {after:?}, expected the split {before:?}"
        ));
    }
    Ok(())
}

/// Header sorting and the splitter at touch density.
fn touch(
    context: &mut Context,
    folder: &Path,
    other: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let mut config = Config::new(folder, other);
    config.density = UiDensity::Touch;
    let mut app = App::start(config);
    let specs = specs();
    match heading(&app, "Sort by Size") {
        None => failures.push("touch: no Sort by Size header".into()),
        Some(item) => {
            click_item(&mut app, &item);
            let wanted = expected(&specs, 1, false);
            if !ps::wait(&mut app, Duration::from_secs(5), |app| {
                files(app, LEFT) == wanted
            }) {
                failures.push(format!(
                    "touch: the Size header sorted {:?}",
                    files(&app, LEFT).iter().take(8).collect::<Vec<_>>()
                ));
            }
        }
    }
    let (left_before, _) = widths(&app);
    match splitter_item(&app) {
        None => failures.push("touch: no Panel splitter".into()),
        Some(item) => {
            let (_, _, width, _) = inspect::rect(&item);
            if width < 11.0 {
                failures.push(format!(
                    "touch: the splitter is {width} px wide, expected 12"
                ));
            }
            let center = inspect::center(&item);
            input::drag(
                app.slint(),
                center,
                LogicalPosition::new(center.x + 100.0, center.y),
                10,
            );
            app.settle();
            if (widths(&app).0 - (left_before + 100.0)).abs() > 2.0 {
                failures.push(format!(
                    "touch: dragging the splitter 100 px right made the left panel {} px (from {left_before})",
                    widths(&app).0
                ));
            }
        }
    }
    app.screenshot(context, "touch-sorted", "Touch density: sorted by size");
    app.shutdown()
}
