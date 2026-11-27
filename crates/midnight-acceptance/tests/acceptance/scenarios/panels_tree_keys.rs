//! Inline folder branches with the keyboard and with the other panel features: arrows,
//! Home and End through nested rows; Insert and Space mark (and measure) nested
//! folders; Ctrl+I inverts the top level and keeps nested marks; Ctrl+Shift+A keeps only
//! top-level files; Ctrl+D clears every level; sorting orders each branch's folders
//! (name both ways, modified time); collapsing a branch with the cursor inside moves it
//! to the branch; Enter on a nested folder opens it and Backspace returns with the
//! cursor on it; an Expression filter keeps a matching folder's ancestor as context
//! that cannot be marked. A quick search on a nested row never blocks a command
//! (aac18026): F7 and Shift+F8 close it, the nested match keeps its depth and the cursor,
//! and Shift+F8 deletes that nested folder.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use slint::platform::Key;
use std::{
    path::Path,
    time::{Duration, SystemTime},
};

fn depth(app: &App, name: &str) -> Option<i32> {
    app.rows(LEFT)
        .iter()
        .find(|row| row.name == name)
        .map(|row| row.tree_depth)
}

/// The rows shown below `parent` at depth one more, in order.
fn children(app: &App, parent: &str) -> Vec<String> {
    let rows = app.rows(LEFT);
    let Some(index) = rows.iter().position(|row| row.name == parent) else {
        return Vec::new();
    };
    let level = rows[index].tree_depth;
    rows[index + 1..]
        .iter()
        .take_while(|row| row.tree_depth > level)
        .filter(|row| row.tree_depth == level + 1)
        .map(|row| row.name.to_string())
        .collect()
}

fn press(app: &mut App, key: Key) {
    input::key(app.slint(), key);
    app.settle();
}

fn expect_marks(app: &mut App, what: &str, wanted: &[&str], failures: &mut Vec<String>) {
    let wanted = ps::sorted(wanted);
    let target = wanted.clone();
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT) == target
    }) {
        failures.push(format!(
            "{what}: marks {:?}, expected {wanted:?}",
            ps::marked(app, LEFT)
        ));
    }
}

fn expect_cursor(app: &App, what: &str, wanted: &str, failures: &mut Vec<String>) {
    if ps::cursor(app, LEFT).as_deref() != Some(wanted) {
        failures.push(format!(
            "{what}: the cursor is on {:?}, expected {wanted}",
            ps::cursor(app, LEFT)
        ));
    }
}

