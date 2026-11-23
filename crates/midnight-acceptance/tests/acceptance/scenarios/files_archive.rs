//! Archive panels as Copy/Move sources and targets, through the installed archive
//! filesystem plugin (a staged process, `viewers_stage::run`): a multi-selection of a
//! file, a nested folder and an empty folder copied out of a ZIP, the same tree copied
//! out of a tar.gz (the two copies must be equal; each copy counts its members before it
//! copies them and names each by its path inside the archive), F6 of a member refused with its
//! reason, and a native file copied into an archive panel refused with the archive left
//! unchanged. Then F5 of a password-protected archive's members (T-C15): the archive's
//! password is asked before the Copy dialog; Cancel copies the other members and skips
//! the protected ones without asking about each; a wrong password is refused in the
//! dialog, and the right one copies every member's exact bytes; the archive stays
//! unlocked while it is open and asks again once it was left and entered again.
use crate::{
    Context,
    app::{App, Config},
    encode::{self, AesStrength, Entry, Method},
    files_support::{self as support, compare_trees, io_error, tree_facts},
    input, panel, viewers_stage,
    viewers_support::{self as viewers, Dismiss, Submit},
};
use midnight_core::{CopyOperationState, CopyProgress, DiscoveryState, PaneId};
use slint::platform::Key;
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
/// A hang guard for the password dialog and its answers.
const LIMIT: Duration = Duration::from_secs(30);
/// The password of `locked.zip`'s protected members, non-ASCII as a user may type it.
const PASSWORD: &str = "WP3-heslo-žluť-🔑";
/// What the password dialog says after F5 of protected members (T-C15).
const COPY_PURPOSE: &str =
    "Enter its password to copy its protected members; without it they are skipped.";
const WRONG_PASSWORD: &str = "Wrong password — try again.";
/// `locked.zip`'s members and their bytes: a ZipCrypto and an AES-256 member protected by
/// [`PASSWORD`], and a plain one.
const LOCKED: [(&str, &str, bool); 3] = [
    ("aes.txt", "AES-256 secret, deflated.\n", true),
    ("plain.txt", "Plain member.\n", false),
    ("zc.txt", "ZipCrypto secret.\n", true),
];

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
/// The job must count the members before it copies them, so that its progress is a
/// share of known totals with an estimate, and show each member by its path inside
/// `archive` (the archive file's name), not by its name alone.
fn copy_out(
    app: &mut App,
    context: &mut Context,
    destination: &Path,
    shot: &str,
    archive: &str,
    failures: &mut Vec<String>,
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
    let counted = &done.discovery;
    if counted.state != DiscoveryState::Complete
        || counted.files != done.files_completed
        || counted.bytes != Some(done.copied_bytes)
    {
        failures.push(format!(
            "F5 out of {archive} did not count its members first: {counted:?}, copied {}              files and {} bytes",
            done.files_completed, done.copied_bytes
        ));
    }
    let id = done.id;
    let shown: Vec<_> = app.copies[start..]
        .iter()
        .filter(|progress| progress.id == id)
        .filter_map(|progress| progress.current_path.clone())
        .collect();
    if let Some(path) = shown.iter().find(|path| {
        !path
            .components()
            .any(|part| part.as_os_str() == std::ffi::OsStr::new(archive))
    }) {
        failures.push(format!(
            "F5 out of {archive} showed {} without the archive's path",
            path.display()
        ));
    }
    context.note(format!(
        "F5 out of {archive}: counted {} files, {} folders, {:?} bytes first; {} reports          named a member, the last {:?}",
        counted.files,
        counted.directories,
        counted.bytes,
        shown.len(),
        shown.last()
    ));
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
    let done = copy_out(
        &mut app,
        context,
        &zip_out,
        "copy-from-zip",
        "bundle.zip",
        &mut failures,
    )?;
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
    let done = copy_out(
        &mut app,
        context,
        &tar_out,
        "copy-from-tar",
        "source.tar.gz",
        &mut failures,
    )?;
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

    // T-C15: F5 of a password-protected archive's members, with the right panel back on a
    // native folder (F5 towards an archive panel is refused before anything is asked).
    back_to(&mut app, RIGHT, &root.join("archives"))?;
    panel::focus_list(&mut app, LEFT)?;
    crate::scenarios::files_transfers::navigate_to(&mut app, LEFT, &root.join("archives"))?;
    if let Err(error) = locked_copies(&mut app, context, &root, &mut failures) {
        failures.push(format!("T-C15: {error}"));
    }
    support::dismiss_all(&mut app);
    app.shutdown()?;
    support::report(&failures)
}

