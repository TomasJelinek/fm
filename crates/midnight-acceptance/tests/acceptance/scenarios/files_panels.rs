//! Panel features driven by keys and clicks: every sort order in both directions,
//! history, parent and root, breadcrumbs and a typed path, swapping panels, refresh,
//! Properties, tabs (including the hover-only close button), Hot Paths and Make File
//! List with its preview; the command line's shell controls and history picker. The
//! panel filter, filename search and Show hidden files are in `files_filter_search`.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, io_error},
    input, inspect, panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use midnight_ui::{FileListPresentation, HotPathsPresentation};
use slint::{ComponentHandle, Model, platform::Key};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

const LEFT: PaneId = PaneId::Left;

/// File rows (no folders, no `..`) in displayed order.
fn file_rows(app: &App, pane: PaneId) -> Vec<String> {
    app.rows(pane)
        .iter()
        .filter(|row| !row.directory && !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect()
}

/// Folders are listed before files.
fn folders_first(app: &App, pane: PaneId) -> bool {
    let rows: Vec<bool> = app
        .rows(pane)
        .iter()
        .filter(|row| !row.parent_entry)
        .map(|row| row.directory)
        .collect();
    rows.windows(2).all(|pair| pair[0] || !pair[1])
}

/// A sorting scenario file (name, size, modified offset) to its expected sort key.
type SortKey = fn(&(&str, usize, u64)) -> (u8, String, usize, u64);

fn set_modified(path: &Path, time: SystemTime) -> Result<(), String> {
    crate::tools_support::times_handle(path)
        .and_then(|file| file.set_modified(time))
        .map_err(|error| io_error(path, error))
}

pub fn sorting(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "sorting")?;
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    // name, size, modified offset (minutes). The names sort the same in every common
    // collation: Windows sorts names by the user's locale, where Czech puts "ch" after
    // "h", so "charlie" would sort after "echo".
    let files: [(&str, usize, u64); 5] = [
        ("alpha.txt", 40, 3),
        ("bravo.log", 10, 1),
        ("coral.dat", 30, 5),
        ("delta.bin", 20, 2),
        ("echo", 50, 4),
    ];
    for (name, size, minutes) in files {
        let path = folder.join(name);
        fs::write(&path, vec![b'x'; size]).map_err(|error| io_error(&path, error))?;
        set_modified(&path, base + Duration::from_secs(minutes * 60))?;
    }
    for (name, minutes) in [("folder-a", 7), ("folder-b", 6)] {
        fs::create_dir(folder.join(name)).map_err(|error| io_error(&folder, error))?;
        set_modified(&folder.join(name), base + Duration::from_secs(minutes * 60))?;
    }
    let mut app = App::start(Config::new(&folder, &context.work));
    let mut failures = Vec::new();
    panel::focus_list(&mut app, LEFT)?;
    let by = |key: SortKey| {
        let mut sorted = files.to_vec();
        sorted.sort_by_key(key);
        sorted
            .into_iter()
            .map(|(name, _, _)| name.to_owned())
            .collect::<Vec<_>>()
    };
    let orders = [
        ("F3", "name", by(|entry| (0, entry.0.to_owned(), 0, 0))),
        // Files without an extension stay last in both directions.
        (
            "F4",
            "extension",
            by(|entry| {
                let extension = entry.0.rsplit_once('.').map(|(_, ext)| ext.to_owned());
                (
                    u8::from(extension.is_none()),
                    extension.unwrap_or_default(),
                    0,
                    0,
                )
            }),
        ),
        (
            "F5",
            "modified time",
            by(|entry| (0, String::new(), 0, entry.2)),
        ),
        ("F6", "size", by(|entry| (0, String::new(), entry.1, 0))),
    ];
    for (key, column, ascending) in orders {
        let function = match key {
            "F3" => Key::F3,
            "F4" => Key::F4,
            "F5" => Key::F5,
            _ => Key::F6,
        };
        let mut directions = Vec::new();
        for _ in 0..2 {
            // The sort state changes with every press (a new column, or the direction).
            let before = (app.window.get_left_sort(), app.window.get_left_descending());
            input::chord(app.slint(), &[Key::Control], function);
            let deadline = Instant::now() + Duration::from_secs(5);
            while (app.window.get_left_sort(), app.window.get_left_descending()) == before
                && Instant::now() < deadline
            {
                app.idle(Duration::from_millis(10));
            }
            let descending = app.window.get_left_descending();
            let mut expected = ascending.clone();
            if descending {
                if column == "extension" {
                    // Reverse the extension order, keep "echo" (no extension) last.
                    let last = expected.pop();
                    expected.reverse();
                    expected.extend(last);
                } else {
                    expected.reverse();
                }
            }
            let wanted = expected.clone();
            let deadline = Instant::now() + Duration::from_secs(10);
            while file_rows(&app, LEFT) != wanted && Instant::now() < deadline {
                app.idle(Duration::from_millis(20));
            }
            let shown = file_rows(&app, LEFT);
            if shown != expected {
                failures.push(format!(
                    "Ctrl+{key} ({column}, descending {descending}): files {shown:?}, expected {expected:?}"
                ));
            }
            if !folders_first(&app, LEFT) {
                failures.push(format!("Ctrl+{key}: folders are not listed first"));
            }
            directions.push(descending);
        }
        if directions[0] == directions[1] {
            failures.push(format!(
                "pressing Ctrl+{key} twice did not reverse the {column} order"
            ));
        }
        app.screenshot(
            context,
            &format!("sorted-{}", column.replace(' ', "-")),
            &format!("Sorted by {column}, pressed twice"),
        );
    }
    context.note("Ctrl+F3/F4/F5/F6 each pressed twice: name, extension (extensionless last), modified time and size in both directions, folders first");
    app.shutdown()?;
    support::report(&failures)
}

