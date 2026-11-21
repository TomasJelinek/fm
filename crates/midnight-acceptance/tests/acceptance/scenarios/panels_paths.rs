//! Navigation by pointer and by the keys the other scenarios leave out: the toolbar's
//! Back, Forward, Parent and Refresh buttons (disabled without history) and the Back
//! and Forward history menus; Ctrl+PageUp, Ctrl+PageDown (into a folder, never opening
//! a file), Shift+Backspace and Ctrl+Backslash; double clicks on a folder and on `..`;
//! Tab and Shift+Tab between the panels; the path bar: a hovered segment highlights its
//! prefix, a right click copies it (pasted back into the path editor with a separator),
//! a long path collapses its middle into one marker that opens the whole path for
//! editing, the blank space after the path opens the editor and Escape cancels it; the
//! drive menu (Alt+F1, Alt+F2, `.` As Other Panel, the filesystem root by pointer); an
//! empty folder and a search without matches say so.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect, panel,
    scenarios::panels_support::{self as ps, LEFT, RIGHT},
};
use i_slint_core::{item_tree::ItemRc, items::AccessibleRole};
use slint::{LogicalPosition, platform::Key};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn at(app: &App, pane: midnight_core::PaneId) -> Option<PathBuf> {
    app.location(pane)
}

/// Wait until the left panel shows `path` (listing complete); whether it did.
fn reached(app: &mut App, path: &Path) -> bool {
    let wanted = path.to_path_buf();
    ps::wait(app, Duration::from_secs(10), |app| {
        app.location(LEFT).as_ref() == Some(&wanted) && !app.loading(LEFT)
    })
}

fn go(app: &mut App, path: &Path) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &path.to_string_lossy());
    input::key(app.slint(), Key::Return);
    if !reached(app, path) {
        return Err(format!("Ctrl+G did not open {}", path.display()));
    }
    Ok(())
}

fn enabled(item: &ItemRc) -> bool {
    crate::tools_support::enabled(item)
}

/// The left half's toolbar button whose label starts with `prefix`.
fn tool(app: &App, prefix: &str) -> Option<ItemRc> {
    ps::button_starting(app, prefix).map(|(_, item)| item)
}

fn click(app: &mut App, item: &ItemRc) {
    let (x, y, w, h) = inspect::visible_rect(item).unwrap_or_else(|| inspect::rect(item));
    input::click_at(app.slint(), LogicalPosition::new(x + w / 2.0, y + h / 2.0));
    app.settle();
}

pub fn paths(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut config = Config::new(&root, root.join("text"));
    config.clipboard = true;
    let mut app = App::start(config);
    let mut failures = Vec::new();
    toolbar_history(&mut app, context, &root, &mut failures)?;
    keys(&mut app, &root, &mut failures)?;
    path_bar(&mut app, context, &root, &mut failures)?;
    drives(&mut app, context, &root, &mut failures)?;
    details_copy(&mut app, &root, &mut failures)?;
    empty_states(&mut app, context, &mut failures)?;
    app.shutdown()?;
    drop(app);
    quick_filter_empty(context, &mut failures)?;
    support::report(&failures)
}

