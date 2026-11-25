//! The Copy/Move dialog's Links choice where the destination decides it, on volumes a
//! plain run does not have. Each part needs a folder named by an environment variable
//! (audit H5) and reports `SKIP: <variable> not set` without it; with no part run the
//! scenario is `SKIP`:
//! - `MIDNIGHT_ACCEPTANCE_FAT_DIR` (a FAT32 or exFAT folder): the dialog proposes "Copy
//!   the files links point to" with the reason "This drive cannot store links (FAT…), so
//!   the files they point to are copied."; choosing "Keep links as links" shows the
//!   warning, and the first Enter of a Copy holds with it ("… Select Copy again to keep
//!   links anyway, …"), the second starts it. A Windows Copy then leaves each link out
//!   without a question, names it in the job details and completes ("N links left out
//!   where no links are created; …", finding W-3); Linux refuses the destination for the
//!   files (it has no O_TMPFILE). A Move from another drive is refused in the dialog
//!   before any job ("Items from another drive cannot be moved to this drive (FAT…): …",
//!   finding O-1).
//! - `MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR` (a folder on a removable drive that stores
//!   links): the reason "Removable drive: …"; a selection already on that drive keeps
//!   links; a project moved there with the defaults arrives without links, unless the
//!   drive refuses a Move from another drive in the dialog (O-1).
//! - `MIDNIGHT_ACCEPTANCE_NETWORK_DIR` (Windows, a folder on a share): the reason
//!   "Network location: …", the warning and the hold, then the Copy completes leaving
//!   each link out as on FAT; a Move from another drive is refused in the dialog ("… to a
//!   network location: …").
//! - A typed destination of either kind while the dialog proposed a fixed local folder:
//!   the first Enter holds with "The link handling changed for this folder. <reason>
//!   Select Copy again to start." and shows the new default, the second starts; the
//!   reverse says "Links are kept as links here."; a choice the user made never holds.
//! - `MIDNIGHT_ACCEPTANCE_OTHER_EXT_DIR` (Linux, another ext filesystem; on Windows the
//!   private installation's drive, `MIDNIGHT_ACCEPTANCE_STAGE_ROOT`, when the run is on
//!   another one): a project moved there keeping links has its links recreated with
//!   the same targets, the originals removed and the targets untouched.
//! - `MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR` (Linux, a folder without O_TMPFILE, such as
//!   one under /mnt/c in WSL): a Copy there is refused in words.
//! - `MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS` (Windows): a folder holding `dir-link`, a
//!   directory symbolic link to its `dir`, and `file-link`, a file symbolic link to its
//!   `file.txt`, made in Developer Mode or elevated. Kept as links, without the right to
//!   create symbolic links the directory link becomes a junction with a note and the file
//!   link asks "Creating a symbolic link to a file needs Developer Mode or administrator
//!   rights…" with Skip and Skip all, no Retry; with the right both are recreated.
use super::files_links::{describe, identities, is_link_to, outcome, project};
use crate::{
    Context, Scenario,
    app::{App, Config},
    files_support::{self as support, Choice, LinkChoice, LinkInput, LinkView, TransferSpec},
    panel,
};
use midnight_core::{CopyOperationState, PaneId};
use slint::platform::Key;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const LEFT: PaneId = PaneId::Left;

/// A project's links: each name and the target it reads back (`files_links::project`).
type Links = Vec<(&'static str, PathBuf)>;

/// Judges a finished Copy or Move (F5 or F6) of a project from `base` to `destination`:
/// its failures.
type Judge = dyn Fn(Key, &Path, &Path, &Result<support::TransferRun, String>) -> Vec<String>;

/// This module's entries in the registry (it owns them; `scenarios/mod.rs` lists the
/// module).
pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "files_links_volumes",
        // Exercises host-provided FAT, removable, network and cross-volume devices.
        isolation: crate::Isolation::Exclusive,
        area: "transfers",
        summary: "The Links choice where the destination decides it (each part gated by its MIDNIGHT_ACCEPTANCE_* folder, SKIP without): FAT/exFAT and a Windows network location propose copying the files with their reason, warn about keeping links and hold the first Enter of a Copy, which then leaves each link out without a question and names it; a Move from another drive there is refused in the dialog; a removable drive proposes copying the files unless the selection is on it, and a project moved there arrives without links or is refused; a typed folder whose default differs holds once and shows its default; a Move keeping links to another volume recreates them; a Linux folder without O_TMPFILE is refused in words; Windows symbolic links kept as links become a junction with a note or ask, without the right to create them.",
        run: links_volumes,
        timeout: Duration::from_secs(10 * 60),
    }]
}

