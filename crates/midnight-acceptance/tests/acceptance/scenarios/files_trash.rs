//! Delete to Trash (F8) for real, into the staged process's private Trash
//! (`viewers_stage::run_private`: `$XDG_DATA_HOME/Trash` is a folder in the work
//! folder): a file, a folder, a multi-selection, awkward names (`.trashinfo` paths and
//! shortened labels), a read-only file, a nested row of the inline tree; then the
//! write-protected cases. A write-protected folder inside the deleted tree is deleted
//! (owner write is added for it); an entry of a write-protected folder outside the
//! deleted tree is refused with a Retry/Skip/Skip all/Cancel question, and Retry works
//! once the folder is writable. Last, F8 of a file on another filesystem than the Trash
//! (tmpfs `$XDG_RUNTIME_DIR`), where no Trash takes it: F8 opens the permanent delete's
//! confirmation at once, as Shift+F8 does, with why ("There is no Trash on ..."), and no
//! job starts before it is answered. Escape keeps the file; Enter deletes it permanently,
//! and the private Trash is unchanged.
//!
//! Windows has no private Recycle Bin, so F8 is never confirmed where one could take the
//! file. There the scenario checks only F8 on a gated network or removable volume
//! (`MIDNIGHT_ACCEPTANCE_NETWORK_DIR`, `MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR`), which has no
//! Recycle Bin: the same permanent delete's confirmation, with why. A gated volume whose
//! Recycle Bin takes the file (a USB disk that Windows counts as fixed) gets the Trash
//! confirmation, which is cancelled.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, DeleteEnd, Timings, facts, io_error, tree_facts},
    input, inspect, panel, tools_support as tools, viewers_stage,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{DeleteMode, MutationKind, OperationProgress, OperationState, PaneId};
use slint::platform::{Key, WindowAdapter};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(15 * 60);
const LEFT: PaneId = PaneId::Left;

pub fn trash(context: &mut Context) -> Result<(), String> {
    if cfg!(windows) {
        return without_recycle_bin(context);
    }
    viewers_stage::run_private(context, TIMEOUT, run)
}

/// Windows: F8 on the gated volumes that have no Recycle Bin. The account's own Recycle
/// Bin is never used.
fn without_recycle_bin(context: &mut Context) -> Result<(), String> {
    let mut checked = Vec::new();
    for variable in [
        "MIDNIGHT_ACCEPTANCE_NETWORK_DIR",
        "MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR",
    ] {
        let folder = match crate::fixtures::gated(context, variable) {
            Ok(folder) => folder,
            Err(error) if error.starts_with("SKIP:") => continue,
            Err(error) => return Err(error),
        };
        match without_trash(
            context,
            &folder,
            &[
                "There is no Recycle Bin on ",
                "The Recycle Bin is turned off for ",
                "A policy turns off the Recycle Bin",
            ],
            None,
        )
        .map_err(|error| format!("{variable}: {error}"))?
        {
            Asked::Permanent => checked.push(variable),
            Asked::Trash(shown) => context.note(format!(
                "{variable}: that volume has a Recycle Bin; F8 asked for the Trash and was \
                 cancelled: {shown}"
            )),
        }
    }
    if checked.is_empty() {
        return Err(
            "SKIP: Windows recycles into the account's own Recycle Bin, one per volume \
             (`<volume>\\$Recycle.Bin\\<SID>`), which no variable redirects to a private \
             folder, and keeps no FreeDesktop Trash records; F8 for real would put the run's \
             files into the user's Recycle Bin. F8 without a Recycle Bin needs \
             MIDNIGHT_ACCEPTANCE_NETWORK_DIR or MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR on a \
             volume without one"
                .into(),
        );
    }
    context.note(format!(
        "F8 without a Recycle Bin asked for a permanent delete in {checked:?}"
    ));
    Ok(())
}

#[cfg(unix)]
fn chmod(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| io_error(path, error))
}

