//! Helpers of the Find scenarios (`find_*.rs`, `tools_find.rs`): the Find window's form,
//! status line, details page and virtualized results; the list Feed to panel shows; the
//! main menu; and a stand-in for the application's Find viewer hook.
use crate::{
    app::{self, App},
    input, inspect, panel,
    tools_support::{self as tools, Tool},
};
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
};
use midnight_core::PaneId;
use midnight_ui::{FeedPresentation, FeedRow, FeedView};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

/// Wide enough for every results column, the Identical group column included.
pub const SIZE: (u32, u32) = (1300, 900);
pub const MODES: [&str; 3] = ["Masks", "Regex", "Expression"];
pub const UNITS: [&str; 3] = ["Minutes", "Hours", "Days"];
/// The details page and error lines open on click; both say so.
pub const MORE: &str = "Click for more details";
const STATUSES: [&str; 10] = [
    "Ready ·",
    "Searching",
    "Stopping",
    "Hashing",
    "Comparing",
    "Completed",
    "Incomplete",
    "Cancelled",
    "Failed",
    "Error ·",
];

/// The Find form as a user fills it in.
#[derive(Clone, Copy)]
pub struct Query<'a> {
    pub mode: &'a str,
    pub masks: &'a str,
    pub name_case: bool,
    pub subfolders: bool,
    pub content: Option<&'a str>,
    pub content_case: bool,
    pub whole_words: bool,
    pub content_regex: bool,
    /// Modified within: the number and its unit (Minutes, Hours or Days).
    pub modified: Option<(&'a str, &'a str)>,
    pub identical: bool,
    pub archives: bool,
}

pub const BASE: Query<'static> = Query {
    mode: "Masks",
    masks: "*.*",
    name_case: false,
    subfolders: true,
    content: None,
    content_case: false,
    whole_words: false,
    content_regex: false,
    modified: None,
    identical: false,
    archives: false,
};

/// One result row, from its accessible label.
#[derive(Clone, Debug)]
pub struct Row {
    pub index: usize,
    pub group: Option<String>,
    pub name: String,
    pub extension: String,
    /// The Path column: the parent folder (for archive members, inside the archive).
    pub folder: String,
    pub size: String,
    pub modified: String,
    pub selected: bool,
}

impl Row {
    /// The folder joined with the name.
    pub fn path(&self) -> PathBuf {
        Path::new(&self.folder).join(&self.name)
    }
}

pub fn is_status(label: &str) -> bool {
    STATUSES.iter().any(|prefix| label.starts_with(prefix))
}

/// The status line ("Completed · 3 found · 12 examined"): under the results, or in the
/// query area of a short window with large text, where it may be scrolled out of view.
pub fn status(window: &Tool) -> String {
    if let Some(shown) = inspect::labels(window.window(), AccessibleRole::Button)
        .into_iter()
        .rfind(|label| is_status(label))
    {
        return shown;
    }
    let mut found = String::new();
    for root in inspect::roots(window.window()) {
        inspect::walk(&root, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && let Some(label) = inspect::label(item)
                && is_status(&label)
            {
                found = label;
            }
        });
    }
    found
}

pub fn terminal(text: &str) -> bool {
    ["Completed", "Incomplete", "Cancelled", "Failed", "Error"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

/// The red error line above the results, if one is shown.
pub fn error_line(window: &Tool) -> Option<String> {
    inspect::controls(window.window(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, item)| !is_status(label) && tools::description(item) == MORE)
        .map(|(label, _)| label)
}

/// Wait until the error line shows a text that satisfies `wanted`; returns it.
pub fn wait_error(
    app: &mut App,
    window: &Tool,
    what: &str,
    wanted: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.settle();
        if let Some(text) = error_line(window)
            && wanted(&text)
        {
            return Ok(text);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what}: the Find window shows the error {:?} (status {:?})",
                error_line(window),
                status(window)
            ));
        }
        app.idle(Duration::from_millis(20));
    }
}