/// Click a breadcrumb ("Open <prefix>") of the left path bar.
fn click_breadcrumb(app: &mut App, prefix: &Path) -> Result<(), String> {
    let wanted = prefix.to_string_lossy().into_owned();
    let labels = inspect::labels(app.slint(), AccessibleRole::Button);
    let label = labels
        .iter()
        .find(|label| {
            label
                .strip_prefix("Open ")
                .is_some_and(|text| text.trim_end_matches('/') == wanted.trim_end_matches('/'))
        })
        .cloned()
        .ok_or_else(|| format!("no breadcrumb for {wanted}; buttons {labels:?}"))?;
    // Both panels show breadcrumbs; take the one inside the left panel.
    let left_edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    let item = inspect::find_all(app.slint(), AccessibleRole::Button, &label)
        .into_iter()
        .find(|item| inspect::center(item).x < left_edge)
        .ok_or("no breadcrumb in the left panel")?;
    input::click_at(app.slint(), inspect::center(&item));
    Ok(())
}

fn location(app: &App) -> Option<PathBuf> {
    app.location(LEFT)
}

/// Files, folders and file bytes below `folder` (itself not counted), links not
/// followed, as a folder's Properties count them.
fn contents(folder: &Path) -> (u64, u64, u64) {
    let mut totals = (0, 0, 0);
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            let inner = contents(&entry.path());
            totals = (
                totals.0 + inner.0,
                totals.1 + inner.1 + 1,
                totals.2 + inner.2,
            );
        } else if metadata.is_file() {
            totals = (totals.0 + 1, totals.1, totals.2 + metadata.len());
        }
    }
    totals
}