pub fn tree_keys(context: &mut Context) -> Result<(), String> {
    let root = support::scratch(context, "tree-keys")?;
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for (name, minutes) in [("a-two", 2), ("b-one", 1), ("c-three", 3)] {
        let child = root.join("branch").join(name);
        ps::folder(&child.join("deeper"))?;
        ps::write(&child.join("file.bin"), vec![1_u8; 100 * minutes as usize])?;
        crate::tools_support::times_handle(&child)
            .and_then(|folder| folder.set_modified(base + Duration::from_secs(minutes * 60)))
            .map_err(|error| format!("{}: {error}", child.display()))?;
    }
    ps::folder(&root.join("other/x-ray"))?;
    ps::folder(&root.join("other/alpha"))?;
    for name in ["top1.txt", "top2.txt"] {
        ps::write(&root.join(name), name)?;
    }
    let mut app = App::start(Config::new(&root, &context.work));
    let mut failures = Vec::new();
    panel::focus_list(&mut app, LEFT)?;
    support::toggle_tree(&mut app, "branch")?;
    support::wait_tree_depth(&mut app, "a-two", Some(1));
    app.screenshot(context, "branch-expanded", "branch/ expanded inline");

    // Arrows, Home and End go through the nested rows in displayed order.
    panel::select_row(&mut app, LEFT, "branch")?;
    for (key, wanted) in [
        (Key::DownArrow, "a-two"),
        (Key::DownArrow, "b-one"),
        (Key::DownArrow, "c-three"),
        (Key::DownArrow, "other"),
        (Key::UpArrow, "c-three"),
        (Key::End, "top2.txt"),
        (Key::Home, ".."),
    ] {
        press(&mut app, key);
        expect_cursor(
            &app,
            &format!("{key:?} in the expanded tree"),
            wanted,
            &mut failures,
        );
    }

    // Marks on nested rows.
    panel::select_row(&mut app, LEFT, "b-one")?;
    press(&mut app, Key::Insert);
    expect_cursor(&app, "Insert on nested b-one", "c-three", &mut failures);
    press(&mut app, Key::Space);
    expect_cursor(&app, "Space on nested c-three", "other", &mut failures);
    expect_marks(
        &mut app,
        "Insert and Space on nested folders",
        &["b-one", "c-three"],
        &mut failures,
    );
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        app.rows(LEFT).iter().any(|row| {
            row.name == "c-three"
                && !row.size_running
                && row.size.chars().any(|c| c.is_ascii_digit())
        })
    }) {
        failures.push("Space on the nested c-three did not measure it".into());
    }
    input::chord(app.slint(), &[Key::Control], "i");
    expect_marks(
        &mut app,
        "Ctrl+I with nested marks",
        &[
            "b-one", "c-three", "branch", "other", "top1.txt", "top2.txt",
        ],
        &mut failures,
    );
    input::chord(app.slint(), &[Key::Control, Key::Shift], "a");
    expect_marks(
        &mut app,
        "Ctrl+Shift+A in the tree",
        &["top1.txt", "top2.txt"],
        &mut failures,
    );
    panel::select_row(&mut app, LEFT, "a-two")?;
    press(&mut app, Key::Insert);
    expect_marks(
        &mut app,
        "Insert on nested a-two",
        &["a-two", "top1.txt", "top2.txt"],
        &mut failures,
    );
    input::chord(app.slint(), &[Key::Control], "d");
    expect_marks(&mut app, "Ctrl+D in the tree", &[], &mut failures);

    // Sorting orders each branch's folders.
    let toward = |app: &mut App, column: i32, descending: bool| {
        let key = match column {
            2 => Key::F5,
            _ => Key::F3,
        };
        for _ in 0..3 {
            if app.window.get_left_sort() == column
                && app.window.get_left_descending() == descending
            {
                break;
            }
            input::chord(app.slint(), &[Key::Control], key);
            app.settle();
        }
    };
    for (column, descending, wanted) in [
        (0, true, ["c-three", "b-one", "a-two"]),
        (2, false, ["b-one", "a-two", "c-three"]),
        (2, true, ["c-three", "a-two", "b-one"]),
        (0, false, ["a-two", "b-one", "c-three"]),
    ] {
        toward(&mut app, column, descending);
        let target: Vec<String> = wanted.iter().map(|name| (*name).to_owned()).collect();
        if !ps::wait(&mut app, Duration::from_secs(5), |app| {
            children(app, "branch") == target
        }) {
            failures.push(format!(
                "sorted by {} {}: branch/ lists {:?}, expected {wanted:?}",
                if column == 2 { "modified" } else { "name" },
                if descending {
                    "descending"
                } else {
                    "ascending"
                },
                children(&app, "branch")
            ));
        }
    }

    // Collapsing the branch with the cursor inside moves it to the branch.
    panel::select_row(&mut app, LEFT, "b-one")?;
    support::toggle_tree(&mut app, "branch")?;
    support::wait_tree_depth(&mut app, "b-one", None);
    expect_cursor(
        &app,
        "collapsing branch/ with the cursor on b-one",
        "branch",
        &mut failures,
    );

    // Enter on a nested folder opens it; Backspace returns with the cursor on it.
    support::toggle_tree(&mut app, "branch")?;
    support::wait_tree_depth(&mut app, "c-three", Some(1));
    panel::select_row(&mut app, LEFT, "c-three")?;
    press(&mut app, Key::Return);
    let nested = root.join("branch/c-three");
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        app.location(LEFT).as_deref() == Some(nested.as_path())
    }) {
        failures.push(format!(
            "Enter on the nested c-three went to {:?}",
            app.location(LEFT)
        ));
    } else {
        press(&mut app, Key::Backspace);
        panel::wait_location(&mut app, LEFT, &root.join("branch"));
        expect_cursor(
            &app,
            "Backspace from the nested folder",
            "c-three",
            &mut failures,
        );
        press(&mut app, Key::Backspace);
        panel::wait_location(&mut app, LEFT, &root);
        expect_cursor(&app, "Backspace to the root", "branch", &mut failures);
    }

    // An Expression filter keeps the ancestor of a match as unmarkable context.
    panel::focus_list(&mut app, LEFT)?;
    if depth(&app, "x-ray").is_none() {
        support::toggle_tree(&mut app, "other")?;
        support::wait_tree_depth(&mut app, "x-ray", Some(1));
    }
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    crate::options_support::choose(&mut app, "Filter mode", "Expression")?;
    input::fill(app.slint(), "File filter expression", "name like \"x*\"")?;
    input::click_button(app.slint(), "OK")?;
    app.until("the filter to apply", |app| {
        !app.window.get_panel_filter_view().open
    });
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        ps::shown(app, LEFT) == ["other", "x-ray"]
    }) {
        failures.push(format!(
            "the x* expression over the expanded other/ shows {:?}, expected other (context) and x-ray",
            ps::shown(&app, LEFT)
        ));
    } else {
        app.screenshot(
            context,
            "filter-context",
            "A filtered tree: other/ kept as context",
        );
        panel::select_row(&mut app, LEFT, "other")?;
        press(&mut app, Key::Insert);
        input::chord(app.slint(), &[Key::Control], "a");
        app.settle();
        if ps::marked(&app, LEFT).contains(&"other".to_owned()) {
            failures.push("the filter's context row other/ could be marked".into());
        }
        panel::select_row(&mut app, LEFT, "x-ray")?;
        press(&mut app, Key::Insert);
        if !ps::marked(&app, LEFT).contains(&"x-ray".to_owned()) {
            failures.push("Insert did not mark the matching nested x-ray".into());
        }
        input::chord(app.slint(), &[Key::Control], "d");
        app.settle();
    }
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    input::click_button(app.slint(), "Show all")?;
    app.until("the filter to close", |app| {
        !app.window.get_panel_filter_view().open
    });
    tree_commands(&mut app, &root, &mut failures)?;
    app.shutdown()?;
    support::report(&failures)
}

