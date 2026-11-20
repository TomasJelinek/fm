//! Helpers of the file-operation and panel scenarios (`scenarios/files_*.rs`): the
//! Copy/Move, Delete, Rename, New file/folder dialogs and the decision window driven
//! by real keys and clicks, and the on-disk facts the scenarios compare.
use crate::{
    Context,
    app::{self, App},
    input, inspect, panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{
    CopyOperationState, CopyProgress, OperationId, OperationProgress, OperationState, PaneId,
};
use slint::platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

/// `Err` text for an I/O failure on `path`.
pub fn io_error(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// What a file system entry is, as a scenario compares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    File { len: u64, sha256: String },
    Folder,
    Link(PathBuf),
    Other,
}

/// An entry's content, permission bits and times (links are not followed).
///
/// Equality ignores `accessed`: reading a file to hash it updates its access time.
#[derive(Clone, Debug)]
pub struct Facts {
    pub content: Content,
    pub mode: u32,
    pub modified: SystemTime,
    pub accessed: SystemTime,
}

impl PartialEq for Facts {
    fn eq(&self, other: &Self) -> bool {
        self.content == other.content && self.mode == other.mode && self.modified == other.modified
    }
}

impl Eq for Facts {}

pub fn facts(path: &Path) -> Result<Facts, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    let kind = metadata.file_type();
    let content = if kind.is_symlink() {
        Content::Link(fs::read_link(path).map_err(|error| io_error(path, error))?)
    } else if kind.is_dir() {
        Content::Folder
    } else if kind.is_file() {
        let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
        Content::File {
            len: bytes.len() as u64,
            sha256: crate::encode::hex(&crate::encode::sha256(&bytes)),
        }
    } else {
        Content::Other
    };
    Ok(Facts {
        content,
        mode: mode_of(&metadata),
        modified: metadata.modified().map_err(|error| io_error(path, error))?,
        accessed: metadata.accessed().map_err(|error| io_error(path, error))?,
    })
}

#[cfg(unix)]
fn mode_of(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn mode_of(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o666
    }
}

/// Every entry below `root` (relative path to facts), links not followed.
pub fn tree_facts(root: &Path) -> Result<BTreeMap<String, Facts>, String> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeMap<String, Facts>) -> Result<(), String> {
        for entry in fs::read_dir(path).map_err(|error| io_error(path, error))? {
            let entry = entry.map_err(|error| io_error(path, error))?;
            let full = entry.path();
            let found = facts(&full)?;
            let relative = full
                .strip_prefix(root)
                .unwrap_or(&full)
                .to_string_lossy()
                .into_owned();
            let folder = found.content == Content::Folder;
            output.insert(relative, found);
            if folder {
                visit(root, &full, output)?;
            }
        }
        Ok(())
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

/// Compare two trees by content only, or also by mode and modified time.
pub fn compare_trees(
    expected: &BTreeMap<String, Facts>,
    actual: &BTreeMap<String, Facts>,
    metadata: bool,
) -> Vec<String> {
    let mut differences = Vec::new();
    for (path, wanted) in expected {
        match actual.get(path) {
            None => differences.push(format!("missing {path}")),
            Some(found) if found.content != wanted.content => differences.push(format!(
                "{path}: {:?} instead of {:?}",
                found.content, wanted.content
            )),
            Some(found)
                if metadata
                    && found.content != Content::Folder
                    && (found.mode != wanted.mode || found.modified != wanted.modified) =>
            {
                differences.push(format!(
                    "{path}: mode {:o} modified {:?} instead of mode {:o} modified {:?}",
                    found.mode, found.modified, wanted.mode, wanted.modified
                ));
            }
            Some(_) => {}
        }
    }
    for path in actual.keys() {
        if !expected.contains_key(path) {
            differences.push(format!("unexpected {path}"));
        }
    }
    differences
}

/// The exact names in `folder` (no normalisation), sorted.
pub fn names_on_disk(folder: &Path) -> Vec<String> {
    panel::disk_names(folder)
}

pub fn cursor_name(app: &App, pane: PaneId) -> Option<String> {
    let names = app.names(pane);
    usize::try_from(app.cursor(pane))
        .ok()
        .and_then(|row| names.get(row).cloned())
}

pub fn selected_names(app: &App, pane: PaneId) -> Vec<String> {
    app.rows(pane)
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect()
}

#[track_caller]
pub fn wait_row(app: &mut App, pane: PaneId, name: &str, present: bool) {
    let wanted = name.to_owned();
    app.until(
        &format!(
            "{name:?} to {} the {pane:?} panel",
            if present { "appear in" } else { "leave" }
        ),
        move |app| app.names(pane).contains(&wanted) == present,
    );
}

/// Wait until `name` is (or is no longer) listed; how long that took after `since`.
#[track_caller]
pub fn row_delay(
    app: &mut App,
    pane: PaneId,
    name: &str,
    present: bool,
    since: Instant,
) -> Duration {
    wait_row(app, pane, name, present);
    since.elapsed()
}

/// Nothing modal is open and panel input is accepted.
pub fn main_idle(app: &App) -> bool {
    !app.window.get_embedded_modal_open()
        && inspect::roots(app.slint()).len() == 1
        && app.window.get_operation_token().is_empty()
        && app.window.get_copy_draft_token().is_empty()
        && app.window.get_metadata_token().is_empty()
        && !app.window.get_mutation_input_blocked()
        && app.tool_windows().is_empty()
}

/// Close whatever a failed step left open (decision windows with Decide later,
/// dialogs with Escape) so the next step starts from the main window.
pub fn dismiss_all(app: &mut App) {
    for _ in 0..4 {
        for window in app.tool_windows() {
            input::key(window.window(), Key::Escape);
        }
        if !main_idle(app) {
            input::key(app.slint(), Key::Escape);
        }
        app.idle(Duration::from_millis(120));
        if main_idle(app) {
            return;
        }
    }
}

/// The open decision window (name conflict or operation error), if any.
pub fn decision_window(app: &App) -> Option<Rc<MinimalSoftwareWindow>> {
    app.tool_windows().into_iter().rev().find(|window| {
        inspect::find(window.window(), AccessibleRole::Button, "Decide later").is_some()
    })
}

/// Visible texts and button labels of a tool window, for failure messages.
pub fn describe_window(window: &MinimalSoftwareWindow) -> String {
    format!(
        "texts {:?}; buttons {:?}",
        inspect::visible_texts(window.window()),
        inspect::labels(window.window(), AccessibleRole::Button)
    )
}

/// Wait for a decision window, size it like a desktop dialog and return it.
#[track_caller]
pub fn wait_decision(app: &mut App, what: &str) -> Rc<MinimalSoftwareWindow> {
    app.until(what, |app| decision_window(app).is_some());
    let window = decision_window(app).expect("decision window");
    app::layout_tool(&window, 820, 560);
    app.settle();
    window
}

/// One answer in the decision window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Skip,
    /// The name conflict's "Skip all".
    SkipAll,
    Replace,
    ReplaceAll,
    Merge,
    MergeAll,
    /// Type a different name and press Copy as.
    CopyAs(String),
    CancelJob,
    /// The operation error's "Retry".
    Retry,
    /// The operation error's "Skip all this error type".
    SkipAllErrors,
    /// Leave the question open: the job waits ("Needs a decision") until Resolve.
    DecideLater,
}

