//! Symbolic links under F5 and F6: a link to a folder, to a file, a dangling link and a
//! link to its own folder (a cycle).
//!
//! F5 with the link option cleared recreates every link as a link with exactly the same
//! target text. With it checked (as offered), the folder link is followed, the file link
//! is copied as the file's content, and a link that cannot be followed (the dangling one,
//! and the self link met again inside the followed folder) is recreated as a link with a
//! note in the job details. A folder holding only a link to itself copies once, keeping
//! the link. F6 moves each link itself. Sources and link targets never change.
//!
//! Windows copies do not recreate links (docs/developer/TRANSFERS.md) and file symbolic
//! links need a privilege or Developer Mode, so there the scenario uses directory
//! junctions, which need neither, against the Windows contract of the link option
//! ("Follow directory links"): cleared, each junction arrives as an empty folder;
//! checked, a junction to a folder arrives as that folder's tree; F6 on one volume moves
//! the junction itself. Following a dangling junction or one to its own folder is
//! recorded, not judged (the source refuses unqualified targets and cycles), except that
//! no copy may create a link.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, Choice, Content, TransferRun, TransferSpec, tree_facts},
    panel,
};
use midnight_core::PaneId;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
const LINKS: [&str; 4] = ["dangling", "to-file", "to-folder", "to-self"];

/// What arrived at `path`, in words.
fn describe(path: &Path) -> String {
    match support::facts(path) {
        Err(_) => "nothing".into(),
        Ok(facts) => match facts.content {
            Content::Link(target) => format!("a link to {}", target.display()),
            Content::Folder => format!(
                "a folder with {} entries",
                tree_facts(path).map_or(0, |tree| tree.len())
            ),
            Content::File { len, .. } => format!("a {len}-byte file"),
            Content::Other => "a special entry".into(),
        },
    }
}

fn outcome(run: &Result<TransferRun, String>) -> String {
    match run {
        Err(error) => format!("refused: {error}"),
        Ok(run) => format!(
            "{:?}{}{}{}",
            run.progress.state,
            run.decisions
                .first()
                .map(|text| format!(", asked: {text}"))
                .unwrap_or_default(),
            run.notice
                .as_ref()
                .map(|(title, body)| format!(", notice {title:?}: {body:?}"))
                .unwrap_or_default(),
            if run.progress.notes.is_empty() {
                String::new()
            } else {
                format!(", notes: {:?}", run.progress.notes)
            }
        ),
    }
}

/// The run completed without a question, a notice, a skip or a failure.
fn clean(run: &Result<TransferRun, String>, what: &str, failures: &mut Vec<String>) -> bool {
    match run {
        Err(error) => failures.push(format!("[AF-3] {what}: {error}")),
        Ok(run) => {
            if let Err(error) = support::completed(run, what) {
                failures.push(format!("[AF-3] {error}"));
            } else if !run.decisions.is_empty() || run.notice.is_some() {
                failures.push(format!(
                    "[AF-3] {what} asked {:?} or noticed {:?}",
                    run.decisions, run.notice
                ));
            } else {
                return true;
            }
        }
    }
    false
}

fn is_link_to(path: &Path, target: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && fs::read_link(path).ok().as_deref() == Some(target)
}

/// Whether `copy` holds the same tree (content, names and kinds) as `original`.
fn same_tree(original: &Path, copy: &Path) -> bool {
    let shape = |root: &Path| {
        tree_facts(root).map(|tree| {
            tree.into_iter()
                .map(|(path, facts)| (path, facts.content))
                .collect::<Vec<_>>()
        })
    };
    matches!((shape(original), shape(copy)), (Ok(left), Ok(right)) if left == right)
}

#[cfg(unix)]
fn has_note(run: &Result<TransferRun, String>, name: &str) -> bool {
    run.as_ref().is_ok_and(|run| {
        run.progress
            .notes
            .iter()
            .any(|note| note.contains(name) && note.contains("copied as a link"))
    })
}

pub fn links(context: &mut Context) -> Result<(), String> {
    #[cfg(unix)]
    return symbolic(context);
    #[cfg(windows)]
    return junctions(context);
}