#[cfg(not(unix))]
fn chmod(_: &Path, _: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path).map_or(0, |metadata| metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode(_: &Path) -> u32 {
    0
}

/// `%XX` sequences decoded, as `.trashinfo` writes paths.
fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(value) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            output.push(value);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    output
}

/// The label a trashed entry starts with: the name's first 32 bytes at most, cut at a
/// character boundary.
fn label(name: &str) -> &str {
    let mut end = name.len().min(32);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// One entry of the Trash: its name in `files/` and its `.trashinfo` text.
struct Trashed {
    name: String,
    info: String,
}

fn trashed(trash: &Path) -> Vec<Trashed> {
    panel::disk_names(&trash.join("files"))
        .into_iter()
        .map(|name| Trashed {
            info: fs::read_to_string(trash.join("info").join(format!("{name}.trashinfo")))
                .unwrap_or_default(),
            name,
        })
        .collect()
}

/// The Trash entry whose `.trashinfo` names `original`, checked against the format.
pub(super) fn entry_for(trash: &Path, original: &Path) -> Result<PathBuf, String> {
    let wanted = original.as_os_str().as_encoded_bytes();
    let entries = trashed(trash);
    let entry = entries
        .iter()
        .find(|entry| {
            entry
                .info
                .lines()
                .find_map(|line| line.strip_prefix("Path="))
                .is_some_and(|path| percent_decode(path) == wanted)
        })
        .ok_or_else(|| {
            format!(
                "no Trash entry names {}; entries {:?}",
                original.display(),
                entries.iter().map(|entry| &entry.name).collect::<Vec<_>>()
            )
        })?;
    let lines: Vec<&str> = entry.info.lines().collect();
    let date = lines
        .iter()
        .find_map(|line| line.strip_prefix("DeletionDate="))
        .unwrap_or_default();
    let date_ok = date.len() == 19
        && date.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            10 => byte == b'T',
            13 | 16 => byte == b':',
            _ => byte.is_ascii_digit(),
        });
    if lines.first() != Some(&"[Trash Info]") || !date_ok {
        return Err(format!(
            "{}.trashinfo is malformed: {:?}",
            entry.name, entry.info
        ));
    }
    let name = original
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !entry
        .name
        .starts_with(&format!("{}-midnight-", label(&name)))
    {
        return Err(format!(
            "the Trash entry {:?} does not start with the label {:?}",
            entry.name,
            label(&name)
        ));
    }
    Ok(trash.join("files").join(&entry.name))
}

fn ok(progress: &OperationProgress, what: &str) -> Result<(), String> {
    support::operation_ok(progress, what)
}