impl Choice {
    fn button(&self) -> &'static str {
        match self {
            Self::Skip => "Skip",
            Self::SkipAll => "Skip all",
            Self::Replace => "Replace · keep original",
            Self::ReplaceAll => "Replace all · keep originals",
            Self::Merge => "Merge folders",
            Self::MergeAll => "Merge all folders",
            Self::CopyAs(_) => "Copy as",
            Self::CancelJob => "Cancel job",
            Self::Retry => "Retry",
            Self::SkipAllErrors => "Skip all this error type",
            Self::DecideLater => "Decide later",
        }
    }
}

/// Answer the open decision window with `choice` by clicking its button.
pub fn answer(
    app: &mut App,
    window: &Rc<MinimalSoftwareWindow>,
    choice: &Choice,
) -> Result<(), String> {
    if let Choice::CopyAs(name) = choice {
        input::fill(window.window(), "Copy under a different name", name)
            .map_err(|error| format!("decision window: {error}"))?;
        app.settle();
    }
    input::click_button(window.window(), choice.button())
        .map_err(|error| format!("decision window {choice:?}: {error}"))?;
    app.settle();
    Ok(())
}

/// What a confirmed Copy/Move did.
pub struct TransferRun {
    pub progress: Arc<CopyProgress>,
    /// Texts of each decision window answered, in order.
    pub decisions: Vec<String>,
    /// From the confirming key to the terminal report.
    pub elapsed: Duration,
    /// When the terminal report arrived.
    pub finished: Instant,
    /// The notice the job left (title, body); it has been closed with Escape.
    pub notice: Option<(String, String)>,
    /// The job's distinct states (with wait reason and phase) and when they were
    /// first seen, in ms after the confirming key.
    pub timeline: Vec<String>,
}

