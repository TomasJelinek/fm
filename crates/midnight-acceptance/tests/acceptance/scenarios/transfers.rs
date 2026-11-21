//! Copy (F5), Move (F6) and permanent Delete (Shift+F8) of single files and folders,
//! confirmed in the real dialogs and verified on disk.
use crate::{
    Context,
    app::{App, Config},
    input, panel,
};
use midnight_core::{CopyOperationState, OperationState, PaneId, TransferIntent};
use slint::platform::Key;
use std::{fs, path::Path, sync::Arc};

/// Recursively copy a fixture subtree (files, folders, links as links).
pub fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(from).map_err(|error| format!("{}: {error}", from.display()))?;
    if metadata.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let target = fs::read_link(from).map_err(|error| error.to_string())?;
            return std::os::unix::fs::symlink(target, to).map_err(|error| error.to_string());
        }
        #[cfg(not(unix))]
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir_all(to).map_err(|error| format!("{}: {error}", to.display()))?;
        for entry in fs::read_dir(from).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::copy(from, to)
            .map(|_| ())
            .map_err(|error| format!("copy {} to {}: {error}", from.display(), to.display()))
    }
}

/// A listing of a tree: relative path, kind and content (or link target).
pub fn snapshot(root: &Path) -> Result<Vec<(String, String)>, String> {
    fn visit(root: &Path, path: &Path, output: &mut Vec<(String, String)>) -> Result<(), String> {
        let mut entries: Vec<_> = fs::read_dir(path)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .flatten()
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let full = entry.path();
            let relative = crate::files_support::slashed(full.strip_prefix(root).unwrap_or(&full));
            let metadata = fs::symlink_metadata(&full).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(&full).map_err(|error| error.to_string())?;
                output.push((relative, format!("link -> {}", target.display())));
            } else if metadata.is_dir() {
                output.push((relative.clone(), "folder".into()));
                visit(root, &full, output)?;
            } else {
                let bytes =
                    fs::read(&full).map_err(|error| format!("{}: {error}", full.display()))?;
                output.push((
                    relative,
                    format!(
                        "{} bytes sha256 {}",
                        bytes.len(),
                        crate::encode::hex(&crate::encode::sha256(&bytes))
                    ),
                ));
            }
        }
        Ok(())
    }
    let mut output = Vec::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

fn same_tree(expected: &[(String, String)], actual_root: &Path) -> Result<(), String> {
    let actual = snapshot(actual_root)?;
    if actual == expected {
        return Ok(());
    }
    let difference: Vec<String> = expected
        .iter()
        .filter(|entry| !actual.contains(entry))
        .map(|(path, kind)| format!("missing/different {path}: {kind}"))
        .chain(
            actual
                .iter()
                .filter(|entry| !expected.contains(entry))
                .map(|(path, kind)| format!("unexpected {path}: {kind}")),
        )
        .take(12)
        .collect();
    Err(format!(
        "{} differs from the source: {difference:#?}",
        actual_root.display()
    ))
}