pub fn parse_row(label: &str, index: usize, selected: bool) -> Row {
    let mut parts: Vec<&str> = label.split(" · ").collect();
    let group = if parts.first().is_some_and(|part| part.starts_with("Group ")) && parts.len() >= 7
    {
        let group = format!("{} · {}", parts[0], parts[1]);
        parts.drain(..2);
        Some(group)
    } else {
        None
    };
    let field = |index: usize| parts.get(index).copied().unwrap_or_default().to_owned();
    Row {
        index,
        group,
        name: field(0),
        extension: field(1),
        folder: field(2),
        size: field(3),
        modified: field(4),
        selected,
    }
}

/// Rows of the "Find results" list currently instantiated, by token. The list
/// instantiates rows for its viewport while it is drawn, so the window is drawn first.
pub fn visible_rows(window: &Tool) -> BTreeMap<String, Row> {
    let _ = app::render(window);
    let mut rows = BTreeMap::new();
    if let Some(list) = inspect::find(window.window(), AccessibleRole::List, "Find results") {
        inspect::walk(&list, &mut |item| {
            if item.accessible_role() == AccessibleRole::ListItem
                && item.is_visible()
                && let Some(label) = inspect::label(item)
            {
                let property = |which| {
                    item.accessible_string_property(which)
                        .map(|value| value.to_string())
                };
                let index = property(AccessibleStringProperty::ItemIndex)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(usize::MAX);
                let selected =
                    property(AccessibleStringProperty::ItemSelected).as_deref() == Some("true");
                rows.insert(tools::value(item), parse_row(&label, index, selected));
            }
        });
    }
    rows
}

pub fn result_count(window: &Tool) -> usize {
    inspect::find(window.window(), AccessibleRole::List, "Find results")
        .and_then(|list| tools::item_count(&list))
        .unwrap_or(0)
}

/// Turn the mouse wheel over the results list by `delta` (positive scrolls up).
pub fn wheel(app: &mut App, window: &Tool, delta: f32) -> Result<(), String> {
    let list = tools::control(window.window(), AccessibleRole::List, "Find results")?;
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: inspect::center(&list),
            delta_x: 0.0,
            delta_y: delta,
        });
    app.settle();
    Ok(())
}

/// Scroll the results list back to its first row.
pub fn to_top(app: &mut App, window: &Tool) -> Result<(), String> {
    wheel(app, window, 1.0e7)
}

/// The shown rows' tokens with their places, and the result count.
fn shown_order(window: &Tool) -> (usize, Vec<(String, usize)>) {
    (
        result_count(window),
        visible_rows(window)
            .into_iter()
            .map(|(token, row)| (token, row.index))
            .collect(),
    )
}