#[cfg(unix)]
fn symbolic(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "source")?;
    support::stage(context, &["links", "deep", "text"], &source)?;
    let before = tree_facts(&source)?;
    let mut outcomes = Vec::new();
    let mut failures = Vec::new();
    let targets: Vec<PathBuf> = LINKS
        .iter()
        .map(|name| fs::read_link(source.join("links").join(name)))
        .collect::<Result<_, _>>()
        .map_err(|error| format!("links: {error}"))?;

    // Link option cleared: every link is recreated as a link with the same target.
    let target = support::scratch(context, "copy follow cleared")?;
    let mut app = App::start(Config::new(source.join("links"), &target));
    for (name, link) in LINKS.iter().zip(&targets) {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                follow_links: Some(false),
                choices: vec![Choice::CancelJob],
                ..TransferSpec::copy()
            },
        );
        let copied = target.join(name);
        outcomes.push(format!(
            "F5 {name} (follow cleared): {}; {}",
            describe(&copied),
            outcome(&run)
        ));
        if clean(
            &run,
            &format!("F5 of links/{name} (follow cleared)"),
            &mut failures,
        ) && !is_link_to(&copied, link)
        {
            failures.push(format!(
                "[AF-3] F5 of links/{name} (follow cleared) gave {}, not a link to {}",
                describe(&copied),
                link.display()
            ));
        }
        support::dismiss_all(&mut app);
    }
    app.screenshot(
        context,
        "links-copied-no-follow",
        "Target after copying the four links as links",
    );
    app.shutdown()?;

    // Link option checked (as offered): links are followed where they can be.
    let target = support::scratch(context, "copy follow checked")?;
    let mut app = App::start(Config::new(source.join("links"), &target));
    let mut runs = Vec::new();
    for name in LINKS {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                follow_links: Some(true),
                choices: vec![Choice::CancelJob],
                shot: (name == "to-folder").then_some("copy-link-dialog"),
                ..TransferSpec::copy()
            },
        );
        outcomes.push(format!(
            "F5 {name} (follow checked): {}; {}",
            describe(&target.join(name)),
            outcome(&run)
        ));
        clean(
            &run,
            &format!("F5 of links/{name} (follow checked)"),
            &mut failures,
        );
        runs.push(run);
        support::dismiss_all(&mut app);
    }
    let followed = [
        (
            "to-folder",
            same_tree(&source.join("deep/level-01"), &target.join("to-folder")),
            "the linked folder's tree",
        ),
        (
            "to-file",
            fs::symlink_metadata(target.join("to-file")).is_ok_and(|metadata| metadata.is_file())
                && fs::read(target.join("to-file")).ok()
                    == fs::read(source.join("text/plain.txt")).ok(),
            "the linked file's content",
        ),
        (
            "dangling",
            is_link_to(&target.join("dangling"), &targets[0]) && has_note(&runs[0], "dangling"),
            "the same link, with a note",
        ),
        (
            "to-self",
            fs::symlink_metadata(target.join("to-self")).is_ok_and(|metadata| metadata.is_dir())
                && is_link_to(&target.join("to-self/to-self"), Path::new("."))
                && is_link_to(&target.join("to-self/dangling"), &targets[0])
                && same_tree(
                    &source.join("deep/level-01"),
                    &target.join("to-self/to-folder"),
                )
                && has_note(&runs[3], "to-self"),
            "the folder copied once, its own self link kept as a link, with a note",
        ),
    ];
    for (name, ok, expected) in followed {
        if !ok {
            failures.push(format!(
                "[AF-3] F5 of links/{name} (follow checked) gave {}, not {expected}",
                describe(&target.join(name))
            ));
        }
    }
    app.screenshot(
        context,
        "links-copied-follow",
        "Target after copying the four links with Follow links",
    );
    app.shutdown()?;
    // The sources, including what the links point to, are unchanged.
    let differences = support::compare_trees(&before, &tree_facts(&source)?, true);
    if !differences.is_empty() {
        failures.push(format!(
            "copying the links changed the sources: {differences:?}"
        ));
    }

    // A folder whose only entry links to the folder itself: following it copies the
    // folder once and keeps the link as a link.
    let cycle = support::scratch(context, "cycle-source")?.join("cycle-only");
    fs::create_dir(&cycle).map_err(|error| support::io_error(&cycle, error))?;
    std::os::unix::fs::symlink(".", cycle.join("loop"))
        .map_err(|error| support::io_error(&cycle, error))?;
    let target = support::scratch(context, "cycle-copy")?;
    let mut app = App::start(Config::new(cycle.parent().unwrap_or(&cycle), &target));
    panel::select_row(&mut app, PaneId::Left, "cycle-only")?;
    let started = std::time::Instant::now();
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            follow_links: Some(true),
            choices: vec![Choice::CancelJob],
            ..TransferSpec::copy()
        },
    );
    let copied = target.join("cycle-only");
    outcomes.push(format!(
        "F5 of a folder holding only a self link (follow checked): {} ms, {}; {}",
        started.elapsed().as_millis(),
        describe(&copied),
        outcome(&run)
    ));
    if clean(&run, "F5 of cycle-only (follow checked)", &mut failures)
        && (tree_facts(&copied).map(|tree| tree.len()).ok() != Some(1)
            || !is_link_to(&copied.join("loop"), Path::new("."))
            || !has_note(&run, "loop"))
    {
        failures.push(format!(
            "[AF-3] following a self link gave {} instead of the folder with its link kept",
            describe(&copied)
        ));
    }
    if fs::read_link(cycle.join("loop")).ok().as_deref() != Some(Path::new(".")) {
        failures.push("copying the cycle changed its source link".into());
    }
    support::dismiss_all(&mut app);
    app.screenshot(
        context,
        "cycle-copied",
        "Target after copying a self-linked folder",
    );
    app.shutdown()?;

    // F6 moves the link itself.
    let target = support::scratch(context, "moved")?;
    let mut app = App::start(Config::new(source.join("links"), &target));
    for (name, link) in LINKS.iter().zip(&targets) {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                choices: vec![Choice::CancelJob],
                ..TransferSpec::moving()
            },
        );
        let moved = target.join(name);
        outcomes.push(format!(
            "F6 {name}: {}; {}",
            describe(&moved),
            outcome(&run)
        ));
        let still_there = fs::symlink_metadata(source.join("links").join(name)).is_ok();
        if clean(&run, &format!("F6 of links/{name}"), &mut failures) {
            if !is_link_to(&moved, link) || still_there {
                failures.push(format!(
                    "[AF-3] F6 of links/{name} gave {}, not the moved link (source still there: {still_there})",
                    describe(&moved)
                ));
            } else {
                support::wait_row(&mut app, PaneId::Right, name, true);
            }
        }
        support::dismiss_all(&mut app);
    }
    if tree_facts(&source.join("deep")).ok().as_ref()
        != Some(
            &before
                .iter()
                .filter_map(|(path, facts)| {
                    path.strip_prefix("deep/")
                        .map(|relative| (relative.to_owned(), facts.clone()))
                })
                .collect(),
        )
    {
        failures.push("moving links/to-folder changed the folder it points to".into());
    }
    app.screenshot(context, "links-moved", "Target after moving the four links");
    app.shutdown()?;
    for outcome in &outcomes {
        context.note(outcome.clone());
    }
    support::report(&failures)
}

