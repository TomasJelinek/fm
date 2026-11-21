//! Journey: a deep tree with links and write-protected folders, copied twice (Follow
//! links cleared and checked), measured, and both copies deleted permanently; the
//! originals and everything the links point to stay as they were.
//!
//! 1. F5 with "Follow links" cleared: the tree arrives whole, every link a link with
//!    the same target text, the protected folders still protected.
//! 2. F5 with "Follow links" checked: the folder link arrives as the folder's tree, the
//!    file link as the file, the dangling link and the link to an enclosing folder stay
//!    links, each with a note.
//! 3. Ctrl+Shift+S: each copy's size is its bytes on disk, and the followed copy is
//!    larger by exactly what the two followed links point to.
//! 4. Shift+F8 of each copy: deleted whole without a question (Delete clears owner write
//!    protection inside the deleted tree), gone from the panel.
//! 5. The original tree (content, modes, times) and the link targets are unchanged.
//!
//! Not on Windows (`SKIP`): its checks are Linux link and mode semantics. Windows copies
//! never recreate links (docs/developer/TRANSFERS.md), file links need a privilege or
//! Developer Mode, and a folder is protected by its ACL, not a mode. `files_links` covers
//! junctions there.
// The journey's helpers are Unix-only; on Windows only the SKIP remains.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]
use super::journeys_support::{self as journey, Journey, wait};
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as files, Content, DeleteEnd, TransferSpec, compare_trees, tree_facts},
    input, panel,
};
use midnight_core::{CopyOperationState, PaneId};
use slint::platform::Key;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const RIGHT: PaneId = PaneId::Right;
const LEFT: PaneId = PaneId::Left;
const STEP: Duration = Duration::from_secs(60);
/// Links inside `tree/l1/l2/l3` and their target text.
const LINKS: [(&str, &str); 4] = [
    ("dir-link", "../../../../../outside/target-dir"),
    ("file-link", "../../../../../outside/target.txt"),
    ("dangling", "../missing-target"),
    ("up", ".."),
];
const L3: &str = "tree/l1/l2/l3";

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_links_readonly",
        area: "journeys",
        summary: "A deep tree with folder, file, dangling and enclosing links and write-protected folders copied with Follow links cleared and checked, measured with Ctrl+Shift+S, and both copies deleted with Shift+F8 without a question; originals and link targets unchanged.",
        run: links_readonly,
        timeout: Duration::from_secs(8 * 60),
    }
}

pub fn links_readonly(context: &mut Context) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = context;
        Err(
            "SKIP: the journey checks Linux link and mode semantics: Windows copies never \
             recreate links, file links need a privilege or Developer Mode, and folders are \
             protected by ACLs (files_links covers junctions)"
                .into(),
        )
    }
    #[cfg(unix)]
    {
        let (source, outside, target) = prepare(context)?;
        let mut config = Config::new(&source, &target);
        config.size_bytes = true;
        let mut app = App::start(config);
        let mut journey = Journey::default();
        let ended = steps(&mut app, context, &mut journey, &source, &outside, &target);
        files::dismiss_all(&mut app);
        let ended = ended.and(app.shutdown());
        journey.finish(context, ended)
    }
}

#[cfg(unix)]
fn prepare(context: &Context) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    use std::os::unix::fs::symlink;
    let work = &context.work;
    let outside = work.join("outside");
    journey::write(&outside.join("target-dir/t1.txt"), "target one\n")?;
    journey::write(
        &outside.join("target-dir/sub/t2.txt"),
        "target two, deeper\n",
    )?;
    journey::write(&outside.join("target.txt"), "the target file\n")?;
    let source = work.join("src");
    let tree = source.join("tree");
    journey::write(&tree.join("l1/a.txt"), "a\n")?;
    journey::write(&tree.join("l1/ro/r.txt"), "read only\n")?;
    journey::write(&tree.join("l1/ro/inner/i.txt"), "inner\n")?;
    journey::write(&tree.join("l1/l2/b.txt"), "bb\n")?;
    journey::write(&tree.join("l1/l2/l3/c.txt"), "ccc\n")?;
    for (name, target) in LINKS {
        let path = source.join(L3).join(name);
        symlink(target, &path).map_err(|error| files::io_error(&path, error))?;
    }
    for (path, mode) in [
        ("l1/ro/r.txt", 0o444),
        ("l1/ro/inner/i.txt", 0o444),
        ("l1/ro/inner", 0o555),
        ("l1/ro", 0o555),
    ] {
        crate::tools_support::set_mode(&tree.join(path), mode)?;
    }
    let target = work.join("dst");
    for folder in ["plain", "followed"] {
        std::fs::create_dir_all(target.join(folder))
            .map_err(|error| files::io_error(&target, error))?;
    }
    Ok((source, outside, target))
}