pub fn navigation(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, root.join("text")));
    let mut failures = Vec::new();

    // Enter and parent: the cursor returns to the folder left.
    panel::enter(&mut app, LEFT, "docs")?;
    panel::enter(&mut app, LEFT, "page_files")?;
    panel::leave(&mut app, LEFT)?;
    // History: root → docs → page_files → docs; back twice, forward twice.
    for (keys, wanted) in [
        (Key::LeftArrow, root.join("docs/page_files")),
        (Key::LeftArrow, root.join("docs")),
        (Key::RightArrow, root.join("docs/page_files")),
    ] {
        input::chord(app.slint(), &[Key::Alt], keys);
        let target = wanted.clone();
        let deadline = Instant::now() + Duration::from_secs(10);
        while location(&app).as_ref() != Some(&target) && Instant::now() < deadline {
            app.idle(Duration::from_millis(20));
        }
        if location(&app).as_ref() != Some(&target) {
            failures.push(format!(
                "Alt+{keys:?} went to {:?}, expected {}",
                location(&app),
                wanted.display()
            ));
        }
    }
    context.note(format!(
        "back history after root → docs → page_files → docs: {:?}",
        app.window
            .get_left_back_history()
            .iter()
            .map(|row| row.label.to_string())
            .collect::<Vec<_>>()
    ));

    // Filesystem root and back.
    panel::wait_input(&mut app);
    let before_root = location(&app);
    input::chord(app.slint(), &[Key::Control], Key::Backspace);
    let filesystem_root = crate::files_support::filesystem_root(&context.fixture.root);
    panel::wait_location(&mut app, LEFT, &filesystem_root);
    app.screenshot(
        context,
        "filesystem-root",
        "Ctrl+Backspace: filesystem root",
    );
    // Alt+Left goes back to the folder shown before the root.
    input::chord(app.slint(), &[Key::Alt], Key::LeftArrow);
    let back = before_root.clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    while (location(&app) != back || app.loading(LEFT)) && Instant::now() < deadline {
        app.idle(Duration::from_millis(20));
    }
    if location(&app) != before_root {
        failures.push(format!(
            "Alt+Left from the filesystem root went to {:?}, expected {before_root:?}",
            location(&app)
        ));
    }
    panel::wait_input(&mut app);

    // A typed path (Ctrl+G).
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &root.join("images").to_string_lossy());
    input::key(app.slint(), Key::Return);
    panel::wait_location(&mut app, LEFT, &root.join("images"));

    // A typed path that does not exist must be reported (a notice or the panel's
    // error), and the panel must not pretend to show it.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &root.join("no-such-folder").to_string_lossy());
    input::key(app.slint(), Key::Return);
    let reported_in = Instant::now() + Duration::from_secs(10);
    while support::shown_notice(&app).is_none()
        && app.error(LEFT).is_empty()
        && Instant::now() < reported_in
    {
        app.idle(Duration::from_millis(20));
    }
    app.idle(Duration::from_millis(200));
    let after_missing = location(&app);
    let error = app.error(LEFT);
    let notice = support::take_notice(&mut app);
    context.note(format!(
        "typed missing path: panel at {after_missing:?}, panel error {error:?}, notice {notice:?}"
    ));
    if notice.is_none() && error.is_empty() {
        failures
            .push("Ctrl+G to a missing folder reported nothing (no notice, no panel error)".into());
    } else if after_missing.as_deref() == Some(root.join("no-such-folder").as_path())
        && error.is_empty()
    {
        failures.push("the panel shows the missing folder without its error".into());
    }
    support::dismiss_all(&mut app);
    input::key(app.slint(), Key::Escape);
    app.settle();

    // Breadcrumb: back to the fixture root by clicking its path segment.
    if location(&app).as_deref() != Some(root.join("images").as_path()) {
        panel::focus_list(&mut app, LEFT)?;
        input::chord(app.slint(), &[Key::Control], "g");
        app.settle();
        input::replace_text(app.slint(), &root.join("images").to_string_lossy());
        input::key(app.slint(), Key::Return);
        panel::wait_location(&mut app, LEFT, &root.join("images"));
    }
    click_breadcrumb(&mut app, &root)?;
    panel::wait_location(&mut app, LEFT, &root);
    app.screenshot(
        context,
        "breadcrumb",
        "Breadcrumb click back to the fixture root",
    );

    // Swap panels (Ctrl+U) twice.
    let before = (
        app.window.get_left_pane_left(),
        app.window.get_right_pane_left(),
    );
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "u");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.window.get_panels_swapped() && Instant::now() < deadline {
        app.idle(Duration::from_millis(10));
    }
    let swapped = app.window.get_panels_swapped();
    let after = (
        app.window.get_left_pane_left(),
        app.window.get_right_pane_left(),
    );
    if !swapped || (after.0 < after.1) == (before.0 < before.1) {
        failures.push(format!(
            "Ctrl+U did not swap the panels (swapped {swapped}, x before {before:?}, after {after:?})"
        ));
    }
    if location(&app).as_deref() != Some(root.as_path())
        || app.location(PaneId::Right).as_deref() != Some(root.join("text").as_path())
    {
        failures.push("Ctrl+U changed the panels' folders".into());
    }
    app.screenshot(context, "swapped", "Ctrl+U: panels swapped");
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "u");
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.window.get_panels_swapped() && Instant::now() < deadline {
        app.idle(Duration::from_millis(10));
    }
    if app.window.get_panels_swapped() {
        failures.push("a second Ctrl+U did not swap the panels back".into());
    }

    // Properties (Alt+Enter): Linux has no system properties dialog
    // (`PropertyService::show_properties` returns false), so the application shows its
    // own read-only summary, as it does here without desktop services.
    panel::select_row(&mut app, LEFT, "text")?;
    input::chord(app.slint(), &[Key::Alt], Key::Return);
    app.until("the Properties answer", |app| {
        support::shown_notice(app).is_some() || !app.tool_windows().is_empty()
    });
    // The summary names the folder, its type, permissions and time, and counts
    // its contents (files, folders and bytes as on disk) once the backend has.
    let text = root.join("text");
    let (files, folders, bytes) = contents(&text);
    let counted = format!(
        "Contents: {files} {}, {folders} {}, ",
        if files == 1 { "file" } else { "files" },
        if folders == 1 { "folder" } else { "folders" }
    );
    let counted_shown = counted.clone();
    let finished = Instant::now() + Duration::from_secs(20);
    while Instant::now() < finished
        && !support::shown_notice(&app).is_some_and(|(_, body)| body.contains(&counted_shown))
    {
        app.idle(Duration::from_millis(20));
    }
    let notice = support::shown_notice(&app);
    let windows = app.tool_windows().len();
    context.note(format!(
        "Alt+Enter Properties of the text folder: notice {notice:?}, {windows} window(s)"
    ));
    app.screenshot(context, "properties", "Alt+Enter Properties of a folder");
    match &notice {
        None => failures.push(format!(
            "Alt+Enter on the text folder showed no Properties ({windows} window(s))"
        )),
        Some((title, body)) => {
            // Owner, group, created and accessed come from the desktop service's
            // property details, which this headless harness does not install.
            let mut missing: Vec<String> = ["Type: Folder", "Modified: "]
                .iter()
                .filter(|part| !body.contains(*part))
                .map(|part| (*part).to_owned())
                .collect();
            if cfg!(unix) && !body.contains("Permissions: ") {
                missing.push("Permissions: ".into());
            }
            let total = body
                .lines()
                .find(|line| line.starts_with(&counted))
                .and_then(|line| crate::tools_support::number(&line[counted.len()..]));
            if total != Some(bytes) {
                missing.push(format!("{counted}{bytes} bytes"));
            }
            if title.as_str() != text.to_string_lossy() || !missing.is_empty() {
                failures.push(format!(
                    "Properties of the text folder: title {title:?}, missing {missing:?} in {body:?}"
                ));
            }
        }
    }
    support::dismiss_all(&mut app);
    app.shutdown()?;

    // Refresh (Ctrl+R) on a writable folder: an entry created behind the panel's back
    // appears, cursor and selection stay.
    let folder = support::scratch(context, "refresh")?;
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(folder.join(name), name).map_err(|error| io_error(&folder, error))?;
    }
    let mut app = App::start(Config::new(&folder, &context.work));
    panel::select_row(&mut app, LEFT, "a.txt")?;
    input::key(app.slint(), Key::Insert);
    panel::select_row(&mut app, LEFT, "c.txt")?;
    fs::write(folder.join("new.txt"), "new").map_err(|error| io_error(&folder, error))?;
    input::chord(app.slint(), &[Key::Control], "r");
    support::wait_row(&mut app, LEFT, "new.txt", true);
    if support::cursor_name(&app, LEFT).as_deref() != Some("c.txt")
        || support::selected_names(&app, LEFT) != ["a.txt"]
    {
        failures.push(format!(
            "after Ctrl+R the cursor is on {:?} and the selection is {:?}",
            support::cursor_name(&app, LEFT),
            support::selected_names(&app, LEFT)
        ));
    }
    app.shutdown()?;
    drop(app);
    command_line(context, &mut failures)?;
    support::report(&failures)
}