/// Pump events until `condition` holds (or panic after `limit` with the app's
/// diagnostics), recording each new state of the Copy/Move jobs reported after `start`.
fn wait_recording(
    app: &mut App,
    what: &str,
    (start, limit): (usize, Duration),
    confirmed: Instant,
    timeline: &mut Vec<String>,
    condition: impl Fn(&App) -> bool,
) {
    let deadline = Instant::now() + limit;
    let mut seen = start;
    loop {
        app.settle();
        for progress in &app.copies[seen..] {
            let state = format!(
                "{:?}{}/{:?}",
                progress.state,
                progress
                    .wait_reason
                    .as_ref()
                    .map(|reason| format!("({reason:?})"))
                    .unwrap_or_default(),
                progress.phase
            );
            if timeline
                .last()
                .is_none_or(|last| !last.starts_with(&format!("{state} @")))
            {
                timeline.push(format!("{state} @{} ms", confirmed.elapsed().as_millis()));
            }
        }
        seen = app.copies.len();
        if condition(app) {
            return;
        }
        if Instant::now() >= deadline {
            // One more settle, then the app's own timeout report.
            app.until_for(what, Duration::from_millis(1), condition);
            return;
        }
        app.idle(Duration::from_millis(5));
    }
}

/// The notice shown in the main window (title, body), if one is open. (The message
/// properties keep the last notice after it is closed; the presentation token tells
/// whether it is shown.)
pub fn shown_notice(app: &App) -> Option<(String, String)> {
    use slint::ComponentHandle;
    let presentation = app.window.global::<midnight_ui::NoticePresentation>();
    (!presentation.get_token().is_empty()).then(|| {
        (
            presentation.get_title().to_string(),
            presentation.get_body().to_string(),
        )
    })
}

/// Take the shown notice, if any, and close it with Escape.
pub fn take_notice(app: &mut App) -> Option<(String, String)> {
    let notice = shown_notice(app)?;
    input::key(app.slint(), Key::Escape);
    app.until("the notice to close", |app| shown_notice(app).is_none());
    Some(notice)
}

/// One F5/F6 run: options of the dialog and the answers to its decision windows.
pub struct TransferSpec<'a> {
    pub key: Key,
    /// Replace the suggested destination (the other panel's folder).
    pub destination: Option<&'a Path>,
    pub preserve_time: bool,
    /// Set the link option ("Follow links" on Linux) when it is shown.
    pub follow_links: Option<bool>,
    /// Set "Wait for other operations to finish".
    pub wait: Option<bool>,
    pub choices: Vec<Choice>,
    pub shot: Option<&'a str>,
    /// Screenshot name for the first decision window.
    pub decision_shot: Option<&'a str>,
    /// How long to wait for the job to end or ask (default [`app::WAIT`]): thousands of
    /// small files, each written, synced and read back, take minutes on a busy disk.
    pub limit: Option<Duration>,
}

impl TransferSpec<'_> {
    pub fn copy() -> Self {
        Self {
            key: Key::F5,
            destination: None,
            preserve_time: false,
            follow_links: None,
            wait: None,
            choices: Vec::new(),
            shot: None,
            decision_shot: None,
            limit: None,
        }
    }

    pub fn moving() -> Self {
        Self {
            key: Key::F6,
            ..Self::copy()
        }
    }
}

/// Click the dialog's check box until `checked` reports `wanted`.
fn set_check(
    app: &mut App,
    label: &str,
    wanted: bool,
    checked: fn(&App) -> bool,
) -> Result<(), String> {
    if checked(app) != wanted {
        input::click(app.slint(), AccessibleRole::Checkbox, label)?;
        app.settle();
    }
    if checked(app) != wanted {
        return Err(format!("clicking {label:?} did not set it to {wanted}"));
    }
    Ok(())
}

fn operation_form(app: &App) -> midnight_ui::OperationPresentation<'_> {
    use slint::ComponentHandle;
    app.window.global::<midnight_ui::OperationPresentation>()
}

/// The label of the dialog's queueing option.
pub const WAIT_OPTION: &str = "Wait for other operations to finish";