/// F5 of `tree` into `destination` with the link option set to `follow`.
fn copy_tree(
    app: &mut App,
    context: &mut Context,
    destination: &Path,
    follow: bool,
) -> Result<files::TransferRun, String> {
    panel::select_row(app, LEFT, "tree")?;
    files::transfer(
        app,
        context,
        TransferSpec {
            destination: Some(destination),
            follow_links: Some(follow),
            ..TransferSpec::copy()
        },
    )
}

/// The tree's facts as `copy` must hold them: links followed where `follow` says.
fn expected(
    source: &BTreeMap<String, files::Facts>,
    outside: &Path,
    follow: bool,
) -> Result<BTreeMap<String, Content>, String> {
    let mut wanted: BTreeMap<String, Content> = source
        .iter()
        .map(|(path, facts)| (path.clone(), facts.content.clone()))
        .collect();
    if follow {
        let dir_link = format!("{L3}/dir-link");
        wanted.insert(dir_link.clone(), Content::Folder);
        for (path, facts) in tree_facts(&outside.join("target-dir"))? {
            wanted.insert(format!("{dir_link}/{path}"), facts.content);
        }
        wanted.insert(
            format!("{L3}/file-link"),
            files::facts(&outside.join("target.txt"))?.content,
        );
    }
    Ok(wanted)
}

fn differences(wanted: &BTreeMap<String, Content>, copy: &Path) -> Result<Vec<String>, String> {
    let found: BTreeMap<String, Content> = tree_facts(copy)?
        .into_iter()
        .map(|(path, facts)| (path, facts.content))
        .collect();
    let mut differences = Vec::new();
    for (path, content) in wanted {
        match found.get(path) {
            None => differences.push(format!("missing {path}")),
            Some(other) if other != content => {
                differences.push(format!("{path}: {other:?} instead of {content:?}"));
            }
            Some(_) => {}
        }
    }
    for path in found.keys().filter(|path| !wanted.contains_key(*path)) {
        differences.push(format!("unexpected {path}"));
    }
    Ok(differences)
}

/// The facts of the source folder, which holds only `tree` (paths start with `tree`).
fn source_facts(source: &Path) -> Result<BTreeMap<String, files::Facts>, String> {
    tree_facts(source)
}

/// The shown size of the folder row `name` of `pane` once it is measured.
fn measured(app: &App, pane: PaneId, name: &str) -> Option<u64> {
    app.rows(pane)
        .iter()
        .find(|row| row.name == name && row.directory)
        .filter(|row| row.size_state == 0 && !row.size_running)
        .and_then(|row| journey::digits(&row.size))
}