fn run(context: &mut Context) -> Result<(), String> {
    // Only the staged process has a private data folder, beside its run folder.
    let staged = context.run_dir.parent().and_then(Path::parent);
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|data| staged.is_some_and(|staged| data.starts_with(staged)))
        .ok_or("files_trash runs only in its staged process, with a private XDG_DATA_HOME")?;
    let trash_root = data.join("Trash");
    let root = support::scratch(context, "trash")?;
    support::stage(
        context,
        &["text", "code", "images", "names", "readonly", "deep"],
        &root,
    )?;
    support::stage(context, &["readonly"], &root.join("perm"))?;
    // `many` four times over: a delete of one copy can end, and its window close, before
    // the window's Pause is pressed (Linux run of 0aba1c1a).
    crate::scenarios::transfers::copy_tree(&context.fixture.path("many"), &root.join("bulk"))?;
    for copy in 1..=3 {
        crate::scenarios::transfers::copy_tree(
            &context.fixture.path("many"),
            &root.join(format!("bulk/again-{copy}")),
        )?;
    }
    let ro_parent = root.join("ro-parent");
    fs::create_dir(&ro_parent).map_err(|error| io_error(&ro_parent, error))?;
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(ro_parent.join(name), format!("{name}\n"))
            .map_err(|error| io_error(&ro_parent, error))?;
    }
    chmod(&ro_parent, 0o555)?;
    let mut app = App::start(Config::new(&root, &context.work));
    let mut failures = Vec::new();
    let mut timings = Timings::default();

    // A file: in the Trash with its bytes and a well-formed .trashinfo.
    panel::enter(&mut app, LEFT, "text")?;
    panel::select_row(&mut app, LEFT, "plain.txt")?;
    let before = facts(&root.join("text/plain.txt"))?;
    let started = Instant::now();
    let progress = support::trash(&mut app, context, Some("trash-confirmation"))?;
    timings.add("F8 confirm to report", started.elapsed());
    if let Err(error) = ok(&progress, "F8 text/plain.txt") {
        failures.push(error);
    }
    support::wait_row(&mut app, LEFT, "plain.txt", false);
    match entry_for(&trash_root, &root.join("text/plain.txt")) {
        Err(error) => failures.push(error),
        Ok(entry) => {
            if facts(&entry).ok().as_ref() != Some(&before) {
                failures.push(format!(
                    "the trashed plain.txt is {:?}, not {before:?}",
                    facts(&entry)
                ));
            }
        }
    }
    for folder in ["", "files", "info"] {
        let path = trash_root.join(folder);
        if mode(&path) & 0o777 != 0o700 {
            failures.push(format!(
                "{} has mode {:o}, not 0700",
                path.display(),
                mode(&path)
            ));
        }
    }
    panel::leave(&mut app, LEFT)?;

    // A folder: its whole tree, modes and times, in the Trash.
    let tree = tree_facts(&root.join("code"))?;
    panel::select_row(&mut app, LEFT, "code")?;
    let progress = support::trash(&mut app, context, None)?;
    if let Err(error) = ok(&progress, "F8 code/") {
        failures.push(error);
    }
    match entry_for(&trash_root, &root.join("code")) {
        Err(error) => failures.push(error),
        Ok(entry) => {
            let differences = support::compare_trees(&tree, &tree_facts(&entry)?, true);
            if !differences.is_empty() {
                failures.push(format!("the trashed code/ differs: {differences:?}"));
            }
        }
    }

    // A multi-selection: every file of images/.
    panel::enter(&mut app, LEFT, "images")?;
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "a");
    let names = support::names_on_disk(&root.join("images"));
    let wanted = names.clone();
    app.until("Ctrl+A in images/", move |app| {
        let mut selected = support::selected_names(app, LEFT);
        selected.sort();
        selected == wanted
    });
    let progress = support::trash(&mut app, context, Some("trash-multi"))?;
    if let Err(error) = ok(&progress, "F8 of images/*") {
        failures.push(error);
    }
    for name in &names {
        if let Err(error) = entry_for(&trash_root, &root.join("images").join(name)) {
            failures.push(error);
        }
    }
    if !support::names_on_disk(&root.join("images")).is_empty() {
        failures.push("images/ is not empty after F8 of every file".into());
    }
    panel::leave(&mut app, LEFT)?;

    // Awkward names: percent-encoded paths, labels cut at 32 bytes.
    panel::enter(&mut app, LEFT, "names")?;
    let long = format!("{}.txt", "long-name-".repeat(22));
    for name in [
        "Ünïcödé.txt",
        "with spaces.txt",
        "日本語のファイル.txt",
        "#hash&amp;$dollar%percent.txt",
        long.as_str(),
    ] {
        if let Err(error) = panel::select_row(&mut app, LEFT, name) {
            failures.push(error);
            continue;
        }
        let progress = support::trash(&mut app, context, None)?;
        if let Err(error) = ok(&progress, &format!("F8 names/{name}")) {
            failures.push(error);
            continue;
        }
        support::wait_row(&mut app, LEFT, name, false);
        if let Err(error) = entry_for(&trash_root, &root.join("names").join(name)) {
            failures.push(error);
        }
    }
    panel::leave(&mut app, LEFT)?;

    // A read-only file keeps its mode in the Trash.
    panel::enter(&mut app, LEFT, "readonly")?;
    panel::select_row(&mut app, LEFT, "locked.txt")?;
    let progress = support::trash(&mut app, context, None)?;
    if let Err(error) = ok(&progress, "F8 readonly/locked.txt") {
        failures.push(error);
    }
    match entry_for(&trash_root, &root.join("readonly/locked.txt")) {
        Err(error) => failures.push(error),
        Ok(entry) if mode(&entry) & 0o777 != 0o444 => failures.push(format!(
            "the trashed locked.txt has mode {:o}",
            mode(&entry)
        )),
        Ok(_) => {}
    }

    // A write-protected folder with contents is the whole deleted tree: owner write is
    // added for it and it goes to the Trash. While that is missing, the refusal must
    // leave it unchanged.
    let locked = tree_facts(&root.join("readonly/locked-folder"))?;
    panel::select_row(&mut app, LEFT, "locked-folder")?;
    let (start, ended) = support::delete_with(&mut app, false)?;
    let result = match ended {
        DeleteEnd::Done(progress) if progress.state == OperationState::Completed => {
            entry_for(&trash_root, &root.join("readonly/locked-folder")).map(|_| ())
        }
        DeleteEnd::Done(progress) => {
            let kept = tree_facts(&root.join("readonly/locked-folder"))
                .is_ok_and(|after| support::compare_trees(&locked, &after, true).is_empty());
            if !kept {
                failures.push("a refused Trash of the write-protected folder changed it".into());
            }
            Err(format!(
                "{:?}: {:?}",
                progress.state,
                progress.errors.first()
            ))
        }
        DeleteEnd::Asked(window) => {
            let text = support::describe_window(&window);
            support::cancel_delete(&mut app, &window, start);
            Err(format!("asked instead: {text}"))
        }
    };
    if let Err(error) = result {
        failures.push(format!(
            "F8 of a write-protected folder with contents: {error}"
        ));
    }
    support::dismiss_all(&mut app);
    panel::leave(&mut app, LEFT)?;

    // An entry of a write-protected folder (outside the deleted tree): the question
    // offers Retry, Skip, Skip all and Cancel; Retry works once the folder is writable.
    panel::enter(&mut app, LEFT, "ro-parent")?;
    panel::select_row(&mut app, LEFT, "a.txt")?;
    let (start, ended) = support::delete_with(&mut app, false)?;
    let result = match ended {
        DeleteEnd::Asked(window) => {
            let offered = inspect::labels(window.window(), AccessibleRole::Button);
            app.screenshot_window(context, &window, "delete-question", "A refused delete asks");
            let missing: Vec<&str> = ["Retry", "Skip", "Cancel"]
                .into_iter()
                .filter(|wanted| !offered.iter().any(|label| label.starts_with(wanted)))
                .collect();
            chmod(&ro_parent, 0o755)?;
            let answered = support::answer_delete(&mut app, &window, &["Retry", "Skip"]);
            let ended = support::wait_delete(&mut app, start);
            chmod(&ro_parent, 0o555)?;
            match ended {
                DeleteEnd::Done(progress) if progress.state == OperationState::Completed => {
                    if !missing.is_empty() {
                        Err(format!("the question offers no {missing:?}: {offered:?}"))
                    } else if answered.as_deref() != Ok("Retry") {
                        Err(format!("Retry could not be answered: {answered:?}"))
                    } else {
                        entry_for(&trash_root, &ro_parent.join("a.txt")).map(|_| ())
                    }
                }
                DeleteEnd::Done(progress) => Err(format!("Retry ended {:?}", progress.state)),
                DeleteEnd::Asked(window) => {
                    let text = support::describe_window(&window);
                    support::cancel_delete(&mut app, &window, start);
                    Err(format!("Retry asked again: {text}"))
                }
            }
        }
        DeleteEnd::Done(progress) => {
            if !ro_parent.join("a.txt").is_file() {
                failures.push("a refused delete in a write-protected folder removed a.txt".into());
            }
            Err(format!(
                "no question; the delete ended {:?} ({:?})",
                progress.state,
                progress.errors.first()
            ))
        }
    };
    if let Err(error) = result {
        failures.push(format!(
            "F8 of a file in a write-protected folder asks, and Retry works: {error}"
        ));
    }
    support::dismiss_all(&mut app);

    // Two such files: Skip all keeps both and ends the job.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    for name in ["b.txt", "c.txt"] {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    let (start, ended) = support::delete_with(&mut app, false)?;
    let result = match ended {
        DeleteEnd::Asked(window) => {
            // A Delete failure's Skip all is "Skip all errors" (decision_windows.rs): it
            // covers every later failure of the Delete, not one error type.
            let clicked = support::answer_delete(
                &mut app,
                &window,
                &["Skip all", "Skip all errors", "Skip all this error type"],
            );
            match (clicked, support::wait_delete(&mut app, start)) {
                (Err(error), DeleteEnd::Asked(window)) => {
                    support::cancel_delete(&mut app, &window, start);
                    Err(error)
                }
                (Err(error), _) => Err(error),
                (Ok(_), DeleteEnd::Done(progress)) => {
                    if ro_parent.join("b.txt").is_file() && ro_parent.join("c.txt").is_file() {
                        Ok(())
                    } else {
                        Err(format!(
                            "Skip all ended {:?} without both files",
                            progress.state
                        ))
                    }
                }
                (Ok(_), DeleteEnd::Asked(window)) => {
                    let text = support::describe_window(&window);
                    support::cancel_delete(&mut app, &window, start);
                    Err(format!("Skip all asked again: {text}"))
                }
            }
        }
        DeleteEnd::Done(progress) => Err(format!(
            "no question; the delete ended {:?} with {} error(s)",
            progress.state,
            progress.errors.len()
        )),
    };
    if !ro_parent.join("b.txt").is_file() || !ro_parent.join("c.txt").is_file() {
        failures.push("a refused delete in a write-protected folder removed a file".into());
    }
    if let Err(error) = result {
        failures.push(format!(
            "F8 of two files in a write-protected folder: Skip all keeps both: {error}"
        ));
    }
    support::dismiss_all(&mut app);
    panel::leave(&mut app, LEFT)?;

    // A nested row of the inline tree goes to the Trash with its tree.
    support::toggle_tree(&mut app, "deep")?;
    support::wait_tree_depth(&mut app, "level-01", Some(1));
    support::toggle_tree(&mut app, "level-01")?;
    support::wait_tree_depth(&mut app, "level-02", Some(2));
    support::toggle_tree(&mut app, "level-02")?;
    support::wait_tree_depth(&mut app, "level-03", Some(3));
    let nested = root.join("deep/level-01/level-02/level-03");
    let tree = tree_facts(&nested)?;
    panel::select_row(&mut app, LEFT, "level-03")?;
    let progress = support::trash(&mut app, context, Some("trash-tree-row"))?;
    if let Err(error) = ok(&progress, "F8 of the nested level-03 row") {
        failures.push(error);
    }
    support::wait_tree_depth(&mut app, "level-03", None);
    match entry_for(&trash_root, &nested) {
        Err(error) => failures.push(error),
        Ok(entry) => {
            let differences = support::compare_trees(&tree, &tree_facts(&entry)?, true);
            if !differences.is_empty() {
                failures.push(format!("the trashed level-03 differs: {differences:?}"));
            }
        }
    }
    support::toggle_tree(&mut app, "deep")?;
    support::wait_tree_depth(&mut app, "level-01", None);

    // Shift+F8 of a tree holding a write-protected folder with contents: it is deleted
    // whole, without a question.
    panel::enter(&mut app, LEFT, "perm")?;
    panel::select_row(&mut app, LEFT, "readonly")?;
    let (start, ended) = support::delete_with(&mut app, true)?;
    let result = match ended {
        DeleteEnd::Done(progress)
            if progress.state == OperationState::Completed
                && !root.join("perm/readonly").exists() =>
        {
            Ok(())
        }
        DeleteEnd::Done(progress) => Err(format!(
            "{:?} ({:?}); left {:?}",
            progress.state,
            progress.errors.first(),
            support::names_on_disk(&root.join("perm/readonly"))
        )),
        DeleteEnd::Asked(window) => {
            let text = support::describe_window(&window);
            support::cancel_delete(&mut app, &window, start);
            Err(format!("asked: {text}"))
        }
    };
    if let Err(error) = result {
        failures.push(format!(
            "Shift+F8 of a tree with a write-protected folder deletes it whole: {error}"
        ));
    }
    support::dismiss_all(&mut app);
    panel::leave(&mut app, LEFT)?;

    // A long permanent delete: its window holds still while paused, then Cancel
    // operation stops it, and every file left is whole.
    let originals = tree_facts(&root.join("bulk"))?;
    panel::select_row(&mut app, LEFT, "bulk")?;
    let start = app.operations.len();
    let before = app.tool_windows().len();
    panel::wait_input(&mut app);
    input::chord(app.slint(), &[Key::Shift], Key::F8);
    support::wait_operation_dialog(&mut app, "the permanent delete confirmation");
    input::key(app.slint(), Key::Return);
    app.until("the delete's window or its end", move |app| {
        app.tool_windows().len() > before
            || app.operations[start..]
                .iter()
                .any(|progress| progress.state.is_terminal())
    });
    let ended = |app: &App| {
        app.operations[start..]
            .iter()
            .find(|progress| progress.state.is_terminal())
            .cloned()
    };
    match tools::tool_titled(&app, "Permanently delete").filter(|_| ended(&app).is_none()) {
        None => context.note(format!(
            "the delete of bulk/ ended {:?} before its window could be used",
            ended(&app).map(|progress| progress.state)
        )),
        Some(window) => {
            // Pause before settling where it can: a successful delete closes its window,
            // and every settle before the click is time for it to end.
            crate::app::layout_tool(&window, 760, 560);
            let mut paused = tools::click_last(window.window(), "Pause");
            app.settle();
            if paused.is_err() && ended(&app).is_none() {
                paused = tools::click_last(window.window(), "Pause");
            }
            let held = |app: &App| {
                app.operations[start..]
                    .last()
                    .map(|progress| progress.processed_entries)
            };
            if paused.is_ok() {
                let watched = window.clone();
                let deadline = Instant::now() + Duration::from_secs(20);
                while !inspect::visible_texts(watched.window())
                    .iter()
                    .any(|text| text == "Paused")
                    && ended(&app).is_none()
                    && Instant::now() < deadline
                {
                    app.idle(Duration::from_millis(20));
                }
                let at = held(&app);
                app.idle(Duration::from_millis(800));
                if ended(&app).is_none() && held(&app) != at {
                    failures.push(format!(
                        "the paused delete moved on: {at:?} → {:?}",
                        held(&app)
                    ));
                }
                app.screenshot_window(context, &window, "delete-paused", "A paused delete");
                let _ = tools::click_last(window.window(), "Resume");
            } else if let Some(progress) = ended(&app) {
                // Its window closed with the end, so there was no Pause to press.
                context.note(format!(
                    "the delete of bulk/ ended {:?} before its Pause could be pressed",
                    progress.state
                ));
            } else {
                failures.push(format!(
                    "the delete's window has no Pause: {:?}",
                    inspect::visible_texts(window.window())
                ));
            }
            if ended(&app).is_none()
                && tools::click_last(window.window(), "Cancel operation").is_ok()
            {
                let watched = window.clone();
                let deadline = Instant::now() + Duration::from_secs(30);
                while ended(&app).is_none() && Instant::now() < deadline {
                    if inspect::visible_texts(watched.window())
                        .iter()
                        .any(|text| text == "Cancel this operation?")
                    {
                        let _ = tools::click_last(watched.window(), "Cancel operation");
                    }
                    app.idle(Duration::from_millis(30));
                }
            }
            let progress = support::wait_operation(&mut app, start);
            // A cancelled delete keeps its window open with the outcome, as Copy does;
            // no notice repeats it.
            app.idle(Duration::from_millis(400));
            let ended = inspect::visible_texts(window.window());
            if progress.state == OperationState::Cancelled {
                if !window.window().is_visible()
                    || !ended
                        .iter()
                        .any(|text| text.starts_with("Delete cancelled;"))
                    || !ended.iter().any(|text| text.contains("Items deleted: "))
                    || ended.iter().any(|text| text.starts_with("Items remaining"))
                {
                    failures.push(format!(
                        "the cancelled delete's window does not show its outcome: {:?}",
                        ended.iter().take(12).collect::<Vec<_>>()
                    ));
                }
                if let Some(notice) = support::shown_notice(&app) {
                    failures.push(format!(
                        "a notice follows the cancelled delete whose window is open: {notice:?}"
                    ));
                }
                app.screenshot_window(context, &window, "delete-cancelled", "A cancelled delete");
            }
            let kept = tree_facts(&root.join("bulk")).unwrap_or_default();
            let partial: Vec<&String> = kept
                .iter()
                .filter(|(path, facts)| {
                    originals.get(*path).map(|original| &original.content) != Some(&facts.content)
                })
                .map(|(path, _)| path)
                .collect();
            if !partial.is_empty() {
                failures.push(format!(
                    "the cancelled delete left changed files: {:?}",
                    partial.iter().take(5).collect::<Vec<_>>()
                ));
            }
            context.note(format!(
                "the delete of bulk/ ended {:?}: {} of {} entries kept, all whole",
                progress.state,
                kept.len(),
                originals.len()
            ));
        }
    }
    support::dismiss_all(&mut app);

    let entries = trashed(&trash_root);
    context.note(format!(
        "{} entries in the private Trash {}: {:?}",
        entries.len(),
        trash_root.display(),
        entries
            .iter()
            .map(|entry| &entry.name)
            .take(12)
            .collect::<Vec<_>>()
    ));
    context.note(timings.summary());
    app.shutdown()?;
    // A failed Trash is never a permanent delete.
    if let Err(error) = trash_elsewhere(context, &trash_root) {
        failures.push(format!("F8 on another filesystem: {error}"));
    }
    support::report(&failures)
}

