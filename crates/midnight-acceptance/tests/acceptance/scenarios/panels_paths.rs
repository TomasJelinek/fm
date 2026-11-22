//! Navigation by pointer and by the keys the other scenarios leave out: the toolbar's
//! Back, Forward, Parent and Refresh buttons (disabled without history) and the Back
//! and Forward history menus; Ctrl+PageUp, Ctrl+PageDown (into a folder, never opening
//! a file), Shift+Backspace and Ctrl+Backslash; double clicks on a folder and on `..`;
//! Tab and Shift+Tab between the panels; the path bar: a hovered segment highlights its
//! prefix, a right click copies it (pasted back into the path editor with a separator),
//! a long path collapses its middle into one marker that opens the whole path for
//! editing, the blank space after the path opens the editor and Escape cancels it; the
//! drive menu (Alt+F1, Alt+F2, `.` As Other Panel, the filesystem root by pointer); an
//! empty folder and a search without matches say so. A typed path that names a file
//! opens the file's folder with the cursor on it (quoted, with a final separator, with
//! spaces around it, with `/` on Windows, relative), or says why the panel does not
//! show it (hidden, filtered out, not listed in that letter case); a missing file's path
//! is reported and an archive's path focuses the archive without opening it.
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
    details_values(&mut app, context, &mut failures)?;
    empty_states(&mut app, context, &mut failures)?;
    app.shutdown()?;
    drop(app);
    quick_filter_empty(context, &mut failures)?;
    typed_files(context, &mut failures)?;
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

/// The left panel shows `folder` with its listing complete and the cursor on `name`.
fn shows_file(app: &App, folder: &Path, name: &str) -> bool {
    app.location(LEFT).as_deref() == Some(folder)
        && !app.loading(LEFT)
        && ps::cursor(app, LEFT).as_deref() == Some(name)
}

/// The "Not shown" note that leads the left details strip, if any.
fn not_shown(app: &App) -> Option<panel::Detail> {
    panel::details_of(app, LEFT)
        .into_iter()
        .find(|detail| detail.label == "Not shown")
}

/// Go to `folder` by its typed path and wait for its listing.
fn typed_folder(app: &mut App, folder: &Path) -> Result<(), String> {
    panel::type_path(app, LEFT, &folder.to_string_lossy())?;
    if !reached(app, folder) {
        return Err(format!(
            "the typed folder path {} did not open (panel at {:?}, notice {:?})",
            folder.display(),
            at(app, LEFT),
            support::shown_notice(app)
        ));
    }
    Ok(())
}

/// Type `text`, a path naming the file `name` in `folder`: the panel must show that
/// folder with the cursor on the file, nothing marked, no notice and no note, and the
/// keyboard back in the list.
fn typed_file(
    app: &mut App,
    what: &str,
    text: &str,
    folder: &Path,
    name: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::type_path(app, LEFT, text)?;
    let ended = ps::wait(app, Duration::from_secs(10), |app| {
        shows_file(app, folder, name) || support::shown_notice(app).is_some()
    });
    let mut problems = Vec::new();
    if !ended || !shows_file(app, folder, name) {
        problems.push(format!(
            "the panel shows {:?} (loading {}) with the cursor on {:?}",
            at(app, LEFT),
            app.loading(LEFT),
            ps::cursor(app, LEFT)
        ));
    }
    if let Some(notice) = support::take_notice(app) {
        problems.push(format!("a notice {notice:?}"));
    }
    let marks = ps::marked(app, LEFT);
    if !marks.is_empty() {
        problems.push(format!("marks {marks:?}"));
    }
    if let Some(note) = not_shown(app) {
        problems.push(format!("the note {:?}: {:?}", note.value, note.explanation));
    }
    if app.window.get_focused_pane() != 0 {
        problems.push(format!(
            "the keyboard is not back in the list (focused pane {})",
            app.window.get_focused_pane()
        ));
    }
    if !problems.is_empty() {
        failures.push(format!(
            "the typed file path {what} ({text:?}): {}",
            problems.join("; ")
        ));
    }
    Ok(())
}

/// Wait until the left panel shows `folder` with a "Not shown" note, or a notice opens.
fn wait_note(app: &mut App, folder: &Path) -> bool {
    let wanted = folder.to_path_buf();
    ps::wait(app, Duration::from_secs(10), |app| {
        (app.location(LEFT).as_ref() == Some(&wanted)
            && !app.loading(LEFT)
            && not_shown(app).is_some())
            || support::shown_notice(app).is_some()
    })
}