/// Run `part` in this run's own folder in the gated volume of `var`
/// (`fixtures::gated`, `SKIP: <var> not set` without it), which goes away afterwards.
fn in_gated(
    context: &mut Context,
    var: &str,
    part: impl FnOnce(&mut Context, &Path) -> Result<Vec<String>, String>,
) -> Result<Vec<String>, String> {
    let folder = crate::fixtures::gated(context, var)?;
    part(context, &folder)
}

/// Run `part` in a new folder of `root` and remove that folder afterwards, as
/// `fixtures::gated` does for a gated volume.
fn in_folder(
    context: &mut Context,
    root: &Path,
    part: impl FnOnce(&mut Context, &Path) -> Result<Vec<String>, String>,
) -> Result<Vec<String>, String> {
    let run = context
        .run_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = crate::fixtures::create_fresh(root, &format!("midnight-acceptance-{run}"))?;
    let result = part(context, &folder);
    crate::fixtures::restore_owner_write(&folder);
    if let Err(error) = crate::fixtures::remove_tree(&folder) {
        context.note(format!(
            "the folder {} was not removed: {error}",
            folder.display()
        ));
    }
    result
}

fn links_volumes(context: &mut Context) -> Result<(), String> {
    type Part = fn(&mut Context) -> Result<Vec<String>, String>;
    let parts: [(&str, Part); 6] = [
        ("FAT", fat),
        ("removable drive", removable),
        ("network location", network),
        ("Move to another volume", other_volume),
        ("no O_TMPFILE", no_tmpfile),
        ("Windows symbolic links", windows_symlinks),
    ];
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    for (name, part) in parts {
        match part(context) {
            Ok(found) => {
                context.note(format!("{name}: ran"));
                failures.extend(
                    found
                        .into_iter()
                        .map(|failure| format!("{name}: {failure}")),
                );
            }
            Err(reason) if reason.starts_with("SKIP: ") => {
                context.note(format!("{name}: {reason}"));
                skipped.push(format!("{name} ({})", &reason["SKIP: ".len()..]));
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    if failures.is_empty() && skipped.len() == parts.len() {
        return Err(format!("SKIP: no gated volume: {}", skipped.join("; ")));
    }
    support::report(&failures)
}

/// What the Move dialog does with a Move from another drive to a destination (O-1).
enum MoveRule {
    /// Admitted, or refused in the O-1 words where a provider refuses it: the native
    /// adapters refuse none (a Move from another drive goes wherever Copy goes).
    Either,
}

/// The link handling a destination proposes and explains: its reason, and its warnings
/// when links are kept there although none can be stored (Copy, then Move).
struct Proposal {
    /// Names the screenshots.
    label: &'static str,
    reason: String,
    warnings: Option<[String; 2]>,
    moves: MoveRule,
}

impl Proposal {
    /// A drive that cannot store links, named `filesystem` as the dialog names it.
    fn cannot_store(filesystem: &str) -> Self {
        let warning = |stays: &str| {
            format!(
                "This drive cannot store links ({filesystem}): each link is left out{stays}, \
                 and the job details name it."
            )
        };
        Self {
            label: "fat",
            reason: format!(
                "This drive cannot store links ({filesystem}), so the files they point to \
                 are copied."
            ),
            warnings: Some([warning(""), warning(" and stays where it is")]),
            // A Move from another drive goes wherever Copy goes (O-1 superseded); on
            // Linux Copy itself refuses a FAT drive (no O_TMPFILE).
            moves: MoveRule::Either,
        }
    }

    fn network() -> Self {
        let warning = |stays: &str| {
            format!(
                "No links are created at this network location: each link is left out\
                 {stays}, and the job details name it."
            )
        };
        Self {
            label: "network",
            reason: "Network location: no links are created there, so the files they point \
                     to are copied."
                .into(),
            warnings: Some([warning(""), warning(" and stays where it is")]),
            moves: MoveRule::Either,
        }
    }

    fn removable() -> Self {
        Self {
            label: "removable",
            reason: "Removable drive: the files links point to are copied, so that no link \
                     there points back to this computer."
                .into(),
            warnings: None,
            moves: MoveRule::Either,
        }
    }

    /// The warning for keeping links with `key` (F5 or F6), if any.
    fn warning(&self, key: Key) -> Option<&str> {
        self.warnings
            .as_ref()
            .map(|[copy, moving]| if key == Key::F6 { moving } else { copy }.as_str())
    }
}

/// Differences of the view the dialog `offered` from a proposal to copy the files with
/// `proposal`'s reason, shown and read by a screen reader.
fn proposes_copy(offered: Option<&LinkView>, proposal: &Proposal) -> Vec<String> {
    let Some(view) = offered else {
        return vec!["the dialog offered no Links choice".into()];
    };
    let mut differences = Vec::new();
    if view.chosen() != Some(LinkChoice::CopyFiles) {
        differences.push(format!("it proposed {:?}", view.choice));
    }
    // The reason is the Links box's tooltip and what a screen reader reads for it (below),
    // not a line of its own.
    if view.reason != proposal.reason {
        differences.push(format!(
            "its reason is {:?}, not {:?}",
            view.reason, proposal.reason
        ));
    }
    if !view.warning.is_empty() || view.description.as_deref() != Some(proposal.reason.as_str()) {
        differences.push(format!(
            "it warns {:?} and is read as {:?}",
            view.warning, view.description
        ));
    }
    differences
}

/// A project (`files_links::project`) in `src` of the run's work folder, the panel's
/// left folder.
fn source_project(context: &Context, name: &str) -> Result<(PathBuf, Links), String> {
    let base = support::scratch(context, name)?;
    let links = project(&base)?;
    Ok((base, links))
}

/// F5 and F6 of a project to `folder`, a destination that copies the files by default
/// (`proposal`): the proposal, then keeping links: its warning, the first Enter of a
/// Copy holding with it (when links cannot be stored there), the second starting; a Move
/// from another drive refused in the dialog where `proposal` says so (O-1). `after`
/// judges the job. Then the typed-destination holds of the same folder.
fn proposed_copy(
    context: &mut Context,
    folder: &Path,
    proposal: &Proposal,
    after: &Judge,
) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    for key in [Key::F5, Key::F6] {
        let verb = if key == Key::F6 { "Move" } else { "Copy" };
        let (base, _) = source_project(context, &format!("{verb}-source"))?;
        let destination = crate::fixtures::create_fresh(folder, verb)?;
        let mut app = App::start(Config::new(&base, &destination));
        panel::select_row(&mut app, LEFT, "proj")?;
        // No destination refuses a Move from another drive up front any more (O-1
        // superseded); the words are checked where a provider still refuses one.
        let refusal: Option<&str> = match proposal.moves {
            MoveRule::Either => None,
        };
        let warning = proposal.warning(key);
        let shot = format!("{}-{}-keep-links", proposal.label, verb.to_lowercase());
        let jobs = app.copies.len();
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                key,
                links: Some(LinkChoice::Keep),
                link_input: LinkInput::Pointer,
                hold: warning.is_some() && refusal.is_none(),
                choices: vec![Choice::CancelJob, Choice::CancelJob],
                shot: Some(&shot),
                ..TransferSpec::copy()
            },
        );
        context.note(format!(
            "{verb} of proj to {} keeping links: {}",
            destination.display(),
            outcome(&run)
        ));
        // O-1: a Move from another drive that this destination cannot take is refused by
        // the dialog, whatever the Links choice, and nothing starts or moves.
        let refused_up_front = matches!(&run, Err(error) if support::is_move_refusal(error))
            && app.copies.len() == jobs
            && fs::symlink_metadata(base.join("proj")).is_ok();
        if let Some(words) = refusal {
            let wanted = format!("the dialog refused the transfer: {words}");
            if !refused_up_front || !matches!(&run, Err(error) if *error == wanted) {
                failures.push(format!(
                    "{verb}: finding O-1: a Move from another drive was not refused before \
                     any job with {words:?}: {} ({} job(s))",
                    outcome(&run),
                    app.copies.len() - jobs
                ));
            }
            support::dismiss_all(&mut app);
            app.shutdown()?;
            continue;
        }
        if key == Key::F6 && matches!(proposal.moves, MoveRule::Either) && refused_up_front {
            context.note(format!(
                "{verb}: refused in the dialog before any job (O-1)"
            ));
            support::dismiss_all(&mut app);
            app.shutdown()?;
            continue;
        }
        match &run {
            Err(error) => failures.push(format!("{verb} keeping links: {error}")),
            Ok(done) => {
                for difference in proposes_copy(done.offered.as_ref(), proposal) {
                    failures.push(format!("{verb}: {difference}"));
                }
                let shown = done.links.as_ref();
                match warning {
                    Some(warning) => {
                        if !shown.is_some_and(|view| {
                            view.warning == warning
                                && view.warning_shown
                                && view.description.as_deref() == Some(warning)
                        }) {
                            failures.push(format!(
                                "{verb}: keeping links shows {shown:?}, not the warning \
                                 {warning:?}"
                            ));
                        }
                        let held = format!(
                            "{warning} Select {verb} again to keep links anyway, or choose \
                             \u{201c}{}\u{201d}.",
                            support::COPY_LINKED_FILES
                        );
                        if !done
                            .hold
                            .as_ref()
                            .is_some_and(|hold| hold.text == held && hold.shown)
                        {
                            failures.push(format!(
                                "{verb}: the first Enter held with {:?}, not {held:?}",
                                done.hold
                            ));
                        }
                    }
                    None => {
                        if shown.is_some_and(|view| !view.warning.is_empty()) {
                            failures.push(format!(
                                "{verb}: keeping links warns {shown:?} where links can be \
                                 stored"
                            ));
                        }
                    }
                }
            }
        }
        failures.extend(after(key, &base, &destination, &run));
        support::dismiss_all(&mut app);
        app.shutdown()?;
    }
    failures.extend(typed_destination(context, folder, proposal)?);
    Ok(failures)
}