fn toolbar_history(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    for prefix in ["Back (", "Forward ("] {
        match tool(app, prefix) {
            None => failures.push(format!("no toolbar button {prefix:?}")),
            Some(item) if enabled(&item) => failures.push(format!(
                "the {prefix:?} button is enabled without any history"
            )),
            Some(_) => {}
        }
    }
    // History: root → text → images → docs → docs/page_files.
    let trail = [
        root.join("text"),
        root.join("images"),
        root.join("docs"),
        root.join("docs").join("page_files"),
    ];
    for path in &trail {
        go(app, path)?;
    }
    // The Back history menu lists the earlier folders, newest first; choosing the third
    // goes back three steps at once.
    match pick_history(app, "Back", 2) {
        Err(error) => failures.push(error),
        Ok((labels, chosen)) => {
            let wanted: Vec<String> = [&trail[2], &trail[1], &trail[0]]
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .chain([root.to_string_lossy().into_owned()])
                .collect();
            if labels != wanted {
                failures.push(format!(
                    "the Back history menu lists {labels:?}, expected {wanted:?}"
                ));
            }
            if !reached(app, &trail[0]) {
                failures.push(format!(
                    "choosing {chosen:?} in the Back history went to {:?}",
                    at(app, LEFT)
                ));
            }
        }
    }
    app.screenshot(
        context,
        "after-back-history",
        "Back history: three steps back at once",
    );
    // The Forward history now lists the later folders; choosing the last one goes there.
    match pick_history(app, "Forward", 2) {
        Err(error) => failures.push(error),
        Ok((labels, _)) => {
            let wanted: Vec<String> = trail[1..]
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            if labels != wanted {
                failures.push(format!(
                    "the Forward history menu lists {labels:?}, expected {wanted:?}"
                ));
            }
            if !reached(app, &trail[3]) {
                failures.push(format!(
                    "choosing the last Forward entry went to {:?}",
                    at(app, LEFT)
                ));
            }
        }
    }
    // The buttons: Back, Forward, Parent (cursor on the folder left), Refresh.
    for (prefix, wanted) in [
        ("Back (", trail[2].clone()),
        ("Forward (", trail[3].clone()),
        ("Parent folder", trail[2].clone()),
    ] {
        match tool(app, prefix) {
            None => failures.push(format!("no toolbar button {prefix:?}")),
            Some(item) => {
                click(app, &item);
                if !reached(app, &wanted) {
                    failures.push(format!(
                        "the {prefix:?} button went to {:?}, expected {}",
                        at(app, LEFT),
                        wanted.display()
                    ));
                }
            }
        }
    }
    if ps::cursor(app, LEFT).as_deref() != Some("page_files") {
        failures.push(format!(
            "after the Parent button the cursor is on {:?}, not the folder left (page_files)",
            ps::cursor(app, LEFT)
        ));
    }
    if app.window.get_focused_pane() != 0 {
        failures.push("after the toolbar buttons the file list has no keyboard focus".into());
    }
    // The toolbar's Folder sizes measures the folders shown (sizes in bytes).
    match tool(app, "Folder sizes") {
        None => failures.push("no toolbar Folder sizes button".into()),
        Some(item) => {
            click(app, &item);
            let expected = ps::disk_size(&root.join("docs").join("page_files"));
            let measured = |app: &App| {
                app.rows(LEFT)
                    .iter()
                    .find(|row| row.name == "page_files")
                    .filter(|row| !row.size_running && row.size_state == 0)
                    .map(|row| row.size.to_string())
            };
            if !ps::wait(app, Duration::from_secs(20), |app| {
                measured(app).is_some_and(|size| size.chars().any(|c| c.is_ascii_digit()))
            }) {
                failures.push(format!(
                    "the toolbar's Folder sizes did not measure page_files/: {:?}",
                    measured(app)
                ));
            } else {
                context.note(format!(
                    "toolbar Folder sizes: page_files/ shows {:?}, the disk holds {expected} bytes",
                    measured(app)
                ));
            }
        }
    }
    let listings = app.listings[0];
    match tool(app, "Refresh") {
        None => failures.push("no Refresh button".into()),
        Some(item) => {
            click(app, &item);
            if !ps::wait(app, Duration::from_secs(10), |app| {
                app.listings[0] > listings
            }) {
                failures.push("the Refresh button listed nothing again".into());
            }
            if ps::cursor(app, LEFT).as_deref() != Some("page_files") {
                failures.push(format!(
                    "Refresh moved the cursor to {:?}",
                    ps::cursor(app, LEFT)
                ));
            }
        }
    }
    Ok(())
}