/// F8 on another filesystem than the private Trash (tmpfs `$XDG_RUNTIME_DIR`): no Trash
/// takes the file there.
fn trash_elsewhere(context: &mut Context, trash_root: &Path) -> Result<(), String> {
    #[cfg(unix)]
    let parent = {
        use std::os::unix::fs::MetadataExt;
        let device = |path: &Path| fs::metadata(path).map(|metadata| metadata.dev()).ok();
        let trash_device = device(trash_root.parent().unwrap_or(trash_root));
        crate::fixtures::runtime_volume()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_dir() && device(path) != trash_device)
    };
    #[cfg(not(unix))]
    let parent: Option<PathBuf> = None;
    let Some(parent) = parent else {
        context.note(
            "SKIP: F8 on another filesystem: no $XDG_RUNTIME_DIR on another filesystem than \
             the private Trash",
        );
        return Ok(());
    };
    let run = context
        .run_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = crate::fixtures::create_fresh(&parent, &format!("midnight-acceptance-{run}"))?;
    let result = without_trash(
        context,
        &folder,
        &["There is no Trash on "],
        Some(trash_root),
    );
    let _ = crate::fixtures::remove_tree(&folder);
    match result? {
        Asked::Permanent => Ok(()),
        Asked::Trash(shown) => Err(format!(
            "F8 asked for the Trash, which cannot take the file: {shown}"
        )),
    }
}