/// Every result, in display order: rows are virtualized, so the list is scrolled
/// with the mouse wheel, a screenful at a time, until all of them were seen.
pub fn all_rows(app: &mut App, window: &Tool) -> Result<Vec<Row>, String> {
    // The engine sorts on a worker and delivers the order up to a quarter second after
    // the results: rows that move while the list is scrolled could pass unseen, so the
    // shown rows must hold still (their tokens and places) for 300 ms first.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last = shown_order(window);
    while Instant::now() < deadline {
        app.idle(Duration::from_millis(300));
        let now = shown_order(window);
        if now == last {
            break;
        }
        last = now;
    }
    let total = result_count(window);
    let mut seen = visible_rows(window);
    if seen.is_empty() && total > 0 {
        // A list left scrolled past a shorter new result list shows nothing, and the
        // wheel cannot move content that fits (`find_result_actions` reports that);
        // Home in the focused list brings the first row back, as a user can.
        let list = tools::control(window.window(), AccessibleRole::List, "Find results")?;
        input::click_at(window.window(), inspect::center(&list));
        input::key(window.window(), Key::Home);
        app.settle();
        seen = visible_rows(window);
    }
    if seen.len() < total {
        let list = tools::control(window.window(), AccessibleRole::List, "Find results")?;
        let step = (inspect::rect(&list).3 * 0.75).max(40.0);
        to_top(app, window)?;
        let mut stalled = 0;
        while seen.len() < total && stalled < 3 {
            let before = seen.len();
            seen.extend(visible_rows(window));
            wheel(app, window, -step)?;
            stalled = if seen.len() == before { stalled + 1 } else { 0 };
        }
        seen.extend(visible_rows(window));
        // A screenful at a time can pass rows that a frame had not laid out yet: go
        // through again a row's height at a time, letting each frame settle.
        if seen.len() < total {
            to_top(app, window)?;
            app.idle(Duration::from_millis(50));
            let step = (inspect::rect(&list).3 / 12.0).max(16.0);
            let mut stalled = 0;
            while seen.len() < total && stalled < 6 {
                let before = seen.len();
                seen.extend(visible_rows(window));
                wheel(app, window, -step)?;
                app.idle(Duration::from_millis(20));
                stalled = if seen.len() == before { stalled + 1 } else { 0 };
            }
            seen.extend(visible_rows(window));
        }
        // Back to the top, as a reader leaves the list: the next search starts there.
        to_top(app, window)?;
    }
    if seen.len() != total {
        let shown: BTreeSet<usize> = seen.values().map(|row| row.index).collect();
        let missing: Vec<usize> = (0..total).filter(|index| !shown.contains(index)).collect();
        return Err(format!(
            "scrolling showed {} of {total} results (rows never shown: {missing:?}; list {} \
             px high)",
            seen.len(),
            tools::control(window.window(), AccessibleRole::List, "Find results")
                .map_or(0.0, |list| inspect::rect(&list).3)
        ));
    }
    let mut rows: Vec<Row> = seen.into_values().collect();
    rows.sort_by_key(|row| row.index);
    Ok(rows)
}

/// Wait until the result names, in display order, are `wanted`: the engine sorts on a
/// worker and delivers the order up to a quarter second after the results. Returns the
/// rows.
pub fn wait_names(
    app: &mut App,
    window: &Tool,
    wanted: &[&str],
    what: &str,
) -> Result<Vec<Row>, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let rows = all_rows(app, window)?;
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        if names == wanted {
            return Ok(rows);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what}: the results read {names:?}, expected {wanted:?}"
            ));
        }
        app.idle(Duration::from_millis(50));
    }
}

/// Paths of the rows relative to `root` (or whole, outside it), in display order.
pub fn ordered(rows: &[Row], root: &Path) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let full = row.path();
            full.strip_prefix(root).map_or_else(
                |_| full.display().to_string(),
                crate::files_support::slashed,
            )
        })
        .collect()
}

/// Relative paths of the rows below `root`.
pub fn relative(rows: &[Row], root: &Path) -> BTreeSet<String> {
    ordered(rows, root).into_iter().collect()
}

pub fn set(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|item| (*item).to_owned()).collect()
}

/// Make sure the More options section is shown (its controls are hidden otherwise).
pub fn more_options(app: &mut App, window: &Tool) -> Result<(), String> {
    if tools::has(window.window(), AccessibleRole::Button, "More options") {
        input::click_button(window.window(), "More options")?;
        app.settle();
    }
    tools::control(window.window(), AccessibleRole::Checkbox, "Identical files").map(|_| ())
}