/// Open the `which` history menu (Back or Forward) of the toolbar and choose its entry
/// `index`: the listed labels and the chosen one.
fn pick_history(app: &mut App, which: &str, index: usize) -> Result<(Vec<String>, String), String> {
    panel::wait_input(app);
    let combo = inspect::labels(app.slint(), AccessibleRole::Combobox)
        .into_iter()
        .find(|label| label.starts_with(&format!("{which} (")) && label.ends_with(" history"))
        .ok_or_else(|| format!("no {which} history menu"))?;
    input::click(app.slint(), AccessibleRole::Combobox, &combo)?;
    app.settle();
    let labels: Vec<String> = ps::popup_buttons(app)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    let chosen = labels
        .get(index)
        .cloned()
        .ok_or_else(|| format!("the {which} history menu lists only {labels:?}"))?;
    ps::click_popup_button(app, &chosen)?;
    Ok((labels, chosen))
}

fn keys(app: &mut App, root: &Path, failures: &mut Vec<String>) -> Result<(), String> {
    go(app, &root.join("docs"))?;
    // Ctrl+PageDown enters a folder.
    panel::select_row(app, LEFT, "page_files")?;
    input::chord(app.slint(), &[Key::Control], Key::PageDown);
    if !reached(app, &root.join("docs").join("page_files")) {
        failures.push(format!(
            "Ctrl+PageDown on a folder went to {:?}",
            at(app, LEFT)
        ));
    }
    // Ctrl+PageUp goes to the parent, cursor on the folder left.
    input::chord(app.slint(), &[Key::Control], Key::PageUp);
    if !reached(app, &root.join("docs")) {
        failures.push(format!("Ctrl+PageUp went to {:?}", at(app, LEFT)));
    } else if ps::cursor(app, LEFT).as_deref() != Some("page_files") {
        failures.push(format!(
            "after Ctrl+PageUp the cursor is on {:?}",
            ps::cursor(app, LEFT)
        ));
    }
    // Ctrl+PageDown on a file opens nothing and stays.
    let file = ps::shown(app, LEFT)
        .into_iter()
        .find(|name| root.join("docs").join(name).is_file())
        .ok_or("docs/ has no file")?;
    panel::select_row(app, LEFT, &file)?;
    input::chord(app.slint(), &[Key::Control], Key::PageDown);
    if let Err(error) = ps::stays_idle(app, Duration::from_millis(600)) {
        failures.push(format!("Ctrl+PageDown on the file {file}: {error}"));
        support::dismiss_all(app);
    }
    if at(app, LEFT).as_deref() != Some(root.join("docs").as_path()) {
        failures.push(format!(
            "Ctrl+PageDown on the file {file} left the folder: {:?}",
            at(app, LEFT)
        ));
    }
    // Shift+Backspace and Ctrl+Backslash: the filesystem root.
    for (modifiers, key, name) in [
        (
            &[Key::Shift][..],
            slint::SharedString::from(Key::Backspace),
            "Shift+Backspace",
        ),
        (
            &[Key::Control][..],
            slint::SharedString::from("\\"),
            "Ctrl+Backslash",
        ),
    ] {
        go(app, &root.join("docs"))?;
        input::chord(app.slint(), modifiers, key);
        let filesystem_root = crate::files_support::filesystem_root(root);
        if !reached(app, &filesystem_root) {
            failures.push(format!(
                "{name} went to {:?}, expected {}",
                at(app, LEFT),
                filesystem_root.display()
            ));
        }
    }
    // Double clicks: a folder opens; `..` goes back with the cursor on the folder left.
    go(app, root)?;
    match panel::row_center(app, LEFT, "docs") {
        None => failures.push("no docs row to double-click".into()),
        Some(position) => {
            ps::double_click(app, position);
            if !reached(app, &root.join("docs")) {
                failures.push(format!("double-clicking docs went to {:?}", at(app, LEFT)));
            } else {
                match panel::row_center(app, LEFT, "..") {
                    None => failures.push("docs/ shows no .. row".into()),
                    Some(position) => {
                        ps::double_click(app, position);
                        if !reached(app, root) {
                            failures
                                .push(format!("double-clicking .. went to {:?}", at(app, LEFT)));
                        } else if ps::cursor(app, LEFT).as_deref() != Some("docs") {
                            failures.push(format!(
                                "after double-clicking .. the cursor is on {:?}",
                                ps::cursor(app, LEFT)
                            ));
                        }
                    }
                }
            }
        }
    }
    // Tab and Shift+Tab switch the panels.
    panel::focus_list(app, LEFT)?;
    for (shift, pane) in [(false, 1), (false, 0), (true, 1), (true, 0)] {
        if shift {
            input::chord(app.slint(), &[Key::Shift], Key::Tab);
        } else {
            input::key(app.slint(), Key::Tab);
        }
        app.settle();
        if app.window.get_active_pane() != pane || app.window.get_focused_pane() != pane {
            failures.push(format!(
                "{}Tab: active pane {}, focused {}, expected {pane}",
                if shift { "Shift+" } else { "" },
                app.window.get_active_pane(),
                app.window.get_focused_pane()
            ));
        }
    }
    Ok(())
}

