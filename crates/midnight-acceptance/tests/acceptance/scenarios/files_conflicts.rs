//! Copy and Move onto existing names: Replace (original kept in recovery), Skip, Copy as
//! (keep both), Replace all (each original kept in recovery) / Skip all, folder Merge,
//! Cancel job midway, copying within the same folder, the refusal to move a folder into
//! itself, Move with Skip, Replace (original kept in recovery), another name and Merge
//! folders.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Choice, TransferRun, TransferSpec, io_error},
    input, panel,
};
use midnight_core::{CopyOperationState, PaneId};
use slint::platform::Key;
use std::{fs, path::Path};

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
    }
    fs::write(path, text).map_err(|error| io_error(path, error))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("<{error}>"))
}

fn expect_text(failures: &mut Vec<String>, path: &Path, wanted: &str, what: &str) {
    let found = read(path);
    if found != wanted {
        failures.push(format!(
            "{what}: {} holds {found:?}, expected {wanted:?}",
            path.display()
        ));
    }
}

/// Recovery folders the application created in `folder` and the files they keep: its
/// `.midnight-recovery-*` folders and the private `.midnight-move-*` stage in which a
/// Move on Linux keeps a replaced original as recovery data, beside the source
/// (transfers.md).
fn recovered(folder: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(".midnight-recovery-") && !name.starts_with(".midnight-move-") {
            continue;
        }
        let mut stack = vec![entry.path()];
        while let Some(path) = stack.pop() {
            for child in fs::read_dir(&path).into_iter().flatten().flatten() {
                let child_path = child.path();
                if child_path.is_dir() {
                    stack.push(child_path);
                } else {
                    found.push((
                        child.file_name().to_string_lossy().into_owned(),
                        read(&child_path),
                    ));
                }
            }
        }
    }
    found
}

/// The recovery copies that appeared in `folder` since `before` was taken: each kept
/// text that is there more often than before, once per new copy.
fn new_recovery(folder: &Path, before: &[(String, String)]) -> Vec<String> {
    let mut earlier: Vec<&String> = before.iter().map(|(_, text)| text).collect();
    let mut added = Vec::new();
    for (_, text) in recovered(folder) {
        match earlier.iter().position(|seen| **seen == text) {
            Some(index) => {
                earlier.swap_remove(index);
            }
            None => added.push(text),
        }
    }
    added.sort();
    added
}

/// Finding AF-1: a skip the user chose must not be reported as a failed copy.
fn skip_notice(failures: &mut Vec<String>, run: &TransferRun, what: &str) {
    if let Some((title, body)) = &run.notice
        && title.to_lowercase().contains("fail")
    {
        failures.push(format!(
            "[AF-1] {what}: the chosen skip ends {:?} and is reported as {title:?}: {body:?}",
            run.progress.state
        ));
    }
}