/// Type a path naming `name` in `folder`, which the panel does not show: the folder
/// opens, a "Not shown" note leads the details strip with `value`, and its explanation
/// holds `explanation`; no notice appears and the cursor is not on the file.
fn typed_unshown(
    app: &mut App,
    what: &str,
    folder: &Path,
    name: &str,
    value: &str,
    explanation: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::type_path(app, LEFT, &folder.join(name).to_string_lossy())?;
    let noted = wait_note(app, folder);
    let note = not_shown(app);
    if !noted
        || note
            .as_ref()
            .is_none_or(|note| note.value != value || !note.explanation.contains(explanation))
    {
        failures.push(format!(
            "{what}: the panel shows {:?} with the note {note:?} and the notice {:?}; expected \
             {} noting {value:?} with {explanation:?}",
            at(app, LEFT),
            support::shown_notice(app),
            folder.display()
        ));
    }
    if ps::cursor(app, LEFT).as_deref() == Some(name) {
        failures.push(format!(
            "{what}: the cursor is on the file the panel does not show"
        ));
    }
    support::dismiss_all(app);
    Ok(())
}

/// Paths typed in the path editor that name a file (2b901122;
/// docs/developer/PATH_PASTE.md): the file's folder opens with the cursor on the file, as
/// typed, pasted ("Copy as path" quotes, the separator Paste adds) or relative; a file
/// the panel does not show is noted with the reason; a missing file is reported; an
/// archive is focused, not opened; on Windows a name in other letter case finds the
/// listed one unless the folder keeps letter case apart.
fn typed_files(context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let typed = support::scratch(context, "typed")?;
    let elsewhere = support::scratch(context, "elsewhere")?;
    for (name, text) in [
        ("plain file.txt", "plain"),
        (".dot.txt", "dot"),
        ("filter-me.log", "log"),
        ("ReadMe.TXT", "read me"),
    ] {
        ps::write(&typed.join(name), text)?;
    }
    crate::fixtures::hide(&typed.join(".dot.txt"))?;
    let bundle = context.fixture.root.join("archives").join("bundle.zip");
    std::fs::copy(&bundle, typed.join("bundle.zip"))
        .map_err(|error| support::io_error(&bundle, error))?;
    let mut app = App::start(Config::new(&elsewhere, &context.work));

    // One file's path, typed and as pasted, always from another folder.
    let text = typed.join("plain file.txt").to_string_lossy().into_owned();
    let separator = std::path::MAIN_SEPARATOR;
    let mut variants = vec![
        ("as typed", text.clone()),
        ("in double quotes", format!("\"{text}\"")),
        (
            "quoted, with the separator Paste adds",
            format!("\"{text}\"{separator}"),
        ),
        ("with a final separator", format!("{text}{separator}")),
        ("with spaces around it", format!("  {text}  ")),
    ];
    if cfg!(windows) {
        variants.push(("with / separators", text.replace('\\', "/")));
    }
    for (what, text) in &variants {
        typed_folder(&mut app, &elsewhere)?;
        typed_file(&mut app, what, text, &typed, "plain file.txt", failures)?;
    }
    app.screenshot(
        context,
        "typed-file-path",
        "A typed file path: its folder with the cursor on the file",
    );

    // A name relative to the shown folder moves the cursor without listing it again.
    let listings = app.listings[0];
    typed_file(
        &mut app,
        "relative to the shown folder",
        "ReadMe.TXT",
        &typed,
        "ReadMe.TXT",
        failures,
    )?;
    if app.listings[0] != listings {
        failures.push(format!(
            "the relative file name listed the folder again ({} listings, {listings} before)",
            app.listings[0]
        ));
    }

    // An archive's path focuses the archive; the panel does not open it.
    typed_folder(&mut app, &elsewhere)?;
    typed_file(
        &mut app,
        "of an archive",
        &typed.join("bundle.zip").to_string_lossy(),
        &typed,
        "bundle.zip",
        failures,
    )?;

    // A missing file is reported with its path; the panel keeps its folder.
    let gone = typed.join("gone.txt");
    panel::type_path(&mut app, LEFT, &gone.to_string_lossy())?;
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        support::shown_notice(app).is_some()
    }) {
        failures.push(format!(
            "the missing file's path {} reported nothing (panel at {:?}, error {:?})",
            gone.display(),
            at(&app, LEFT),
            app.error(LEFT)
        ));
    } else {
        let (title, body) = support::shown_notice(&app).unwrap_or_default();
        // The notice names the path in its folder box, beside the title and the reason.
        let folder = {
            use slint::ComponentHandle;
            app.window
                .global::<midnight_ui::NoticePresentation>()
                .get_folder_path()
                .to_string()
        };
        let named = format!("{title}\n{body}\n{folder}").contains(gone.to_string_lossy().as_ref());
        if title != "Cannot open folder" || !named {
            failures.push(format!(
                "the missing file's path gave the notice {title:?} / {body:?} / folder \
                 {folder:?}; expected \"Cannot open folder\" naming {}",
                gone.display()
            ));
        }
        input::key(app.slint(), Key::Escape);
        app.settle();
    }
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        support::shown_notice(app).is_none()
            && app.location(LEFT).as_deref() == Some(typed.as_path())
    }) {
        failures.push(format!(
            "after Escape from the missing file's notice: panel at {:?}, notice {:?}",
            at(&app, LEFT),
            support::shown_notice(&app)
        ));
    }
    support::dismiss_all(&mut app);

    // Hidden files hidden (Ctrl+H): the note says why and which key shows them.
    typed_folder(&mut app, &typed)?;
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "h");
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        !app.names(LEFT).iter().any(|name| name == ".dot.txt") && !app.loading(LEFT)
    }) {
        failures.push(format!(
            "Ctrl+H did not hide .dot.txt: {:?}",
            app.names(LEFT)
        ));
    }
    typed_folder(&mut app, &elsewhere)?;
    typed_unshown(
        &mut app,
        "a hidden file's path",
        &typed,
        ".dot.txt",
        ".dot.txt is hidden",
        "Ctrl+H shows them",
        failures,
    )?;
    app.screenshot(
        context,
        "typed-hidden-file",
        "A typed path of a hidden file: the note says why",
    );
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "h");
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        app.names(LEFT).iter().any(|name| name == ".dot.txt") && !app.loading(LEFT)
    }) {
        failures.push("a second Ctrl+H did not show .dot.txt again".into());
    }

    // The panel filter leaves the file out: the note says so and which key edits it.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        app.window.get_panel_filter_view().open
    }) {
        return Err("Ctrl+F12 opened no Panel filter".into());
    }
    support::choose(&mut app, "Filter mode", "Masks", 3)?;
    input::fill(app.slint(), "File filter expression", "*.txt")?;
    input::click_button(app.slint(), "OK")?;
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        !app.window.get_panel_filter_view().open
            && !app.names(LEFT).iter().any(|name| name == "filter-me.log")
    }) {
        failures.push(format!(
            "the Masks filter *.txt did not leave filter-me.log out: {:?}",
            app.names(LEFT)
        ));
    }
    typed_unshown(
        &mut app,
        "a filtered-out file's path",
        &typed,
        "filter-me.log",
        "filter-me.log is filtered out",
        "Ctrl+F12 edits the filter",
        failures,
    )?;
    match inspect::find(app.slint(), AccessibleRole::Button, "Clear panel filter") {
        Some(item) => {
            input::click_at(app.slint(), inspect::center(&item));
            app.settle();
        }
        None => failures.push("the filter indicator has no × (Clear panel filter)".into()),
    }
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        app.names(LEFT).iter().any(|name| name == "filter-me.log")
    }) {
        failures.push("clearing the filter did not list filter-me.log again".into());
    }

    // Windows folders ignore letter case: a name in other case finds the listed one.
    #[cfg(windows)]
    letter_case(&mut app, context, &typed, failures)?;
    app.shutdown()
}