/// The left path bar's segment buttons ("Open <prefix>"), in order.
fn segments(app: &App) -> Vec<(String, ItemRc)> {
    let edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(label, item)| {
            label
                .strip_prefix("Open ")
                .is_some_and(|prefix| Path::new(prefix).is_absolute())
                && inspect::center(item).x < edge
                && inspect::visible_rect(item).is_some()
                && inspect::rect(item).1 < app.window.get_left_list_top()
        })
        .collect()
}

fn path_bar(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    go(app, &root.join("docs").join("page_files"))?;
    // Hovering a segment highlights its prefix.
    let docs = root.join("docs").to_string_lossy().into_owned();
    match segments(app)
        .into_iter()
        .find(|(label, _)| label.trim_end_matches('/') == format!("Open {docs}"))
    {
        None => failures.push(format!(
            "no path segment for {docs}; segments {:?}",
            segments(app)
                .iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>()
        )),
        Some((_, item)) => {
            input::move_to(app.slint(), inspect::center(&item));
            app.settle();
            let highlighted = app.window.get_left_highlighted_path().to_string();
            if highlighted.trim_end_matches('/') != docs {
                failures.push(format!(
                    "hovering the docs segment highlights {highlighted:?}, expected {docs:?}"
                ));
            }
            // A right click copies the prefix, raw, without navigating.
            let copied = |app: &App| {
                app.clipboard
                    .as_ref()
                    .and_then(|clipboard| clipboard.text())
            };
            ps::right_click(app.slint(), inspect::center(&item));
            app.settle();
            if !ps::wait(app, Duration::from_secs(3), |app| {
                copied(app).is_some_and(|text| text.trim_end_matches('/') == docs)
            }) {
                failures.push(format!(
                    "right-clicking the docs path segment copied {:?}, expected {docs:?}",
                    copied(app)
                ));
            }
            if at(app, LEFT).as_deref() != Some(root.join("docs").join("page_files").as_path()) {
                failures.push(format!(
                    "right-clicking a path segment navigated to {:?}",
                    at(app, LEFT)
                ));
            }
            // Pasted into the path editor it ends with the platform's separator.
            panel::focus_list(app, LEFT)?;
            input::chord(app.slint(), &[Key::Control], "g");
            app.settle();
            input::chord(app.slint(), &[Key::Control], "a");
            input::chord(app.slint(), &[Key::Control], "v");
            let wanted = format!("{docs}{}", std::path::MAIN_SEPARATOR);
            if !ps::wait(app, Duration::from_secs(3), |app| {
                app.window.get_left_path_draft() == wanted.as_str()
            }) {
                failures.push(format!(
                    "pasting the copied segment into the path editor shows {:?}, expected {wanted:?}",
                    app.window.get_left_path_draft()
                ));
            }
            input::key(app.slint(), Key::Escape);
            app.settle();
            if at(app, LEFT).as_deref() != Some(root.join("docs").join("page_files").as_path())
                || app.window.get_focused_pane() != 0
            {
                failures.push(format!(
                    "Escape in the path editor: panel at {:?}, focused pane {}",
                    at(app, LEFT),
                    app.window.get_focused_pane()
                ));
            }
        }
    }
    // A long path: whole segments only, one marker for the middle, the last folder
    // shown; the marker opens the whole path for editing.
    let mut deep = root.join("deep");
    for level in 1..=12 {
        deep = deep.join(format!("level-{level:02}"));
    }
    go(app, &deep)?;
    let shown = segments(app);
    let parts = deep.components().count();
    let marker = inspect::find_all(app.slint(), AccessibleRole::Button, "Edit full folder path")
        .into_iter()
        .find(|item| {
            inspect::center(item).x
                < app.window.get_left_pane_left() + app.window.get_left_pane_width()
        });
    app.screenshot(context, "long-path", "A long path collapsed in the middle");
    if marker.is_none() || shown.len() >= parts {
        failures.push(format!(
            "the {}-component path shows {} segments and {} middle marker",
            parts,
            shown.len(),
            if marker.is_some() { "a" } else { "no" }
        ));
    }
    let last = format!("Open {}", deep.to_string_lossy());
    if !shown
        .iter()
        .any(|(label, _)| label.trim_end_matches('/') == last)
    {
        failures.push(format!(
            "the collapsed path does not show its last folder; segments {:?}",
            shown.iter().map(|(label, _)| label).collect::<Vec<_>>()
        ));
    }
    let mut previous_right = f32::MIN;
    for (label, item) in &shown {
        let (x, _, width, _) = inspect::rect(item);
        if x + 0.5 < previous_right {
            failures.push(format!("path segment {label:?} overlaps the one before it"));
        }
        previous_right = x + width;
    }
    if let Some(marker) = marker {
        click(app, &marker);
        let full = deep.to_string_lossy().into_owned();
        if app.window.get_left_path_draft() != full.as_str() {
            failures.push(format!(
                "the middle marker opened the editor with {:?}, expected the whole path",
                app.window.get_left_path_draft()
            ));
        }
        input::key(app.slint(), Key::Escape);
        app.settle();
    }
    // The blank space after the path opens the editor; Enter opens the typed folder.
    go(app, &root.join("docs"))?;
    let blank = inspect::find_all(app.slint(), AccessibleRole::Button, "Edit folder path")
        .into_iter()
        .find(|item| {
            inspect::center(item).x
                < app.window.get_left_pane_left() + app.window.get_left_pane_width()
        });
    match blank {
        None => failures.push("no blank space after the path to click".into()),
        Some(item) => {
            let (_, _, width, _) = inspect::rect(&item);
            if width < 31.5 {
                failures.push(format!(
                    "the blank space after the path is only {width} px wide"
                ));
            }
            click(app, &item);
            let current = root.join("docs").to_string_lossy().into_owned();
            if app.window.get_left_path_draft().as_str() != current {
                failures.push(format!(
                    "clicking after the path opened the editor with {:?}, expected {current:?}",
                    app.window.get_left_path_draft()
                ));
            }
            input::replace_text(app.slint(), &root.join("images").to_string_lossy());
            input::key(app.slint(), Key::Return);
            if !reached(app, &root.join("images")) {
                failures.push(format!(
                    "Enter in the clicked path editor went to {:?}",
                    at(app, LEFT)
                ));
            }
        }
    }
    Ok(())
}