/// `locked.zip` in `folder` ([`LOCKED`]).
fn write_locked(folder: &Path) -> Result<(), String> {
    let [
        (aes, aes_text, _),
        (plain, plain_text, _),
        (crypto, crypto_text, _),
    ] = LOCKED;
    let zip = folder.join("locked.zip");
    let bytes = encode::zip_entries(&[
        Entry::file(aes, aes_text)
            .method(Method::Deflated)
            .aes(AesStrength::Aes256, PASSWORD),
        Entry::file(plain, plain_text),
        Entry::file(crypto, crypto_text).zip_crypto(PASSWORD),
    ]);
    fs::write(&zip, bytes).map_err(|error| io_error(&zip, error))
}

/// Mark exactly `names` in the left panel.
fn mark_members(app: &mut App, names: &[&str]) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    for name in names {
        panel::select_row(app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    app.settle();
    let marked = support::selected_names(app, LEFT);
    let mut wanted: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    wanted.sort();
    let mut found = marked.clone();
    found.sort();
    if found != wanted {
        return Err(format!("Insert marked {marked:?}, not {names:?}"));
    }
    Ok(())
}

/// How the password dialog that F5 opened is answered.
#[derive(Clone, Copy)]
enum Answer {
    /// Its Cancel button: the Copy dialog follows, and the copy skips the protected
    /// members.
    Cancel,
    /// A wrong password by keyboard (refused in the dialog), then [`PASSWORD`] by
    /// pointer.
    Unlock,
}

/// F5 of the left panel's marked members of `locked.zip` into `destination`: the
/// password dialog first, its sentence checked, answered by `answer`, then the Copy
/// dialog, confirmed with Enter. The job's last report; Err when it asked a question.
fn copy_locked(
    app: &mut App,
    destination: &Path,
    answer: Answer,
    failures: &mut Vec<String>,
) -> Result<Arc<CopyProgress>, String> {
    panel::wait_input(app);
    let start = app.copies.len();
    input::key(app.slint(), Key::F5);
    let asked = viewers::wait_password_dialog(app, LIMIT)?;
    let sentence = format!("locked.zip is password-protected. {COPY_PURPOSE}");
    if asked.sentence() != sentence || !asked.error.is_empty() {
        failures.push(format!(
            "T-C15: F5 asked {:?} (reason {:?}), not {sentence:?}",
            asked.sentence(),
            asked.error
        ));
    }
    if !app.window.get_copy_draft_token().is_empty() {
        failures.push("T-C15: the Copy dialog opened before the password was asked".into());
    }
    match answer {
        Answer::Cancel => viewers::dismiss_password(app, Dismiss::Cancel)?,
        Answer::Unlock => {
            match viewers::submit_password(app, "not the password", Submit::Enter, LIMIT)? {
                Some(refused) if refused.error == WRONG_PASSWORD => {}
                other => failures.push(format!(
                    "T-C15: a wrong password left {other:?}, not {WRONG_PASSWORD:?}"
                )),
            }
            if let Some(refused) = viewers::submit_password(app, PASSWORD, Submit::Pointer, LIMIT)?
            {
                return Err(format!("the right password was refused: {refused:?}"));
            }
        }
    }
    app.until("the Copy dialog after the password dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    input::fill(
        app.slint(),
        "Destination folder",
        &destination.to_string_lossy(),
    )?;
    app.settle();
    input::key(app.slint(), Key::Return);
    app.until(
        "the copy of locked.zip's members to end or ask",
        move |app| {
            support::decision_window(app).is_some()
                || support::job(app, start).is_some_and(|job| job.state.is_terminal())
        },
    );
    if let Some(window) = support::decision_window(app) {
        let text = support::describe_window(&window);
        support::answer(app, &window, &support::Choice::CancelJob)?;
        support::wait_job(app, start);
        support::dismiss_all(app);
        return Err(format!("the copy asked {text} (answered Cancel job)"));
    }
    let done = support::wait_job(app, start);
    support::dismiss_all(app);
    Ok(done)
}

/// T-C15 in `root`'s `archives/` folder, which the left panel shows.
fn locked_copies(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let folder = root.join("archives");
    write_locked(&folder)?;
    let names: Vec<&str> = LOCKED.iter().map(|(name, ..)| *name).collect();
    support::wait_row(app, LEFT, "locked.zip", true);
    open(app, LEFT, "locked.zip", &names)?;

    // Cancel: the other members are copied, the protected ones skipped without a
    // question about each.
    let skipped = support::scratch(context, "locked-cancelled")?;
    mark_members(app, &names)?;
    let done = copy_locked(app, &skipped, Answer::Cancel, failures)?;
    context.note(format!(
        "F5 of locked.zip's members, password dialog cancelled: {:?}, skipped {}, failed {}, \
         message {:?}, notes {:?}",
        done.state, done.skipped, done.failed, done.message, done.notes
    ));
    let arrived = support::names_on_disk(&skipped);
    if matches!(
        done.state,
        CopyOperationState::Cancelled | CopyOperationState::Failed
    ) || arrived != ["plain.txt"]
        || fs::read_to_string(skipped.join("plain.txt"))
            .ok()
            .as_deref()
            != Some(LOCKED[1].1)
    {
        failures.push(format!(
            "T-C15: without the password the copy ended {:?} with {arrived:?}, not the plain \
             member alone",
            done.state
        ));
    }

    // A wrong password is refused in the dialog; the right one copies every member.
    let unlocked = support::scratch(context, "locked-unlocked")?;
    mark_members(app, &names)?;
    let done = copy_locked(app, &unlocked, Answer::Unlock, failures)?;
    let differing: Vec<&str> = LOCKED
        .iter()
        .filter(|(name, text, _)| {
            fs::read_to_string(unlocked.join(name)).ok().as_deref() != Some(*text)
        })
        .map(|(name, ..)| *name)
        .collect();
    if done.state != CopyOperationState::Completed || !differing.is_empty() {
        failures.push(format!(
            "T-C15: with the password the copy ended {:?}; these members differ: {differing:?}",
            done.state
        ));
    }

    // Unlocked while it is open: F5 goes straight to the Copy dialog.
    mark_members(app, &["zc.txt"])?;
    panel::wait_input(app);
    input::key(app.slint(), Key::F5);
    app.until("the Copy dialog or the password dialog", |app| {
        !app.window.get_copy_draft_token().is_empty() || viewers::password_dialog(app).is_some()
    });
    if viewers::password_dialog(app).is_some() {
        failures.push("T-C15: the open, unlocked archive asked for its password again".into());
        viewers::dismiss_password(app, Dismiss::Escape)?;
    }
    support::dismiss_all(app);

    // Left and entered again, it asks again; Escape keeps it locked.
    back_to(app, LEFT, &folder)?;
    open(app, LEFT, "locked.zip", &names)?;
    mark_members(app, &["zc.txt"])?;
    panel::wait_input(app);
    let start = app.copies.len();
    input::key(app.slint(), Key::F5);
    match viewers::wait_password_dialog(app, LIMIT) {
        Ok(_) => viewers::dismiss_password(app, Dismiss::Escape)?,
        Err(error) => failures.push(format!(
            "T-C15: entered again, the archive did not ask for its password: {error}"
        )),
    }
    support::dismiss_all(app);
    if app.copies.len() != start {
        failures.push("T-C15: dismissing the dialogs started a copy".into());
    }
    back_to(app, LEFT, &folder)?;
    Ok(())
}
