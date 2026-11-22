//! Inline tree expand-all (Ctrl+click on a folder expander) and folder sizes
//! (Ctrl+Shift+S) compared with the bytes on disk. Expand-all stops at the tree's
//! limits: 10,000 nested rows, and 64 levels with the reason on the last folder and in
//! an "Expansion incomplete" notice.
use crate::{
    Context,
    app::{App, Config},
    input, inspect, panel,
    scenarios::panels_support as ps,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::Key;
use std::{collections::BTreeMap, fs, path::Path, time::Duration};

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
            .all(|name| rows.iter().any(|row| row.name == name))
    });
    let rows = app.rows(pane);
    for (level, name) in expected.iter().enumerate() {
        let row = rows
            .iter()
            .find(|row| row.name == name)
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
    app.shutdown()?;
    drop(app);
    tree_limits(context)
}

/// Ctrl+click on the expander of the left panel's top-level folder `name`.
fn expand_recursively(app: &mut App, name: &str) -> Result<(), String> {
    let pane = PaneId::Left;
    app.until(&format!("the {name} folder's expander"), |app| {
        app.rows(pane)
            .iter()
            .any(|row| row.name == name && matches!(row.tree_disclosure, 1 | 3))
    });
    let edge = app.window.get_left_list_left() + app.window.get_left_list_width();
    let expander = ["Discover", "Expand"]
        .iter()
        .find_map(|verb| {
            inspect::controls(app.slint(), AccessibleRole::Button)
                .into_iter()
                .filter(|(text, _)| *text == format!("{verb} folders in {name}"))
                .map(|(_, item)| item)
                .find(|item| inspect::center(item).x < edge)
        })
        .ok_or_else(|| format!("no expander of {name} in the left panel"))?;
    input::click_with(app.slint(), &[Key::Control], inspect::center(&expander));
    app.settle();
    Ok(())
}

/// Nested rows (depth one or more) of the left panel, and the deepest depth shown.
fn nested(app: &App) -> (usize, i32) {
    let rows = app.rows(PaneId::Left);
    let deepest = rows.iter().map(|row| row.tree_depth).max().unwrap_or(0);
    (
        rows.iter().filter(|row| row.tree_depth >= 1).count(),
        deepest,
    )
}

/// Wait until the recursive expansion is done: no row is loading and the nested rows
/// have not changed for 3 s (it publishes in batches); their count and depth.
fn expansion_done(app: &mut App, limit: std::time::Duration) -> (usize, i32) {
    use std::time::{Duration, Instant};
    let loading = |app: &App| {
        app.rows(PaneId::Left)
            .iter()
            .any(|row| row.tree_disclosure == 2)
    };
    let started = Instant::now();
    let mut last = nested(app);
    let mut still = Instant::now();
    while started.elapsed() < limit {
        app.idle(Duration::from_millis(100));
        let now = nested(app);
        if now != last || loading(app) {
            last = now;
            still = Instant::now();
        } else if now.0 > 0 && still.elapsed() >= Duration::from_secs(3) {
            break;
        }
    }
    last
}

/// The deepest row's name and disclosure state (0 none … 6 failed), for diagnostics.
fn deepest_row(app: &App) -> Option<(String, i32, i32)> {
    app.rows(PaneId::Left)
        .iter()
        .max_by_key(|row| row.tree_depth)
        .map(|row| (row.name.to_string(), row.tree_depth, row.tree_disclosure))
}