/// Press F5/F6 on the focused panel, set the dialog up from `spec` and confirm with
/// Enter. Returns the index of the job's first report in `app.copies` and when it was
/// confirmed; the dialog's own refusal is an `Err`.
fn confirm_dialog(
    app: &mut App,
    context: &mut Context,
    spec: &TransferSpec<'_>,
) -> Result<(usize, Instant), String> {
    panel::wait_input(app);
    let start = app.copies.len();
    input::key(app.slint(), spec.key);
    app.until("the Copy/Move dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    let moving = app.window.get_copy_draft_move();
    if moving != (spec.key == Key::F6) {
        return Err(format!("{:?} opened a dialog with move={moving}", spec.key));
    }
    if let Some(destination) = spec.destination {
        input::fill(
            app.slint(),
            "Destination folder",
            &destination.to_string_lossy(),
        )?;
        app.settle();
    }
    if spec.preserve_time {
        set_check(app, "Preserve exact modified time", true, |app| {
            operation_form(app).get_option_checked()
        })?;
    }
    // The link option's label differs by platform; use the one the dialog shows.
    let link_label = operation_form(app).get_follow_links_label().to_string();
    if let Some(follow) = spec.follow_links
        && inspect::find(app.slint(), AccessibleRole::Checkbox, &link_label).is_some()
    {
        set_check(app, &link_label, follow, |app| {
            operation_form(app).get_follow_directory_links_checked()
        })?;
    }
    if let Some(wait) = spec.wait {
        set_check(app, WAIT_OPTION, wait, |app| {
            inspect::find(app.slint(), AccessibleRole::Checkbox, WAIT_OPTION)
                .and_then(|item| {
                    item.accessible_string_property(
                        i_slint_core::accessibility::AccessibleStringProperty::Checked,
                    )
                })
                .is_some_and(|value| value == "true")
        })?;
    }
    if let Some(name) = spec.shot {
        app.screenshot(
            context,
            name,
            if moving { "Move dialog" } else { "Copy dialog" },
        );
    }
    let confirmed = Instant::now();
    // Enter confirms from the destination field, the check boxes and the primary button.
    input::key(app.slint(), Key::Return);
    app.settle();
    let error = app.window.get_copy_draft_error().to_string();
    if !error.is_empty() && !app.window.get_copy_draft_token().is_empty() {
        return Err(format!("the dialog refused the transfer: {error}"));
    }
    Ok((start, confirmed))
}

/// Confirm a Copy/Move (as [`transfer`] does) without waiting for it: returns the index
/// of its first report in `app.copies` once the job was admitted.
pub fn start_transfer(
    app: &mut App,
    context: &mut Context,
    spec: TransferSpec<'_>,
) -> Result<usize, String> {
    let (start, _) = confirm_dialog(app, context, &spec)?;
    app.until("the job to be admitted", move |app| {
        app.copies.len() > start || shown_notice(app).is_some()
    });
    if app.copies.len() == start {
        return Err(format!("no job started: {:?}", shown_notice(app)));
    }
    Ok(start)
}

/// The job admitted after `start` in `app.copies`: the first one reported there that
/// did not report before (other jobs keep reporting beside it).
pub fn job_id(app: &App, start: usize) -> Option<OperationId> {
    let earlier: BTreeSet<OperationId> = app.copies[..start].iter().map(|job| job.id).collect();
    app.copies[start..]
        .iter()
        .map(|job| job.id)
        .find(|id| !earlier.contains(id))
}

/// The latest report of the job admitted after `start` (see [`job_id`]).
pub fn job(app: &App, start: usize) -> Option<Arc<CopyProgress>> {
    let id = job_id(app, start)?;
    app.copies[start..]
        .iter()
        .rev()
        .find(|progress| progress.id == id)
        .cloned()
}

/// Wait until the job whose reports start at `start` ended; its last report.
#[track_caller]
pub fn wait_job(app: &mut App, start: usize) -> Arc<CopyProgress> {
    app.until("the job to end", move |app| {
        job(app, start).is_some_and(|progress| progress.state.is_terminal())
    });
    job(app, start).expect("a reported job")
}

/// Press F5/F6 on the focused panel, set the dialog up, confirm, answer each decision
/// window in turn and wait for the terminal report.
pub fn transfer(
    app: &mut App,
    context: &mut Context,
    spec: TransferSpec<'_>,
) -> Result<TransferRun, String> {
    let (start, confirmed) = confirm_dialog(app, context, &spec)?;
    let mut decisions = Vec::new();
    let mut timeline = Vec::new();
    let mut choices = spec.choices.into_iter();
    let limit = spec.limit.unwrap_or(app::WAIT);
    loop {
        wait_recording(
            app,
            "the transfer to finish or ask",
            (start, limit),
            confirmed,
            &mut timeline,
            |app| {
                app.copies[start..]
                    .iter()
                    .any(|progress| progress.state.is_terminal())
                    || decision_window(app).is_some()
                    || (!app.window.get_copy_draft_token().is_empty()
                        && !app.window.get_copy_draft_error().is_empty())
                    // Refused before a job existed: only a notice tells why.
                    || (app.copies[start..].is_empty()
                        && app.window.get_copy_draft_token().is_empty()
                        && shown_notice(app).is_some())
            },
        );
        if app.copies[start..].is_empty()
            && app.window.get_copy_draft_token().is_empty()
            && let Some(notice) = take_notice(app)
        {
            return Err(format!("refused before a job started: {notice:?}"));
        }
        if let Some(progress) = app.copies[start..]
            .iter()
            .find(|progress| progress.state.is_terminal())
            .cloned()
        {
            let finished = Instant::now();
            app.until("the Copy/Move dialog to close", |app| {
                app.window.get_copy_draft_token().is_empty()
            });
            // A decision window left over from a cancelled job is not ours to keep. A
            // question can close a moment after its job ended; one still open then is
            // left over.
            let deadline = Instant::now() + Duration::from_secs(2);
            while decision_window(app).is_some() && Instant::now() < deadline {
                app.idle(Duration::from_millis(20));
            }
            if let Some(window) = decision_window(app) {
                app::layout_tool(&window, 820, 560);
                app.settle();
                app.screenshot_window(
                    context,
                    &window,
                    "leftover-decision",
                    "A question still open after its job ended",
                );
                return Err(format!(
                    "a decision window is still open after the job ended {:?} (job {:?}, \
                     {} decision(s) answered, failed {}, skipped {}): {}",
                    progress.state,
                    progress.id,
                    decisions.len(),
                    progress.failed,
                    progress.skipped,
                    describe_window(&window)
                ));
            }
            app.settle();
            let notice = take_notice(app);
            return Ok(TransferRun {
                progress,
                decisions,
                elapsed: finished - confirmed,
                finished,
                notice,
                timeline,
            });
        }
        let error = app.window.get_copy_draft_error().to_string();
        if !error.is_empty() && !app.window.get_copy_draft_token().is_empty() {
            return Err(format!("the dialog refused the transfer: {error}"));
        }
        let window = wait_decision(app, "the decision window");
        let text = describe_window(&window);
        if decisions.is_empty()
            && let Some(name) = spec.decision_shot
        {
            app.screenshot_window(context, &window, name, "Decision window");
        }
        let Some(choice) = choices.next() else {
            return Err(format!("unexpected decision window: {text}"));
        };
        decisions.push(text);
        let token = app.window.get_copy_token();
        answer(app, &window, &choice)?;
        let answered = window.clone();
        app.until("the decision to be taken", move |app| {
            !answered.window().is_visible() || app.window.get_copy_token() != token
        });
    }
}

/// What a Copy/Move job did, in one line: renames, files, bytes and verification work.
pub fn describe_run(run: &TransferRun) -> String {
    let progress = &run.progress;
    format!(
        "{} ms, {:?}, {} renamed, {} files, {} B written, {} B verified, {} B proof reads, {} source files removed; states {}",
        run.elapsed.as_millis(),
        progress.state,
        progress.native_entries_moved,
        progress.files_completed,
        progress.copied_bytes,
        progress.verified_bytes,
        progress.validation_bytes,
        progress.source_files_removed,
        run.timeline.join(" → ")
    )
}

/// The run must have completed every root without skips or failures.
pub fn completed(run: &TransferRun, what: &str) -> Result<(), String> {
    let progress = &run.progress;
    if progress.state != CopyOperationState::Completed
        || progress.failed != 0
        || progress.skipped != 0
    {
        return Err(format!(
            "{what} ended {:?} (skipped {}, failed {}, completed roots {}/{}): {progress:?}",
            progress.state,
            progress.skipped,
            progress.failed,
            progress.completed_roots,
            progress.requested_roots
        ));
    }
    Ok(())
}

/// Wait for the embedded operation dialog (rename, delete, create).
#[track_caller]
pub fn wait_operation_dialog(app: &mut App, what: &str) {
    app.until(what, |app| !app.window.get_operation_token().is_empty());
}

/// Wait for the first terminal mutation report after `start`. A delete failure's
/// question (Retry, Skip, …) is not expected here and fails at once.
#[track_caller]
pub fn wait_operation(app: &mut App, start: usize) -> Arc<OperationProgress> {
    match wait_delete(app, start) {
        DeleteEnd::Done(progress) => progress,
        DeleteEnd::Asked(window) => panic!(
            "the file operation asked a question: {}",
            describe_window(&window)
        ),
    }
}

/// Buttons in a framed window's header corner (its top 48 px, at its right edge): an
/// internal × that duplicates the native title bar's close button. Their labels.
pub fn header_close_buttons(window: &MinimalSoftwareWindow) -> Vec<String> {
    let slint = window.window();
    let size = slint.size().to_logical(slint.scale_factor());
    inspect::controls(slint, AccessibleRole::Button)
        .into_iter()
        .filter(|(_, item)| {
            let (x, y, w, h) = inspect::rect(item);
            h > 0.0 && y < 48.0 && x + w > size.width - 64.0 && w <= 48.0
        })
        .map(|(label, _)| label)
        .collect()
}

/// The shared question window a failed delete opens (Retry when it applies, Skip,
/// Skip all this error type, Cancel job, Decide later).
pub fn delete_question(app: &App) -> Option<Rc<MinimalSoftwareWindow>> {
    decision_window(app)
}

/// How a delete ended: its terminal report, or a question it asks.
pub enum DeleteEnd {
    Done(Arc<OperationProgress>),
    Asked(Rc<MinimalSoftwareWindow>),
}

/// Wait until the mutation reported after `start` ended or asks a question.
#[track_caller]
pub fn wait_delete(app: &mut App, start: usize) -> DeleteEnd {
    app.until("the file operation to end or ask", move |app| {
        app.operations[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
            || delete_question(app).is_some()
    });
    match app.operations[start..]
        .iter()
        .find(|progress| progress.state.is_terminal())
        .cloned()
    {
        Some(progress) => {
            app.settle();
            DeleteEnd::Done(progress)
        }
        None => {
            // Sized like a desktop dialog, so it can be read and rendered.
            let window = delete_question(app).expect("a question");
            app::layout_tool(&window, 820, 560);
            app.settle();
            DeleteEnd::Asked(window)
        }
    }
}

/// Answer a delete question with the first of `labels` it offers, then wait (at most
/// 10 s) until that question is gone: its window hidden or showing another question.
/// Returns the label clicked.
pub fn answer_delete(
    app: &mut App,
    window: &Rc<MinimalSoftwareWindow>,
    labels: &[&str],
) -> Result<String, String> {
    let before = inspect::visible_texts(window.window());
    let label = labels
        .iter()
        .find(|label| inspect::find(window.window(), AccessibleRole::Button, label).is_some())
        .ok_or_else(|| {
            format!(
                "the question offers none of {labels:?}: {}",
                describe_window(window)
            )
        })?;
    input::click_button(window.window(), label)
        .map_err(|error| format!("the question's {label:?}: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while window.window().is_visible()
        && inspect::visible_texts(window.window()) == before
        && Instant::now() < deadline
    {
        app.idle(Duration::from_millis(20));
    }
    app.settle();
    Ok((*label).to_owned())
}

/// Answer the questions of the delete reported after `start` with Cancel job until it
/// ends (at most three); its last report, if it ended.
pub fn cancel_delete(
    app: &mut App,
    window: &Rc<MinimalSoftwareWindow>,
    start: usize,
) -> Option<Arc<OperationProgress>> {
    let mut window = window.clone();
    for _ in 0..3 {
        answer_delete(app, &window, &["Cancel job", "Cancel"]).ok()?;
        match wait_delete(app, start) {
            DeleteEnd::Done(progress) => return Some(progress),
            DeleteEnd::Asked(next) => window = next,
        }
    }
    None
}

/// F8 (or Shift+F8 when `permanent`) on the focused panel, confirmed with Enter; waits
/// until the job ends or asks. Returns where its reports start and how it ended.
pub fn delete_with(app: &mut App, permanent: bool) -> Result<(usize, DeleteEnd), String> {
    panel::wait_input(app);
    let start = app.operations.len();
    if permanent {
        input::chord(app.slint(), &[Key::Shift], Key::F8);
    } else {
        input::key(app.slint(), Key::F8);
    }
    wait_operation_dialog(app, "the delete confirmation");
    if app.window.get_operation_dangerous() != permanent {
        return Err(format!(
            "{} opened the wrong confirmation",
            if permanent { "Shift+F8" } else { "F8" }
        ));
    }
    input::key(app.slint(), Key::Return);
    Ok((start, wait_delete(app, start)))
}

pub fn operation_ok(progress: &OperationProgress, what: &str) -> Result<(), String> {
    if progress.state != OperationState::Completed || progress.failed != 0 {
        return Err(format!(
            "{what} ended {:?} ({} failed): {} {:?}",
            progress.state, progress.failed, progress.message, progress.errors
        ));
    }
    Ok(())
}

/// Shift+F8 on the focused panel, confirmed with Enter.
pub fn delete_permanently(
    app: &mut App,
    context: &mut Context,
    shot: Option<&str>,
) -> Result<Arc<OperationProgress>, String> {
    panel::wait_input(app);
    let start = app.operations.len();
    input::chord(app.slint(), &[Key::Shift], Key::F8);
    wait_operation_dialog(app, "the permanent delete confirmation");
    if !app.window.get_operation_dangerous() {
        return Err("Shift+F8 did not ask for a permanent delete".into());
    }
    if let Some(name) = shot {
        app.screenshot(context, name, "Permanent delete confirmation");
    }
    input::key(app.slint(), Key::Return);
    let progress = wait_operation(app, start);
    app.settle();
    // A completed delete has nothing to tell; a failed one leaves its notice open for
    // the caller to read.
    if progress.state == OperationState::Completed
        && let Some(notice) = take_notice(app)
    {
        return Err(format!(
            "a completed Shift+F8 still showed a notice: {notice:?}"
        ));
    }
    Ok(progress)
}

/// Open the F2/F7/Shift+F4 dialog, replace its name with `text`, press Enter.
pub fn named_operation(
    app: &mut App,
    modifiers: &[Key],
    key: Key,
    text: &str,
) -> Result<usize, String> {
    panel::wait_input(app);
    let start = app.operations.len();
    input::chord(app.slint(), modifiers, key);
    wait_operation_dialog(app, "the name dialog");
    input::replace_text(app.slint(), text);
    app.settle();
    input::key(app.slint(), Key::Return);
    app.settle();
    Ok(start)
}

/// Durations of repeated steps, summarised in a note.
#[derive(Default)]
pub struct Timings(Vec<(String, String, Duration)>);

impl Timings {
    pub fn add(&mut self, what: &str, duration: Duration) {
        self.add_for(what, "", duration);
    }

    /// Record a step of `item` (named in the summary when it was the slowest).
    pub fn add_for(&mut self, what: &str, item: &str, duration: Duration) {
        self.0.push((what.to_owned(), item.to_owned(), duration));
    }

    /// "label: n runs, median X ms, max Y ms (item)" per label.
    pub fn summary(&self) -> String {
        let mut labels: Vec<&str> = Vec::new();
        for (label, _, _) in &self.0 {
            if !labels.contains(&label.as_str()) {
                labels.push(label);
            }
        }
        let mut parts = Vec::new();
        for label in labels {
            let mut values: Vec<(Duration, &str)> = self
                .0
                .iter()
                .filter(|(name, _, _)| name == label)
                .map(|(_, item, value)| (*value, item.as_str()))
                .collect();
            values.sort();
            let (slowest, item) = values.last().copied().unwrap_or_default();
            parts.push(format!(
                "{label}: {} runs, median {} ms, max {} ms{}",
                values.len(),
                values[values.len() / 2].0.as_millis(),
                slowest.as_millis(),
                if item.is_empty() {
                    String::new()
                } else {
                    format!(" ({item})")
                }
            ));
        }
        parts.join("; ")
    }
}

/// A scratch folder below the scenario's work folder, created fresh.
pub fn scratch(context: &Context, name: &str) -> Result<PathBuf, String> {
    let path = context.work.join(name);
    if path.exists() {
        crate::fixtures::remove_tree(&path).map_err(|error| io_error(&path, error))?;
    }
    fs::create_dir_all(&path).map_err(|error| io_error(&path, error))?;
    Ok(path)
}

/// Copy fixture folders (relative names) into `to`, keeping their names.
pub fn stage(context: &Context, folders: &[&str], to: &Path) -> Result<(), String> {
    for folder in folders {
        crate::scenarios::transfers::copy_tree(&context.fixture.path(folder), &to.join(folder))?;
    }
    restore_fixture_modes(context, to)
}

/// `transfers::copy_tree` gives copies default permissions; give `readonly/` its
/// fixture modes back so the staged tree matches the fixture.
fn restore_fixture_modes(context: &Context, to: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for relative in ["readonly/locked.txt", "readonly/locked-folder"] {
            let staged = to.join(relative);
            if staged.exists() {
                let mode = fs::metadata(context.fixture.path(relative))
                    .map_err(|error| io_error(&staged, error))?
                    .permissions()
                    .mode();
                fs::set_permissions(&staged, fs::Permissions::from_mode(mode))
                    .map_err(|error| io_error(&staged, error))?;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (context, to);
    Ok(())
}

/// Click a control that may be scrolled out of its list's view (a main-menu entry):
/// scroll the list with the mouse wheel until the control is visible, then click it.
pub fn click_scrolled(
    window: &slint::Window,
    role: AccessibleRole,
    label: &str,
) -> Result<(), String> {
    for _ in 0..60 {
        if inspect::find(window, role, label).is_some() {
            return input::click(window, role, label);
        }
        // The control exists but is clipped away: find it regardless of visibility.
        let mut hidden = None;
        for root in inspect::roots(window).iter().rev() {
            inspect::walk(root, &mut |item| {
                if hidden.is_none()
                    && item.accessible_role() == role
                    && inspect::label(item).is_some_and(|text| text == label)
                {
                    hidden = Some(item.clone());
                }
            });
        }
        let item = hidden.ok_or_else(|| format!("no {role:?} {label:?} at all"))?;
        let clip = inspect::clip_rect(&item)
            .ok_or_else(|| format!("{role:?} {label:?} is hidden but not scrolled away"))?;
        let full = inspect::rect(&item);
        let below = full.1 + full.3 / 2.0 > clip.1 + clip.3 / 2.0;
        window.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(clip.0 + clip.2 / 2.0, clip.1 + clip.3 / 2.0),
            delta_x: 0.0,
            delta_y: if below { -60.0 } else { 60.0 },
        });
        inspect::settle_tree(window);
    }
    Err(format!("{role:?} {label:?} never scrolled into view"))
}

/// The accessible value (a combo box's current text, a field's text) of a control.
pub fn accessible_value(
    window: &slint::Window,
    role: AccessibleRole,
    label: &str,
) -> Option<String> {
    inspect::find(window, role, label)?
        .accessible_string_property(i_slint_core::accessibility::AccessibleStringProperty::Value)
        .map(|value| value.to_string())
}

/// Choose `wanted` in a combo box: open it with a click, move with the arrow keys
/// until its value is `wanted` (at most `limit` steps down), confirm with Enter.
pub fn choose(app: &mut App, label: &str, wanted: &str, limit: usize) -> Result<(), String> {
    for _ in 0..=limit {
        if accessible_value(app.slint(), AccessibleRole::Combobox, label).as_deref() == Some(wanted)
        {
            return Ok(());
        }
        input::click(app.slint(), AccessibleRole::Combobox, label)?;
        app.settle();
        input::key(app.slint(), Key::DownArrow);
        input::key(app.slint(), Key::Return);
        app.settle();
    }
    Err(format!(
        "{label:?} shows {:?}, not {wanted:?}",
        accessible_value(app.slint(), AccessibleRole::Combobox, label)
    ))
}

/// Checks of behavior that is announced but not implemented yet. Each keeps its full
/// strength; while it fails, the scenario reports `PENDING` (when nothing else failed)
/// and names the check with its reason, so no failure hides behind it and a check that
/// starts passing is noticed (its mark should then be removed).
#[derive(Default)]
pub struct Pending(Vec<(String, &'static str, Option<String>)>);

impl Pending {
    /// Record `result` for the pending check `what`, implemented once `reason` lands.
    pub fn check(
        &mut self,
        what: impl Into<String>,
        reason: &'static str,
        result: Result<(), String>,
    ) {
        let what = what.into();
        // On the console at once too, so a later panic does not hide it.
        if let Err(failure) = &result {
            eprintln!("acceptance: pending check {what:?} fails: {failure}");
        }
        self.0.push((what, reason, result.err()));
    }
}

/// [`report`], then the pending checks: notes for each, and `Err("PENDING: …")` when
/// only pending checks failed.
pub fn report_with(
    context: &mut Context,
    failures: &[String],
    pending: &Pending,
) -> Result<(), String> {
    let mut still = Vec::new();
    for (what, reason, failure) in &pending.0 {
        match failure {
            Some(failure) => {
                context.note(format!("PENDING {what} ({reason}): {failure}"));
                still.push(format!("{what} ({reason}): {failure}"));
            }
            None => context.note(format!(
                "pending check passes now, remove its pending mark: {what} ({reason})"
            )),
        }
    }
    if !failures.is_empty() {
        let mut message = format!(
            "{} problem(s):\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
        if !still.is_empty() {
            message.push_str(&format!(
                "\n  and {} pending check(s) still failing (see the notes)",
                still.len()
            ));
        }
        return Err(message);
    }
    if still.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "PENDING: {} check(s) wait for announced behavior:\n  {}",
            still.len(),
            still.join("\n  ")
        ))
    }
}

/// F8 on the focused panel: the Trash confirmation (not the permanent one), confirmed
/// with Enter; the terminal report. A failed delete leaves its notice open.
pub fn trash(
    app: &mut App,
    context: &mut Context,
    shot: Option<&str>,
) -> Result<Arc<OperationProgress>, String> {
    panel::wait_input(app);
    let start = app.operations.len();
    input::key(app.slint(), Key::F8);
    wait_operation_dialog(app, "the Trash confirmation");
    if app.window.get_operation_dangerous() {
        return Err("F8 asked for a permanent delete".into());
    }
    if let Some(name) = shot {
        app.screenshot(context, name, "Move to Trash confirmation");
    }
    input::key(app.slint(), Key::Return);
    let progress = wait_operation(app, start);
    app.settle();
    Ok(progress)
}

/// The depth of the left panel's row `name` in the inline tree (None: not shown).
pub fn tree_depth(app: &App, name: &str) -> Option<i32> {
    app.rows(PaneId::Left)
        .iter()
        .find(|row| row.name == name)
        .map(|row| row.tree_depth)
}

/// Click the expander of `name` in the left panel (without modifiers: one level).
pub fn toggle_tree(app: &mut App, name: &str) -> Result<(), String> {
    panel::wait_input(app);
    let wanted = name.to_owned();
    app.until(&format!("{name}'s expander"), move |app| {
        app.rows(PaneId::Left).iter().any(|row| {
            row.name == wanted && (row.tree_expanded || matches!(row.tree_disclosure, 1 | 3))
        })
    });
    let row = app
        .rows(PaneId::Left)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| format!("no {name} row"))?;
    let verb = if row.tree_expanded {
        "Collapse"
    } else if row.tree_disclosure == 1 {
        "Discover"
    } else {
        "Expand"
    };
    let label = format!("{verb} folders in {name}");
    let left_edge = app.window.get_left_list_left() + app.window.get_left_list_width();
    let expander = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(text, _)| *text == label)
        .map(|(_, item)| item)
        .find(|item| inspect::center(item).x < left_edge)
        .ok_or_else(|| format!("no {label:?} expander in the left panel"))?;
    input::click_at(app.slint(), inspect::center(&expander));
    app.settle();
    Ok(())
}

/// Wait until the left panel's row `name` is at tree depth `depth` (None: gone).
#[track_caller]
pub fn wait_tree_depth(app: &mut App, name: &str, depth: Option<i32>) {
    let wanted = name.to_owned();
    app.until(&format!("{name} to be at depth {depth:?}"), move |app| {
        tree_depth(app, &wanted) == depth
    });
}

/// Join failures into one message, or Ok when there are none.
pub fn report(failures: &[String]) -> Result<(), String> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} problem(s):\n  {}",
            failures.len(),
            failures.join("\n  ")
        ))
    }
}