/// Press F5/F6 on the focused row, confirm with Enter, wait for the terminal report.
fn transfer(
    app: &mut App,
    context: &mut Context,
    key: Key,
    shot: Option<&str>,
) -> Result<Arc<midnight_core::CopyProgress>, String> {
    let start = app.copies.len();
    input::key(app.slint(), key);
    app.until("the Copy/Move dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    let moving = app.window.get_copy_draft_move();
    if moving != (key == Key::F6) {
        return Err(format!("{key:?} opened a dialog with move={moving}"));
    }
    if let Some(name) = shot {
        app.screenshot(
            context,
            name,
            if moving { "Move dialog" } else { "Copy dialog" },
        );
    }
    input::key(app.slint(), Key::Return);
    app.until("the transfer to finish", |app| {
        app.copies[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
    });
    let progress = app.copies[start..]
        .iter()
        .find(|progress| progress.state.is_terminal())
        .cloned()
        .ok_or("no terminal transfer report")?;
    if progress.state != CopyOperationState::Completed {
        return Err(format!(
            "the transfer ended {:?}: {progress:?}",
            progress.state
        ));
    }
    app.until("the dialog to close", |app| {
        app.window.get_copy_draft_token().is_empty()
    });
    Ok(progress)
}

/// Shift+F8 on the focused row, confirm with Enter, wait for completion.
fn delete(app: &mut App, context: &mut Context, shot: Option<&str>) -> Result<(), String> {
    let start = app.operations.len();
    input::chord(app.slint(), &[Key::Shift], Key::F8);
    app.until("the delete confirmation", |app| {
        !app.window.get_operation_token().is_empty()
    });
    if !app.window.get_operation_dangerous() {
        return Err("Shift+F8 did not ask for a permanent delete".into());
    }
    if let Some(name) = shot {
        app.screenshot(context, name, "Permanent delete confirmation");
    }
    input::key(app.slint(), Key::Return);
    app.until("the delete to finish", |app| {
        app.operations[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
    });
    let progress = app.operations[start..]
        .iter()
        .find(|progress| progress.state.is_terminal())
        .cloned()
        .ok_or("no terminal delete report")?;
    if progress.state != OperationState::Completed {
        return Err(format!(
            "the delete ended {:?}: {progress:?}",
            progress.state
        ));
    }
    Ok(())
}

#[track_caller]
fn wait_row(app: &mut App, pane: PaneId, name: &str, present: bool) {
    let wanted = name.to_owned();
    app.until(
        &format!(
            "{name:?} to {} the {pane:?} panel",
            if present { "appear in" } else { "leave" }
        ),
        move |app| app.names(pane).contains(&wanted) == present,
    );
}

pub fn copy_move_delete(context: &mut Context) -> Result<(), String> {
    let source = context.work.join("source");
    let destination = context.work.join("destination");
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let fixture = context.fixture.root.clone();
    for (from, to) in [
        ("text/utf8.txt", "unicode čeština 🦀.txt"),
        ("sizes/block.bin", "block.bin"),
        ("names", "names folder"),
        ("deep/level-01", "deep tree"),
        ("checksums/beta.bin", "beta.bin"),
    ] {
        copy_tree(&fixture.join(from), &source.join(to))?;
    }
    let mut app = App::start(Config::new(&source, &destination));
    let (left, right) = (PaneId::Left, PaneId::Right);

    // Copy a single file.
    let expected_file =
        fs::read(source.join("unicode čeština 🦀.txt")).map_err(|error| error.to_string())?;
    panel::select_row(&mut app, left, "unicode čeština 🦀.txt")?;
    let copied = transfer(&mut app, context, Key::F5, Some("copy-dialog"))?;
    if copied.intent != TransferIntent::Copy || copied.files_completed != 1 {
        return Err(format!("unexpected file copy report: {copied:?}"));
    }
    let written =
        fs::read(destination.join("unicode čeština 🦀.txt")).map_err(|error| error.to_string())?;
    if written != expected_file {
        return Err("the copied file's bytes differ".into());
    }
    wait_row(&mut app, right, "unicode čeština 🦀.txt", true);

    // Copy a folder with awkward names.
    let expected_folder = snapshot(&source.join("names folder"))?;
    panel::select_row(&mut app, left, "names folder")?;
    transfer(&mut app, context, Key::F5, None)?;
    same_tree(&expected_folder, &destination.join("names folder"))?;
    same_tree(&expected_folder, &source.join("names folder"))?;
    wait_row(&mut app, right, "names folder", true);

    // Move a file.
    let expected_block = fs::read(source.join("block.bin")).map_err(|error| error.to_string())?;
    panel::select_row(&mut app, left, "block.bin")?;
    let moved = transfer(&mut app, context, Key::F6, Some("move-dialog"))?;
    if moved.intent != TransferIntent::Move {
        return Err(format!("F6 reported {:?}", moved.intent));
    }
    if source.join("block.bin").exists() {
        return Err("the moved file is still in the source folder".into());
    }
    if fs::read(destination.join("block.bin")).map_err(|error| error.to_string())? != expected_block
    {
        return Err("the moved file's bytes differ".into());
    }
    wait_row(&mut app, left, "block.bin", false);
    wait_row(&mut app, right, "block.bin", true);

    // Move a folder.
    let expected_deep = snapshot(&source.join("deep tree"))?;
    panel::select_row(&mut app, left, "deep tree")?;
    transfer(&mut app, context, Key::F6, None)?;
    if source.join("deep tree").exists() {
        return Err("the moved folder is still in the source folder".into());
    }
    same_tree(&expected_deep, &destination.join("deep tree"))?;
    wait_row(&mut app, left, "deep tree", false);
    app.screenshot(
        context,
        "after-transfers",
        "Both panels after the copies and moves",
    );

    // Permanently delete a file and a folder in the right panel.
    panel::select_row(&mut app, right, "unicode čeština 🦀.txt")?;
    delete(&mut app, context, Some("delete-confirmation"))?;
    if destination.join("unicode čeština 🦀.txt").exists() {
        return Err("the deleted file still exists".into());
    }
    wait_row(&mut app, right, "unicode čeština 🦀.txt", false);
    panel::select_row(&mut app, right, "names folder")?;
    delete(&mut app, context, None)?;
    if destination.join("names folder").exists() {
        return Err("the deleted folder still exists".into());
    }
    wait_row(&mut app, right, "names folder", false);
    // The untouched entries are intact.
    for (path, expected) in [
        (
            source.join("beta.bin"),
            fs::read(fixture.join("checksums/beta.bin")).map_err(|error| error.to_string())?,
        ),
        (destination.join("block.bin"), expected_block),
    ] {
        if fs::read(&path).map_err(|error| error.to_string())? != expected {
            return Err(format!("{} changed", path.display()));
        }
    }
    same_tree(&expected_folder, &source.join("names folder"))?;
    context
        .note("F5 file + folder, F6 file + folder, Shift+F8 file + folder: all verified on disk");
    app.shutdown()
}