/// The tree's limits (`TreeProjection`: 10,000 rows, 64 levels): Ctrl+click on a folder
/// of 101 folders holding 100 folders each shows at most 10,000 nested rows and the list
/// keeps working; Ctrl+click on a 66-level chain shows it down to level 64 and no
/// deeper: folder "64" shows why, and an "Expansion incomplete" notice names it (P-9).
fn tree_limits(context: &mut Context) -> Result<(), String> {
    let base = context.work.join("limits");
    let wide = base.join("wide");
    for outer in 0..101 {
        for inner in 0..100 {
            let path = wide
                .join(format!("w{outer:03}"))
                .join(format!("s{inner:02}"));
            fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    let mut chain = base.join("chain");
    for level in 1..=66 {
        chain = chain.join(format!("{level:02}"));
    }
    fs::create_dir_all(&chain).map_err(|error| format!("{}: {error}", chain.display()))?;
    let mut app = App::start(Config::new(&base, &context.work));
    let mut failures = Vec::new();
    panel::focus_list(&mut app, PaneId::Left)?;

    expand_recursively(&mut app, "chain")?;
    let (rows, deepest) = expansion_done(&mut app, std::time::Duration::from_secs(60));
    let deepest_name = app
        .rows(PaneId::Left)
        .iter()
        .filter(|row| row.tree_depth == deepest)
        .map(|row| row.name.to_string())
        .next();
    let reason = app.error(PaneId::Left);
    context.note(format!(
        "Ctrl+click on the 66-level chain: {rows} nested rows, the deepest at depth {deepest} \
         ({deepest_name:?}); the panel's error line {reason:?}"
    ));
    if deepest != 64 || rows != 64 || deepest_name.as_deref() != Some("64") {
        context.note(format!(
            "the chain's deepest row (name, depth, disclosure): {:?}",
            deepest_row(&app)
        ));
        failures.push(format!(
            "P-9: Ctrl+click on a 66-level chain showed {rows} nested rows down to depth \
             {deepest} ({deepest_name:?}; the panel says {reason:?}); the tree stops at 64 levels"
        ));
    }
    // Folder "64" says why its subfolders are not shown, and so does an "Expansion
    // incomplete" notice that names the folder.
    let limit = "Subfolders here are more than 64 levels deep and are not shown.";
    let last = app
        .rows(PaneId::Left)
        .iter()
        .find(|row| row.tree_depth == 64 && row.name == "64")
        .map(|row| (row.tree_disclosure, row.tree_error.to_string()));
    if last
        .as_ref()
        .is_none_or(|(disclosure, error)| *disclosure != 6 || error != limit)
    {
        failures.push(format!(
            "P-9: folder \"64\" shows (disclosure, error) {last:?}; expected Failed with {limit:?}"
        ));
    }
    let folder = (1..=64).fold(base.join("chain"), |path, level| {
        path.join(format!("{level:02}"))
    });
    ps::wait(&mut app, Duration::from_secs(10), |app| {
        crate::files_support::shown_notice(app).is_some()
    });
    let notice = crate::files_support::shown_notice(&app);
    let named = {
        use slint::ComponentHandle;
        app.window
            .global::<midnight_ui::NoticePresentation>()
            .get_folder_path()
            .to_string()
    };
    let reason_text = "the tree shows at most 64 levels of subfolders, so the folders inside \
                       “64” are not shown.";
    if notice
        .as_ref()
        .is_none_or(|(title, body)| title != "Expansion incomplete" || !body.contains(reason_text))
        || !named.contains(folder.to_string_lossy().as_ref())
    {
        failures.push(format!(
            "P-9: at the 64-level limit the notice is {notice:?} naming {named:?}; expected \
             \"Expansion incomplete\" with {reason_text:?} naming {}",
            folder.display()
        ));
    }
    app.screenshot(
        context,
        "chain-64",
        "A 66-level chain expanded to its 64-level limit",
    );
    crate::files_support::dismiss_all(&mut app);

    // Collapse the chain first: the wide folder alone fills the tree.
    let (before, _) = nested(&app);
    let label = "Collapse folders in chain";
    if let Some(item) = inspect::find(app.slint(), AccessibleRole::Button, label) {
        input::click_at(app.slint(), inspect::center(&item));
        app.settle();
    }
    expand_recursively(&mut app, "wide")?;
    let (rows, deepest) = expansion_done(&mut app, std::time::Duration::from_secs(120));
    context.note(format!(
        "Ctrl+click on 101 folders of 100 folders each: {rows} nested rows (before {before}), \
         the deepest at depth {deepest}; the panel's error line {:?}",
        app.error(PaneId::Left)
    ));
    if !(5_000..=10_000).contains(&rows) || deepest != 2 {
        failures.push(format!(
            "Ctrl+click on 101 folders of 100 folders each showed {rows} nested rows down to \
             depth {deepest}; the tree holds at most 10,000 rows"
        ));
    }
    if let Some(notice) = crate::files_support::shown_notice(&app) {
        context.note(format!("the 10,000-row limit's notice: {notice:?}"));
        crate::files_support::dismiss_all(&mut app);
    }
    // The list still works at its limit: End and Home reach both ends.
    panel::focus_list(&mut app, PaneId::Left)?;
    input::key(app.slint(), Key::End);
    app.settle();
    let last = app.rows(PaneId::Left).len() as i32 - 1;
    if app.cursor(PaneId::Left) != last {
        failures.push(format!(
            "End in the 10,000-row tree put the cursor on row {} of {}",
            app.cursor(PaneId::Left),
            last + 1
        ));
    }
    input::key(app.slint(), Key::Home);
    app.settle();
    if app.cursor(PaneId::Left) != 0 {
        failures.push("Home in the 10,000-row tree did not reach the first row".into());
    }
    app.screenshot(
        context,
        "tree-10000",
        "Ctrl+click on 101 × 100 folders: the row limit",
    );
    app.shutdown()?;
    crate::files_support::report(&failures)
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
