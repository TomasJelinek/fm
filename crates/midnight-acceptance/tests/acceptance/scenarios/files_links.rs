//! Links under F5 and F6 through the dialog's Links choice: a link to a folder, to a
//! file, a dangling link and a link to its own folder (a cycle).
//!
//! On a fixed local folder the dialog proposes "Keep links as links": every link is
//! recreated as a link with exactly the same target text, and the job details name each
//! link whose relative target leads outside the copied items. "Copy the files links point
//! to" follows the folder link (its tree arrives), copies the file link as the file's
//! content and keeps a link that cannot be followed (the dangling one, and the self link
//! met again inside the followed folder) as a link with a note in the job details. A
//! folder holding only a link to itself copies once, keeping the link. F6 keeping links
//! moves each link itself. Sources and link targets never change. The choice is made by
//! keyboard (Tab to Links, Up or Down, Enter), by pointer (a click on the entry) and with
//! the list's own keys (Enter in the open list chooses, it does not confirm).
//!
//! Windows uses directory junctions, which need no privilege (file symbolic links need
//! Developer Mode or administrator rights: `files_links_volumes`): kept as links, all
//! three are recreated as junctions with the same target; copying the files, the junction
//! to a folder arrives as that folder's tree, the dangling junction stays a junction with
//! a note, and the one to its own folder is followed once, as on Linux (the junctions met
//! again inside that copy stay junctions, each with a note); F6 moves all three.
use crate::{
    Context,
    app::{App, Config},
    files_support::{
        self as support, Choice, Content, LinkChoice, LinkInput, TransferRun, TransferSpec,
        tree_facts,
    },
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
pub fn describe(path: &Path) -> String {
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

pub fn outcome(run: &Result<TransferRun, String>) -> String {
    match run {
        Err(error) => format!("refused: {error}"),
        Ok(run) => format!(
            "{:?}{}{}{}{}{}",
            run.progress.state,
            run.links
                .as_ref()
                .map(|links| format!(" with {:?}", links.choice))
                .unwrap_or_default(),
            run.hold
                .as_ref()
                .map(|hold| format!(
                    ", held first ({:?}, shown {}, Links then {:?})",
                    hold.text,
                    hold.shown,
                    hold.links.as_ref().map(|links| &links.choice)
                ))
                .unwrap_or_default(),
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

/// The run completed without a question, a notice, a skip or a failure, with the Links
/// choice `links` shown when it was confirmed.
fn clean(
    run: &Result<TransferRun, String>,
    what: &str,
    links: LinkChoice,
    failures: &mut Vec<String>,
) -> bool {
    match run {
        Err(error) => failures.push(format!("[AF-3] {what}: {error}")),
        Ok(run) => {
            let shown = run.links.as_ref().and_then(support::LinkView::chosen);
            if let Err(error) = support::completed(run, what) {
                failures.push(format!("[AF-3] {error}"));
            } else if !run.decisions.is_empty() || run.notice.is_some() {
                failures.push(format!(
                    "[AF-3] {what} asked {:?} or noticed {:?}",
                    run.decisions, run.notice
                ));
            } else if shown != Some(links) {
                failures.push(format!(
                    "{what} was confirmed with Links {:?}, not {:?}",
                    run.links,
                    links.label()
                ));
            } else {
                return true;
            }
        }
    }
    false
}

pub fn is_link_to(path: &Path, target: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && fs::read_link(path).ok().as_deref() == Some(target)
}

/// Whether `copy` holds the same tree (content, names and kinds) as `original`.
pub fn same_tree(original: &Path, copy: &Path) -> bool {
    let shape = |root: &Path| {
        tree_facts(root).map(|tree| {
            tree.into_iter()
                .map(|(path, facts)| (path, facts.content))
                .collect::<Vec<_>>()
        })
    };
    matches!((shape(original), shape(copy)), (Ok(left), Ok(right)) if left == right)
}

/// The job's notes that name `name` and say `words`.
pub fn notes_about(run: &Result<TransferRun, String>, name: &str, words: &str) -> usize {
    run.as_ref().map_or(0, |run| {
        run.progress
            .notes
            .iter()
            .filter(|note| note.contains(name) && note.contains(words))
            .count()
    })
}

/// A note that the link `name` was kept as a link because it could not be followed.
fn kept_note(run: &Result<TransferRun, String>, name: &str) -> bool {
    notes_about(run, name, "copied as a link because") == 1
}

pub fn links(context: &mut Context) -> Result<(), String> {
    #[cfg(unix)]
    return symbolic(context);
    #[cfg(windows)]
    return junctions(context);
}

/// The note of a kept link whose relative target `text` climbs out of what was copied.
#[cfg(unix)]
const LEADS_OUTSIDE: &str = "which leads outside the copied items";

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

    // Keep links as links: the default on a fixed local folder (to-self takes it as
    // offered), chosen by keyboard for the others. Every link arrives as a link with the
    // same target; each relative target that climbs out of the copied link is noted.
    let target = support::scratch(context, "copy keep")?;
    let mut app = App::start(Config::new(source.join("links"), &target));
    for (name, link) in LINKS.iter().zip(&targets) {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let offered = *name == "to-self";
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                links: (!offered).then_some(LinkChoice::Keep),
                link_input: LinkInput::Keyboard,
                choices: vec![Choice::CancelJob],
                shot: (*name == "to-folder").then_some("copy-link-dialog-keep"),
                ..TransferSpec::copy()
            },
        );
        let copied = target.join(name);
        outcomes.push(format!(
            "F5 {name} (keep links{}): {}; {}",
            if offered {
                ", as offered"
            } else {
                ", by keyboard"
            },
            describe(&copied),
            outcome(&run)
        ));
        let what = format!("F5 of links/{name} keeping links");
        if clean(&run, &what, LinkChoice::Keep, &mut failures) {
            if !is_link_to(&copied, link) {
                failures.push(format!(
                    "[AF-3] {what} gave {}, not a link to {}",
                    describe(&copied),
                    link.display()
                ));
            }
            // "." stays inside; the others climb out of the selected link's folder.
            let noted = notes_about(&run, name, LEADS_OUTSIDE);
            let wanted = usize::from(*name != "to-self");
            if noted != wanted {
                failures.push(format!(
                    "{what}: {noted} note(s) that its relative target leads outside the \
                     copied items, not {wanted}: {:?}",
                    run.as_ref().map(|run| &run.progress.notes)
                ));
            }
        }
        support::dismiss_all(&mut app);
    }
    app.screenshot(
        context,
        "links-copied-keep",
        "Target after copying the four links as links",
    );
    app.shutdown()?;

    // Copy the files links point to, chosen by pointer: links are followed where they
    // can be.
    let target = support::scratch(context, "copy files")?;
    let mut app = App::start(Config::new(source.join("links"), &target));
    let mut runs = Vec::new();
    for name in LINKS {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                links: Some(LinkChoice::CopyFiles),
                link_input: LinkInput::Pointer,
                choices: vec![Choice::CancelJob],
                shot: (name == "to-folder").then_some("copy-link-dialog"),
                ..TransferSpec::copy()
            },
        );
        outcomes.push(format!(
            "F5 {name} (copy the files): {}; {}",
            describe(&target.join(name)),
            outcome(&run)
        ));
        clean(
            &run,
            &format!("F5 of links/{name} copying the files"),
            LinkChoice::CopyFiles,
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
            is_link_to(&target.join("dangling"), &targets[0])
                && kept_note(&runs[0], "dangling")
                && notes_about(&runs[0], "dangling", "its target does not exist") == 1,
            "the same link, with a note that its target does not exist",
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
                // Inside the followed folder: the self link and the dangling link stay
                // links, each with its reason.
                && notes_about(&runs[3], "to-self/to-self", "contains it") == 1
                && notes_about(&runs[3], "to-self/dangling", "its target does not exist") == 1,
            "the folder copied once, its own self link kept as a link, with a note",
        ),
    ];
    for (name, ok, expected) in followed {
        if !ok {
            failures.push(format!(
                "[AF-3] F5 of links/{name} copying the files gave {}, not {expected}",
                describe(&target.join(name))
            ));
        }
    }
    app.screenshot(
        context,
        "links-copied-follow",
        "Target after copying the four links with Copy the files links point to",
    );
    app.shutdown()?;
    // The sources, including what the links point to, are unchanged.
    let differences = support::compare_trees(&before, &tree_facts(&source)?, true);
    if !differences.is_empty() {
        failures.push(format!(
            "copying the links changed the sources: {differences:?}"
        ));
    }

    // A folder whose only entry links to the folder itself: copying the files copies the
    // folder once and keeps the link as a link (chosen with the open list's keys).
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
            links: Some(LinkChoice::CopyFiles),
            link_input: LinkInput::ListKeys,
            choices: vec![Choice::CancelJob],
            ..TransferSpec::copy()
        },
    );
    let copied = target.join("cycle-only");
    outcomes.push(format!(
        "F5 of a folder holding only a self link (copy the files): {} ms, {}; {}",
        started.elapsed().as_millis(),
        describe(&copied),
        outcome(&run)
    ));
    if clean(
        &run,
        "F5 of cycle-only copying the files",
        LinkChoice::CopyFiles,
        &mut failures,
    ) && (tree_facts(&copied).map(|tree| tree.len()).ok() != Some(1)
        || !is_link_to(&copied.join("loop"), Path::new("."))
        || !kept_note(&run, "loop"))
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

    // F6 with the offered Links choice (keep links) moves the link itself.
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
        if clean(
            &run,
            &format!("F6 of links/{name}"),
            LinkChoice::Keep,
            &mut failures,
        ) {
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
    move_projects(context, &mut failures, &mut outcomes)?;
    for outcome in &outcomes {
        context.note(outcome.clone());
    }
    support::report(&failures)
}