/// Windows: `readme.txt` puts the cursor on `ReadMe.TXT` with no note; in a folder that
/// keeps letter case apart (`fsutil file setCaseSensitiveInfo`) with `ab.txt` and
/// `AB.txt`, `Ab.txt` is "not listed here" and the cursor stays.
#[cfg(windows)]
fn letter_case(
    app: &mut App,
    context: &mut Context,
    typed: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    typed_folder(app, typed)?;
    panel::select_row(app, LEFT, "plain file.txt")?;
    typed_file(
        app,
        "in other letter case",
        "readme.txt",
        typed,
        "ReadMe.TXT",
        failures,
    )?;
    let sensitive = typed.join("case-sensitive");
    ps::folder(&sensitive)?;
    let enabled = std::process::Command::new("fsutil")
        .args(["file", "setCaseSensitiveInfo"])
        .arg(&sensitive)
        .arg("enable")
        .stdin(std::process::Stdio::null())
        .output();
    // fsutil reports a refusal (such as access denied) with a zero status: ask it back.
    let query = std::process::Command::new("fsutil")
        .args(["file", "queryCaseSensitiveInfo"])
        .arg(&sensitive)
        .stdin(std::process::Stdio::null())
        .output();
    let refused = match (enabled, query) {
        (Ok(_), Ok(query)) if String::from_utf8_lossy(&query.stdout).contains("is enabled") => None,
        (Ok(output), _) => Some(String::from_utf8_lossy(&output.stdout).trim().to_owned()),
        (Err(error), _) => Some(error.to_string()),
    };
    if let Some(reason) = refused {
        context.note(format!(
            "SKIP: a folder that keeps letter case apart: fsutil setCaseSensitiveInfo was \
             refused: {reason}"
        ));
        return Ok(());
    }
    for name in ["ab.txt", "AB.txt", "zz.txt"] {
        ps::write(&sensitive.join(name), name)?;
    }
    let names = support::names_on_disk(&sensitive);
    if names.len() != 3 {
        context.note(format!(
            "SKIP: a folder that keeps letter case apart: it holds {names:?}"
        ));
        return Ok(());
    }
    typed_folder(app, &sensitive)?;
    panel::select_row(app, LEFT, "zz.txt")?;
    panel::type_path(app, LEFT, "Ab.txt")?;
    let noted = wait_note(app, &sensitive);
    let note = not_shown(app);
    if !noted
        || note
            .as_ref()
            .is_none_or(|note| note.value != "Ab.txt is not listed here")
    {
        failures.push(format!(
            "Ab.txt among ab.txt and AB.txt: the panel shows {:?} with the note {note:?} and \
             the notice {:?}; expected \"Ab.txt is not listed here\"",
            at(app, LEFT),
            support::shown_notice(app)
        ));
    }
    if ps::cursor(app, LEFT).as_deref() != Some("zz.txt") {
        failures.push(format!(
            "Ab.txt among ab.txt and AB.txt moved the cursor to {:?}",
            ps::cursor(app, LEFT)
        ));
    }
    support::dismiss_all(app);
    Ok(())
}

