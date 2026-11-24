//! F5 Copy and F6 Move of every kind of fixture entry, one at a time to the other panel
//! and as a multi-selection into a subfolder, verified on disk (bytes, exact names and,
//! where the application promises them, permissions and modified times) and in the
//! panels.
//!
//! What is promised (docs/developer/TRANSFERS.md, TRANSFER_OPTIONS.md, SECURITY.md):
//! Copy keeps the verified content and exact name, and gives every file and new folder
//! it creates, inside copied folders too, the source's read, write and execute bits
//! masked by the process umask (never setuid, setgid or sticky; a new folder may inherit
//! set-group-ID from its parent); "Preserve times and attributes" gives every copied file
//! and folder its source's modified and accessed times exactly, on Windows also its
//! creation time and Read-only, Hidden, System, Archive and Not content indexed, on Linux
//! its permission bits without the umask. A same-volume Move is a native rename, so
//! content, mode and times stay those of the moved entry.
use crate::{
    Context,
    app::{App, Config},
    files_support::{
        self as support, Choice, Content, Facts, Timings, TransferSpec, compare_trees, facts,
        io_error, tree_facts,
    },
    input, panel,
};
use midnight_core::PaneId;
use slint::platform::Key;
use std::{collections::BTreeMap, fs, path::Path, time::Instant};

/// Fixture folders whose entries are copied and moved one by one.
const KINDS: [&str; 8] = [
    "text", "code", "docs", "images", "archives", "sizes", "names", "readonly",
];

/// The names in `folder` as the panel lists them (no `..`).
fn entries(folder: &Path) -> Vec<String> {
    support::names_on_disk(folder)
}

/// A copy's permission bits as SECURITY.md promises them: the source's read, write and
/// execute bits masked by `umask`, without setuid, setgid or sticky (a folder may keep a
/// set-group-ID bit inherited from its parent). `Some` describes a deviation; links have
/// no mode of their own.
fn mode_deviation(path: &str, source: &Facts, copy: &Facts, umask: u32) -> Option<String> {
    let expected = source.mode & 0o777 & !umask;
    let found = match copy.content {
        Content::Link(_) => return None,
        Content::Folder => copy.mode & !0o2000,
        _ => copy.mode,
    };
    (found != expected).then(|| {
        format!(
            "{path} {:o}→{:o} (expected {expected:o})",
            source.mode, copy.mode
        )
    })
}

/// A source's times and attributes, read before the copy reads it (`fs::symlink_metadata`
/// opens nothing for reading, so it does not move the access time itself).
fn before_copy(path: &Path) -> Result<fs::Metadata, String> {
    fs::symlink_metadata(path).map_err(|error| io_error(path, error))
}

