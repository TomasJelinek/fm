//! Shift+F8 permanent delete of every kind of fixture entry one at a time (links remove
//! the link, never their target), multi-selections, folder trees and read-only
//! entries; F8 only opens the Trash confirmation, which is cancelled.
//!
//! The scenario runs in a staged child process (`viewers_stage::run_private`) whose
//! `XDG_DATA_HOME` is a private folder, so the application's Trash (the home Trash,
//! `$XDG_DATA_HOME/Trash`, on the source's device only) is a private folder too: the
//! account's own Trash is neither used nor read.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Timings, tree_facts},
    input, panel, viewers_stage,
};
use midnight_core::{OperationState, PaneId};
use slint::platform::Key;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(20 * 60);

const KINDS: [&str; 6] = ["text", "code", "docs", "images", "archives", "names"];

/// The staged process's private Trash (`$XDG_DATA_HOME/Trash/files`), listed before and
/// after to prove nothing was moved there.
fn trash_entries(data: &Path) -> Vec<String> {
    panel::disk_names(&data.join("Trash/files"))
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub fn delete_kinds(context: &mut Context) -> Result<(), String> {
    viewers_stage::run_private(context, TIMEOUT, run)
}

fn run(context: &mut Context) -> Result<(), String> {
    // Only the staged process has a private data folder, beside its run folder
    // (`<work>/staged/{run/<run>,xdg/data}`); never look at the account's.
    let staged = context.run_dir.parent().and_then(Path::parent);
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|data| staged.is_some_and(|staged| data.starts_with(staged)))
        .ok_or("files_delete runs only in its staged process, with a private XDG_DATA_HOME")?;
    context.note(format!(
        "Private Trash of the staged process: {}",
        data.join("Trash").display()
    ));
    let root = support::scratch(context, "delete")?;
    support::stage(
        context,
        &[
            "text", "code", "docs", "images", "archives", "sizes", "names", "links", "readonly",
            "deep",
        ],
        &root,
    )?;
    let trash_before = trash_entries(&data);
    let mut app = App::start(Config::new(&root, &context.work));
    let pane = PaneId::Left;
    let mut failures = Vec::new();
    let mut timings = Timings::default();

    // F8: the Trash confirmation opens and is cancelled with Escape.
    panel::enter(&mut app, pane, "text")?;
    panel::select_row(&mut app, pane, "plain.txt")?;
    input::key(app.slint(), Key::F8);
    support::wait_operation_dialog(&mut app, "the Trash confirmation");
    let title = app.window.get_operation_title().to_string();
    let body = app.window.get_operation_body().to_string();
    if app.window.get_operation_dangerous() {
        failures.push("F8 opened a dangerous (permanent) confirmation".into());
    }
    if !format!("{title} {body}").contains("Trash") {
        failures.push(format!(
            "the F8 confirmation does not mention the Trash: {title:?} {body:?}"
        ));
    }
    app.screenshot(
        context,
        "trash-confirmation",
        "F8 Trash confirmation (cancelled)",
    );
    input::key(app.slint(), Key::Escape);
    app.until("the Trash confirmation to close", |app| {
        app.window.get_operation_token().is_empty()
    });
    app.idle(std::time::Duration::from_millis(300));
    if !root.join("text/plain.txt").is_file() {
        failures.push("cancelling the Trash confirmation removed plain.txt".into());
    }
    context.note(format!("F8 confirmation: {title:?} — {body:?}"));
    panel::leave(&mut app, pane)?;

    // Links: only the link goes; the folder and file they point to stay.
    let level = tree_facts(&root.join("deep/level-01"))?;
    let plain = support::facts(&root.join("text/plain.txt"))?;
    panel::enter(&mut app, pane, "links")?;
    for name in support::names_on_disk(&root.join("links")) {
        panel::select_row(&mut app, pane, &name)?;
        let progress = support::delete_permanently(&mut app, context, None)?;
        if let Err(error) = support::operation_ok(&progress, &format!("Shift+F8 links/{name}")) {
            failures.push(error);
        }
        support::wait_row(&mut app, pane, &name, false);
        if exists(&root.join("links").join(&name)) {
            failures.push(format!("links/{name} still exists"));
        }
    }
    if tree_facts(&root.join("deep/level-01")).ok().as_ref() != Some(&level) {
        failures.push("deleting links/to-folder changed the folder it points to".into());
    }
    if support::facts(&root.join("text/plain.txt")).ok().as_ref() != Some(&plain) {
        failures.push("deleting links/to-file changed the file it points to".into());
    }
    panel::leave(&mut app, pane)?;

    // Every entry of each kind folder, one at a time.
    let mut deleted = 0;
    for kind in KINDS {
        panel::enter(&mut app, pane, kind)?;
        for name in support::names_on_disk(&root.join(kind)) {
            let path = root.join(kind).join(&name);
            if let Err(error) = panel::select_row(&mut app, pane, &name) {
                failures.push(format!("{kind}/{name}: {error}"));
                continue;
            }
            let shot =
                (kind == "names" && name == "with spaces.txt").then_some("delete-confirmation");
            let started = Instant::now();
            let progress = support::delete_permanently(&mut app, context, shot)?;
            timings.add_for(
                "Shift+F8 confirm to report",
                &format!("{kind}/{name}"),
                started.elapsed(),
            );
            if let Err(error) = support::operation_ok(&progress, &format!("Shift+F8 {kind}/{name}"))
            {
                failures.push(error);
                continue;
            }
            let shown = Instant::now();
            support::wait_row(&mut app, pane, &name, false);
            timings.add_for(
                "report to row removed",
                &format!("{kind}/{name}"),
                shown.elapsed(),
            );
            if exists(&path) {
                failures.push(format!("{kind}/{name} still exists after Shift+F8"));
            }
            deleted += 1;
        }
        let left: Vec<String> = support::names_on_disk(&root.join(kind));
        if !left.is_empty() {
            failures.push(format!("{kind}/ still holds {left:?}"));
        }
        app.screenshot(
            context,
            &format!("deleted-{kind}"),
            &format!("{kind}/ after deleting every entry"),
        );
        panel::leave(&mut app, pane)?;
    }

    // Read-only entries: a 0444 file in a writable folder can be removed. A 0555 folder
    // with contents is the whole deleted tree: owner write is added for it and it is
    // deleted without a question.
    panel::enter(&mut app, pane, "readonly")?;
    panel::select_row(&mut app, pane, "locked.txt")?;
    let progress = support::delete_permanently(&mut app, context, None)?;
    if let Err(error) = support::operation_ok(&progress, "Shift+F8 readonly/locked.txt") {
        failures.push(error);
    } else if exists(&root.join("readonly/locked.txt")) {
        failures.push("readonly/locked.txt still exists".into());
    }
    let locked = tree_facts(&root.join("readonly/locked-folder"))?;
    panel::select_row(&mut app, pane, "locked-folder")?;
    let progress = match support::delete_with(&mut app, true)? {
        (_, support::DeleteEnd::Done(progress)) => progress,
        (start, support::DeleteEnd::Asked(window)) => {
            // Asked instead of deleting the tree: answer Cancel job and record it.
            let question = support::describe_window(&window);
            failures.push(format!(
                "Shift+F8 of a write-protected folder with contents asked instead of deleting \
                 it: {question}"
            ));
            support::cancel_delete(&mut app, &window, start)
                .ok_or_else(|| format!("Cancel job did not end the delete: {question}"))?
        }
    };
    let notice = support::shown_notice(&app);
    support::dismiss_all(&mut app);
    // Delete adds owner write permission to write-protected folders inside the deleted
    // tree, so the whole folder goes without a question.
    if progress.state != OperationState::Completed || exists(&root.join("readonly/locked-folder")) {
        failures.push(format!(
            "Shift+F8 of a write-protected folder with contents ended {:?} ({:?})",
            progress.state,
            progress.errors.first().map(ToString::to_string)
        ));
    }
    let outcome = match progress.state {
        OperationState::Completed if !exists(&root.join("readonly/locked-folder")) => {
            "deleted".to_owned()
        }
        OperationState::Completed => {
            failures.push(
                "Shift+F8 of the read-only folder reported success but the folder is still there"
                    .into(),
            );
            "reported success, folder kept".to_owned()
        }
        state => {
            // Refused: everything must still be in place, and the user is told why.
            if tree_facts(&root.join("readonly/locked-folder"))
                .ok()
                .as_ref()
                != Some(&locked)
            {
                failures.push("a refused delete of the read-only folder changed it".into());
            }
            if progress.errors.is_empty() && notice.is_none() {
                failures.push(format!(
                    "Shift+F8 of the read-only folder ended {state:?} without an error or notice"
                ));
            }
            format!(
                "{state:?}: {:?}; notice {notice:?}",
                progress.errors.first().map(ToString::to_string)
            )
        }
    };
    context.note(format!(
        "Shift+F8 readonly/locked-folder (mode 0555): {outcome}"
    ));
    panel::leave(&mut app, pane)?;

    // A multi-selection (Ctrl+A in sizes/, including the large random file) and a
    // twelve-level folder tree.
    panel::enter(&mut app, pane, "sizes")?;
    panel::focus_list(&mut app, pane)?;
    input::chord(app.slint(), &[Key::Control], "a");
    let wanted = support::names_on_disk(&root.join("sizes"));
    app.until("Ctrl+A to select sizes/", move |app| {
        let mut selected = support::selected_names(app, PaneId::Left);
        selected.sort();
        selected == wanted
    });
    let progress = support::delete_permanently(&mut app, context, Some("delete-multi"))?;
    if let Err(error) = support::operation_ok(&progress, "Shift+F8 of sizes/*") {
        failures.push(error);
    }
    let left = support::names_on_disk(&root.join("sizes"));
    if !left.is_empty() {
        failures.push(format!("the multi-selection delete left {left:?}"));
    }
    app.until("sizes/ to list nothing", |app| {
        app.names(PaneId::Left).iter().all(|name| name == "..")
    });
    panel::leave(&mut app, pane)?;
    panel::select_row(&mut app, pane, "deep")?;
    let progress = support::delete_permanently(&mut app, context, None)?;
    if let Err(error) = support::operation_ok(&progress, "Shift+F8 deep/") {
        failures.push(error);
    }
    if exists(&root.join("deep")) {
        failures.push("deep/ still exists".into());
    }
    support::wait_row(&mut app, pane, "deep", false);

    let trash_after = trash_entries(&data);
    if trash_after != trash_before {
        failures.push(format!(
            "the Trash changed: before {trash_before:?}, after {trash_after:?}"
        ));
    }
    context.note(format!(
        "Shift+F8 removed {deleted} entries one at a time, 4 links (targets intact), sizes/ as a Ctrl+A multi-selection and the 12-level deep/ tree"
    ));
    context.note(timings.summary());
    app.shutdown()?;
    support::report(&failures)
}