/// The digit groups of a shown value, in order ("14. 11. 2023" → [14, 11, 2023]).
fn digit_groups(text: &str) -> Vec<u64> {
    text.split(|character: char| !character.is_ascii_digit())
        .filter(|group| !group.is_empty())
        .filter_map(|group| group.parse().ok())
        .collect()
}

/// The left details strip's copy button for `label` (its accessible label is
/// "<label>: <value>. Copy to clipboard"), with the value it shows.
fn detail_button(app: &App, label: &str) -> Option<(String, ItemRc)> {
    let top = app.window.get_left_details_top();
    let edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find_map(|(text, item)| {
            let value = text
                .strip_prefix(&format!("{label}: "))?
                .strip_suffix(". Copy to clipboard")?
                .to_owned();
            (inspect::rect(&item).1 >= top - 1.0 && inspect::center(&item).x < edge)
                .then_some((value, item))
        })
}

/// What the in-process clipboard holds, and how many copies it took so far.
fn clipboard(app: &App) -> (Option<String>, u64) {
    app.clipboard.as_ref().map_or((None, 0), |clipboard| {
        (clipboard.text(), clipboard.writes())
    })
}

/// Wait for one more copy to the clipboard than `before`, holding `wanted`.
fn copied_once_more(app: &mut App, before: u64, wanted: &str) -> bool {
    ps::wait(app, Duration::from_secs(3), |app| {
        let (text, writes) = clipboard(app);
        writes > before && text.as_deref() == Some(wanted)
    })
}