/// A project folder `proj` beside the `data` its links point to, in `base`: a folder link
/// (`dir-link` to `data/dir`), a dangling link (`dangling` to `data/missing`), on Unix a
/// file link (`file-link` to `data/target.txt`), and `plain.txt`. Unix links are relative
/// (`../data/…`); Windows ones are junctions, which need no privilege, so there is no
/// file link. Returns each link's name and target as it reads back.
pub fn project(base: &Path) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let write = |path: &Path, text: &str| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| support::io_error(parent, error))?;
        }
        fs::write(path, text).map_err(|error| support::io_error(path, error))
    };
    write(&base.join("data/target.txt"), "the linked file\n")?;
    write(&base.join("data/dir/a.txt"), "a\n")?;
    write(&base.join("data/dir/sub/b.txt"), "bb\n")?;
    write(&base.join("proj/plain.txt"), "plain\n")?;
    let proj = base.join("proj");
    #[cfg(unix)]
    let links = {
        let links = [
            ("dir-link", PathBuf::from("../data/dir")),
            ("dangling", PathBuf::from("../data/missing")),
            ("file-link", PathBuf::from("../data/target.txt")),
        ];
        for (name, target) in &links {
            std::os::unix::fs::symlink(target, proj.join(name))
                .map_err(|error| support::io_error(&proj.join(name), error))?;
        }
        links
    };
    #[cfg(windows)]
    let links = {
        let links = [
            ("dir-link", base.join("data").join("dir")),
            ("dangling", base.join("data").join("missing")),
        ];
        for (name, target) in &links {
            crate::fixtures::junction(&proj.join(name), target)?;
        }
        links
    };
    Ok(links.into_iter().collect())
}