/// T9: the dialog proposed a fixed local folder (keep links) and `folder` (copy the files)
/// is typed instead: the first Enter holds with "The link handling changed for this
/// folder. <reason> Select Copy again to start." and shows "Copy the files links point
/// to", the second starts with it. The reverse (proposed `folder`, a fixed folder typed)
/// holds with "… Links are kept as links here. …". A choice made by the user never holds
/// for a changed folder.
fn typed_destination(
    context: &mut Context,
    folder: &Path,
    proposal: &Proposal,
) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    let fixed = support::scratch(context, "typed-fixed")?;
    let gated = crate::fixtures::create_fresh(folder, "typed")?;
    let changed = |reason: &str| {
        format!("The link handling changed for this folder. {reason} Select Copy again to start.")
    };
    for (proposed, typed, reason, now) in [
        (
            &fixed,
            &gated,
            proposal.reason.as_str(),
            LinkChoice::CopyFiles,
        ),
        (
            &gated,
            &fixed,
            "Links are kept as links here.",
            LinkChoice::Keep,
        ),
    ] {
        let (base, _) = source_project(context, "typed-source")?;
        let mut app = App::start(Config::new(&base, proposed));
        panel::select_row(&mut app, LEFT, "proj")?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                destination: Some(typed),
                hold: true,
                choices: vec![Choice::CancelJob, Choice::CancelJob],
                ..TransferSpec::copy()
            },
        );
        let what = format!(
            "typing {} where {} was proposed",
            typed.display(),
            proposed.display()
        );
        context.note(format!("{what}: {}", outcome(&run)));
        match &run {
            Err(error) => failures.push(format!("{what}: {error}")),
            Ok(done) => {
                let wanted = changed(reason);
                if !done.hold.as_ref().is_some_and(|hold| {
                    hold.text == wanted
                        && hold.shown
                        && hold.links.as_ref().and_then(LinkView::chosen) == Some(now)
                }) {
                    failures.push(format!(
                        "{what}: the first Enter held with {:?}, not {wanted:?} showing {:?}",
                        done.hold,
                        now.label()
                    ));
                }
                if done.links.as_ref().and_then(LinkView::chosen) == Some(now) {
                    failures.push(format!(
                        "{what}: the dialog showed {:?} before the folder was checked",
                        now.label()
                    ));
                }
            }
        }
        support::dismiss_all(&mut app);
        app.shutdown()?;
    }
    // The user's own choice is kept: no hold for the changed folder.
    let (base, _) = source_project(context, "typed-chosen")?;
    let chosen = crate::fixtures::create_fresh(folder, "typed-chosen")?;
    let mut app = App::start(Config::new(&base, &fixed));
    panel::select_row(&mut app, LEFT, "proj")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(&chosen),
            links: Some(LinkChoice::CopyFiles),
            choices: vec![Choice::CancelJob, Choice::CancelJob],
            ..TransferSpec::copy()
        },
    );
    context.note(format!(
        "typing {} after choosing {:?}: {}",
        chosen.display(),
        support::COPY_LINKED_FILES,
        outcome(&run)
    ));
    if let Err(error) = &run {
        failures.push(format!(
            "a typed folder held although the user chose the link handling: {error}"
        ));
    }
    support::dismiss_all(&mut app);
    app.shutdown()?;
    Ok(failures)
}