fn tabs_of(app: &App) -> Vec<(String, bool)> {
    app.window
        .get_left_tabs()
        .iter()
        .map(|tab| (tab.path.to_string(), tab.active))
        .collect()
}

/// The close button of the left panel's tab whose label starts with `label`.
fn close_button(app: &App, label: &str) -> Option<i_slint_core::item_tree::ItemRc> {
    let left_edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(text, _)| text.starts_with(&format!("Close tab: {label}\n")))
        .map(|(_, item)| item)
        .find(|item| inspect::center(item).x < left_edge)
}

/// The tab button itself ("label\npath") in the left panel.
fn tab_button(app: &App, label: &str) -> Option<i_slint_core::item_tree::ItemRc> {
    let left_edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(text, _)| text.starts_with(&format!("{label}\n")))
        .map(|(_, item)| item)
        .find(|item| inspect::center(item).x < left_edge)
}

pub fn tabs(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, &root));
    let mut failures = Vec::new();
    let root_text = root.to_string_lossy().into_owned();

    // Ctrl+T opens a tab at the current folder; navigate it to text/.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "t");
    app.until("a second tab", |app| {
        app.window.get_left_tabs().row_count() == 2
    });
    panel::enter(&mut app, LEFT, "text")?;
    app.until("the new tab to show text/", |app| {
        tabs_of(app)
            .iter()
            .any(|(path, active)| *active && Path::new(path).ends_with("text"))
    });
    // Ctrl+Shift+Tab: back to the first tab (the fixture root).
    input::chord(app.slint(), &[Key::Control, Key::Shift], Key::Tab);
    panel::wait_location(&mut app, LEFT, &root);
    // Clicking the text tab activates it.
    let text_tab = tab_button(&app, "text").ok_or("no tab labelled text")?;
    input::click_at(app.slint(), inspect::center(&text_tab));
    panel::wait_location(&mut app, LEFT, &root.join("text"));
    app.screenshot(context, "two-tabs", "Two tabs, text/ active");

    // The close button is hidden until the pointer is over its tab: move the pointer
    // into the file list first (the click above left it on the tab).
    input::move_to(app.slint(), panel::row_position(&app, LEFT, 0));
    app.idle(Duration::from_millis(150));
    let hidden = close_button(&app, "text")
        .map(|item| inspect::visible_rect(&item).is_some())
        .unwrap_or(false);
    if hidden {
        failures.push("the tab's close button is visible without hovering".into());
    }
    let text_tab = tab_button(&app, "text").ok_or("no tab labelled text")?;
    input::move_to(app.slint(), inspect::center(&text_tab));
    app.idle(Duration::from_millis(150));
    let close = close_button(&app, "text").ok_or("no close button for the text tab")?;
    let shown = inspect::visible_rect(&close);
    if shown.is_none() {
        failures.push("hovering the tab did not show its close button".into());
    }
    app.screenshot(
        context,
        "tab-hover-close",
        "Hovering a tab shows its close button",
    );
    // Move onto the button (keeps it shown) and click it.
    let position = inspect::center(&close);
    input::move_to(app.slint(), position);
    app.idle(Duration::from_millis(100));
    input::click_at(app.slint(), position);
    app.until("the text tab to close", |app| {
        app.window.get_left_tabs().row_count() == 1
    });
    panel::wait_location(&mut app, LEFT, &root);

    // The + button and Ctrl+W.
    let new_tab = inspect::labels(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|label| label.starts_with("Open a new folder tab"))
        .ok_or("no new-tab button")?;
    let left_edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    let plus = inspect::find_all(app.slint(), AccessibleRole::Button, &new_tab)
        .into_iter()
        .find(|item| inspect::center(item).x < left_edge)
        .ok_or("no new-tab button in the left panel")?;
    input::click_at(app.slint(), inspect::center(&plus));
    app.until("the + tab", |app| {
        app.window.get_left_tabs().row_count() == 2
    });
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "w");
    app.until("Ctrl+W to close the tab", |app| {
        app.window.get_left_tabs().row_count() == 1
    });
    input::chord(app.slint(), &[Key::Control], "w");
    app.idle(Duration::from_millis(200));
    if app.window.get_left_tabs().row_count() != 1 {
        failures.push("Ctrl+W closed the last tab".into());
    }
    if tabs_of(&app).first().map(|(path, _)| path.as_str()) != Some(root_text.as_str()) {
        failures.push(format!("the remaining tab is {:?}", tabs_of(&app)));
    }
    context.note("tabs: Ctrl+T, Ctrl+Shift+Tab, click to switch, hover-only close button, + button, Ctrl+W, last tab kept (Ctrl+Shift+digit: panels_tabs)");
    app.shutdown()?;
    support::report(&failures)
}