/// Fill the form as `query` says.
pub fn fill(app: &mut App, window: &Tool, query: Query<'_>) -> Result<(), String> {
    let w = window.window();
    tools::choose_combo(app, w, "Find match mode", &MODES, query.mode)?;
    input::fill(w, "File name masks", query.masks)?;
    tools::set_check(app, w, "Match name case", query.name_case)?;
    tools::set_check(app, w, "Subfolders", query.subfolders)?;
    if query.content.is_none() && tools::is_checked(w, "Containing text")? {
        tools::set_check(app, w, "Match case", false)?;
        tools::set_check(app, w, "Whole words", false)?;
        tools::set_check(app, w, "Regular expression", false)?;
    }
    tools::set_check(app, w, "Containing text", query.content.is_some())?;
    if let Some(text) = query.content {
        input::fill(w, "Text to find", text)?;
        tools::set_check(app, w, "Match case", query.content_case)?;
        tools::set_check(app, w, "Whole words", query.whole_words)?;
        tools::set_check(app, w, "Regular expression", query.content_regex)?;
    }
    more_options(app, window)?;
    tools::set_check(app, w, "Modified within", query.modified.is_some())?;
    if let Some((number, unit)) = query.modified {
        input::fill(w, "Modified age", number)?;
        tools::choose_combo(app, w, "Modified age unit", &UNITS, unit)?;
    }
    // Identical files disables Search inside archives: set that one first.
    if !query.identical {
        tools::set_check(app, w, "Identical files", false)?;
    }
    tools::set_check(app, w, "Search inside archives", query.archives)?;
    tools::set_check(app, w, "Identical files", query.identical)?;
    app.settle();
    Ok(())
}

/// Click Find and wait for a terminal status; returns it.
pub fn run(app: &mut App, window: &Tool) -> Result<String, String> {
    run_for(app, window, app::WAIT)
}