fn drives(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    go(app, &root.join("images"))?;
    let right = at(app, RIGHT).ok_or("the right panel shows no folder")?;
    // Alt+F1: the left drive menu; `.` opens the other panel's folder.
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Alt], Key::F1);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.window.get_volume_popup_pane() == 0
    }) {
        failures.push("Alt+F1 opened no left drive menu".into());
    } else {
        app.screenshot(context, "drive-menu", "Alt+F1: the left drive menu");
        input::key(app.slint(), ".");
        if !reached(app, &right) {
            failures.push(format!(
                "`.` in the drive menu went to {:?}, expected the other panel's {}",
                at(app, LEFT),
                right.display()
            ));
        }
        if app.window.get_volume_popup_pane() >= 0 {
            failures.push("the drive menu stayed open after `.`".into());
        }
    }
    // Alt+F2 opens the right one; Escape closes it without navigating.
    input::chord(app.slint(), &[Key::Alt], Key::F2);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.window.get_volume_popup_pane() == 1
    }) {
        failures.push("Alt+F2 opened no right drive menu".into());
    } else {
        input::key(app.slint(), Key::Escape);
        if !ps::wait(app, Duration::from_secs(3), |app| {
            app.window.get_volume_popup_pane() < 0
        }) {
            failures.push("Escape did not close the right drive menu".into());
        }
        if at(app, RIGHT).as_deref() != Some(right.as_path()) {
            failures.push("Escape in the right drive menu navigated".into());
        }
    }
    // By pointer: the drive menu's entries. Its volume entry keeps the folder this
    // panel last showed on that volume, so the current one; As Other Panel by click
    // opens the other panel's folder.
    go(app, &root.join("images"))?;
    let edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    let picker = inspect::find_all(app.slint(), AccessibleRole::Combobox, "Drive or volume")
        .into_iter()
        .find(|item| inspect::center(item).x < edge);
    match picker {
        None => failures.push("no left Drive or volume menu".into()),
        Some(item) => {
            click(app, &item);
            let labels: Vec<String> = ps::popup_buttons(app)
                .into_iter()
                .map(|(label, _)| label)
                .collect();
            context.note(format!("left drive menu entries: {labels:?}"));
            // `/` on Unix; on Windows the fixture's drive (`D:`), however it is labelled.
            let drive = crate::files_support::filesystem_root(root)
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_owned();
            if !labels.iter().any(|label| {
                if cfg!(windows) {
                    label.contains(&drive)
                } else {
                    label.as_str() == "/" || label.starts_with("/ — ")
                }
            }) {
                failures.push(format!(
                    "the drive menu lists no filesystem root: {labels:?}"
                ));
            }
            match labels
                .iter()
                .find(|label| label.ends_with("As Other Panel"))
                .cloned()
            {
                None => {
                    failures.push(format!("the drive menu has no As Other Panel: {labels:?}"));
                    support::dismiss_all(app);
                }
                Some(label) => {
                    ps::click_popup_button(app, &label)?;
                    if !reached(app, &right) {
                        failures.push(format!(
                            "clicking {label:?} went to {:?}, expected the other panel's {}",
                            at(app, LEFT),
                            right.display()
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Texts shown inside the left list (not its rows' names), with their rectangles.
fn list_messages(app: &App) -> Vec<(String, inspect::Rect)> {
    let (left, top) = (
        app.window.get_left_list_left(),
        app.window.get_left_list_top(),
    );
    let (width, height) = (
        app.window.get_left_list_width(),
        app.window.get_left_list_height(),
    );
    let names = app.names(LEFT);
    let mut found = Vec::new();
    for root in inspect::roots(app.slint()).iter().take(1) {
        inspect::walk(root, &mut |item| {
            let text = item
                .downcast::<i_slint_core::items::SimpleText>()
                .map(|text| text.as_pin_ref().text().to_string())
                .or_else(|| {
                    item.downcast::<i_slint_core::items::ComplexText>()
                        .map(|text| text.as_pin_ref().text().to_string())
                });
            if let Some(text) = text
                && !text.trim().is_empty()
                && item.is_visible()
                && !names.contains(&text)
                && inspect::label(item).is_none_or(|label| !names.contains(&label))
                && let Some(rect) = inspect::visible_rect(item)
                && rect.0 >= left
                && rect.0 < left + width
                && rect.1 >= top
                && rect.1 < top + height
            {
                found.push((text, rect));
            }
        });
    }
    found
}

/// A message below the `..` row of the left list that `wanted` accepts.
fn message_below_parent(app: &App, wanted: &dyn Fn(&str) -> bool) -> Result<String, String> {
    let parent = panel::row_center(app, LEFT, "..")
        .ok_or_else(|| format!("no `..` row; rows {:?}", app.names(LEFT)))?;
    let messages = list_messages(app);
    messages
        .iter()
        .find(|(text, rect)| wanted(text) && rect.1 > parent.y)
        .map(|(text, _)| text.clone())
        .ok_or_else(|| format!("no such message below `..`; the list shows {messages:?}"))
}

/// Empty folders and listings whose entries are all hidden say so below `..`, which
/// still goes to the parent.
fn empty_states(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    const EMPTY: &str = "This folder is empty";
    let empty = support::scratch(context, "empty")?;
    go(app, &empty)?;
    if let Err(error) = message_below_parent(app, &|text| text == EMPTY) {
        failures.push(format!(
            "an empty folder does not say {EMPTY:?} below `..`: {error}"
        ));
    }
    app.screenshot(context, "empty-folder", "An empty folder");
    // `..` still works: Enter on it goes up with the cursor on the folder left.
    panel::select_row(app, LEFT, "..")?;
    input::key(app.slint(), Key::Return);
    if !reached(app, &context.work) {
        failures.push(format!(
            "Enter on `..` of the empty folder went to {:?}",
            at(app, LEFT)
        ));
    } else if ps::cursor(app, LEFT).as_deref() != Some("empty") {
        failures.push(format!(
            "after leaving the empty folder the cursor is on {:?}",
            ps::cursor(app, LEFT)
        ));
    }
    // Everything hidden by the hidden-files setting: a message of its own.
    let hidden = support::scratch(context, "only-hidden")?;
    ps::write(&hidden.join(".dot"), "dot")?;
    // Windows hides by the hidden attribute, not by name.
    crate::fixtures::hide(&hidden.join(".dot"))?;
    go(app, &hidden)?;
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "h");
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::shown(app, LEFT).is_empty()
    }) {
        failures.push(format!("Ctrl+H left {:?} listed", ps::shown(app, LEFT)));
    } else if let Err(error) = message_below_parent(app, &|text| {
        text != EMPTY && text.to_lowercase().contains("hidden")
    }) {
        failures.push(format!(
            "a folder whose entries are all hidden does not say so (a message about hidden entries, not {EMPTY:?}): {error}"
        ));
    }
    app.screenshot(
        context,
        "all-hidden",
        "Every entry hidden by Show hidden files",
    );
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "h");
    ps::wait(app, Duration::from_secs(5), |app| {
        !ps::shown(app, LEFT).is_empty()
    });
    // Everything filtered out by the panel filter: a message of its own.
    let filtered = support::scratch(context, "filtered")?;
    ps::write(&filtered.join("alpha.txt"), "a")?;
    go(app, &filtered)?;
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    crate::options_support::choose(app, "Filter mode", "Masks")?;
    input::fill(app.slint(), "File filter expression", "*.log")?;
    input::click_button(app.slint(), "OK")?;
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::shown(app, LEFT).is_empty()
    }) {
        failures.push(format!(
            "the *.log filter left {:?} listed",
            ps::shown(app, LEFT)
        ));
    } else if let Err(error) = message_below_parent(app, &|text| {
        text != EMPTY && text.to_lowercase().contains("filter")
    }) {
        failures.push(format!(
            "a folder whose entries are all filtered out does not say so (a message about the filter, not {EMPTY:?}): {error}"
        ));
    }
    app.screenshot(context, "all-filtered", "Every entry filtered out");
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    input::click_button(app.slint(), "Show all")?;
    app.until("the filter to close", |app| {
        !app.window.get_panel_filter_view().open
    });
    Ok(())
}

/// With "Show only filename search matches", a search without matches says so.
fn quick_filter_empty(context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let folder = support::scratch(context, "no-match")?;
    ps::write(&folder.join("alpha.txt"), "a")?;
    let store = crate::options_support::SettingsStore::seeded(|preferences| {
        preferences.quick_search_filter = true;
    });
    let mut config = Config::new(&folder, &context.work);
    config.settings = Some(store);
    let mut app = App::start(config);
    if !app.window.get_quick_search_filter() {
        failures.push("the seeded \"Show only filename search matches\" is not on".into());
    }
    panel::focus_list(&mut app, LEFT)?;
    input::type_text(app.slint(), "zq");
    app.settle();
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        ps::shown(app, LEFT).is_empty()
    }) {
        failures.push(format!(
            "the quick filter \"zq\" still lists {:?}",
            ps::shown(&app, LEFT)
        ));
    }
    let messages = list_messages(&app);
    if !messages.iter().any(|(text, _)| text == "No matching names") {
        failures.push(format!(
            "a quick-filtered search without matches does not say \"No matching names\": {messages:?}"
        ));
    }
    app.screenshot(context, "no-matching-names", "Quick filter without matches");
    input::key(app.slint(), Key::Escape);
    app.settle();
    if ps::shown(&app, LEFT) != ["alpha.txt"] {
        failures.push(format!(
            "Escape did not restore the listing: {:?}",
            ps::shown(&app, LEFT)
        ));
    }
    app.shutdown()
}