/// What "Preserve times and attributes" promises (TRANSFERS.md, preserved times and
/// attributes), against the source as it was before the copy (`before_copy`): the modified
/// time exactly, and a file's access time from before the copy read it (reading it moves
/// the source's own); on Windows also the creation time and the Read-only, Hidden, System,
/// Archive and Not content indexed attributes; on Linux the permission bits, not masked by
/// the umask. A folder's access time is not checked: listing it to total the job moves it
/// first where the volume records access. `Some` describes a deviation.
fn preserved_deviation(
    label: &str,
    wanted: &fs::Metadata,
    copy: &Path,
) -> Result<Option<String>, String> {
    let found = fs::symlink_metadata(copy).map_err(|error| io_error(copy, error))?;
    let mut differences = Vec::new();
    let mut times = vec![("modified", wanted.modified(), found.modified())];
    if !wanted.is_dir() {
        times.push(("accessed", wanted.accessed(), found.accessed()));
    }
    #[cfg(windows)]
    times.push(("created", wanted.created(), found.created()));
    for (name, wanted, found) in times {
        let (wanted, found) = (
            wanted.map_err(|error| io_error(copy, error))?,
            found.map_err(|error| io_error(copy, error))?,
        );
        if wanted != found {
            differences.push(format!("{name} {found:?} instead of {wanted:?}"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const PRESERVED: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x2000;
        let (wanted, found) = (
            wanted.file_attributes() & PRESERVED,
            found.file_attributes() & PRESERVED,
        );
        if wanted != found {
            differences.push(format!("attributes {found:#x} instead of {wanted:#x}"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let wanted = wanted.permissions().mode() & 0o777;
        let found = found.permissions().mode() & 0o777;
        if wanted != found {
            differences.push(format!("mode {found:o} instead of {wanted:o}"));
        }
    }
    Ok((!differences.is_empty()).then(|| format!("{label}: {}", differences.join(", "))))
}

/// [`mode_deviation`] of every entry of a copied tree (relative paths below `path`).
fn tree_mode_deviations(
    path: &str,
    source: &BTreeMap<String, Facts>,
    copy: &BTreeMap<String, Facts>,
    umask: u32,
) -> Vec<String> {
    source
        .iter()
        .filter_map(|(relative, wanted)| {
            let found = copy.get(relative)?;
            mode_deviation(&format!("{path}/{relative}"), wanted, found, umask)
        })
        .collect()
}

/// Enter `name` in both panels (left source, right target).
fn enter_both(app: &mut App, name: &str) -> Result<(), String> {
    panel::enter(app, PaneId::Right, name)?;
    panel::enter(app, PaneId::Left, name)?;
    Ok(())
}

fn leave_both(app: &mut App) -> Result<(), String> {
    for pane in [PaneId::Right, PaneId::Left] {
        panel::focus_list(app, pane)?;
        panel::leave(app, pane)?;
    }
    Ok(())
}

/// A folder of entries with distinct permissions (an executable script, a group-writable
/// file, a private file and a private folder), staged next to the fixture copies.
#[cfg(unix)]
fn stage_modes(folder: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(folder.join("tools")).map_err(|error| io_error(folder, error))?;
    for (name, mode, content) in [
        ("run.sh", 0o755, "#!/bin/sh\necho run\n"),
        ("shared.txt", 0o664, "shared\n"),
        ("private.txt", 0o600, "private\n"),
        ("tools/inner.sh", 0o750, "#!/bin/sh\n"),
    ] {
        let path = folder.join(name);
        fs::write(&path, content).map_err(|error| io_error(&path, error))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
            .map_err(|error| io_error(&path, error))?;
    }
    fs::set_permissions(folder.join("tools"), fs::Permissions::from_mode(0o750))
        .map_err(|error| io_error(folder, error))
}

#[cfg(not(unix))]
fn stage_modes(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|error| io_error(folder, error))
}

pub fn copy_every_kind(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "source")?;
    support::stage(context, &KINDS, &source)?;
    support::stage(context, &["deep"], &source)?;
    stage_modes(&source.join("modes"))?;
    let target = support::scratch(context, "target")?;
    let into = support::scratch(context, "into")?;
    let mut one_by_one: Vec<&str> = KINDS.to_vec();
    one_by_one.push("modes");
    for kind in &one_by_one {
        fs::create_dir(target.join(kind)).map_err(|error| io_error(&target, error))?;
        fs::create_dir(into.join(kind)).map_err(|error| io_error(&into, error))?;
    }
    let source_before = tree_facts(&source)?;
    // Copies get the source's permission bits masked by this process's umask; without
    // a readable umask (only Linux has `/proc/self/status`) they cannot be checked.
    let umask = process_umask();
    let mut deviations = Vec::new();
    let mut app = App::start(Config::new(&source, &target));
    let (left, right) = (PaneId::Left, PaneId::Right);
    let mut failures = Vec::new();
    let mut timings = Timings::default();
    let mut copied = 0;
    let mut time_changes = 0;
    let mut slow = Vec::new();
    for kind in one_by_one {
        enter_both(&mut app, kind)?;
        for name in entries(&source.join(kind)) {
            let from = source.join(kind).join(&name);
            let to = target.join(kind).join(&name);
            let expected = facts(&from)?;
            let tree = (expected.content == Content::Folder)
                .then(|| tree_facts(&from))
                .transpose()?;
            if let Err(error) = panel::select_row(&mut app, left, &name) {
                failures.push(format!("{kind}/{name}: {error}"));
                continue;
            }
            let shot = (kind == "names" && name == "with spaces.txt").then_some("copy-dialog");
            let run = match support::transfer(
                &mut app,
                context,
                TransferSpec {
                    shot,
                    ..TransferSpec::copy()
                },
            ) {
                Ok(run) => run,
                Err(error) => {
                    failures.push(format!("F5 {kind}/{name}: {error}"));
                    support::dismiss_all(&mut app);
                    continue;
                }
            };
            if let Err(error) = support::completed(&run, &format!("F5 {kind}/{name}")) {
                failures.push(error);
                continue;
            }
            timings.add_for(
                "F5 confirm to report",
                &format!("{kind}/{name}"),
                run.elapsed,
            );
            if run.elapsed > std::time::Duration::from_secs(2) {
                slow.push(format!("{kind}/{name}: {}", support::describe_run(&run)));
            }
            let shown = support::row_delay(&mut app, right, &name, true, run.finished);
            timings.add_for("report to row shown", &format!("{kind}/{name}"), shown);
            if !support::names_on_disk(&target.join(kind)).contains(&name) {
                failures.push(format!(
                    "{kind}/{name}: the exact name is not in the target folder"
                ));
                continue;
            }
            let written = facts(&to)?;
            match &tree {
                Some(tree) => {
                    let copy = tree_facts(&to)?;
                    let differences = compare_trees(tree, &copy, false);
                    if !differences.is_empty() {
                        failures.push(format!(
                            "{kind}/{name}: copied tree differs: {differences:?}"
                        ));
                    }
                    if let Some(umask) = umask {
                        deviations.extend(tree_mode_deviations(
                            &format!("{kind}/{name}"),
                            tree,
                            &copy,
                            umask,
                        ));
                    }
                }
                None if written.content != expected.content => failures.push(format!(
                    "{kind}/{name}: copied {:?}, source {:?}",
                    written.content, expected.content
                )),
                None => {}
            }
            if let Some(umask) = umask {
                deviations.extend(mode_deviation(
                    &format!("{kind}/{name}"),
                    &expected,
                    &written,
                    umask,
                ));
            }
            if written.modified != expected.modified {
                time_changes += 1;
            }
            copied += 1;
        }
        app.screenshot(
            context,
            &format!("copied-{kind}"),
            &format!("{kind}/ copied one entry at a time"),
        );
        leave_both(&mut app)?;
    }

    // Multi-selection into a subfolder: Ctrl+A (files only) with "Preserve times and
    // attributes", destination typed into the dialog.
    let mut preserved = 0;
    for kind in KINDS {
        panel::enter(&mut app, left, kind)?;
        panel::focus_list(&mut app, left)?;
        input::chord(app.slint(), &[Key::Control], "a");
        let files: Vec<String> = entries(&source.join(kind))
            .into_iter()
            .filter(|name| source.join(kind).join(name).is_file())
            .collect();
        let wanted = files.clone();
        app.until(
            &format!("Ctrl+A to select the files of {kind}/"),
            move |app| {
                let mut selected = support::selected_names(app, PaneId::Left);
                selected.sort();
                selected == wanted
            },
        );
        let destination = into.join(kind);
        let originals = files
            .iter()
            .map(|name| before_copy(&source.join(kind).join(name)))
            .collect::<Result<Vec<_>, _>>()?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                destination: Some(destination.as_path()),
                preserve: true,
                shot: (kind == "text").then_some("copy-into-subfolder-dialog"),
                ..TransferSpec::copy()
            },
        );
        match run {
            Err(error) => failures.push(format!("F5 {kind}/* into a subfolder: {error}")),
            Ok(run) => {
                if let Err(error) = support::completed(&run, &format!("F5 {kind}/*")) {
                    failures.push(error);
                }
                timings.add_for("multi-file F5 confirm to report", kind, run.elapsed);
                for (name, original) in files.iter().zip(&originals) {
                    let from = source.join(kind).join(name);
                    let to = destination.join(name);
                    let deviation =
                        preserved_deviation(&format!("{kind}/{name} into"), original, &to);
                    let expected = facts(&from)?;
                    match (facts(&to), deviation) {
                        (Err(error), _) | (_, Err(error)) => {
                            failures.push(format!("{kind}/{name} into: {error}"))
                        }
                        (Ok(found), _) if found.content != expected.content => failures
                            .push(format!("{kind}/{name} into: content {:?}", found.content)),
                        (_, Ok(Some(deviation))) => failures.push(deviation),
                        (Ok(_), Ok(None)) => preserved += 1,
                    }
                }
                // Completed roots are deselected.
                let left_selected = support::selected_names(&app, left);
                if !left_selected.is_empty() {
                    failures.push(format!(
                        "after copying {kind}/* the source panel still selects {left_selected:?}"
                    ));
                }
            }
        }
        panel::leave(&mut app, left)?;
    }

    // A twelve-level folder tree in one job.
    let deep_root = facts(&source.join("deep"))?;
    let deep = tree_facts(&source.join("deep"))?;
    panel::select_row(&mut app, left, "deep")?;
    match support::transfer(&mut app, context, TransferSpec::copy()) {
        Err(error) => failures.push(format!("F5 deep/: {error}")),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F5 deep/") {
                failures.push(error);
            }
            timings.add_for("tree F5 confirm to report", "deep", run.elapsed);
            let copy = tree_facts(&target.join("deep"))?;
            let differences = compare_trees(&deep, &copy, false);
            if !differences.is_empty() {
                failures.push(format!("the copied deep/ tree differs: {differences:?}"));
            }
            if let Some(umask) = umask {
                deviations.extend(mode_deviation(
                    "deep",
                    &deep_root,
                    &facts(&target.join("deep"))?,
                    umask,
                ));
                deviations.extend(tree_mode_deviations("deep", &deep, &copy, umask));
            }
            support::wait_row(&mut app, right, "deep", true);
        }
    }

    // The same tree with "Preserve times and attributes", typed into another folder: every
    // folder keeps its source's times although the job wrote its contents after creating it.
    let mut preserved_tree = 0;
    let entries: Vec<String> = std::iter::once(String::new())
        .chain(deep.keys().cloned())
        .collect();
    let originals = entries
        .iter()
        .map(|relative| before_copy(&source.join("deep").join(relative)))
        .collect::<Result<Vec<_>, _>>()?;
    panel::select_row(&mut app, left, "deep")?;
    match support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(into.as_path()),
            preserve: true,
            ..TransferSpec::copy()
        },
    ) {
        Err(error) => failures.push(format!("F5 deep/ preserving: {error}")),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F5 deep/ preserving") {
                failures.push(error);
            }
            timings.add_for("tree F5 preserving confirm to report", "deep", run.elapsed);
            for (relative, original) in entries.iter().zip(&originals) {
                let to = into.join("deep").join(relative);
                match preserved_deviation(&format!("deep/{relative}"), original, &to) {
                    Err(error) => failures.push(error),
                    Ok(Some(deviation)) => failures.push(deviation),
                    Ok(None) => preserved_tree += 1,
                }
            }
            // Reading the copies' contents moves their access times: compare them last.
            let differences = compare_trees(&deep, &tree_facts(&into.join("deep"))?, false);
            if !differences.is_empty() {
                failures.push(format!("the preserved deep/ tree differs: {differences:?}"));
            }
        }
    }

    // The sources are untouched.
    let differences = compare_trees(&source_before, &tree_facts(&source)?, true);
    if !differences.is_empty() {
        failures.push(format!("copying changed the sources: {differences:?}"));
    }
    match umask {
        Some(umask) if !deviations.is_empty() => failures.push(format!(
            "[AF-2] {} copied entries do not have the source's permission bits masked by the \
             umask {umask:04o} (SECURITY.md): {deviations:?}",
            deviations.len()
        )),
        Some(umask) => context.note(format!(
            "Permission bits of every copied file and folder, inside copied folders too, are \
             the source's masked by the umask {umask:04o}"
        )),
        None if cfg!(windows) => context.note(
            "Permission bits were not checked: Windows files have no mode bits (a copy \
             inherits its ACL from the destination folder)",
        ),
        None => context.note(
            "Permission bits were not checked: the process umask cannot be read here (no \
             /proc/self/status)",
        ),
    }
    context.note(format!(
        "F5 copied {copied} entries one at a time (bytes, exact names and trees verified), the 12-level deep/ tree, {preserved} files as per-folder multi-selections into subfolders with \"Preserve times and attributes\" and the deep/ tree again with it ({preserved_tree} files and folders): times equal to the nanosecond (Windows: created, modified, accessed and the Read-only, Hidden, System, Archive and Not content indexed attributes; Linux: modified, accessed and the permission bits)"
    ));
    context.note(format!(
        "not promised, observed: {time_changes} plain copies got a new modified time"
    ));
    if !slow.is_empty() {
        context.note(format!("copies slower than 2 s: {}", slow.join("; ")));
    }
    context.note(timings.summary());
    app.shutdown()?;
    support::report(&failures)
}

