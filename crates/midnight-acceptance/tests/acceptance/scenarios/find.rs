//! Find (Alt+F7) by mask over the fixture, compared with a walk of the disk, then
//! "Go to file" from a result.
use crate::{
    Context,
    app::{self, App, Config},
    input, inspect, panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow};
use std::{collections::BTreeSet, fs, path::Path, rc::Rc};

/// Names of regular files below `path` whose name ends with `suffix` (links not followed).
fn disk_matches(path: &Path, suffix: &str, found: &mut BTreeSet<String>) {
    for entry in fs::read_dir(path).into_iter().flatten().flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() {
            disk_matches(&entry.path(), suffix, found);
        } else if kind.is_file() && name.to_lowercase().ends_with(suffix) {
            found.insert(name);
        }
    }
}

fn status(window: &MinimalSoftwareWindow) -> Option<String> {
    inspect::labels(window.window(), AccessibleRole::Button)
        .into_iter()
        .rfind(|label| {
            [
                "Ready",
                "Searching",
                "Completed",
                "Incomplete",
                "Cancelled",
                "Failed",
                "Error",
            ]
            .iter()
            .any(|prefix| label.starts_with(prefix))
        })
}

/// Result names: the first field of each row of the "Find results" list.
fn results(window: &MinimalSoftwareWindow) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if let Some(list) = inspect::find(window.window(), AccessibleRole::List, "Find results") {
        inspect::walk(&list, &mut |item| {
            if item.accessible_role() == AccessibleRole::ListItem
                && item.is_visible()
                && let Some(label) = inspect::label(item)
                && let Some(name) = label.split(" · ").next()
            {
                names.insert(name.to_owned());
            }
        });
    }
    names
}

fn search(
    app: &mut App,
    window: &Rc<MinimalSoftwareWindow>,
    mask: &str,
) -> Result<BTreeSet<String>, String> {
    input::fill(window.window(), "File name masks", mask)?;
    input::key(window.window(), Key::Return);
    let watched = window.clone();
    app.until(&format!("Find {mask} to complete"), move |_| {
        status(&watched).is_some_and(|text| text.starts_with("Completed"))
    });
    Ok(results(window))
}

pub fn by_mask(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, &root));
    panel::focus_list(&mut app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Alt], Key::F7);
    app.until("the Find window", |app| !app.tool_windows().is_empty());
    let window = app.tool_windows().pop().ok_or("no Find window")?;
    app::layout_tool(&window, 860, 620);
    app.settle();
    for mask in ["*.md", "*.webp", "*.sha256"] {
        let found = search(&mut app, &window, mask)?;
        let mut expected = BTreeSet::new();
        disk_matches(&root, &mask[1..], &mut expected);
        if found != expected {
            return Err(format!(
                "Find {mask} returned {found:?}, the disk has {expected:?}"
            ));
        }
    }
    app.screenshot_window(context, &window, "find-sha256", "Find *.sha256 results");
    // Go to file: select README.md in the results and press Enter.
    search(&mut app, &window, "*.md")?;
    let row = inspect::controls(window.window(), AccessibleRole::ListItem)
        .into_iter()
        .find(|(label, _)| label.starts_with("README.md · "))
        .map(|(_, item)| item)
        .ok_or("no README.md result row")?;
    input::click_at(window.window(), inspect::center(&row));
    app.settle();
    input::key(window.window(), Key::Return);
    let docs = root.join("docs");
    app.until("Go to file to show docs/README.md", move |app| {
        let names = app.names(PaneId::Left);
        app.location(PaneId::Left).as_deref() == Some(docs.as_path())
            && usize::try_from(app.cursor(PaneId::Left))
                .ok()
                .and_then(|row| names.get(row))
                .is_some_and(|name| name == "README.md")
    });
    input::key(window.window(), Key::Escape);
    app.until("the Find window to close", |app| {
        app.tool_windows().is_empty()
    });
    app.screenshot(
        context,
        "goto-readme",
        "Go to file put the cursor on docs/README.md",
    );
    context.note(
        "Find *.md, *.webp and *.sha256 matched the disk exactly; Go to file located README.md",
    );
    app.shutdown()
}