/// The details strip under the list copies the field that is clicked: the focused
/// file's name (Ctrl+click: its whole path) and its size as shown.
fn details_copy(app: &mut App, root: &Path, failures: &mut Vec<String>) -> Result<(), String> {
    let text = root.join("text");
    go(app, &text)?;
    panel::select_row(app, LEFT, "plain.txt")?;
    let copied = |app: &App| {
        app.clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.text())
    };
    let field = |app: &App, label: &str| {
        let top = app.window.get_left_details_top();
        inspect::controls(app.slint(), AccessibleRole::Button)
            .into_iter()
            .find(|(text, item)| {
                text.starts_with(&format!("{label}: "))
                    && text.ends_with(". Copy to clipboard")
                    && inspect::rect(item).1 >= top - 1.0
                    && inspect::center(item).x
                        < app.window.get_left_pane_left() + app.window.get_left_pane_width()
            })
    };
    for (label, control, wanted) in [
        ("Name", false, Some("plain.txt".to_owned())),
        (
            "Name",
            true,
            Some(text.join("plain.txt").to_string_lossy().into_owned()),
        ),
        ("Bytes", false, None),
        ("Date", false, None),
        ("Time", false, None),
        // Windows shows the attribute letters where Unix shows the mode.
        (
            if cfg!(windows) {
                "Attributes"
            } else {
                "Permissions"
            },
            false,
            None,
        ),
    ] {
        let Some((caption, item)) = field(app, label) else {
            failures.push(format!(
                "the details strip shows no {label} field; buttons {:?}",
                inspect::labels(app.slint(), AccessibleRole::Button)
                    .into_iter()
                    .filter(|text| text.ends_with("Copy to clipboard"))
                    .collect::<Vec<_>>()
            ));
            continue;
        };
        // The shown value: "Label: value. Copy to clipboard".
        let shown = caption
            .strip_prefix(&format!("{label}: "))
            .and_then(|rest| rest.strip_suffix(". Copy to clipboard"))
            .unwrap_or_default()
            .to_owned();
        let wanted = wanted.unwrap_or(shown);
        let before = copied(app);
        app.idle(Duration::from_millis(650));
        if control {
            input::click_with(app.slint(), &[Key::Control], inspect::center(&item));
        } else {
            input::click_at(app.slint(), inspect::center(&item));
        }
        app.settle();
        if !ps::wait(app, Duration::from_secs(3), |app| {
            copied(app).is_some_and(|text| text == wanted)
        }) {
            failures.push(format!(
                "{}clicking the {label} field copied {:?} (before {before:?}), expected {wanted:?}",
                if control { "Ctrl+" } else { "" },
                copied(app)
            ));
        }
    }
    Ok(())
}