fn bookmarks(app: &App) -> Vec<(String, i32)> {
    app.window
        .get_bookmarks()
        .iter()
        .map(|row| (row.path.to_string(), row.slot))
        .collect()
}

/// Open the toolbar's Hot Paths menu and click the entry labelled `label`.
fn hot_path_menu(app: &mut App, label: &str) -> Result<(), String> {
    panel::wait_input(app);
    input::click(app.slint(), AccessibleRole::Combobox, "Hot Paths")?;
    app.settle();
    input::click_button(app.slint(), label)
}

pub fn hot_paths(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, &root));
    let mut failures = Vec::new();
    let images = root.join("images").to_string_lossy().into_owned();

    // Add: Ctrl+Shift+Plus saves the current folder in the first free slot.
    panel::enter(&mut app, LEFT, "images")?;
    input::chord(app.slint(), &[Key::Control, Key::Shift], "+");
    app.until("the Hot Path", |app| {
        app.window.get_bookmarks().row_count() > 0
    });
    let saved = bookmarks(&app);
    if !saved
        .iter()
        .any(|(path, slot)| *path == images && *slot == 1)
    {
        failures.push(format!("Ctrl+Shift+Plus saved {saved:?}"));
    }
    // Go to: from the root through the toolbar's Hot Paths menu.
    panel::focus_list(&mut app, LEFT)?;
    panel::leave(&mut app, LEFT)?;
    let choice = app
        .window
        .get_hot_path_choices()
        .iter()
        .find(|choice| choice.tooltip.contains("images") || choice.label.contains("images"))
        .map(|choice| choice.label.to_string())
        .ok_or("the Hot Paths menu lists no images entry")?;
    hot_path_menu(&mut app, &choice)?;
    panel::wait_location(&mut app, LEFT, &root.join("images"));
    panel::focus_list(&mut app, LEFT)?;
    panel::leave(&mut app, LEFT)?;

    // Manage: F10 → Manage Hot Paths…, change the number, remove, add again.
    panel::wait_input(&mut app);
    input::key(app.slint(), Key::F10);
    app.until("the main menu", |app| inspect::roots(app.slint()).len() > 1);
    support::click_scrolled(app.slint(), AccessibleRole::Button, "Manage Hot Paths…")?;
    app.until("the Hot Paths manager", |app| {
        !app.window
            .global::<HotPathsPresentation>()
            .get_token()
            .is_empty()
    });
    app.screenshot(context, "hot-paths-manager", "Manage Hot Paths");
    let row_label = inspect::labels(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .chain(inspect::labels(app.slint(), AccessibleRole::Button))
        .find(|label| label.starts_with(&format!("{images}\n")) || *label == images)
        .ok_or("the manager lists no images row")?;
    let item = inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .chain(inspect::controls(app.slint(), AccessibleRole::Button))
        .find(|(label, _)| *label == row_label)
        .map(|(_, item)| item)
        .ok_or("no images row item")?;
    input::click_at(app.slint(), inspect::center(&item));
    app.settle();
    let before =
        support::accessible_value(app.slint(), AccessibleRole::Combobox, "Hot Path shortcut");
    // One step down: the next number.
    input::click(app.slint(), AccessibleRole::Combobox, "Hot Path shortcut")?;
    app.settle();
    input::key(app.slint(), Key::DownArrow);
    input::key(app.slint(), Key::Return);
    app.settle();
    let chosen =
        support::accessible_value(app.slint(), AccessibleRole::Combobox, "Hot Path shortcut");
    input::click_button(app.slint(), "Apply shortcut")?;
    app.settle();
    let renumbered = bookmarks(&app);
    if renumbered == saved {
        failures.push(format!(
            "Apply shortcut ({before:?} → {chosen:?}) did not change the slot: {renumbered:?}"
        ));
    }
    let remove = inspect::labels(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|label| label.starts_with(&format!("Remove Hot Path: {images}")))
        .ok_or("no remove button for images")?;
    input::click_button(app.slint(), &remove)?;
    app.until("the Hot Path to be removed", |app| {
        app.window.get_bookmarks().row_count() == 0
    });
    input::click_button(app.slint(), "Add current")?;
    app.until("the current folder to be added", |app| {
        app.window.get_bookmarks().row_count() == 1
    });
    let added = bookmarks(&app);
    input::click_button(app.slint(), "Close")?;
    app.until("the manager to close", |app| {
        app.window
            .global::<HotPathsPresentation>()
            .get_token()
            .is_empty()
    });
    context.note(format!(
        "Hot Paths: added {saved:?}, renumbered {renumbered:?} (shortcut {before:?} → {chosen:?}), removed, Add current gave {added:?}; number keys: panels_hot_digits"
    ));
    app.shutdown()?;
    support::report(&failures)
}