#[cfg(unix)]
fn steps(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    source: &Path,
    outside: &Path,
    target: &Path,
) -> Result<(), String> {
    let original = source_facts(source)?;
    let outside_before = tree_facts(outside)?;

    // 1. Follow links cleared.
    let plain = copy_tree(app, context, &target.join("plain"), false)?;
    journey.check(
        "F5 with Follow links cleared completes without a question or notice",
        files::completed(&plain, "the copy with links kept").and_then(|()| {
            if plain.decisions.is_empty() && plain.notice.is_none() {
                Ok(())
            } else {
                Err(format!(
                    "asked {:?}, notice {:?}",
                    plain.decisions, plain.notice
                ))
            }
        }),
    );
    let wanted = expected(&original, outside, false)?;
    let found = differences(&wanted, &target.join("plain"))?;
    journey.check(
        "the copy with links kept equals the tree, every link with its target text",
        if found.is_empty() {
            Ok(())
        } else {
            Err(format!("{found:?}"))
        },
    );
    let protection = journey::mode(&target.join("plain/tree/l1/ro"));
    journey.check(
        "the copied write-protected folder stays write-protected",
        if protection & 0o200 == 0 {
            Ok(())
        } else {
            Err(format!("it has mode {protection:o}"))
        },
    );
    journey.step(format!(
        "F5 (Follow links cleared): {} entries, links kept, ro/ copied as mode {protection:o}",
        wanted.len()
    ));

    // 2. Follow links checked.
    let followed = copy_tree(app, context, &target.join("followed"), true)?;
    journey.check(
        "F5 with Follow links checked completes without a question or notice",
        files::completed(&followed, "the copy with links followed").and_then(|()| {
            if followed.decisions.is_empty() && followed.notice.is_none() {
                Ok(())
            } else {
                Err(format!(
                    "asked {:?}, notice {:?}",
                    followed.decisions, followed.notice
                ))
            }
        }),
    );
    let wanted = expected(&original, outside, true)?;
    let found = differences(&wanted, &target.join("followed"))?;
    journey.check(
        "the followed copy holds the linked folder and file; the dangling and enclosing links stay links",
        if found.is_empty() {
            Ok(())
        } else {
            Err(format!("{found:?}"))
        },
    );
    let notes = &followed.progress.notes;
    journey.check(
        "a note for each link that could not be followed",
        if followed.progress.note_count == 2
            && ["dangling", "up"].iter().all(|name| {
                notes
                    .iter()
                    .any(|note| note.contains(name) && note.contains("copied as a link"))
            })
        {
            Ok(())
        } else {
            Err(format!("{} notes: {notes:?}", followed.progress.note_count))
        },
    );
    journey.step("F5 (Follow links checked): linked folder and file copied, two links noted");

    // 3. Folder sizes.
    panel::focus_list(app, RIGHT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    input::chord(app.slint(), &[Key::Control, Key::Shift], "s");
    wait(app, "both copies to be measured", STEP, |app| {
        measured(app, RIGHT, "plain").is_some() && measured(app, RIGHT, "followed").is_some()
    })?;
    let plain_size = measured(app, RIGHT, "plain").unwrap_or_default();
    let followed_size = measured(app, RIGHT, "followed").unwrap_or_default();
    for (name, shown) in [("plain", plain_size), ("followed", followed_size)] {
        let disk = journey::disk_size(&target.join(name));
        journey.check(
            &format!("the size of {name}/ is its bytes on disk"),
            if shown == disk {
                Ok(())
            } else {
                Err(format!("shown {shown}, disk {disk}"))
            },
        );
    }
    let linked = journey::disk_size(&outside.join("target-dir"))
        + journey::disk_size(&outside.join("target.txt"));
    journey.check(
        "the followed copy is larger by what the followed links point to",
        if followed_size == plain_size + linked {
            Ok(())
        } else {
            Err(format!(
                "{followed_size} = {plain_size} + {linked} does not hold"
            ))
        },
    );
    app.screenshot(context, "sizes", "Both copies measured in bytes");
    journey.step(format!(
        "Ctrl+Shift+S: plain {plain_size} B, followed {followed_size} B (+{linked} B linked)"
    ));

    // 4. Permanent deletes without a question.
    for name in ["plain", "followed"] {
        panel::select_row(app, RIGHT, name)?;
        let what = format!("Shift+F8 of {name}/ (write-protected folders inside)");
        match files::delete_with(app, true)? {
            (_, DeleteEnd::Asked(window)) => {
                journey.check(
                    &what,
                    Err(format!(
                        "it asked a question: {}",
                        files::describe_window(&window)
                    )),
                );
                files::dismiss_all(app);
            }
            (_, DeleteEnd::Done(progress)) => {
                journey.check(&what, files::operation_ok(&progress, &what));
            }
        }
        journey.check(
            &format!("{name}/ is gone from the disk"),
            journey::absent(&target.join(name)),
        );
        journey.check(
            &format!("{name}/ is gone from the panel"),
            wait(app, "the row to go", Duration::from_secs(10), move |app| {
                !journey::shown(app, RIGHT).contains(name)
            }),
        );
        journey.check(
            &format!("no notice after deleting {name}/"),
            match files::take_notice(app) {
                None => Ok(()),
                Some(notice) => Err(format!("{notice:?}")),
            },
        );
    }
    journey.step("Shift+F8 of each copy: deleted whole without a question");

    // 5. Nothing else changed.
    let differences = compare_trees(&original, &source_facts(source)?, true);
    journey.check(
        "the original tree is unchanged (content, modes, times)",
        if differences.is_empty() {
            Ok(())
        } else {
            Err(format!("{differences:?}"))
        },
    );
    let differences = compare_trees(&outside_before, &tree_facts(outside)?, true);
    journey.check(
        "the link targets are unchanged",
        if differences.is_empty() {
            Ok(())
        } else {
            Err(format!("{differences:?}"))
        },
    );
    journey.check(
        "no copy job failed",
        if app
            .copies
            .iter()
            .any(|job| job.state == CopyOperationState::Failed)
        {
            Err("a copy job failed".into())
        } else {
            Ok(())
        },
    );
    journey.step("the original tree and the link targets are unchanged");
    Ok(())
}