/// The details strip's values against the disk and the keyboard and pointer copies
/// that `details_copy` leaves out: a file modified at 12:34:56 UTC on 14 November 2023
/// (that day in every time zone from UTC−11 to UTC+11) shows its exact byte count, a
/// date of that year, month and day in the locale's order and a time of those minutes
/// and seconds, the same date and time as its row; Space and Enter on a focused field
/// copy it again; the free space at the strip's right copies the value it shows.
fn details_values(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let folder = support::scratch(context, "details")?;
    let file = folder.join("dated.txt");
    let modified = ps::instant((2023, 11, 14), (12, 34, 56), 0, 0);
    crate::tools_support::write_at(&file, &[b'd'; 1234], modified)?;
    go(app, &folder)?;
    panel::select_row(app, LEFT, "dated.txt")?;
    let wanted =
        |label: &str| -> Option<String> { detail_button(app, label).map(|(value, _)| value) };
    let (bytes, date, time) = (wanted("Bytes"), wanted("Date"), wanted("Time"));
    if bytes.as_deref().is_none_or(|bytes| {
        digit_groups(bytes)
            .iter()
            .fold(0, |sum, group| sum * 1000 + group)
            != 1234
    }) {
        failures.push(format!(
            "the details strip's Bytes of a 1234-byte file reads {bytes:?}"
        ));
    }
    let mut day = date.as_deref().map(digit_groups).unwrap_or_default();
    day.sort_unstable();
    if day != [11, 14, 2023] && day != [11, 14, 23] {
        failures.push(format!(
            "the details strip's Date of a file modified on 2023-11-14 reads {date:?}"
        ));
    }
    let clock = time.as_deref().map(digit_groups).unwrap_or_default();
    // Hours depend on the time zone; minutes too in a zone of a half or quarter hour.
    if clock.len() < 2
        || ![34, 4, 19, 49].contains(&clock[1])
        || clock.get(2).is_some_and(|seconds| *seconds != 56)
    {
        failures.push(format!(
            "the details strip's Time of a file modified at 12:34:56 UTC reads {time:?}"
        ));
    }
    let row = app
        .rows(LEFT)
        .into_iter()
        .find(|row| row.name == "dated.txt")
        .map(|row| (row.modified_date.to_string(), row.modified_time.to_string()));
    if row.as_ref().map(|(date, time)| (Some(date), Some(time)))
        != Some((date.as_ref(), time.as_ref()))
    {
        failures.push(format!(
            "the details strip says {date:?} {time:?}, the row {row:?}"
        ));
    }
    // A clicked field has the keyboard: Space and Enter copy it again.
    match detail_button(app, "Bytes") {
        None => failures.push("the details strip shows no Bytes field".into()),
        Some((value, item)) => {
            app.idle(Duration::from_millis(650));
            let (_, before) = clipboard(app);
            input::click_at(app.slint(), inspect::center(&item));
            app.settle();
            if !copied_once_more(app, before, &value) {
                failures.push(format!(
                    "clicking the Bytes field did not copy {value:?}: {:?}",
                    clipboard(app)
                ));
            }
            for key in [Key::Space, Key::Return] {
                let (_, before) = clipboard(app);
                input::key(app.slint(), key);
                app.settle();
                if !copied_once_more(app, before, &value) {
                    failures.push(format!(
                        "{key:?} on the focused Bytes field did not copy it again: {:?}",
                        clipboard(app)
                    ));
                }
            }
        }
    }
    // The free space at the strip's right copies what it shows, once the volume's space
    // is known ("Free space —" until then).
    ps::wait(app, Duration::from_secs(10), |app| {
        panel::free_space(app, LEFT).ends_with(" free")
    });
    let free = panel::free_space(app, LEFT);
    let button = inspect::find(
        app.slint(),
        AccessibleRole::Button,
        &format!("{free}. Copy to clipboard"),
    );
    match button.filter(|_| !free.is_empty()) {
        None => failures.push(format!(
            "the details strip shows no free-space button (free space {free:?})"
        )),
        Some(item) => {
            if !free.ends_with(" free") || digit_groups(&free).is_empty() {
                failures.push(format!("the free space reads {free:?}"));
            }
            app.idle(Duration::from_millis(650));
            let (_, before) = clipboard(app);
            let (x, y, w, h) = inspect::visible_rect(&item).unwrap_or_else(|| inspect::rect(&item));
            input::click_at(app.slint(), LogicalPosition::new(x + w / 2.0, y + h / 2.0));
            app.settle();
            if !copied_once_more(app, before, &free) {
                failures.push(format!(
                    "clicking the free space {free:?} copied {:?}",
                    clipboard(app)
                ));
            }
        }
    }
    panel::focus_list(app, LEFT)?;
    Ok(())
}