/// The quick search of the left panel is closed: not active and its text empty.
fn search_closed(app: &App) -> bool {
    !app.window.get_left_search_active() && app.window.get_left_search_query().is_empty()
}

/// Search for `text` in the expanded tree and wait until the cursor is on the nested
/// match `name`.
fn search_nested(
    app: &mut App,
    text: &str,
    name: &str,
    failures: &mut Vec<String>,
) -> Result<bool, String> {
    panel::select_row(app, LEFT, "top1.txt")?;
    input::type_text(app.slint(), text);
    if !ps::wait(app, Duration::from_secs(10), |app| {
        ps::cursor(app, LEFT).as_deref() == Some(name) && !app.window.get_left_search_pending()
    }) {
        failures.push(format!(
            "searching {text:?} in the tree put the cursor on {:?}, not the nested {name}",
            ps::cursor(app, LEFT)
        ));
        input::key(app.slint(), Key::Escape);
        app.settle();
        return Ok(false);
    }
    Ok(true)
}

/// A command on a nested match (aac18026): F7 and Shift+F8 close the search; the match
/// keeps its depth and the cursor; F7 creates in the panel's folder, Shift+F8 deletes
/// the nested folder itself.
fn tree_commands(app: &mut App, root: &Path, failures: &mut Vec<String>) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    if depth(app, "b-one") != Some(1) {
        support::toggle_tree(app, "branch")?;
        support::wait_tree_depth(app, "b-one", Some(1));
    }
    // F7 on the nested match b-one.
    if search_nested(app, "b-o", "b-one", failures)? {
        input::key(app.slint(), Key::F7);
        if !ps::wait(app, Duration::from_secs(10), |app| {
            !app.window.get_operation_token().is_empty()
        }) {
            failures.push("F7 on the nested match b-one opened no dialog".into());
        } else {
            if !ps::wait(app, Duration::from_secs(5), search_closed) {
                failures.push("F7 on a nested match left the quick search open".into());
            }
            if depth(app, "b-one") != Some(1) || ps::cursor(app, LEFT).as_deref() != Some("b-one") {
                failures.push(format!(
                    "F7 closed the search with b-one at depth {:?} and the cursor on {:?}",
                    depth(app, "b-one"),
                    ps::cursor(app, LEFT)
                ));
            }
            let start = app.operations.len();
            input::type_text(app.slint(), "made-in-tree");
            input::key(app.slint(), Key::Return);
            let progress = support::wait_operation(app, start);
            if let Err(error) = support::operation_ok(&progress, "F7 on a nested match") {
                failures.push(error);
            }
            if !root.join("made-in-tree").is_dir() {
                failures.push(format!(
                    "F7 on the nested match created no made-in-tree in the panel's folder: {:?}",
                    support::names_on_disk(root)
                ));
            }
            if depth(app, "a-two") != Some(1) {
                failures.push("creating a folder collapsed the expanded branch".into());
            }
        }
    }
    // Shift+F8 on the nested match c-three deletes it.
    if search_nested(app, "c-t", "c-three", failures)? {
        let start = app.operations.len();
        input::chord(app.slint(), &[Key::Shift], Key::F8);
        if !ps::wait(app, Duration::from_secs(10), |app| {
            !app.window.get_operation_token().is_empty()
        }) {
            failures.push("Shift+F8 on the nested match c-three asked nothing".into());
        } else {
            let body = app.window.get_operation_body().to_string();
            if !body.contains("c-three") {
                failures.push(format!(
                    "Shift+F8 on the nested match c-three asks {body:?}"
                ));
            }
            if !ps::wait(app, Duration::from_secs(5), search_closed) {
                failures.push("Shift+F8 on a nested match left the quick search open".into());
            }
            input::key(app.slint(), Key::Return);
            let progress = support::wait_operation(app, start);
            if let Err(error) = support::operation_ok(&progress, "Shift+F8 on a nested match") {
                failures.push(error);
            }
            let gone = root.join("branch").join("c-three");
            if gone.exists() || !root.join("branch").join("b-one").is_dir() {
                failures.push(format!(
                    "Shift+F8 on the nested c-three left branch/ with {:?}",
                    support::names_on_disk(&root.join("branch"))
                ));
            }
            if !ps::wait(app, Duration::from_secs(10), |app| {
                depth(app, "c-three").is_none() && depth(app, "a-two") == Some(1)
            }) {
                failures.push(format!(
                    "after deleting the nested c-three the tree shows {:?}",
                    ps::shown(app, LEFT)
                ));
            }
        }
    }
    Ok(())
}