/// Mark rows with Insert after putting the cursor on each.
fn mark(app: &mut App, pane: PaneId, names: &[&str]) -> Result<(), String> {
    for name in names {
        panel::select_row(app, pane, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    let mut selected = support::selected_names(app, pane);
    selected.sort();
    let mut wanted: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    wanted.sort();
    if selected != wanted {
        return Err(format!("Insert marked {selected:?} instead of {wanted:?}"));
    }
    Ok(())
}

pub fn conflicts(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "source")?;
    let target = support::scratch(context, "target")?;
    for (name, new, old) in [
        ("one.txt", "one new", "one old"),
        ("two.txt", "two new", "two old"),
        ("three.txt", "three new", "three old"),
    ] {
        write(&source.join(name), new)?;
        write(&target.join(name), old)?;
    }
    write(&source.join("dir/inner.txt"), "inner new")?;
    write(&source.join("dir/extra.txt"), "extra new")?;
    write(&target.join("dir/inner.txt"), "inner old")?;
    write(&target.join("dir/other.txt"), "other old")?;
    write(&source.join("c1.txt"), "c1")?;
    write(&source.join("c2-conflict.txt"), "c2 new")?;
    write(&source.join("c3.txt"), "c3")?;
    write(&target.join("c2-conflict.txt"), "c2 old")?;
    write(&source.join("keep.txt"), "keep")?;
    write(&source.join("nest/sub/deeper.txt"), "deeper")?;
    write(&source.join("mdir/a.txt"), "ma new")?;
    write(&target.join("mdir/b.txt"), "mb old")?;
    let mut app = App::start(Config::new(&source, &target));
    let (left, right) = (PaneId::Left, PaneId::Right);
    let mut failures = Vec::new();
    let mut first_prompt = String::new();

    // Replace one file: the original is kept in a private recovery folder.
    panel::select_row(&mut app, left, "one.txt")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::Replace],
            decision_shot: Some("conflict-decision"),
            ..TransferSpec::copy()
        },
    )?;
    if let Some(prompt) = run.decisions.first() {
        first_prompt = prompt.clone();
    } else {
        failures.push("copying onto one.txt asked nothing".into());
    }
    support::completed(&run, "Replace")?;
    expect_text(&mut failures, &target.join("one.txt"), "one new", "Replace");
    if !recovered(&target).iter().any(|(_, text)| text == "one old") {
        failures.push(format!(
            "Replace kept no recovery copy of the original; recovery holds {:?}",
            recovered(&target)
        ));
    }

    // Skip: the existing file stays.
    panel::select_row(&mut app, left, "two.txt")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::Skip],
            ..TransferSpec::copy()
        },
    )?;
    skip_notice(&mut failures, &run, "Skip");
    if run.decisions.len() != 1 || run.progress.skipped == 0 {
        failures.push(format!(
            "Skip: {} decisions, {} skipped",
            run.decisions.len(),
            run.progress.skipped
        ));
    }
    expect_text(&mut failures, &target.join("two.txt"), "two old", "Skip");

    // Copy as: keep both under a different name.
    panel::select_row(&mut app, left, "three.txt")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::CopyAs("three (copy).txt".into())],
            ..TransferSpec::copy()
        },
    )?;
    support::completed(&run, "Copy as")?;
    expect_text(
        &mut failures,
        &target.join("three.txt"),
        "three old",
        "Copy as",
    );
    expect_text(
        &mut failures,
        &target.join("three (copy).txt"),
        "three new",
        "Copy as",
    );
    support::wait_row(&mut app, right, "three (copy).txt", true);

    // Replace all: one decision for three conflicts.
    for (name, old) in [
        ("one.txt", "one old"),
        ("two.txt", "two old"),
        ("three.txt", "three old"),
    ] {
        write(&target.join(name), old)?;
    }
    mark(&mut app, left, &["one.txt", "two.txt", "three.txt"])?;
    let recovery = recovered(&target);
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::ReplaceAll],
            shot: Some("replace-all-dialog"),
            ..TransferSpec::copy()
        },
    )?;
    support::completed(&run, "Replace all")?;
    if run.decisions.len() != 1 {
        failures.push(format!(
            "Replace all asked {} times for three conflicts",
            run.decisions.len()
        ));
    }
    for (name, new) in [
        ("one.txt", "one new"),
        ("two.txt", "two new"),
        ("three.txt", "three new"),
    ] {
        expect_text(&mut failures, &target.join(name), new, "Replace all");
    }
    // Each replaced original is kept for recovery, once.
    let kept = new_recovery(&target, &recovery);
    if kept != ["one old", "three old", "two old"] {
        failures.push(format!(
            "Replace all kept {kept:?} for recovery, not the three originals"
        ));
    }

    // Skip all: one decision, nothing replaced.
    for (name, old) in [
        ("one.txt", "one old"),
        ("two.txt", "two old"),
        ("three.txt", "three old"),
    ] {
        write(&target.join(name), old)?;
    }
    mark(&mut app, left, &["one.txt", "two.txt", "three.txt"])?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::SkipAll],
            ..TransferSpec::copy()
        },
    )?;
    skip_notice(&mut failures, &run, "Skip all");
    if run.decisions.len() != 1 || run.progress.skipped != 3 {
        failures.push(format!(
            "Skip all: {} decisions, {} skipped of 3",
            run.decisions.len(),
            run.progress.skipped
        ));
    }
    for (name, old) in [
        ("one.txt", "one old"),
        ("two.txt", "two old"),
        ("three.txt", "three old"),
    ] {
        expect_text(&mut failures, &target.join(name), old, "Skip all");
    }
    // Skipped roots stay selected.
    let still = support::selected_names(&app, left);
    if still.len() != 3 {
        failures.push(format!("after Skip all the source selection is {still:?}"));
    }
    panel::focus_list(&mut app, left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // Folder conflict: Merge, then Skip the conflicting child.
    panel::select_row(&mut app, left, "dir")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::Merge, Choice::Skip],
            decision_shot: Some("folder-conflict-decision"),
            ..TransferSpec::copy()
        },
    )?;
    skip_notice(&mut failures, &run, "Merge, then Skip of a child");
    if run.decisions.len() != 2 {
        failures.push(format!(
            "folder merge asked {} times (expected the folder, then inner.txt): {:?}",
            run.decisions.len(),
            run.decisions
        ));
    }
    expect_text(
        &mut failures,
        &target.join("dir/inner.txt"),
        "inner old",
        "Merge + Skip",
    );
    expect_text(
        &mut failures,
        &target.join("dir/extra.txt"),
        "extra new",
        "Merge",
    );
    expect_text(
        &mut failures,
        &target.join("dir/other.txt"),
        "other old",
        "Merge",
    );

    // Cancel job midway: the first file stays copied, the third is never started.
    mark(&mut app, left, &["c1.txt", "c2-conflict.txt", "c3.txt"])?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::CancelJob],
            ..TransferSpec::copy()
        },
    )?;
    if run.progress.state != CopyOperationState::Cancelled {
        failures.push(format!("Cancel job ended {:?}", run.progress.state));
    }
    context.note(format!("Cancel job midway: notice {:?}", run.notice));
    expect_text(&mut failures, &target.join("c1.txt"), "c1", "Cancel job");
    expect_text(
        &mut failures,
        &target.join("c2-conflict.txt"),
        "c2 old",
        "Cancel job",
    );
    if target.join("c3.txt").exists() {
        failures.push("Cancel job still copied c3.txt after the cancelled conflict".into());
    }
    panel::focus_list(&mut app, left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // Copy within the same folder: the destination typed as the source folder.
    panel::select_row(&mut app, left, "keep.txt")?;
    let same = support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(source.as_path()),
            choices: vec![Choice::CopyAs("keep copy.txt".into())],
            ..TransferSpec::copy()
        },
    );
    match same {
        Ok(run) => {
            context.note(format!(
                "copy into its own folder asked: {}",
                run.decisions.join(" | ")
            ));
            support::completed(&run, "Copy into the same folder")?;
            expect_text(
                &mut failures,
                &source.join("keep.txt"),
                "keep",
                "same folder",
            );
            expect_text(
                &mut failures,
                &source.join("keep copy.txt"),
                "keep",
                "same folder",
            );
            support::wait_row(&mut app, left, "keep copy.txt", true);
        }
        Err(error) => failures.push(format!("copy into the same folder: {error}")),
    }
    support::dismiss_all(&mut app);

    // Move a folder into itself and into its own subfolder: refused, nothing moves.
    let nest_before = support::tree_facts(&source.join("nest"))?;
    for destination in [source.join("nest"), source.join("nest/sub")] {
        panel::select_row(&mut app, left, "nest")?;
        let result = support::transfer(
            &mut app,
            context,
            TransferSpec {
                destination: Some(destination.as_path()),
                choices: vec![Choice::CancelJob],
                ..TransferSpec::moving()
            },
        );
        let outcome = match &result {
            Err(error) => format!("refused: {error}"),
            Ok(run) => format!(
                "{:?} after {} decision(s) {}; notice {:?}",
                run.progress.state,
                run.decisions.len(),
                run.decisions.join(" | "),
                run.notice
            ),
        };
        if let Ok(run) = &result
            && run.progress.state == CopyOperationState::Completed
        {
            failures.push(format!(
                "moving nest into {} was not refused: {outcome}",
                destination.display()
            ));
        }
        context.note(format!(
            "F6 nest → {}: {outcome}",
            destination
                .strip_prefix(&source)
                .unwrap_or(&destination)
                .display()
        ));
        support::dismiss_all(&mut app);
        let differences = support::compare_trees(
            &nest_before,
            &support::tree_facts(&source.join("nest"))?,
            true,
        );
        if !differences.is_empty() {
            failures.push(format!(
                "moving nest into itself changed it: {differences:?}"
            ));
        }
    }

    // Move onto an existing name: Skip keeps both, Replace moves and keeps the original.
    write(&target.join("c1.txt"), "c1 old")?;
    panel::select_row(&mut app, left, "c1.txt")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::Skip],
            ..TransferSpec::moving()
        },
    )?;
    if run.decisions.len() != 1 {
        failures.push(format!("Move Skip asked {} times", run.decisions.len()));
    }
    skip_notice(&mut failures, &run, "Move Skip");
    expect_text(&mut failures, &source.join("c1.txt"), "c1", "Move Skip");
    expect_text(&mut failures, &target.join("c1.txt"), "c1 old", "Move Skip");
    panel::select_row(&mut app, left, "c1.txt")?;
    // A Move renames on one volume. Linux exchanges the replaced original into the
    // private stage the moved file left, beside the source; Windows keeps it in a
    // recovery folder beside the destination (transfers.md), so both folders count.
    let recovery = [recovered(&source), recovered(&target)];
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::Replace],
            ..TransferSpec::moving()
        },
    )?;
    support::completed(&run, "Move Replace")?;
    if source.join("c1.txt").exists() {
        failures.push("Move Replace left the source c1.txt".into());
    }
    expect_text(&mut failures, &target.join("c1.txt"), "c1", "Move Replace");
    let kept = [
        new_recovery(&source, &recovery[0]),
        new_recovery(&target, &recovery[1]),
    ];
    // The conflict window promises that Recent Jobs show where the original is kept: the
    // job names that location, and it holds the original.
    let reported = &run.progress.recovery_paths;
    let held: Vec<String> = reported.iter().map(|path| read(path.as_path())).collect();
    context.note(format!(
        "Move Replace kept for recovery: beside the source {:?}, beside the destination {:?}; \
         the job reports {} recovery object(s): {:?}",
        kept[0],
        kept[1],
        run.progress.recovery_count,
        reported
            .iter()
            .map(|path| support::slashed(path.as_path()))
            .collect::<Vec<_>>()
    ));
    if kept.concat() != ["c1 old"] {
        failures.push(format!(
            "Move Replace kept {kept:?} for recovery (source, destination), not the replaced \
             original"
        ));
    }
    if run.progress.recovery_count != 1 || held != ["c1 old"] {
        failures.push(format!(
            "Move Replace reported {} recovery object(s) at {reported:?} holding {held:?}, not \
             the replaced original",
            run.progress.recovery_count
        ));
    }
    support::wait_row(&mut app, left, "c1.txt", false);

    // Move a folder onto an existing one: Merge folders moves its entries in beside the
    // existing ones, and the moved folder is gone from the source.
    panel::select_row(&mut app, left, "mdir")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::Merge],
            ..TransferSpec::moving()
        },
    )?;
    if let Err(error) = support::completed(&run, "Move with Merge folders") {
        failures.push(error);
    }
    if run.decisions.len() != 1 {
        failures.push(format!(
            "Move with Merge folders asked {} times: {:?}",
            run.decisions.len(),
            run.decisions
        ));
    }
    expect_text(
        &mut failures,
        &target.join("mdir/a.txt"),
        "ma new",
        "Move Merge",
    );
    expect_text(
        &mut failures,
        &target.join("mdir/b.txt"),
        "mb old",
        "Move Merge",
    );
    if fs::symlink_metadata(source.join("mdir")).is_ok() {
        failures.push(format!(
            "Move with Merge folders left the source mdir/ holding {:?}",
            support::names_on_disk(&source.join("mdir"))
        ));
    }
    support::wait_row(&mut app, left, "mdir", false);

    // Move onto an existing name under a different name (keep both).
    write(&target.join("c3.txt"), "c3 old")?;
    panel::select_row(&mut app, left, "c3.txt")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            choices: vec![Choice::CopyAs("c3 moved.txt".into())],
            ..TransferSpec::moving()
        },
    )?;
    support::completed(&run, "Move as another name")?;
    if source.join("c3.txt").exists() {
        failures.push("moving c3.txt under another name left the source".into());
    }
    expect_text(
        &mut failures,
        &target.join("c3.txt"),
        "c3 old",
        "Move as another name",
    );
    expect_text(
        &mut failures,
        &target.join("c3 moved.txt"),
        "c3",
        "Move as another name",
    );
    support::wait_row(&mut app, right, "c3 moved.txt", true);
    app.screenshot(
        context,
        "after-conflicts",
        "Both panels after the conflict decisions",
    );
    context.note(format!("first name-conflict window: {first_prompt}"));
    context.note(format!(
        "recovery folder of the target keeps: {:?}",
        recovered(&target)
    ));
    app.shutdown()?;
    support::report(&failures)
}