/// No link arrived below `root`.
fn no_links(root: &Path) -> Vec<String> {
    support::tree_facts(root)
        .map(|tree| {
            tree.into_iter()
                .filter(|(_, facts)| matches!(facts.content, support::Content::Link(_)))
                .map(|(path, _)| path)
                .collect()
        })
        .unwrap_or_default()
}

/// T6: a FAT32 or exFAT folder.
fn fat(context: &mut Context) -> Result<Vec<String>, String> {
    in_gated(context, "MIDNIGHT_ACCEPTANCE_FAT_DIR", |context, folder| {
        // The dialog names the filesystem as the volume reports it: read it first.
        let filesystem = filesystem_named(context, folder)?;
        let proposal = Proposal::cannot_store(&filesystem);
        // The Move from another drive is refused in the dialog (`proposed_copy`): only
        // the Copy reaches a job.
        let after = |_: Key,
                     base: &Path,
                     destination: &Path,
                     run: &Result<support::TransferRun, String>| {
            let mut failures = Vec::new();
            let Ok(run) = run else {
                return failures;
            };
            let links_left = no_links(&destination.join("proj"));
            if !links_left.is_empty() {
                failures.push(format!("links arrived on the FAT drive: {links_left:?}"));
            }
            if cfg!(windows) {
                failures.extend(left_out_quietly(run, base, destination));
            } else {
                // Linux copies files only to filesystems with O_TMPFILE: the Copy asks
                // about plain.txt, never about a link.
                failures.extend(tmpfile_refused(run));
            }
            failures
        };
        proposed_copy(context, folder, &proposal, &after)
    })
}