/// [`run`] with its own limit for searches that take long.
pub fn run_for(app: &mut App, window: &Tool, limit: Duration) -> Result<String, String> {
    tools::click_last(window.window(), "Find")?;
    // The window shows the running search synchronously.
    let started = status(window);
    if !started.starts_with("Searching") && !terminal(&started) {
        return Err(format!(
            "after Find the status reads {started:?} (error {:?})",
            error_line(window)
        ));
    }
    let deadline = Instant::now() + limit;
    loop {
        app.settle();
        let text = status(window);
        let find_shown = inspect::find(window.window(), AccessibleRole::Button, "Find").is_some();
        if terminal(&text) && find_shown {
            return Ok(text);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the search did not finish within {limit:?}: status {text:?}, Find button shown {find_shown}, error {:?}, buttons {:?}",
                error_line(window),
                inspect::labels(window.window(), AccessibleRole::Button)
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

pub fn search(
    app: &mut App,
    window: &Tool,
    query: Query<'_>,
) -> Result<(String, Vec<Row>), String> {
    fill(app, window, query)?;
    let text = run(app, window)?;
    let rows = all_rows(app, window)?;
    Ok((text, rows))
}

/// Alt+F7 on `pane`: a new Find window, sized for the scenario.
pub fn open_on(app: &mut App, pane: PaneId) -> Result<Tool, String> {
    panel::focus_list(app, pane)?;
    Ok(tools::open_tool(
        app,
        "the Find window",
        &[Key::Alt],
        Key::F7,
        SIZE,
    ))
}

pub fn open_find(app: &mut App) -> Result<Tool, String> {
    open_on(app, PaneId::Left)
}

/// The texts of the details page the status line opens; Back returns to the query.
pub fn details(app: &mut App, window: &Tool) -> Result<Vec<String>, String> {
    let line = status(window);
    let item = tools::control(window.window(), AccessibleRole::Button, &line)?;
    input::click_at(window.window(), inspect::center(&item));
    app.settle();
    if !tools::has(window.window(), AccessibleRole::Button, "Back") {
        return Err(format!(
            "clicking the status line {line:?} opened no details"
        ));
    }
    let texts = inspect::visible_texts(window.window());
    input::click_button(window.window(), "Back")?;
    app.settle();
    Ok(texts)
}

/// Whether one of `texts` contains `needle`.
pub fn mentions(texts: &[String], needle: &str) -> bool {
    texts.iter().any(|text| text.contains(needle))
}

/// The number before `unit` on the details line that starts with `line` (numbers are
/// shown with the locale's grouping), such as `("Identical files:", " candidates")`.
pub fn detail_count(texts: &[String], line: &str, unit: &str) -> Option<u64> {
    texts
        .iter()
        .flat_map(|text| text.lines())
        .find(|shown| shown.starts_with(line))
        .and_then(|shown| tools::count_before(shown.trim_start_matches(line), unit))
}

/// The visible result row whose name is `name` (the list is scrolled to it); returns
/// its (index, window rectangle).
pub fn reveal_row(
    app: &mut App,
    window: &Tool,
    wanted: impl Fn(&Row) -> bool,
) -> Result<(Row, inspect::Rect), String> {
    to_top(app, window)?;
    for _ in 0..400 {
        let _ = app::render(window);
        let mut found = None;
        if let Some(list) = inspect::find(window.window(), AccessibleRole::List, "Find results") {
            inspect::walk(&list, &mut |item| {
                if found.is_none()
                    && item.accessible_role() == AccessibleRole::ListItem
                    && let Some(label) = inspect::label(item)
                    && let Some(rect) = inspect::visible_rect(item)
                    && rect.3 >= 8.0
                {
                    let row = parse_row(&label, 0, false);
                    if wanted(&row) {
                        found = Some((item.clone(), rect));
                    }
                }
            });
        }
        if let Some((item, rect)) = found {
            let index = item
                .accessible_string_property(AccessibleStringProperty::ItemIndex)
                .and_then(|value| value.parse().ok())
                .unwrap_or(usize::MAX);
            let label = inspect::label(&item).unwrap_or_default();
            return Ok((parse_row(&label, index, false), rect));
        }
        let before = visible_rows(window);
        wheel(app, window, -60.0)?;
        if visible_rows(window).keys().eq(before.keys()) {
            break;
        }
    }
    Err("no such result row after scrolling the list".into())
}

/// Click the result row `name` (in `folder`, when given) once: it becomes the
/// selected row and the list takes the keyboard.
pub fn select_result(
    app: &mut App,
    window: &Tool,
    name: &str,
    folder: Option<&Path>,
) -> Result<Row, String> {
    let (row, (x, y, w, h)) = reveal_row(app, window, |row| {
        row.name == name && folder.is_none_or(|folder| Path::new(&row.folder) == folder)
    })
    .map_err(|error| format!("result {name}: {error}"))?;
    input::click_at(
        window.window(),
        LogicalPosition::new(x + w.min(160.0) / 2.0, y + h / 2.0),
    );
    app.settle();
    let selected = visible_rows(window)
        .into_values()
        .find(|shown| shown.selected)
        .map(|shown| shown.name);
    if selected.as_deref() != Some(name) {
        return Err(format!("clicking the result {name} selected {selected:?}"));
    }
    Ok(row)
}

/// The name of the selected result row, if one is selected and in view.
pub fn selected_name(window: &Tool) -> Option<String> {
    visible_rows(window)
        .into_values()
        .find(|row| row.selected)
        .map(|row| row.name)
}

/// Whether the visible button with this label is enabled.
pub fn button_enabled(window: &Tool, label: &str) -> Result<bool, String> {
    inspect::find_all(window.window(), AccessibleRole::Button, label)
        .pop()
        .map(|item| tools::enabled(&item))
        .ok_or_else(|| {
            format!(
                "no {label:?} button; buttons: {:?}",
                inspect::labels(window.window(), AccessibleRole::Button)
            )
        })
}

/// Whether the visible check box with this label is enabled.
pub fn check_enabled(window: &Tool, label: &str) -> Result<bool, String> {
    tools::control(window.window(), AccessibleRole::Checkbox, label)
        .map(|item| tools::enabled(&item))
}

/// Wait until the left (or right) panel shows `folder` with the cursor on `name`.
pub fn wait_cursor(app: &mut App, pane: PaneId, folder: &Path, name: &str) {
    let (folder, name) = (folder.to_path_buf(), name.to_owned());
    app.until(
        &format!("the {pane:?} panel to show {} at {name}", folder.display()),
        move |app| {
            let names = app.names(pane);
            app.location(pane).as_deref() == Some(folder.as_path())
                && !app.loading(pane)
                && usize::try_from(app.cursor(pane))
                    .ok()
                    .and_then(|row| names.get(row))
                    .is_some_and(|shown| *shown == name)
        },
    );
}

/// Close a Find window with Escape and wait until it is gone. A closed window gets no
/// more input: its component is gone, and an event sent to it would panic in Slint.
pub fn close(app: &mut App, window: &Tool) {
    if window.window().is_visible() {
        tools::close_tool(app, window, "the Find window");
    }
}

/// After a failed check: close every tool window (Escape closes the help or the Add
/// entry first, then the window, which also ends its search) and the notice, so the
/// next check starts from the main window. One Escape at a time, each only to a window
/// that is still shown (a closed window's component is gone).
pub fn reset(app: &mut App) {
    for _ in 0..8 {
        let open = app.tool_windows();
        if open.is_empty() && tools::shown_notice(app).is_none() {
            return;
        }
        if let Some(window) = open.last() {
            input::key(window.window(), Key::Escape);
        } else {
            input::key(app.slint(), Key::Escape);
        }
        app.idle(Duration::from_millis(100));
    }
}

// ---------------------------------------------------------------------------------------
// Search folders

/// The "Search folders" label: "Search folders (3)" or "(2 of 3)".
pub fn folders_label(window: &Tool) -> String {
    inspect::visible_texts(window.window())
        .into_iter()
        .find(|text| text.starts_with("Search folders"))
        .unwrap_or_default()
}

/// Add a typed folder: Add, the path, then Enter or the entry's OK button.
pub fn add_root(app: &mut App, window: &Tool, path: &Path, with_ok: bool) -> Result<(), String> {
    let w = window.window();
    if !tools::has(w, AccessibleRole::TextInput, "Folder to add") {
        input::click_button(w, "Add")?;
        app.settle();
    }
    let field = tools::control(w, AccessibleRole::TextInput, "Folder to add")?;
    input::click_at(w, inspect::center(&field));
    input::replace_text(w, &path.display().to_string());
    app.settle();
    if with_ok {
        input::click_button(w, "OK")?;
    } else {
        input::key(w, Key::Return);
    }
    app.settle();
    Ok(())
}

/// The Search folders tick boxes: (label, ticked), read from the opened dropdown.
pub fn root_boxes(app: &mut App, window: &Tool) -> Result<Vec<(String, bool)>, String> {
    let w = window.window();
    input::click(w, AccessibleRole::Combobox, "Search folders")?;
    app.settle();
    let mut found = Vec::new();
    if let Some(popup) = inspect::roots(w).last() {
        inspect::walk(popup, &mut |item| {
            if item.accessible_role() == AccessibleRole::Checkbox
                && let Some(label) = inspect::label(item)
            {
                found.push((label, tools::checked(item)));
            }
        });
    }
    input::key(w, Key::Escape);
    app.settle();
    Ok(found)
}

/// Tick or untick the folder `path` in the dropdown.
pub fn tick_root(app: &mut App, window: &Tool, path: &Path, ticked: bool) -> Result<(), String> {
    let w = window.window();
    let label = path.display().to_string();
    input::click(w, AccessibleRole::Combobox, "Search folders")?;
    app.settle();
    if tools::is_checked(w, &label)? != ticked {
        tools::click_check(w, &label)?;
        app.settle();
    }
    let now = tools::is_checked(w, &label)?;
    input::key(w, Key::Escape);
    app.settle();
    if now != ticked {
        return Err(format!("{label} stays ticked={now}"));
    }
    Ok(())
}

/// Choose `path` by its name in the dropdown, then Remove.
pub fn remove_root(app: &mut App, window: &Tool, path: &Path) -> Result<(), String> {
    let w = window.window();
    input::click(w, AccessibleRole::Combobox, "Search folders")?;
    app.settle();
    input::click_button(w, &path.display().to_string())?;
    app.settle();
    input::click_button(w, "Remove")?;
    app.settle();
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Viewers asked for from results

/// A viewer the Find window or a fed list asked the application to open.
#[derive(Clone, Debug)]
pub struct Launched {
    /// The file: the captured Find result, or the focused row of a fed list.
    pub path: PathBuf,
    /// Plain text (Alt+F3).
    pub basic: bool,
    /// A captured Find result (the Find window's View), not a list of files.
    pub captured: bool,
    /// The files the viewer walks with Space and Backspace (a fed list).
    pub entries: Vec<PathBuf>,
}

/// The application's Find viewer hook, as far as this harness can follow it.
/// Production (`viewers.rs`, `install_find_results`) takes every pending launch on
/// `refresh-find-viewer` and opens it in a viewer window; the harness installs no such
/// hook (its viewers only follow F3 in a panel), so these scenarios record each launch
/// the same way, deferred to a timer as production does, and drop it (its capture
/// retires on a backend worker). The viewer window itself is not opened.
pub fn record_launches(app: &App) -> Rc<RefCell<Vec<Launched>>> {
    let launched: Rc<RefCell<Vec<Launched>>> = Rc::default();
    let scheduled = Rc::new(Cell::new(false));
    let presenter = app.presenter.clone();
    let record = launched.clone();
    app.window.on_refresh_find_viewer(move || {
        if scheduled.replace(true) {
            return;
        }
        let (scheduled, presenter, record) = (scheduled.clone(), presenter.clone(), record.clone());
        slint::Timer::single_shot(Duration::ZERO, move || {
            scheduled.set(false);
            loop {
                let Ok(mut state) = presenter.try_borrow_mut() else {
                    return;
                };
                let Some(launch) = state.take_find_viewer() else {
                    break;
                };
                drop(state);
                record.borrow_mut().push(Launched {
                    path: launch
                        .find
                        .as_ref()
                        .map_or_else(|| launch.focused.clone(), |file| file.entry().path.clone()),
                    basic: launch.basic,
                    captured: launch.find.is_some(),
                    entries: launch
                        .entries
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect(),
                });
            }
        });
    });
    launched
}

/// Wait for the next recorded launch after `before`.
pub fn wait_launch(
    app: &mut App,
    launched: &Rc<RefCell<Vec<Launched>>>,
    before: usize,
    what: &str,
) -> Result<Launched, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.settle();
        if let Some(launch) = launched.borrow().get(before) {
            return Ok(launch.clone());
        }
        if Instant::now() >= deadline {
            return Err(format!("{what}: no viewer was asked for"));
        }
        app.idle(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------------------
// Search results fed to a panel

pub fn feed_view(app: &App, pane: PaneId) -> FeedView {
    let feeds = app.window.global::<FeedPresentation>();
    match pane {
        PaneId::Left => feeds.get_left(),
        PaneId::Right => feeds.get_right(),
    }
}

pub fn feed_rows(app: &App, pane: PaneId) -> Vec<FeedRow> {
    let feeds = app.window.global::<FeedPresentation>();
    let rows = match pane {
        PaneId::Left => feeds.get_left_rows(),
        PaneId::Right => feeds.get_right_rows(),
    };
    (0..rows.row_count())
        .filter_map(|row| rows.row_data(row))
        .collect()
}

pub fn feed_names(app: &App, pane: PaneId) -> Vec<String> {
    feed_rows(app, pane)
        .into_iter()
        .map(|row| row.name.to_string())
        .collect()
}

pub fn feed_marked(app: &App, pane: PaneId) -> BTreeSet<String> {
    feed_rows(app, pane)
        .into_iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect()
}

/// The name under the list's cursor.
pub fn feed_cursor(app: &App, pane: PaneId) -> Option<String> {
    let cursor = usize::try_from(feed_view(app, pane).cursor).ok()?;
    feed_names(app, pane).get(cursor).cloned()
}

/// The window rectangle of the fed row `name` (its accessible list item).
pub fn feed_row_rect(app: &App, name: &str) -> Option<inspect::Rect> {
    let prefix = format!("{name} · ");
    inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .filter(|(label, _)| label.starts_with(&prefix))
        .find_map(|(_, item)| inspect::visible_rect(&item))
}

// ---------------------------------------------------------------------------------------
// Notices and the main menu

/// Wait for a notice in the main window; returns it (still shown).
pub fn wait_notice(app: &mut App, what: &str) -> Result<(String, String), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.settle();
        if let Some(notice) = tools::shown_notice(app) {
            return Ok(notice);
        }
        if Instant::now() >= deadline {
            return Err(format!("{what}: no notice appeared"));
        }
        app.idle(Duration::from_millis(20));
    }
}

/// Close the shown notice with Escape.
pub fn dismiss_notice(app: &mut App) {
    if tools::shown_notice(app).is_some() {
        input::key(app.slint(), Key::Escape);
        app.until("the notice to close", |app| {
            tools::shown_notice(app).is_none()
        });
    }
}

const CATEGORIES: [&str; 6] = [
    "All actions",
    "Files",
    "Selection",
    "Panels and navigation",
    "Tools",
    "Application",
];

/// F10, the item's category, and a click on the item `label` (scrolled into view).
pub fn menu(app: &mut App, category: &str, label: &str) -> Result<(), String> {
    input::key(app.slint(), Key::F10);
    app.until("the main menu", |app| tools::popup_open(app.slint()));
    let main = app.main.clone();
    tools::choose_combo(app, main.window(), "Menu category", &CATEGORIES, category)?;
    let anchor = menu_anchor(app)?;
    let item = tools::scroll_to(app, main.window(), &anchor, AccessibleRole::Button, label)?;
    if !tools::enabled(&item) {
        return Err(format!("the menu item {label:?} is disabled"));
    }
    input::click(main.window(), AccessibleRole::Button, label)?;
    app.settle();
    Ok(())
}

fn menu_anchor(app: &App) -> Result<ItemRc, String> {
    let mut anchor = None;
    if let Some(menu) = inspect::roots(app.slint()).last() {
        inspect::walk(menu, &mut |item| {
            if anchor.is_none()
                && item.accessible_role() == AccessibleRole::Button
                && item.is_visible()
                && inspect::label(item).is_some_and(|label| label != "Close menu")
            {
                anchor = Some(item.clone());
            }
        });
    }
    anchor.ok_or_else(|| "the main menu shows no item".to_owned())
}

/// The main window's visible buttons whose label starts with `prefix`.
pub fn main_buttons(app: &App, prefix: &str) -> Vec<(String, ItemRc)> {
    inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(label, _)| label.starts_with(prefix))
        .collect()
}

/// A numeric keypad key as the native event loop delivers it (the window's physical
/// key hook), as `files_selection` sends it.
pub fn keypad(app: &mut App, name: &str) -> bool {
    let handled = app
        .window
        .invoke_physical_shortcut(name.into(), false, false, false, false);
    app.settle();
    handled
}

/// Findings collected while a scenario goes on; reported together at its end.
#[derive(Default)]
pub struct Findings(Vec<String>);

impl Findings {
    pub fn add(&mut self, text: impl Into<String>) {
        self.0.push(text.into());
    }

    /// Record a failed check and go on.
    pub fn check(&mut self, what: &str, result: Result<(), String>) {
        if let Err(error) = result {
            self.0.push(format!("{what}: {error}"));
        }
    }

    pub fn into_result(self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self.0.join("\n"))
        }
    }
}
