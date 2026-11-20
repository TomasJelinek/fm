//! The inline tree: expand and collapse folders with their expander, then copy, rename
//! and delete folders that are only visible inside expanded branches.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, TransferSpec, tree_facts},
    input, inspect, panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{OperationState, PaneId};
use slint::platform::Key;
use std::time::{Duration, Instant};

const LEFT: PaneId = PaneId::Left;

fn row_depth(app: &App, name: &str) -> Option<i32> {
    app.rows(LEFT)
        .iter()
        .find(|row| row.name == name)
        .map(|row| row.tree_depth)
}

/// Click the expander of `name` in the left panel (without modifiers: one level).
fn toggle(app: &mut App, name: &str) -> Result<(), String> {
    panel::wait_input(app);
    let wanted = name.to_owned();
    app.until(&format!("{name}'s expander"), move |app| {
        app.rows(LEFT).iter().any(|row| {
            row.name == wanted && (row.tree_expanded || matches!(row.tree_disclosure, 1 | 3))
        })
    });
    let row = app
        .rows(LEFT)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| format!("no {name} row"))?;
    let verb = if row.tree_expanded {
        "Collapse"
    } else if row.tree_disclosure == 1 {
        "Discover"
    } else {
        "Expand"
    };
    let label = format!("{verb} folders in {name}");
    let left_edge = app.window.get_left_list_left() + app.window.get_left_list_width();
    let expander = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(text, _)| *text == label)
        .map(|(_, item)| item)
        .find(|item| inspect::center(item).x < left_edge)
        .ok_or_else(|| format!("no {label:?} expander in the left panel"))?;
    input::click_at(app.slint(), inspect::center(&expander));
    app.settle();
    Ok(())
}

#[track_caller]
fn wait_depth(app: &mut App, name: &str, depth: Option<i32>) {
    let wanted = name.to_owned();
    app.until(&format!("{name} to be at depth {depth:?}"), move |app| {
        row_depth(app, &wanted) == depth
    });
}

pub fn tree_operations(context: &mut Context) -> Result<(), String> {
    let root = support::scratch(context, "tree")?;
    support::stage(context, &["deep"], &root)?;
    let target = support::scratch(context, "tree-target")?;
    let mut app = App::start(Config::new(&root, &target));
    let mut failures = Vec::new();
    panel::focus_list(&mut app, LEFT)?;

    // Expand deep, collapse it, expand it again and one level further.
    toggle(&mut app, "deep")?;
    wait_depth(&mut app, "level-01", Some(1));
    toggle(&mut app, "deep")?;
    wait_depth(&mut app, "level-01", None);
    toggle(&mut app, "deep")?;
    wait_depth(&mut app, "level-01", Some(1));
    toggle(&mut app, "level-01")?;
    wait_depth(&mut app, "level-02", Some(2));
    toggle(&mut app, "level-02")?;
    wait_depth(&mut app, "level-03", Some(3));
    app.screenshot(
        context,
        "tree-expanded",
        "deep/ expanded three levels inline",
    );

    // Copy a nested folder (depth 3) to the other panel.
    let nested = root.join("deep/level-01/level-02/level-03");
    let expected = tree_facts(&nested)?;
    panel::select_row(&mut app, LEFT, "level-03")?;
    match support::transfer(&mut app, context, TransferSpec::copy()) {
        Err(error) => failures.push(format!("F5 of the nested level-03: {error}")),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F5 of the nested level-03") {
                failures.push(error);
            }
            let differences =
                support::compare_trees(&expected, &tree_facts(&target.join("level-03"))?, false);
            if !differences.is_empty() {
                failures.push(format!("the copied level-03 differs: {differences:?}"));
            }
            support::wait_row(&mut app, PaneId::Right, "level-03", true);
        }
    }

    // Rename the nested folder in place: the tree shows the new name at the same depth.
    panel::select_row(&mut app, LEFT, "level-03")?;
    let start = support::named_operation(&mut app, &[], Key::F2, "level-03 renamed")?;
    let progress = support::wait_operation(&mut app, start);
    let finished = Instant::now();
    if progress.state != OperationState::Completed {
        failures.push(format!(
            "F2 inside the tree ended {:?}: {:?}",
            progress.state, progress.errors
        ));
    } else {
        let renamed = root.join("deep/level-01/level-02/level-03 renamed");
        if !renamed.is_dir() || nested.exists() {
            failures.push("the nested rename is not on disk".into());
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while row_depth(&app, "level-03 renamed") != Some(3) && Instant::now() < deadline {
            app.idle(Duration::from_millis(10));
        }
        if row_depth(&app, "level-03 renamed") != Some(3) {
            failures.push(format!(
                "the renamed nested folder is at depth {:?}",
                row_depth(&app, "level-03 renamed")
            ));
        }
        context.note(format!(
            "nested rename listed {} ms after its report",
            finished.elapsed().as_millis()
        ));
        panel::wait_input(&mut app);
        if support::cursor_name(&app, LEFT).as_deref() != Some("level-03 renamed") {
            failures.push(format!(
                "after the nested rename the cursor is on {:?}",
                support::cursor_name(&app, LEFT)
            ));
        }
    }
    app.screenshot(
        context,
        "tree-renamed",
        "Nested folder renamed inside the tree",
    );

    // Delete the nested folder: gone from disk and from the tree.
    panel::select_row(&mut app, LEFT, "level-03 renamed")?;
    let progress = support::delete_permanently(&mut app, context, None)?;
    if let Err(error) = support::operation_ok(&progress, "Shift+F8 inside the tree") {
        failures.push(error);
    }
    if root
        .join("deep/level-01/level-02/level-03 renamed")
        .exists()
    {
        failures.push("the nested folder still exists after Shift+F8".into());
    }
    wait_depth(&mut app, "level-03 renamed", None);
    // level-02 now has no subfolder.
    let level02 = app
        .rows(LEFT)
        .into_iter()
        .find(|row| row.name == "level-02");
    context.note(format!(
        "after deleting its only subfolder, level-02 shows disclosure {:?} expanded {:?}",
        level02.as_ref().map(|row| row.tree_disclosure),
        level02.as_ref().map(|row| row.tree_expanded)
    ));

    // Collapse back to the root listing.
    toggle(&mut app, "deep")?;
    wait_depth(&mut app, "level-01", None);
    app.screenshot(context, "tree-collapsed", "deep/ collapsed again");
    app.shutdown()?;
    support::report(&failures)
}