/// W-3: a Copy keeping links where none are created leaves each link out without a
/// question, names it in the job details ("<path> was left out: …"), counts it, and
/// completes with "N link(s) left out where no links are created; …"; the project's file
/// arrives.
fn left_out_quietly(run: &support::TransferRun, base: &Path, destination: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let links = no_links(&base.join("proj"));
    let count = links.len() as u64;
    let summary = if count == 1 {
        "1 link left out where no links are created; the job details name it.".to_owned()
    } else {
        format!("{count} links left out where no links are created; the job details name each.")
    };
    if !run.decisions.is_empty() {
        failures.push(format!(
            "finding W-3: the Copy keeping links asked {:?}",
            run.decisions
        ));
    }
    let progress = &run.progress;
    if progress.state != CopyOperationState::Completed
        || progress.links_left_out != count
        || !progress.message.contains(&summary)
    {
        failures.push(format!(
            "finding W-3: the Copy keeping links ended {:?} with {} link(s) left out and \
             {:?}, not Completed with {summary:?}",
            progress.state, progress.links_left_out, progress.message
        ));
    }
    for name in &links {
        let named = format!("{name} was left out: ");
        if !progress.notes.iter().any(|note| note.contains(&named)) {
            failures.push(format!(
                "no job detail says {named:?}: {:?}",
                progress.notes
            ));
        }
    }
    if fs::read_to_string(destination.join("proj").join("plain.txt"))
        .ok()
        .as_deref()
        != Some("plain\n")
    {
        failures.push("plain.txt did not arrive beside the left-out links".into());
    }
    failures
}

/// Linux, a FAT drive: the Copy's only question is the O_TMPFILE refusal (answered with
/// Cancel job), never one about a link.
fn tmpfile_refused(run: &support::TransferRun) -> Vec<String> {
    match run.decisions.first() {
        Some(question)
            if question.contains("O_TMPFILE")
                && !run
                    .decisions
                    .iter()
                    .any(|asked| asked.contains("so this link is left out")) =>
        {
            Vec::new()
        }
        _ => vec![format!(
            "the Copy to the FAT drive asked {:?}, not the O_TMPFILE refusal alone",
            run.decisions
        )],
    }
}

/// How many of the job's notes name `name` and say `words`.
fn noted(run: &support::TransferRun, name: &str, words: &str) -> usize {
    run.progress
        .notes
        .iter()
        .filter(|note| note.contains(name) && note.contains(words))
        .count()
}