/// What F8 asked for.
enum Asked {
    /// The permanent delete's confirmation, checked.
    Permanent,
    /// The Trash confirmation (its title and warning), cancelled.
    Trash(String),
}

/// F8 of `elsewhere.txt`, a new file in `folder`, where no Trash takes it: the permanent
/// delete's confirmation opens at once, exactly as Shift+F8's (its title, question and
/// dangerous button), its warning saying why first (one of `reasons`), and no job runs
/// before it is answered. Escape keeps the file; Enter deletes it permanently. The private
/// Trash, if any, is unchanged. A Trash confirmation instead is cancelled at once and
/// reported as `Asked::Trash`: it is never confirmed.
fn without_trash(
    context: &mut Context,
    folder: &Path,
    reasons: &[&str],
    trash_root: Option<&Path>,
) -> Result<Asked, String> {
    let file = folder.join("elsewhere.txt");
    fs::write(&file, "kept\n").map_err(|error| io_error(&file, error))?;
    let before = facts(&file)?;
    let entries = trash_root.map(|root| trashed(root).len());
    let mut app = App::start(Config::new(folder, &context.work));
    let result = (|| -> Result<Asked, String> {
        for accept in [false, true] {
            panel::select_row(&mut app, LEFT, "elsewhere.txt")?;
            let start = app.operations.len();
            input::key(app.slint(), Key::F8);
            support::wait_operation_dialog(&mut app, "the confirmation F8 opens");
            let title = app.window.get_operation_title().to_string();
            let warning = app.window.get_operation_warning().to_string();
            let body = app.window.get_operation_body().to_string();
            if !app.window.get_operation_dangerous() {
                // Never confirmed: the Trash would take the file.
                input::key(app.slint(), Key::Escape);
                app.until("the Trash confirmation to close", |app| {
                    app.window.get_operation_token().is_empty()
                });
                if facts(&file)? != before {
                    return Err("cancelling the Trash confirmation changed elsewhere.txt".into());
                }
                return Ok(Asked::Trash(format!("{title}: {warning}")));
            }
            let mut lines = warning.lines();
            let why = lines.next().unwrap_or_default();
            if title != "Permanently delete"
                || !reasons.iter().any(|reason| why.starts_with(reason))
                || !why.ends_with(", so this item will be deleted permanently.")
                || lines.next() != Some("Permanent deletion cannot be undone.")
                || !body.starts_with("Permanently delete \u{201c}elsewhere.txt\u{201d}?")
            {
                return Err(format!(
                    "F8 without a Trash is not the permanent delete's confirmation with why: \
                     {title:?}, {warning:?}, {body:?}"
                ));
            }
            let ran = |app: &App| {
                app.operations[start..].iter().any(|progress| {
                    matches!(
                        progress.state,
                        OperationState::Queued
                            | OperationState::Running
                            | OperationState::Completed
                            | OperationState::CompletedWithErrors
                    )
                })
            };
            if ran(&app) || facts(&file)? != before {
                return Err("a delete ran before its confirmation was answered".into());
            }
            if accept {
                input::key(app.slint(), Key::Return);
                let ended = support::wait_operation(&mut app, start);
                support::operation_ok(&ended, "F8 without a Trash, confirmed")?;
                if ended.kind != MutationKind::Delete(DeleteMode::Permanent) {
                    return Err(format!("the confirmed delete was {:?}", ended.kind));
                }
                if file.exists() {
                    return Err("the confirmed file remains".into());
                }
            } else {
                app.screenshot(
                    context,
                    "trash-unavailable",
                    "F8 without a Trash: the permanent delete's confirmation, with why",
                );
                context.note(format!("F8 without a Trash: {title}: {warning:?}"));
                input::key(app.slint(), Key::Escape);
                app.until("Escape to close the confirmation", |app| {
                    app.window.get_operation_token().is_empty()
                });
                app.idle(Duration::from_millis(300));
                if ran(&app) || facts(&file)? != before {
                    return Err("Escape in the confirmation changed elsewhere.txt".into());
                }
            }
            support::dismiss_all(&mut app);
        }
        Ok(Asked::Permanent)
    })();
    app.shutdown()?;
    if let (Some(root), Some(entries)) = (trash_root, entries)
        && trashed(root).len() != entries
    {
        return Err("F8 without a Trash changed the private Trash".into());
    }
    result
}