/// The identities of every entry below `root` (links not followed), by relative path.
pub fn identities(root: &Path) -> Result<std::collections::BTreeMap<String, (u64, u64)>, String> {
    tree_facts(root)?
        .into_keys()
        .map(|path| {
            let identity = support::identity(&root.join(&path))?;
            Ok((path, identity))
        })
        .collect()
}

/// Moves of a project holding links (`project`), on one volume:
/// - "Copy the files links point to": the folder and the file a link points to arrive as
///   copies, the dangling link moves as a link (noted), `proj` is gone, and what the
///   links point to keeps its bytes and its identity; `plain.txt` is renamed (same
///   identity), as the folder is not renamed whole.
/// - "Keep links as links" (as offered): the folder is renamed whole, so it and every
///   entry in it keep their identity, the links their targets.
fn move_projects(
    context: &mut Context,
    failures: &mut Vec<String>,
    outcomes: &mut Vec<String>,
) -> Result<(), String> {
    for (choice, label) in [
        (LinkChoice::CopyFiles, "copy the files"),
        (LinkChoice::Keep, "keep links"),
    ] {
        let base = support::scratch(
            context,
            match choice {
                LinkChoice::CopyFiles => "move-copy-files",
                LinkChoice::Keep => "move-keep",
            },
        )?;
        let links = project(&base)?;
        let destination = base.join("destination");
        fs::create_dir(&destination).map_err(|error| support::io_error(&destination, error))?;
        let data_before = tree_facts(&base.join("data"))?;
        let data_identity = identities(&base.join("data"))?;
        let proj_identity = identities(&base.join("proj"))?;
        let folder_identity = support::identity(&base.join("proj"))?;
        let mut app = App::start(Config::new(&base, &destination));
        panel::select_row(&mut app, PaneId::Left, "proj")?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                links: (choice == LinkChoice::CopyFiles).then_some(choice),
                link_input: LinkInput::Keyboard,
                choices: vec![Choice::CancelJob],
                ..TransferSpec::moving()
            },
        );
        let moved = destination.join("proj");
        outcomes.push(format!(
            "F6 of proj with links ({label}): {}; {}",
            describe(&moved),
            outcome(&run)
        ));
        let what = format!("F6 of a project holding links ({label})");
        if !clean(&run, &what, choice, failures) {
            support::dismiss_all(&mut app);
            app.shutdown()?;
            continue;
        }
        let mut problems = Vec::new();
        if fs::symlink_metadata(base.join("proj")).is_ok() {
            problems.push(format!(
                "proj is still in the source, holding {:?}",
                support::names_on_disk(&base.join("proj"))
            ));
        }
        for (name, target) in &links {
            let arrived = moved.join(name);
            let followed = choice == LinkChoice::CopyFiles && *name != "dangling";
            let ok = match (*name, followed) {
                ("dir-link", true) => {
                    same_tree(&base.join("data/dir"), &arrived)
                        && !fs::symlink_metadata(&arrived).is_ok_and(|meta| meta.is_symlink())
                }
                ("file-link", true) => {
                    fs::symlink_metadata(&arrived).is_ok_and(|meta| meta.is_file())
                        && fs::read(&arrived).ok() == fs::read(base.join("data/target.txt")).ok()
                }
                _ => is_link_to(&arrived, target),
            };
            if !ok {
                problems.push(format!(
                    "{name} arrived as {}, not {}",
                    describe(&arrived),
                    if followed {
                        "a copy of what it points to"
                    } else {
                        "the same link"
                    }
                ));
            }
        }
        if choice == LinkChoice::CopyFiles && notes_about(&run, "dangling", "moved as a link") != 1
        {
            problems.push(format!(
                "no note says the dangling link was moved as a link: {:?}",
                run.as_ref().map(|run| &run.progress.notes)
            ));
        }
        // Renamed entries keep their identity; copies of what links point to are new.
        let after = identities(&moved).unwrap_or_default();
        let changed: Vec<String> = proj_identity
            .iter()
            .filter(|(path, identity)| {
                let replaced = choice == LinkChoice::CopyFiles
                    && links
                        .iter()
                        .any(|(name, _)| path.as_str() == *name && *name != "dangling");
                !replaced && after.get(path.as_str()) != Some(*identity)
            })
            .map(|(path, _)| path.clone())
            .collect();
        if !changed.is_empty() {
            problems.push(format!(
                "these entries were not renamed (their identity changed): {changed:?}"
            ));
        }
        let renamed_whole = support::identity(&moved).ok() == Some(folder_identity);
        if choice == LinkChoice::Keep && !renamed_whole {
            problems.push("the folder proj was not renamed whole".into());
        }
        if choice == LinkChoice::CopyFiles && renamed_whole {
            problems.push(
                "the folder proj was renamed whole, so its links could not be followed".into(),
            );
        }
        // What the links point to: the same files with the same bytes.
        if tree_facts(&base.join("data")).ok().as_ref() != Some(&data_before)
            || identities(&base.join("data")).ok().as_ref() != Some(&data_identity)
        {
            problems.push("what the links point to changed (bytes or identity)".into());
        }
        if !problems.is_empty() {
            failures.push(format!("[AF-3] {what}: {}", problems.join("; ")));
        }
        app.screenshot(
            context,
            match choice {
                LinkChoice::CopyFiles => "project-moved-copy-files",
                LinkChoice::Keep => "project-moved-keep",
            },
            &format!("A project holding links moved ({label})"),
        );
        app.shutdown()?;
    }
    Ok(())
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
    let before = tree_facts(&source)?;
    let mut outcomes = Vec::new();
    let mut failures = Vec::new();

    // Keep links as links (the default on a fixed local folder; to-self takes it as
    // offered, the others choose it by keyboard): each junction is recreated as a
    // junction to the same folder, without a note.
    let target = support::scratch(context, "copy keep")?;
    let mut app = App::start(Config::new(&links, &target));
    for (name, link) in &junctions {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let offered = *name == "to-self";
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                links: (!offered).then_some(LinkChoice::Keep),
                link_input: LinkInput::Keyboard,
                choices: vec![Choice::CancelJob],
                shot: (*name == "to-folder").then_some("copy-junction-dialog"),
                ..TransferSpec::copy()
            },
        );
        let copied = target.join(name);
        outcomes.push(format!(
            "F5 {name} (keep links{}): {}; {}",
            if offered {
                ", as offered"
            } else {
                ", by keyboard"
            },
            describe(&copied),
            outcome(&run)
        ));
        let what = format!("F5 of the junction {name} keeping links");
        if clean(&run, &what, LinkChoice::Keep, &mut failures) {
            if !is_link_to(&copied, link) {
                failures.push(format!(
                    "[AF-3] {what} gave {}, not a junction to {}",
                    describe(&copied),
                    link.display()
                ));
            }
            if let Ok(run) = &run
                && !run.progress.notes.is_empty()
            {
                failures.push(format!("{what} left notes: {:?}", run.progress.notes));
            }
        }
        support::dismiss_all(&mut app);
    }
    app.screenshot(
        context,
        "junctions-copied-keep",
        "Target after copying three junctions as links",
    );
    app.shutdown()?;

    // Copy the files links point to (by pointer): the junction to a folder arrives as the
    // folder's tree; the dangling one and the one to its own folder cannot be followed
    // and stay junctions, each with a note.
    let target = support::scratch(context, "copy files")?;
    let mut app = App::start(Config::new(&links, &target));
    for (name, link) in &junctions {
        panel::select_row(&mut app, PaneId::Left, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                links: Some(LinkChoice::CopyFiles),
                link_input: LinkInput::Pointer,
                choices: vec![Choice::CancelJob],
                ..TransferSpec::copy()
            },
        );
        let copied = target.join(name);
        outcomes.push(format!(
            "F5 {name} (copy the files): {}; {}",
            describe(&copied),
            outcome(&run)
        ));
        let what = format!("F5 of the junction {name} copying the files");
        if !clean(&run, &what, LinkChoice::CopyFiles, &mut failures) {
            support::dismiss_all(&mut app);
            continue;
        }
        let (ok, expected) = match *name {
            "to-folder" => (
                same_tree(&source.join("deep").join("level-01"), &copied)
                    && !fs::symlink_metadata(&copied).is_ok_and(|meta| meta.is_symlink()),
                "the linked folder's tree",
            ),
            "dangling" => (
                is_link_to(&copied, link)
                    && kept_note(&run, name)
                    && notes_about(&run, name, "its target does not exist") == 1,
                "the same junction, with a note that its target does not exist",
            ),
            // As on Linux: the selected junction to its own folder is followed once; met
            // again inside that copy, it and the dangling junction stay junctions, each
            // with its reason.
            _ => {
                let inner = |child: &str| format!("to-self{}{child}", std::path::MAIN_SEPARATOR);
                (
                    fs::symlink_metadata(&copied)
                        .is_ok_and(|meta| meta.is_dir() && !meta.is_symlink())
                        && is_link_to(&copied.join("to-self"), link)
                        && is_link_to(&copied.join("dangling"), &junctions[0].1)
                        && same_tree(
                            &source.join("deep").join("level-01"),
                            &copied.join("to-folder"),
                        )
                        && notes_about(&run, &inner("to-self"), "contains it") == 1
                        && notes_about(&run, &inner("dangling"), "its target does not exist") == 1,
                    "its folder copied once, the junctions met again inside kept as junctions, \
                     each with its reason",
                )
            }
        };
        if !ok {
            failures.push(format!(
                "[AF-3] {what} gave {} (notes {:?}), not {expected}",
                describe(&copied),
                run.as_ref().map(|run| &run.progress.notes)
            ));
        }
        support::dismiss_all(&mut app);
    }
    app.screenshot(
        context,
        "junctions-copied-follow",
        "Target after copying three junctions with Copy the files links point to",
    );
    app.shutdown()?;
    let differences = support::compare_trees(&before, &tree_facts(&source)?, true);
    if !differences.is_empty() {
        failures.push(format!(
            "copying the junctions changed the sources: {differences:?}"
        ));
    }

    // F6 with the offered Links choice (keep links) moves each junction itself.
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
        if clean(
            &run,
            &format!("F6 of the junction {name}"),
            LinkChoice::Keep,
            &mut failures,
        ) {
            if !is_link_to(&moved, link) || still_there {
                failures.push(format!(
                    "[AF-3] F6 of the junction {name} gave {}, not the moved junction \
                     (source still there: {still_there})",
                    describe(&moved)
                ));
            } else {
                support::wait_row(&mut app, PaneId::Right, name, true);
            }
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
                .map(|relative| (support::slashed(relative), facts.clone()))
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
    move_projects(context, &mut failures, &mut outcomes)?;
    for outcome in &outcomes {
        context.note(outcome.clone());
    }
    support::report(&failures)
}