/// The filesystem name a FAT folder's dialog gives, read from its reason line.
fn filesystem_named(context: &mut Context, folder: &Path) -> Result<String, String> {
    let (base, _) = source_project(context, "probe-source")?;
    let mut app = App::start(Config::new(&base, folder));
    panel::select_row(&mut app, LEFT, "proj")?;
    panel::wait_input(&mut app);
    let trace = app.recent_events();
    crate::input::key(app.slint(), Key::F5);
    app.until("the Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    let arrived = support::wait_link_facts(&mut app, &trace);
    let reason = support::link_view(&app)
        .map(|view| view.reason)
        .unwrap_or_default();
    support::dismiss_all(&mut app);
    app.shutdown()?;
    let name = reason
        .strip_prefix("This drive cannot store links (")
        .and_then(|rest| rest.split_once(')'))
        .map(|(name, _)| name.to_owned());
    match name {
        Some(name) if ["FAT", "FAT32", "FAT16", "FAT12", "exFAT"].contains(&name.as_str()) => {
            Ok(name)
        }
        _ => Err(format!(
            "{} is not reported as a drive that cannot store links (facts arrived {arrived}): \
             the reason line is {reason:?}",
            folder.display()
        )),
    }
}

/// T7 and T10: a removable drive that stores links.
fn removable(context: &mut Context) -> Result<Vec<String>, String> {
    in_gated(
        context,
        "MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR",
        |context, folder| {
            let proposal = Proposal::removable();
            // Keeping links there is allowed: links arrive as links.
            let after = |_: Key, _: &Path, _: &Path, run: &Result<support::TransferRun, String>| {
                let mut failures = Vec::new();
                if let Ok(run) = run
                    && let Err(error) =
                        support::completed(run, "keeping links on a removable drive")
                {
                    failures.push(error);
                }
                failures
            };
            let mut failures = proposed_copy(context, folder, &proposal, &after)?;
            // A selection already on that drive keeps links: the default stays Keep.
            let on_drive = crate::fixtures::create_fresh(folder, "on-drive")?;
            project(&on_drive)?;
            let inside = on_drive.join("destination");
            fs::create_dir(&inside).map_err(|error| support::io_error(&inside, error))?;
            let mut app = App::start(Config::new(&on_drive, &inside));
            panel::select_row(&mut app, LEFT, "proj")?;
            let run = support::transfer(
                &mut app,
                context,
                TransferSpec {
                    links: Some(LinkChoice::Keep),
                    choices: vec![Choice::CancelJob],
                    ..TransferSpec::copy()
                },
            );
            match &run {
                Err(error) => failures.push(format!("copying on the removable drive: {error}")),
                Ok(done) => {
                    if !done
                        .offered
                        .as_ref()
                        .is_some_and(|view| view.plain_default().is_empty())
                    {
                        failures.push(format!(
                            "a selection already on the removable drive was offered {:?}, not \
                         keeping links without a reason",
                            done.offered
                        ));
                    }
                }
            }
            support::dismiss_all(&mut app);
            app.shutdown()?;
            // T10: a project moved from the computer to the drive with the defaults arrives
            // without links; what they pointed to stays on the computer, unchanged.
            let (base, _) = source_project(context, "usb-move")?;
            let data_before = identities(&base.join("data"))?;
            let destination = crate::fixtures::create_fresh(folder, "usb")?;
            let mut app = App::start(Config::new(&base, &destination));
            panel::select_row(&mut app, LEFT, "proj")?;
            let run = support::transfer(
                &mut app,
                context,
                TransferSpec {
                    offered_links: true,
                    choices: vec![Choice::CancelJob],
                    ..TransferSpec::moving()
                },
            );
            context.note(format!("the project moved to the drive: {}", outcome(&run)));
            match &run {
                // A drive that cannot keep a replaced file recoverable refuses the Move in
                // the dialog (O-1); the project stays where it was.
                Err(error)
                    if support::is_move_refusal(error)
                        && fs::symlink_metadata(base.join("proj")).is_ok() =>
                {
                    context.note("the Move to the drive was refused before any job (O-1)");
                }
                Err(error) => failures.push(format!("moving the project to the drive: {error}")),
                Ok(done) => {
                    if done.links.as_ref().and_then(LinkView::chosen) != Some(LinkChoice::CopyFiles)
                    {
                        failures.push(format!(
                            "the Move to the drive was confirmed with {:?}",
                            done.links
                        ));
                    }
                    let left = no_links(&destination.join("proj"));
                    // The dangling link cannot be followed: it is moved as a link, noted.
                    if left != ["dangling"] || fs::symlink_metadata(base.join("proj")).is_ok() {
                        failures.push(format!(
                            "after the Move the drive holds the links {left:?} and the source \
                         {}",
                            describe(&base.join("proj"))
                        ));
                    }
                    if identities(&base.join("data")).ok().as_ref() != Some(&data_before) {
                        failures.push("what the links pointed to changed".into());
                    }
                }
            }
            support::dismiss_all(&mut app);
            app.shutdown()?;
            Ok(failures)
        },
    )
}

/// T8: a Windows network location.
fn network(context: &mut Context) -> Result<Vec<String>, String> {
    if !cfg!(windows) {
        return Err(
            "SKIP: Linux network filesystems keep the link default (only Windows network \
             locations copy the files)"
                .into(),
        );
    }
    in_gated(
        context,
        "MIDNIGHT_ACCEPTANCE_NETWORK_DIR",
        |context, folder| {
            let proposal = Proposal::network();
            // The Move from another drive is refused in the dialog (`proposed_copy`):
            // only the Copy reaches a job.
            let after = |_: Key,
                         base: &Path,
                         destination: &Path,
                         run: &Result<support::TransferRun, String>| {
                let mut failures = Vec::new();
                if let Ok(run) = run {
                    let left = no_links(&destination.join("proj"));
                    if !left.is_empty() {
                        failures.push(format!("links arrived at the network location: {left:?}"));
                    }
                    failures.extend(left_out_quietly(run, base, destination));
                }
                failures
            };
            proposed_copy(context, folder, &proposal, &after)
        },
    )
}

/// T5 across volumes: a project moved keeping links to another volume.
fn other_volume(context: &mut Context) -> Result<Vec<String>, String> {
    // Linux: another ext filesystem, a gated volume. Windows: the private installation's
    // drive, which the runner names (not a gated volume).
    #[cfg(unix)]
    let (var, root) = {
        let var = "MIDNIGHT_ACCEPTANCE_OTHER_EXT_DIR";
        (var, crate::fixtures::gated_dir(var)?)
    };
    #[cfg(windows)]
    let (var, root) = {
        let var = "MIDNIGHT_ACCEPTANCE_STAGE_ROOT";
        let root = std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|root| root.is_dir())
            .ok_or_else(|| format!("SKIP: {var} not set"))?;
        (var, root)
    };
    in_folder(context, &root, |context, folder| {
        let work = support::scratch(context, "cross-volume")?;
        if support::identity(&work)?.0 == support::identity(folder)?.0 {
            return Err(format!(
                "SKIP: {var} is on the run's volume (run with --run-dir on another one)"
            ));
        }
        // A Windows Move across volumes removes each verified source through a recovery
        // stage beside it, which it takes only in a protected folder: there the project
        // starts in the private installation and moves to the work folder.
        let (from, to) = if cfg!(windows) {
            (folder, work.as_path())
        } else {
            (work.as_path(), folder)
        };
        let base = crate::fixtures::create_fresh(from, "project")?;
        let links = project(&base)?;
        let data_before = identities(&base.join("data"))?;
        let destination = crate::fixtures::create_fresh(to, "moved")?;
        let mut app = App::start(Config::new(&base, &destination));
        panel::select_row(&mut app, LEFT, "proj")?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                links: Some(LinkChoice::Keep),
                link_input: LinkInput::Keyboard,
                choices: vec![Choice::CancelJob],
                ..TransferSpec::moving()
            },
        );
        context.note(format!(
            "the project moved keeping links: {}",
            outcome(&run)
        ));
        let mut failures = Vec::new();
        match &run {
            Err(error) => failures.push(format!("the Move keeping links: {error}")),
            Ok(done) => {
                if let Err(error) = support::completed(done, "the Move keeping links") {
                    failures.push(error);
                }
                for (name, target) in &links {
                    // A relative link keeps its text; a junction its absolute target.
                    if !is_link_to(&destination.join("proj").join(name), target) {
                        failures.push(format!(
                            "{name} arrived as {}",
                            describe(&destination.join("proj").join(name))
                        ));
                    }
                }
                if fs::symlink_metadata(base.join("proj")).is_ok() {
                    failures.push("the moved project is still in the source".into());
                }
                if identities(&base.join("data")).ok().as_ref() != Some(&data_before) {
                    failures.push("what the links point to changed".into());
                }
            }
        }
        support::dismiss_all(&mut app);
        app.shutdown()?;
        Ok(failures)
    })
}