pub fn file_list(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "file-list")?;
    fs::write(folder.join("a.txt"), "abc").map_err(|error| io_error(&folder, error))?;
    fs::write(folder.join("bb.log"), "0123456789").map_err(|error| io_error(&folder, error))?;
    fs::create_dir(folder.join("sub")).map_err(|error| io_error(&folder, error))?;
    let output = context.work.join("list.txt");
    let mut app = App::start(Config::new(&folder, &context.work));
    let mut failures = Vec::new();
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "a");
    app.until("Ctrl+A", |app| {
        support::selected_names(app, LEFT).len() == 2
    });
    input::chord(app.slint(), &[Key::Control], "m");
    app.until("Make File List", |app| {
        app.window.global::<FileListPresentation>().get_active()
    });
    input::fill(
        app.slint(),
        "File list line template",
        "$(FileName)$(TAB)$(FileSize)$(CRLF)",
    )?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let preview = loop {
        app.idle(Duration::from_millis(50));
        let preview = app
            .window
            .global::<FileListPresentation>()
            .get_preview_text()
            .to_string();
        if preview.contains("bb.log") || Instant::now() > deadline {
            break preview;
        }
    };
    // Tabs are expanded to eight-column stops in the preview.
    if preview.contains('\t') || !preview.contains("a.txt   3") || !preview.contains("bb.log  10") {
        failures.push(format!("the preview is {preview:?}"));
    }
    support::choose(&mut app, "File list output destination", "File", 3)?;
    input::fill(
        app.slint(),
        "File list destination path",
        &output.to_string_lossy(),
    )?;
    app.screenshot(context, "file-list", "Make File List with a tab template");
    input::click_button(app.slint(), "Create file list")?;
    app.until("the list file", |_| output.is_file());
    app.until("Make File List to close", |app| {
        !app.window.global::<FileListPresentation>().get_active()
    });
    let written = fs::read(&output).map_err(|error| io_error(&output, error))?;
    if written != b"a.txt\t3\r\nbb.log\t10\r\n" {
        failures.push(format!(
            "the list file holds {:?}",
            String::from_utf8_lossy(&written)
        ));
    }
    if !support::selected_names(&app, LEFT).is_empty() {
        failures.push("the selection stayed after a successful list".into());
    }
    context.note(format!(
        "Make File List preview {preview:?}; file verified byte for byte"
    ));
    app.shutdown()?;
    support::report(&failures)
}