/// The process umask (`Umask:` in Linux's `/proc/self/status`), when it can be read.
fn process_umask() -> Option<u32> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Umask:"))
        .and_then(|value| u32::from_str_radix(value.trim(), 8).ok())
}

/// The fixture's owned write-protected folder (mode 0555, with contents): a Move takes
/// it as it is, its own mode included.
const LOCKED_FOLDER: &str = "locked-folder";

/// Folders of `expected` whose mode differs in `actual` (`compare_trees` compares the
/// modes of files and links only).
fn folder_modes(
    expected: &BTreeMap<String, Facts>,
    actual: &BTreeMap<String, Facts>,
) -> Vec<String> {
    expected
        .iter()
        .filter(|(_, wanted)| wanted.content == Content::Folder)
        .filter_map(|(path, wanted)| {
            let found = actual.get(path)?;
            (found.mode != wanted.mode)
                .then(|| format!("{path}: mode {:o} instead of {:o}", found.mode, wanted.mode))
        })
        .collect()
}

pub fn move_every_kind(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "source")?;
    support::stage(context, &KINDS, &source)?;
    support::stage(context, &["deep"], &source)?;
    let bulk = support::scratch(context, "bulk")?;
    support::stage(context, &KINDS, &bulk)?;
    let target = support::scratch(context, "target")?;
    let into = support::scratch(context, "into")?;
    for kind in KINDS {
        fs::create_dir(target.join(kind)).map_err(|error| io_error(&target, error))?;
        fs::create_dir(into.join(kind)).map_err(|error| io_error(&into, error))?;
    }
    let mut app = App::start(Config::new(&source, &target));
    let (left, right) = (PaneId::Left, PaneId::Right);
    let mut failures = Vec::new();
    let mut timings = Timings::default();
    let mut moved = 0;
    let mut total = 0;
    let mut slow = Vec::new();
    for kind in KINDS {
        enter_both(&mut app, kind)?;
        let names = entries(&source.join(kind));
        total += names.len();
        for name in names {
            let from = source.join(kind).join(&name);
            let to = target.join(kind).join(&name);
            let before = facts(&from)?;
            let tree = (before.content == Content::Folder)
                .then(|| tree_facts(&from))
                .transpose()?;
            if let Err(error) = panel::select_row(&mut app, left, &name) {
                failures.push(format!("{kind}/{name}: {error}"));
                continue;
            }
            // No entry may ask, the write-protected folder included; a question is
            // answered with Cancel job so the next move starts clean.
            let run = match support::transfer(
                &mut app,
                context,
                TransferSpec {
                    choices: vec![Choice::CancelJob],
                    shot: (kind == "names" && name == "with spaces.txt").then_some("move-dialog"),
                    ..TransferSpec::moving()
                },
            ) {
                Ok(run) => run,
                Err(error) => {
                    failures.push(format!("F6 {kind}/{name}: {error}"));
                    support::dismiss_all(&mut app);
                    continue;
                }
            };
            if !run.decisions.is_empty() {
                failures.push(format!(
                    "F6 {kind}/{name} asked a question (answered Cancel job): {}",
                    run.decisions.join(" | ")
                ));
                continue;
            }
            if let Err(error) = support::completed(&run, &format!("F6 {kind}/{name}")) {
                failures.push(error);
                continue;
            }
            if run.elapsed > std::time::Duration::from_secs(2) {
                slow.push(format!("{kind}/{name}: {}", support::describe_run(&run)));
            }
            timings.add_for(
                "F6 confirm to report",
                &format!("{kind}/{name}"),
                run.elapsed,
            );
            let gone = support::row_delay(&mut app, left, &name, false, run.finished);
            let shown = support::row_delay(&mut app, right, &name, true, run.finished);
            timings.add_for(
                "report to rows updated",
                &format!("{kind}/{name}"),
                gone.max(shown),
            );
            if fs::symlink_metadata(&from).is_ok() {
                failures.push(format!("{kind}/{name}: still in the source folder"));
            }
            if !support::names_on_disk(&target.join(kind)).contains(&name) {
                failures.push(format!(
                    "{kind}/{name}: the exact name is not in the target"
                ));
                continue;
            }
            match &tree {
                Some(tree) => {
                    let moved_tree = tree_facts(&to)?;
                    let mut differences = compare_trees(tree, &moved_tree, true);
                    differences.extend(folder_modes(tree, &moved_tree));
                    // The folder itself keeps its mode too, a write-protected one included.
                    let mode = facts(&to)?.mode;
                    if mode != before.mode {
                        differences.push(format!("mode {mode:o} instead of {:o}", before.mode));
                    }
                    if !differences.is_empty() {
                        failures.push(format!(
                            "{kind}/{name}: moved tree differs: {differences:?}"
                        ));
                    }
                }
                None => {
                    let after = facts(&to)?;
                    if after.content != before.content
                        || after.mode != before.mode
                        || after.modified != before.modified
                    {
                        failures.push(format!(
                            "{kind}/{name}: moved entry differs: {after:?} instead of {before:?}"
                        ));
                    }
                }
            }
            moved += 1;
        }
        app.screenshot(
            context,
            &format!("moved-{kind}"),
            &format!("{kind}/ moved one entry at a time"),
        );
        leave_both(&mut app)?;
    }
    // Every entry moved, the write-protected folder included.
    if moved != total {
        failures.push(format!("F6 one at a time moved {moved} of {total} entries"));
    }

    // Move a multi-selection of files and folders (Ctrl+Shift+A) into a subfolder.
    let mut bulk_moved = 0;
    let mut bulk_total = 0;
    navigate_to(&mut app, left, &bulk)?;
    for kind in KINDS {
        panel::enter(&mut app, left, kind)?;
        panel::focus_list(&mut app, left)?;
        input::chord(app.slint(), &[Key::Control, Key::Shift], "a");
        let all = entries(&bulk.join(kind));
        bulk_total += all.len();
        let wanted = all.clone();
        app.until(
            &format!("Ctrl+Shift+A to select everything in {kind}/"),
            move |app| {
                let mut selected = support::selected_names(app, PaneId::Left);
                selected.sort();
                selected == wanted
            },
        );
        let before = tree_facts(&bulk.join(kind))?;
        let destination = into.join(kind);
        let started = Instant::now();
        // Any question fails the step and cancels the job, so the next step starts
        // clean.
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                destination: Some(destination.as_path()),
                choices: vec![Choice::CancelJob],
                ..TransferSpec::moving()
            },
        );
        match run {
            Err(error) => failures.push(format!("F6 {kind}/* into a subfolder: {error}")),
            Ok(run) => {
                timings.add_for("multi-selection F6", kind, started.elapsed());
                if run.elapsed > std::time::Duration::from_secs(2) {
                    slow.push(format!("{kind}/*: {}", support::describe_run(&run)));
                }
                if !run.decisions.is_empty() {
                    failures.push(format!(
                        "F6 {kind}/* asked {} question(s) (answered Cancel job): {}",
                        run.decisions.len(),
                        run.decisions.join(" | ")
                    ));
                } else if let Err(error) = support::completed(&run, &format!("F6 {kind}/*")) {
                    failures.push(error);
                }
                // Everything arrives unchanged, folder modes included; nothing stays.
                let remaining = tree_facts(&bulk.join(kind))?;
                if !remaining.is_empty() {
                    failures.push(format!(
                        "F6 {kind}/* left {:?} in the source",
                        remaining.keys().take(8).collect::<Vec<_>>()
                    ));
                }
                let moved_tree = tree_facts(&destination)?;
                let mut differences = compare_trees(&before, &moved_tree, true);
                differences.extend(folder_modes(&before, &moved_tree));
                if !differences.is_empty() {
                    failures.push(format!("{kind}/* moved tree differs: {differences:?}"));
                }
                let arrived = entries(&destination).len();
                let wanted = all.len();
                if arrived != wanted {
                    failures.push(format!(
                        "F6 {kind}/* moved {arrived} of the {wanted} entries it should move"
                    ));
                }
                bulk_moved += arrived;
            }
        }
        panel::leave(&mut app, left)?;
    }
    context.note(format!(
        "F6 moved {moved} of {total} entries one at a time and {bulk_moved} of {bulk_total} entries as per-folder multi-selections into subfolders (the write-protected readonly/{LOCKED_FOLDER} included); content, exact name, mode (folders too) and modified time verified"
    ));
    if !slow.is_empty() {
        context.note(format!("moves slower than 2 s: {}", slow.join("; ")));
    }
    context.note(timings.summary());
    app.shutdown()?;
    support::report(&failures)
}

/// Type a folder into the path editor (Ctrl+G) and wait for the panel to show it.
pub fn navigate_to(app: &mut App, pane: PaneId, folder: &Path) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &folder.to_string_lossy());
    input::key(app.slint(), Key::Return);
    panel::wait_location(app, pane, folder);
    Ok(())
}