/// A Linux folder whose filesystem has no O_TMPFILE: a Copy there is refused in words.
fn no_tmpfile(context: &mut Context) -> Result<Vec<String>, String> {
    if !cfg!(unix) {
        return Err("SKIP: O_TMPFILE is a Linux requirement".into());
    }
    in_gated(
        context,
        "MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR",
        |context, folder| {
            let source = support::scratch(context, "tmpfile-source")?;
            fs::write(source.join("file.txt"), "content\n")
                .map_err(|error| support::io_error(&source, error))?;
            let mut app = App::start(Config::new(&source, folder));
            panel::select_row(&mut app, LEFT, "file.txt")?;
            let run = support::transfer(
                &mut app,
                context,
                TransferSpec {
                    choices: vec![Choice::CancelJob],
                    ..TransferSpec::copy()
                },
            );
            const WORDS: &str = "This filesystem cannot create anonymous O_TMPFILE copies; \
                             verified Copy on Linux needs a local filesystem such as ext4, \
                             XFS, Btrfs or tmpfs.";
            let said = match &run {
                Err(error) => error.contains(WORDS),
                Ok(done) => {
                    done.decisions.iter().any(|text| text.contains(WORDS))
                        || done.progress.message.contains(WORDS)
                        || done
                            .progress
                            .errors
                            .iter()
                            .any(|error| error.to_string().contains(WORDS))
                }
            };
            context.note(format!(
                "a Copy to a folder without O_TMPFILE: {}",
                outcome(&run)
            ));
            let mut failures = Vec::new();
            if !said || folder.join("file.txt").exists() {
                failures.push(format!(
                    "a Copy to a folder without O_TMPFILE was not refused with {WORDS:?}: {}",
                    outcome(&run)
                ));
            }
            support::dismiss_all(&mut app);
            app.shutdown()?;
            Ok(failures)
        },
    )
}