/// The Windows variant: directory junctions (see the module documentation).
#[cfg(windows)]
fn junctions(context: &mut Context) -> Result<(), String> {
    use crate::fixtures::junction;
    let source = support::scratch(context, "source")?;
    support::stage(context, &["deep", "text"], &source)?;
    let links = source.join("links");
    fs::create_dir(&links).map_err(|error| support::io_error(&links, error))?;
    let junctions: [(&str, PathBuf); 3] = [
        ("dangling", source.join("does-not-exist")),
        ("to-folder", source.join("deep").join("level-01")),
        ("to-self", links.clone()),
    ];
    for (name, target) in &junctions {
        junction(&links.join(name), target)?;
    }
    context.note(
        "file links not covered: creating a file symbolic link needs administrator rights or \
         Developer Mode, and Windows copies never recreate links",
    );
    let before = tree_facts(&source)?;
    let mut outcomes = Vec::new();
    let mut failures = Vec::new();
    let is_empty_folder = |path: &Path| {
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
            && fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
    };

    // "Follow directory links" cleared: every junction arrives as an empty folder.
    let target = support::scratch(context, "copy follow cleared")?;
    let mut app = App::start(Config::new(&links, &target));
    for (name, _) in &junctions {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                follow_links: Some(false),
                choices: vec![Choice::CancelJob],
                shot: (*name == "to-folder").then_some("copy-junction-dialog"),
                ..TransferSpec::copy()
            },
        );
        let copied = target.join(name);
        outcomes.push(format!(
            "F5 {name} (follow cleared): {}; {}",
            describe(&copied),
            outcome(&run)
        ));
        if clean(
            &run,
            &format!("F5 of the junction {name} (follow cleared)"),
            &mut failures,
        ) && !is_empty_folder(&copied)
        {
            failures.push(format!(
                "[AF-3] F5 of the junction {name} (follow cleared) gave {}, not an empty folder",
                describe(&copied)
            ));
        }
        support::dismiss_all(&mut app);
    }
    app.screenshot(
        context,
        "junctions-copied-no-follow",
        "Target after copying three junctions without following them",
    );
    app.shutdown()?;

    // Checked: the junction to a folder arrives as the folder's tree; the dangling one
    // and the one to its own folder are recorded. None arrives as a link.
    let target = support::scratch(context, "copy follow checked")?;
    let mut app = App::start(Config::new(&links, &target));
    for (name, _) in &junctions {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                follow_links: Some(true),
                choices: vec![Choice::CancelJob],
                ..TransferSpec::copy()
            },
        );
        let copied = target.join(name);
        outcomes.push(format!(
            "F5 {name} (follow checked): {}; {}",
            describe(&copied),
            outcome(&run)
        ));
        if *name == "to-folder" {
            if clean(
                &run,
                "F5 of the junction to-folder (follow checked)",
                &mut failures,
            ) && !same_tree(&source.join("deep").join("level-01"), &copied)
            {
                failures.push(format!(
                    "[AF-3] F5 of the junction to-folder (follow checked) gave {}, not the \
                     linked folder's tree",
                    describe(&copied)
                ));
            }
        } else if fs::symlink_metadata(&copied).is_ok_and(|metadata| metadata.is_symlink()) {
            failures.push(format!(
                "F5 of the junction {name} (follow checked) created a link at the destination"
            ));
        }
        support::dismiss_all(&mut app);
    }
    app.screenshot(
        context,
        "junctions-copied-follow",
        "Target after copying three junctions with Follow directory links",
    );
    app.shutdown()?;
    let differences = support::compare_trees(&before, &tree_facts(&source)?, true);
    if !differences.is_empty() {
        failures.push(format!(
            "copying the junctions changed the sources: {differences:?}"
        ));
    }

    // F6 on one volume moves the junction to a folder itself; the others are recorded.
    let target = support::scratch(context, "moved")?;
    let mut app = App::start(Config::new(&links, &target));
    for (name, link) in &junctions {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                choices: vec![Choice::CancelJob],
                ..TransferSpec::moving()
            },
        );
        let moved = target.join(name);
        outcomes.push(format!(
            "F6 {name}: {}; {}",
            describe(&moved),
            outcome(&run)
        ));
        let still_there = fs::symlink_metadata(links.join(name)).is_ok();
        if *name == "to-folder" {
            if clean(&run, "F6 of the junction to-folder", &mut failures) {
                if !is_link_to(&moved, link) || still_there {
                    failures.push(format!(
                        "[AF-3] F6 of the junction to-folder gave {}, not the moved junction \
                         (source still there: {still_there})",
                        describe(&moved)
                    ));
                } else {
                    support::wait_row(&mut app, PaneId::Right, name, true);
                }
            }
        } else if !still_there && !is_link_to(&moved, link) {
            failures.push(format!(
                "F6 of the junction {name} removed it but left {}",
                describe(&moved)
            ));
        }
        support::dismiss_all(&mut app);
    }
    let level = Path::new("deep").join("level-01");
    let level_before: std::collections::BTreeMap<String, support::Facts> = before
        .iter()
        .filter_map(|(path, facts)| {
            Path::new(path)
                .strip_prefix(&level)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .map(|relative| (relative.to_string_lossy().into_owned(), facts.clone()))
        })
        .collect();
    if tree_facts(&source.join(&level)).ok().as_ref() != Some(&level_before) {
        failures.push("moving the junction to-folder changed the folder it points to".into());
    }
    app.screenshot(
        context,
        "junctions-moved",
        "Target after moving the junctions",
    );
    app.shutdown()?;
    for outcome in &outcomes {
        context.note(outcome.clone());
    }
    support::report(&failures)
}
