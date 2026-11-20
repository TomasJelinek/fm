//! Inline tree expand-all (Ctrl+click on a folder expander) and folder sizes
//! (Ctrl+Shift+S) compared with the bytes on disk.
use crate::{
    Context,
    app::{App, Config},
    input, inspect, panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::Key;
use std::{collections::BTreeMap, fs, path::Path};

/// Logical bytes of regular files below `path`; links are not followed.
fn disk_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        0
    } else if metadata.is_dir() {
        fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| disk_size(&entry.path()))
            .sum()
    } else {
        metadata.len()
    }
}

pub fn expand_all(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, &root));
    let pane = PaneId::Left;
    panel::focus_list(&mut app, pane)?;
    app.until("the deep folder's expander", |app| {
        app.rows(pane)
            .iter()
            .any(|row| row.name == "deep" && matches!(row.tree_disclosure, 1 | 3))
    });
    let row = app
        .rows(pane)
        .into_iter()
        .find(|row| row.name == "deep")
        .ok_or("no deep row")?;
    let verb = if row.tree_expanded {
        "Collapse"
    } else if row.tree_disclosure == 1 {
        "Discover"
    } else {
        "Expand"
    };
    let label = format!("{verb} folders in deep");
    let expander = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(text, _)| *text == label)
        .map(|(_, item)| item)
        .find(|item| {
            inspect::center(item).x
                < app.window.get_left_list_left() + app.window.get_left_list_width()
        })
        .ok_or_else(|| format!("no {label:?} expander in the left panel"))?;
    input::click_with(app.slint(), &[Key::Control], inspect::center(&expander));
    // The inline tree lists folders: level-NN appears at depth NN.
    let expected: Vec<String> = (1..=12).map(|level| format!("level-{level:02}")).collect();
    let wanted = expected.clone();
    app.until("the deep branch to expand completely", move |app| {
        let rows = app.rows(pane);
        wanted
            .iter()
            .all(|name| rows.iter().any(|row| row.name.as_str() == name.as_str()))
    });
    let rows = app.rows(pane);
    for (level, name) in expected.iter().enumerate() {
        let row = rows
            .iter()
            .find(|row| row.name.as_str() == name.as_str())
            .ok_or_else(|| format!("{name} is not shown"))?;
        if row.tree_depth != level as i32 + 1 {
            return Err(format!(
                "{name} is at depth {}, expected {}",
                row.tree_depth,
                level + 1
            ));
        }
    }
    // Only the Ctrl+clicked branch expanded.
    let other_expanded: Vec<String> = rows
        .iter()
        .filter(|row| row.tree_depth == 0 && row.tree_expanded && row.name != "deep")
        .map(|row| row.name.to_string())
        .collect();
    if !other_expanded.is_empty() {
        return Err(format!("Ctrl+click also expanded {other_expanded:?}"));
    }
    app.screenshot(
        context,
        "deep-expanded",
        "Ctrl+click expanded deep/ to twelve levels inline",
    );
    context.note(format!(
        "Ctrl+click on the expander showed {} inline rows down to depth 12",
        expected.len()
    ));
    app.shutdown()
}

pub fn folder_sizes(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut config = Config::new(&root, &root);
    config.size_bytes = true;
    let mut app = App::start(config);
    let pane = PaneId::Left;
    panel::focus_list(&mut app, pane)?;
    input::chord(app.slint(), &[Key::Control, Key::Shift], "s");
    app.until_for(
        "every folder size",
        std::time::Duration::from_secs(180),
        move |app| {
            app.rows(pane)
                .iter()
                .filter(|row| row.directory && !row.parent_entry)
                .all(|row| {
                    row.size_state == 0
                        && !row.size_running
                        && row.size.chars().any(|c| c.is_ascii_digit())
                })
        },
    );
    let mut mismatches = BTreeMap::new();
    let mut checked = 0;
    for row in app
        .rows(pane)
        .iter()
        .filter(|row| row.directory && !row.parent_entry)
    {
        let shown: String = row.size.chars().filter(char::is_ascii_digit).collect();
        let expected = disk_size(&root.join(row.name.as_str()));
        if shown.parse::<u64>().ok() != Some(expected) {
            mismatches.insert(
                row.name.to_string(),
                format!("shown {:?}, disk {expected}", row.size),
            );
        }
        checked += 1;
    }
    app.screenshot(
        context,
        "folder-sizes",
        "Ctrl+Shift+S folder sizes in bytes",
    );
    if !mismatches.is_empty() {
        return Err(format!(
            "folder sizes differ from the disk: {mismatches:#?}"
        ));
    }
    context.note(format!(
        "{checked} folder sizes equal the logical bytes on disk (links not followed)"
    ));
    app.shutdown()
}
