//! Journey: out of an archive and back. Staged with the installed plugins.
//!
//! 1. Enter `bundle.zip` › `bundle` › `nested` › `deeper` in the right panel: the listings
//!    are the members, their sizes shown in bytes.
//! 2. F5 of the member folder `nested` to the left panel: the tree arrives byte for byte
//!    (compared with the same member read from the fixture's tar).
//! 3. Enter `nested.zip` › `inner.zip` (an archive inside an archive): F5 of its member;
//!    Backspace out with the cursor on the archive left.
//! 4. The left panel's inline tree shows the copied folder's subfolder; F2 renames it
//!    inside the tree: listed at the same depth with the cursor on it.
//! 5. F6 moves the copied folder back next to the archives (the right panel's folder).
//! 6. Ctrl+Shift+S measures it there: the size equals the bytes on disk and the sum of
//!    the member sizes the archive listing showed; the archives are unchanged.
use super::journeys_support::{self as journey, Journey, wait};
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as files, TransferSpec},
    input, panel, viewers_stage,
};
use midnight_core::{OperationState, PaneId};
use slint::platform::Key;
use std::{fs, path::Path, time::Duration};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const LIST: Duration = Duration::from_secs(30);
pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const BUNDLE: [&str; 4] = ["readme.txt", "nested", "ünïcödé 🦀.txt", "empty"];
const RENAMED: &str = "deeper renamed";

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_archive_roundtrip",
        area: "journeys",
        summary: "Into a ZIP and a ZIP inside a ZIP, F5 of a member folder and an inner member out, the inline tree of the copy, F2 inside the tree, F6 back next to the archives, Ctrl+Shift+S equal to the member sizes and the disk; archives unchanged (installed plugins).",
        run: archive_roundtrip,
        timeout: TIMEOUT,
    }
}

pub fn archive_roundtrip(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = files::scratch(context, "arch")?;
    files::stage(context, &["archives"], &root)?;
    let archives = root.join("archives");
    let out = files::scratch(context, "out")?;
    let mut config = Config::new(&out, &archives);
    config.plugins = Some(context.work.join("plugins"));
    config.size_bytes = true;
    let mut app = App::start(config);
    let mut journey = Journey::default();
    let ended = steps(&mut app, context, &mut journey, &archives, &out);
    files::dismiss_all(&mut app);
    let ended = ended.and(app.shutdown());
    journey.finish(context, ended)
}

/// The shown size of the file row `name` of `pane`, in bytes.
fn file_size(app: &App, pane: PaneId, name: &str) -> Result<u64, String> {
    app.rows(pane)
        .iter()
        .find(|row| row.name == name)
        .and_then(|row| journey::digits(&row.size))
        .ok_or_else(|| format!("the {pane:?} panel shows no size for {name:?}"))
}

