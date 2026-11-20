//! Archive panels as Copy/Move sources and targets, through the installed archive
//! filesystem plugin (a staged process, `viewers_stage::run`): a multi-selection of a
//! file, a nested folder and an empty folder copied out of a ZIP, the same tree copied
//! out of a tar.gz (the two copies must be equal), F6 of a member refused with its
//! reason, and a native file copied into an archive panel refused with the archive left
//! unchanged.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, compare_trees, io_error, tree_facts},
    input, panel, viewers_stage,
};
use midnight_core::{CopyOperationState, PaneId};
use slint::platform::Key;
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;

pub fn archive_transfers(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn shown(app: &App, pane: PaneId) -> BTreeSet<String> {
    app.names(pane)
        .into_iter()
        .filter(|name| name != "..")
        .collect()
}

/// Enter `name` with Enter and wait for a finished listing of exactly `expected`.
fn open(app: &mut App, pane: PaneId, name: &str, expected: &[&str]) -> Result<(), String> {
    panel::select_row(app, pane, name)?;
    input::key(app.slint(), Key::Return);
    let wanted: BTreeSet<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    let started = Instant::now();
    loop {
        app.settle();
        if !app.loading(pane) && shown(app, pane) == wanted {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err(format!(
                "entering {name}: the panel shows {:?}, not {wanted:?} (error {:?})",
                shown(app, pane),
                app.error(pane)
            ));
        }
        app.idle(Duration::from_millis(20));
    }
}

/// Backspace out of archive folders until `pane` shows the native `folder`.
fn back_to(app: &mut App, pane: PaneId, folder: &Path) -> Result<(), String> {
    for _ in 0..4 {
        if app.location(pane).as_deref() == Some(folder) {
            return Ok(());
        }
        panel::focus_list(app, pane)?;
        let before = app.location(pane);
        input::key(app.slint(), Key::Backspace);
        let started = Instant::now();
        while app.location(pane) == before && started.elapsed() < Duration::from_secs(10) {
            app.idle(Duration::from_millis(20));
        }
        app.settle();
    }
    if app.location(pane).as_deref() == Some(folder) {
        Ok(())
    } else {
        Err(format!(
            "Backspace did not return to {}: the panel shows {:?}",
            folder.display(),
            app.location(pane)
        ))
    }
}

/// F5 of the left panel's selection into `destination`, confirmed; its last report.
fn copy_out(
    app: &mut App,
    context: &mut Context,
    destination: &Path,
    shot: &str,
) -> Result<std::sync::Arc<midnight_core::CopyProgress>, String> {
    let start = support::start_transfer(
        app,
        context,
        support::TransferSpec {
            destination: Some(destination),
            shot: Some(shot),
            ..support::TransferSpec::copy()
        },
    )?;
    let done = support::wait_job(app, start);
    app.settle();
    support::dismiss_all(app);
    Ok(done)
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = support::scratch(context, "archives")?;
    support::stage(context, &["archives"], &root)?;
    let zip_out = support::scratch(context, "from-zip")?;
    let tar_out = support::scratch(context, "from-tar")?;
    let native = support::scratch(context, "native")?;
    fs::write(native.join("plain.txt"), "plain\n").map_err(|error| io_error(&native, error))?;
    let mut config = Config::new(root.join("archives"), &native);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let mut failures = Vec::new();

    // A file, a nested folder and an empty folder marked inside a ZIP, copied out.
    open(&mut app, LEFT, "bundle.zip", &["bundle"])?;
    open(
        &mut app,
        LEFT,
        "bundle",
        &["readme.txt", "nested", "ünïcödé 🦀.txt", "empty"],
    )?;
    panel::focus_list(&mut app, LEFT)?;
    for name in ["readme.txt", "nested", "empty"] {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    let done = copy_out(&mut app, context, &zip_out, "copy-from-zip")?;
    if done.state != CopyOperationState::Completed || done.failed != 0 {
        failures.push(format!(
            "F5 of three ZIP members ended {:?} (failed {})",
            done.state, done.failed
        ));
    }
    for (path, bytes) in [
        ("readme.txt", Some(b"Inside the bundle.\n".as_slice())),
        ("nested/notes.md", Some(b"# Nested\n".as_slice())),
        ("nested/deeper/data.bin", None),
    ] {
        match (fs::read(zip_out.join(path)), bytes) {
            (Ok(found), Some(wanted)) if found == wanted => {}
            (Ok(found), None) if found.len() == 4096 => {}
            (found, _) => failures.push(format!(
                "the ZIP member {path} was copied as {:?}",
                found.map(|bytes| bytes.len())
            )),
        }
    }
    if !zip_out.join("empty").is_dir() || zip_out.join("ünïcödé 🦀.txt").exists() {
        failures.push(format!(
            "the copied ZIP selection holds {:?}",
            support::names_on_disk(&zip_out)
        ));
    }

    // F6 of a member is refused with its reason, before any dialog.
    panel::select_row(&mut app, LEFT, "ünïcödé 🦀.txt")?;
    input::chord(app.slint(), &[Key::Control], "d");
    let start = app.copies.len();
    panel::wait_input(&mut app);
    input::key(app.slint(), Key::F6);
    app.idle(Duration::from_millis(400));
    let dialog = !app.window.get_copy_draft_token().is_empty();
    let notice = support::shown_notice(&app);
    if dialog
        || app.copies.len() != start
        || !notice
            .as_ref()
            .is_some_and(|(title, body)| format!("{title} {body}").contains("cannot be moved"))
    {
        failures.push(format!(
            "F6 of an archive member: dialog {dialog}, jobs {}, notice {notice:?}",
            app.copies.len() - start
        ));
    }
    context.note(format!("F6 of an archive member: {notice:?}"));
    support::dismiss_all(&mut app);

    // The same tree out of a tar.gz equals the ZIP's copy.
    back_to(&mut app, LEFT, &root.join("archives"))?;
    open(&mut app, LEFT, "source.tar.gz", &["bundle"])?;
    panel::select_row(&mut app, LEFT, "bundle")?;
    let done = copy_out(&mut app, context, &tar_out, "copy-from-tar")?;
    if done.state != CopyOperationState::Completed {
        failures.push(format!("F5 of the tar.gz's bundle ended {:?}", done.state));
    }
    let from_tar = tree_facts(&tar_out.join("bundle"))?;
    let differences = compare_trees(
        &tree_facts(&zip_out)?,
        &{
            let mut subset = from_tar.clone();
            subset.retain(|path, _| {
                ["readme.txt", "nested", "empty"]
                    .iter()
                    .any(|root| path == root || path.starts_with(&format!("{root}/")))
            });
            subset
        },
        false,
    );
    if !differences.is_empty() {
        failures.push(format!(
            "the tar.gz's tree differs from the ZIP's: {differences:?}"
        ));
    }
    back_to(&mut app, LEFT, &root.join("archives"))?;

    // A native file copied into an archive panel is refused; the archive is unchanged.
    let zip = root.join("archives/bundle.zip");
    let zip_bytes = fs::read(&zip).map_err(|error| io_error(&zip, error))?;
    panel::focus_list(&mut app, RIGHT)?;
    crate::scenarios::files_transfers::navigate_to(&mut app, RIGHT, &root.join("archives"))?;
    open(&mut app, RIGHT, "bundle.zip", &["bundle"])?;
    panel::focus_list(&mut app, LEFT)?;
    crate::scenarios::files_transfers::navigate_to(&mut app, LEFT, &native)?;
    panel::select_row(&mut app, LEFT, "plain.txt")?;
    let start = app.copies.len();
    panel::wait_input(&mut app);
    input::key(app.slint(), Key::F5);
    app.idle(Duration::from_millis(400));
    let mut refusal = support::shown_notice(&app).map(|(title, body)| format!("{title}: {body}"));
    if !app.window.get_copy_draft_token().is_empty() {
        input::key(app.slint(), Key::Return);
        app.settle();
        app.idle(Duration::from_millis(400));
        let error = app.window.get_copy_draft_error().to_string();
        if !error.is_empty() {
            refusal = Some(error);
        }
        support::dismiss_all(&mut app);
    }
    if app.copies.len() > start {
        let done = support::wait_job(&mut app, start);
        if done.state == CopyOperationState::Completed {
            failures.push("a native file was copied into an archive panel".into());
        }
        refusal = refusal.or(Some(format!("the job ended {:?}", done.state)));
    }
    if refusal.is_none() {
        failures.push("F5 into an archive panel was neither refused nor reported".into());
    }
    if fs::read(&zip).ok().as_deref() != Some(zip_bytes.as_slice()) {
        failures.push("copying into the archive panel changed bundle.zip".into());
    }
    context.note(format!(
        "F5 of a native file into an archive panel: {refusal:?}"
    ));
    support::dismiss_all(&mut app);
    app.shutdown()?;
    support::report(&failures)
}