/// T13: Windows symbolic links kept as links.
fn windows_symlinks(context: &mut Context) -> Result<Vec<String>, String> {
    if !cfg!(windows) {
        return Err("SKIP: Windows symbolic links".into());
    }
    let prepared = std::env::var_os("MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
        .ok_or("SKIP: MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS not set")?;
    let mut failures = Vec::new();
    for name in ["dir-link", "file-link"] {
        if !fs::symlink_metadata(prepared.join(name)).is_ok_and(|meta| meta.is_symlink()) {
            return Err(format!(
                "MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS holds no symbolic link {name}"
            ));
        }
    }
    // Whether this process may create symbolic links itself (Developer Mode or rights).
    let probe = support::scratch(context, "symlink-probe")?;
    let allowed = symlink_allowed(&probe);
    let destination = support::scratch(context, "symlinks-copied")?;
    let mut app = App::start(Config::new(&prepared, &destination));
    for name in ["dir-link", "file-link"] {
        panel::select_row(&mut app, LEFT, name)?;
        let run = support::transfer(
            &mut app,
            context,
            TransferSpec {
                links: Some(LinkChoice::Keep),
                choices: vec![Choice::Skip],
                ..TransferSpec::copy()
            },
        );
        context.note(format!(
            "{name} kept as a link (creating symbolic links allowed: {allowed}): {}",
            outcome(&run)
        ));
        let target = fs::read_link(prepared.join(name)).unwrap_or_default();
        let copied = destination.join(name);
        let ok = match (&run, allowed, name) {
            (Err(_), ..) => false,
            (Ok(_), true, _) => is_link_to(&copied, &target),
            (Ok(done), false, "dir-link") => {
                // A junction to the same folder, with a note.
                fs::symlink_metadata(&copied).is_ok_and(|meta| meta.is_symlink())
                    && noted(done, name, "was created as a junction to") == 1
                    && done.progress.notes.iter().any(|note| {
                        note.contains(
                            "because creating a symbolic link needs Developer Mode or \
                                       administrator rights",
                        )
                    })
            }
            (Ok(done), false, _) => {
                // A question with Skip and Skip all, no Retry; Skip leaves it out.
                done.decisions.first().is_some_and(|question| {
                    question.contains(
                        "Creating a symbolic link to a file needs Developer Mode or \
                         administrator rights",
                    ) && question.contains("\"Skip\"")
                        && question.contains("\"Skip all")
                        && !question.contains("\"Retry\"")
                }) && fs::symlink_metadata(&copied).is_err()
            }
        };
        if !ok {
            failures.push(format!(
                "{name} kept as a link (creating symbolic links allowed: {allowed}) gave {}: {}",
                describe(&copied),
                outcome(&run)
            ));
        }
        support::dismiss_all(&mut app);
    }
    app.shutdown()?;
    Ok(failures)
}

/// Whether this process may create a directory symbolic link in `folder`.
fn symlink_allowed(folder: &Path) -> bool {
    #[cfg(windows)]
    {
        let link = folder.join("probe-link");
        let made = std::os::windows::fs::symlink_dir(folder, &link).is_ok();
        let _ = fs::remove_dir(&link);
        made
    }
    #[cfg(not(windows))]
    {
        let _ = folder;
        true
    }
}