fn steps(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    archives: &Path,
    out: &Path,
) -> Result<(), String> {
    let digests: Vec<(String, String)> = ["bundle.zip", "nested.zip", "source.tar"]
        .iter()
        .map(|name| journey::sha256(&archives.join(name)).map(|digest| (name.to_string(), digest)))
        .collect::<Result<_, _>>()?;
    let tar = fs::read(archives.join("source.tar"))
        .map_err(|error| files::io_error(&archives.join("source.tar"), error))?;
    let data = journey::tar_member(&tar, "bundle/nested/deeper/data.bin")
        .ok_or("source.tar has no bundle/nested/deeper/data.bin")?;

    // 1. Browse into the ZIP.
    panel::focus_list(app, RIGHT)?;
    journey::open_row(app, RIGHT, "bundle.zip", &["bundle"], LIST)?;
    journey::open_row(app, RIGHT, "bundle", &BUNDLE, LIST)?;
    journey::open_row(app, RIGHT, "nested", &["deeper", "notes.md"], LIST)?;
    let notes = file_size(app, RIGHT, "notes.md")?;
    journey::open_row(app, RIGHT, "deeper", &["data.bin"], LIST)?;
    let bin = file_size(app, RIGHT, "data.bin")?;
    journey.check(
        "the archive listing shows the members' sizes",
        if notes == 9 && bin == data.len() as u64 {
            Ok(())
        } else {
            Err(format!(
                "notes.md {notes}, data.bin {bin} (tar: {})",
                data.len()
            ))
        },
    );
    panel::wait_input(app);
    input::key(app.slint(), Key::Backspace);
    journey::wait_shown(app, RIGHT, &["deeper", "notes.md"], LIST)?;
    input::key(app.slint(), Key::Backspace);
    journey::wait_shown(app, RIGHT, &BUNDLE, LIST)?;
    journey.check(
        "Backspace puts the cursor on the member folder left",
        if files::cursor_name(app, RIGHT).as_deref() == Some("nested") {
            Ok(())
        } else {
            Err(format!("{:?}", files::cursor_name(app, RIGHT)))
        },
    );
    journey.step(format!(
        "into bundle.zip/bundle/nested/deeper: notes.md {notes} B, data.bin {bin} B"
    ));

    // 2. F5 of the member folder.
    panel::select_row(app, RIGHT, "nested")?;
    let run = files::transfer(app, context, TransferSpec::copy())?;
    files::completed(&run, "F5 of the member folder nested")?;
    journey.check(
        "notes.md copied out byte for byte",
        journey::holds(&out.join("nested/notes.md"), "# Nested\n"),
    );
    journey.check(
        "data.bin copied out byte for byte",
        if fs::read(out.join("nested/deeper/data.bin")).ok().as_deref() == Some(data.as_slice()) {
            Ok(())
        } else {
            Err("it differs from the tar's member".into())
        },
    );
    journey.check(
        "the left panel lists the copied folder",
        journey::wait_shown(app, LEFT, &["nested"], LIST),
    );
    journey.step("F5 of the member folder nested: copied out byte for byte");

    // 3. An archive inside an archive.
    journey::back_to(app, RIGHT, archives, 4)?;
    journey.check(
        "Backspace out of the archive puts the cursor on it",
        if files::cursor_name(app, RIGHT).as_deref() == Some("bundle.zip") {
            Ok(())
        } else {
            Err(format!("{:?}", files::cursor_name(app, RIGHT)))
        },
    );
    journey::open_row(app, RIGHT, "nested.zip", &["inner.zip", "outer.txt"], LIST)?;
    journey::open_row(app, RIGHT, "inner.zip", &["inner.txt"], LIST)?;
    panel::select_row(app, RIGHT, "inner.txt")?;
    let run = files::transfer(app, context, TransferSpec::copy())?;
    files::completed(&run, "F5 of the inner archive's member")?;
    journey.check(
        "the inner archive's member copied out",
        journey::holds(&out.join("inner.txt"), "Innermost.\n"),
    );
    journey::back_to(app, RIGHT, archives, 4)?;
    journey.check(
        "Backspace out of the nested archives puts the cursor on nested.zip",
        if files::cursor_name(app, RIGHT).as_deref() == Some("nested.zip") {
            Ok(())
        } else {
            Err(format!("{:?}", files::cursor_name(app, RIGHT)))
        },
    );
    journey.step("into nested.zip/inner.zip: F5 of inner.txt; Backspace out to the folder");

    // 4. The inline tree and a rename inside it.
    panel::focus_list(app, LEFT)?;
    files::toggle_tree(app, "nested")?;
    wait(app, "the inline branch of nested", LIST, |app| {
        files::tree_depth(app, "deeper") == Some(1)
    })?;
    panel::select_row(app, LEFT, "deeper")?;
    let start = files::named_operation(app, &[], Key::F2, RENAMED)?;
    let progress = files::wait_operation(app, start);
    journey.check(
        "F2 inside the tree completes",
        if progress.state == OperationState::Completed {
            Ok(())
        } else {
            Err(format!("{:?}: {:?}", progress.state, progress.errors))
        },
    );
    journey.check(
        "the renamed folder is on disk with its member",
        if fs::read(out.join("nested").join(RENAMED).join("data.bin"))
            .ok()
            .as_deref()
            == Some(data.as_slice())
        {
            journey::absent(&out.join("nested/deeper"))
        } else {
            Err("the renamed folder does not hold data.bin".into())
        },
    );
    journey.check(
        "the tree lists the new name at the same depth with the cursor on it",
        wait(app, "the renamed row", LIST, |app| {
            files::tree_depth(app, RENAMED) == Some(1)
                && files::cursor_name(app, LEFT).as_deref() == Some(RENAMED)
        }),
    );
    app.screenshot(
        context,
        "tree-renamed",
        "The copied folder's branch, renamed inside the tree",
    );
    journey.step("the inline tree shows nested/deeper; F2 renamed it in place");

    // 5. Move it back next to the archives.
    panel::select_row(app, LEFT, "nested")?;
    let run = files::transfer(app, context, TransferSpec::moving())?;
    files::completed(&run, "F6 of nested back next to the archives")?;
    journey.check(
        "the folder moved next to the archives",
        journey::holds(&archives.join("nested/notes.md"), "# Nested\n")
            .and_then(|()| journey::absent(&out.join("nested"))),
    );
    journey.check(
        "the left panel no longer lists it, the right does",
        wait(app, "the panels to follow the move", LIST, |app| {
            !journey::shown(app, LEFT).contains("nested")
                && journey::shown(app, RIGHT).contains("nested")
        }),
    );
    journey.step("F6 moved nested back next to the archives");

    // 6. Its size there.
    panel::focus_list(app, RIGHT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    input::chord(app.slint(), &[Key::Control, Key::Shift], "s");
    let measured = |app: &App| {
        app.rows(RIGHT)
            .iter()
            .find(|row| row.name == "nested" && row.directory)
            .filter(|row| row.size_state == 0 && !row.size_running)
            .and_then(|row| journey::digits(&row.size))
    };
    wait(app, "the size of nested", LIST, |app| {
        measured(app).is_some()
    })?;
    let size = measured(app).unwrap_or_default();
    let disk = journey::disk_size(&archives.join("nested"));
    journey.check(
        "the moved folder's size is its bytes on disk and its members' sizes",
        if size == disk && size == notes + bin {
            Ok(())
        } else {
            Err(format!(
                "shown {size}, disk {disk}, members {notes} + {bin}"
            ))
        },
    );
    for (name, digest) in &digests {
        journey.check(
            &format!("{name} is unchanged"),
            match journey::sha256(&archives.join(name)) {
                Ok(now) if now == *digest => Ok(()),
                other => Err(format!("{other:?}")),
            },
        );
    }
    app.screenshot(
        context,
        "sizes",
        "The moved folder measured next to the archives",
    );
    journey.step(format!(
        "Ctrl+Shift+S: nested is {size} B = {notes} + {bin}"
    ));
    Ok(())
}