/// The command line's shell controls as this harness can show them: without a desktop
/// service (it would launch terminals) the application knows no shell, so the "Command
/// shell" chooser and "Open Terminal" are disabled and Run says the terminal is
/// unavailable; the history picker lists the saved commands newest first, and choosing
/// one runs nothing and leaves the command line as it was; Enter in the command line
/// reports "Terminal unavailable". The platform's shells, "Run with <shell>", the
/// PowerShell/cmd switch with a draft and recall into the draft need the desktop
/// context (SKIP, noted).
fn command_line(context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    use midnight_core::{CommandHistoryEntry, ShellFlavor};
    let folder = support::scratch(context, "command-line")?;
    let shell = if cfg!(windows) {
        ShellFlavor::PowerShell
    } else {
        ShellFlavor::Posix
    };
    let store = crate::options_support::SettingsStore::seeded(move |preferences| {
        preferences.remember_command_history = true;
        preferences.command_history = ["echo newest", "echo older"]
            .iter()
            .map(|command| CommandHistoryEntry {
                command: (*command).to_owned(),
                shell,
            })
            .collect();
    });
    let mut config = Config::new(&folder, &context.work);
    config.settings = Some(store);
    let mut app = App::start(config);
    let enabled = |app: &App, role: AccessibleRole, label: &str| {
        inspect::find(app.slint(), role, label).map(|item| crate::tools_support::enabled(&item))
    };
    let chooser = enabled(&app, AccessibleRole::Combobox, "Command shell");
    // The terminal button reads "Open PowerShell in this folder. Ctrl+click: …" (Windows)
    // or "Open a terminal in this folder. …" (since b2184fb2).
    let open = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, _)| label.starts_with("Open ") && label.contains(" in this folder."))
        .map(|(_, item)| crate::tools_support::enabled(&item));
    if chooser != Some(false) || open != Some(false) || app.window.get_terminal_available() {
        failures.push(format!(
            "without a desktop service: Command shell enabled {chooser:?}, Open Terminal \
             enabled {open:?}, terminal available {}",
            app.window.get_terminal_available()
        ));
    }
    context.note(
        "SKIP: the Command shell chooser's platform shells, \"Run with <shell>\", the \
         PowerShell/cmd switch refused with a draft and a recalled command filling the \
         draft: the shell comes from the desktop service's DesktopContext, which this \
         harness does not install (the same service launches terminals)",
    );
    // The history picker: the saved commands, newest first; choosing one runs nothing.
    match inspect::find(
        app.slint(),
        AccessibleRole::Combobox,
        "Executed command history",
    ) {
        None => failures.push("no Executed command history picker".into()),
        Some(item) => {
            if !crate::tools_support::enabled(&item) {
                failures.push("the history picker is disabled with two saved commands".into());
            } else {
                input::click_at(app.slint(), inspect::center(&item));
                app.settle();
                // Each row names the shell the command was run with.
                let prefix = if cfg!(windows) {
                    "PowerShell"
                } else {
                    "Terminal"
                };
                let wanted = [
                    format!("{prefix} · echo newest"),
                    format!("{prefix} · echo older"),
                ];
                let rows: Vec<String> = inspect::controls(app.slint(), AccessibleRole::Button)
                    .into_iter()
                    .map(|(label, _)| label)
                    .filter(|label| label.ends_with("echo newest") || label.ends_with("echo older"))
                    .collect();
                if rows != wanted {
                    failures.push(format!(
                        "the history picker lists {rows:?}, expected the saved commands newest first"
                    ));
                }
                let before = app.window.get_command_text().to_string();
                match inspect::find(app.slint(), AccessibleRole::Button, &wanted[1]) {
                    None => failures.push("the history picker shows no echo older".into()),
                    Some(row) => {
                        input::click_at(app.slint(), inspect::center(&row));
                        app.settle();
                        let notice = support::shown_notice(&app);
                        if app.window.get_command_text() != before.as_str()
                            || notice
                                .as_ref()
                                .is_none_or(|(title, _)| title != "Command shell unavailable")
                        {
                            failures.push(format!(
                                "choosing echo older without a known shell: the command line \
                                 holds {:?} and the notice is {notice:?}",
                                app.window.get_command_text()
                            ));
                        }
                    }
                }
                support::dismiss_all(&mut app);
            }
        }
    }
    // Enter in the command line: the terminal is unavailable, and says so.
    input::click(app.slint(), AccessibleRole::TextInput, "Command line")?;
    input::type_text(app.slint(), "echo typed");
    input::key(app.slint(), Key::Return);
    if !ps_wait_notice(&mut app, "Terminal unavailable") {
        failures.push(format!(
            "Enter in the command line without a terminal: notice {:?}",
            support::shown_notice(&app)
        ));
    }
    support::dismiss_all(&mut app);
    app.shutdown()
}

/// Wait up to 5 s for a notice titled `title`.
fn ps_wait_notice(app: &mut App, title: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.settle();
        if support::shown_notice(app).is_some_and(|(shown, _)| shown == title) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        app.idle(Duration::from_millis(20));
    }
}
