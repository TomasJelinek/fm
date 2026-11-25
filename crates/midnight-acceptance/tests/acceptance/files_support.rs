//! Helpers of the file-operation and panel scenarios (`scenarios/files_*.rs`): the
//! Copy/Move dialog (its Links choice by pointer, by the open list's keys or by keyboard,
//! and the confirmations it holds), Delete, Rename, New file/folder dialogs and the
//! decision window driven by real keys and clicks, and the on-disk facts the scenarios
//! compare.
use crate::{
    Context,
    app::{self, App},
    input, inspect, panel,
};
use i_slint_core::items::{AccessibleRole, TextInput};
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

/// An entry's identity, links not followed: its device and inode on Unix, its volume
/// serial number and file index on Windows. An entry with the same identity after a
/// Move was renamed, not copied.
pub fn identity(path: &Path) -> Result<(u64, u64), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        identity_windows::of(path).map_err(|error| io_error(path, error))
    }
}

/// `GetFileInformationByHandle`, which the suite's `windows` features do not include, is
/// declared here, as the NTFS fixtures declare `DeviceIoControl`.
#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "one audited GetFileInformationByHandle call that reads an entry's file index"
)]
mod identity_windows {
    use std::{
        ffi::c_void,
        fs::OpenOptions,
        io,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::Path,
    };

    /// Open a folder itself (`FILE_FLAG_BACKUP_SEMANTICS`) and a link itself, not what it
    /// names (`FILE_FLAG_OPEN_REPARSE_POINT`).
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// `information` is a `BY_HANDLE_FILE_INFORMATION`: 13 DWORDs (attributes, three
        /// FILETIMEs, the volume serial number, the size, the link count, the file index).
        fn GetFileInformationByHandle(file: *mut c_void, information: *mut u32) -> i32;
    }

    /// The volume serial number and the 64-bit file index of `path`.
    pub fn of(path: &Path) -> io::Result<(u64, u64)> {
        // No data access: only the entry's own facts are queried.
        let file = OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let mut information = [0_u32; 13];
        // SAFETY: `file` is a live handle for the call's duration, and `information` is a
        // writable, DWORD-aligned BY_HANDLE_FILE_INFORMATION (13 DWORDs) that the
        // synchronous call fills and that outlives it.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok((
            u64::from(information[7]),
            (u64::from(information[11]) << 32) | u64::from(information[12]),
        ))
    }
}

/// What a Move from another drive said before any job where the destination could not
/// keep a replaced file recoverable (finding O-1, superseded: such a Move now goes wherever
/// Copy goes). The scenarios fail when they still see it. Its start and its end.
pub const MOVE_REFUSED_START: &str = "Items from another drive cannot be moved ";
pub const MOVE_REFUSED_END: &str =
    "Move cannot verify their copies there before removing the originals. Copy them instead.";

/// Whether `text` is an O-1 refusal, whatever destination it names.
pub fn is_move_refusal(text: &str) -> bool {
    text.contains(MOVE_REFUSED_START) && text.contains(MOVE_REFUSED_END)
}

/// Whether a Move from another drive may go into `folder`: wherever Copy goes, so the
/// native adapters refuse none up front (the Move then moves or, where an item cannot be
/// removed, keeps that source). `None` is not given any more; the parameter stays for the
/// scenarios that name the folder.
pub fn takes_cross_volume_move(folder: &Path) -> Option<bool> {
    let _ = folder;
    Some(true)
}

/// The root of the filesystem holding `path`, where Ctrl+Backspace goes: `/` on Unix,
/// the drive's root (`D:\`) on Windows.
pub fn filesystem_root(path: &Path) -> PathBuf {
    path.ancestors()
        .last()
        .map_or_else(PathBuf::new, Path::to_path_buf)
}

/// A relative path with `/` between its names on every platform, as the scenarios
/// spell their expectations (a Windows path's `\` never ends up in a key).
pub fn slashed(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every entry below `root` (relative path to facts, `/` between names), links not
/// followed.
pub fn tree_facts(root: &Path) -> Result<BTreeMap<String, Facts>, String> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeMap<String, Facts>) -> Result<(), String> {
        for entry in fs::read_dir(path).map_err(|error| io_error(path, error))? {
            let entry = entry.map_err(|error| io_error(path, error))?;
            let full = entry.path();
            let found = facts(&full)?;
            let relative = slashed(full.strip_prefix(root).unwrap_or(&full));
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

/// Open the Jobs window as a user does, with the function bar's Jobs button.
pub fn open_jobs(app: &mut App) -> Result<(), String> {
    let item = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, _)| label == "Jobs" || label.starts_with("Jobs "))
        .map(|(_, item)| item)
        .ok_or_else(|| {
            format!(
                "the main window shows no Jobs button: {:?}",
                inspect::labels(app.slint(), AccessibleRole::Button)
            )
        })?;
    input::click_at(app.slint(), inspect::center(&item));
    app.settle();
    Ok(())
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
    /// The Links choice the dialog offered once its destination's link facts arrived,
    /// before the run chose; only when the run chose or expected a hold (otherwise the
    /// facts are not awaited).
    pub offered: Option<LinkView>,
    /// The Links choice as the dialog showed it when it was confirmed; None when the
    /// dialog offered none (archive members).
    pub links: Option<LinkView>,
    /// What the dialog said when it held the first confirmation ([`TransferSpec::hold`]).
    pub hold: Option<Hold>,
}

/// The dialog held a confirmation: it explained something and stayed open, and the next
/// confirmation started the transfer.
#[derive(Clone, Debug)]
pub struct Hold {
    /// The dialog's message (`copy-draft-error`).
    pub text: String,
    /// Whether the dialog drew the message (its warning text), not only reported it.
    pub shown: bool,
    /// The Links choice while the dialog held.
    pub links: Option<LinkView>,
}

/// The Copy/Move dialog's Links choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkChoice {
    /// "Copy the files links point to", the first entry.
    CopyFiles,
    /// "Keep links as links", the second entry.
    Keep,
}

impl LinkChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::CopyFiles => COPY_LINKED_FILES,
            Self::Keep => KEEP_LINKS,
        }
    }

    /// The entry's index in the Links list.
    pub fn index(self) -> i32 {
        match self {
            Self::CopyFiles => 0,
            Self::Keep => 1,
        }
    }
}

/// How a scenario makes the Links choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkInput {
    /// Open the list with a click, move to the entry with an arrow key and press Enter in
    /// the open list (which must choose the entry, not confirm the dialog).
    #[default]
    ListKeys,
    /// Open the list with a click and click the entry.
    Pointer,
    /// Tab to the Links box and press Up or Down with the list closed; the dialog is then
    /// confirmed with Enter from the box itself.
    Keyboard,
}

/// What the Copy/Move dialog shows about links (`OperationPresentation`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    /// The entry shown ("" when the index names none).
    pub choice: String,
    pub index: i32,
    /// The entries, in order.
    pub choices: Vec<String>,
    /// Why copying the files is the default here; empty where keeping links is.
    pub reason: String,
    /// What keeping links does here, while that is the choice and cannot work.
    pub warning: String,
    /// The Links box as a screen reader reads it: its value and its description.
    pub value: Option<String>,
    pub description: Option<String>,
    /// Which of the reason and the warning the dialog draws.
    pub reason_shown: bool,
    pub warning_shown: bool,
}

impl LinkView {
    /// The choice shown, when it is one of the two entries.
    pub fn chosen(&self) -> Option<LinkChoice> {
        [LinkChoice::CopyFiles, LinkChoice::Keep]
            .into_iter()
            .find(|choice| choice.index() == self.index && choice.label() == self.choice)
    }

    /// Differences from the default view of a fixed local folder: "Keep links as links"
    /// of exactly the two entries, without a reason or a warning.
    pub fn plain_default(&self) -> Vec<String> {
        let mut differences = Vec::new();
        if self.choices != [COPY_LINKED_FILES, KEEP_LINKS] {
            differences.push(format!("the entries are {:?}", self.choices));
        }
        if self.chosen() != Some(LinkChoice::Keep) || self.value.as_deref() != Some(KEEP_LINKS) {
            differences.push(format!(
                "it shows {:?} (index {}, read as {:?}), not {KEEP_LINKS:?}",
                self.choice, self.index, self.value
            ));
        }
        if !self.reason.is_empty()
            || !self.warning.is_empty()
            || self.reason_shown
            || self.warning_shown
        {
            differences.push(format!(
                "it gives the reason {:?} and the warning {:?}",
                self.reason, self.warning
            ));
        }
        if self
            .description
            .as_deref()
            .is_some_and(|text| !text.is_empty())
        {
            differences.push(format!(
                "a screen reader hears the description {:?}",
                self.description
            ));
        }
        differences
    }
}

/// The open Copy/Move dialog's Links choice; None when the dialog is closed or offers
/// none.
pub fn link_view(app: &App) -> Option<LinkView> {
    use i_slint_core::accessibility::AccessibleStringProperty;
    use slint::Model;
    let form = operation_form(app);
    if app.window.get_copy_draft_token().is_empty() || !form.get_links_visible() {
        return None;
    }
    let choices: Vec<String> = form
        .get_link_choices()
        .iter()
        .map(|choice| choice.to_string())
        .collect();
    let index = form.get_link_choice();
    let combo = inspect::find(app.slint(), AccessibleRole::Combobox, LINKS_LABEL);
    let property = |property| {
        combo
            .as_ref()
            .and_then(|item| item.accessible_string_property(property))
            .map(|value| value.to_string())
    };
    let reason = form.get_link_reason().to_string();
    let warning = form.get_link_warning().to_string();
    let drawn = inspect::visible_texts(app.slint());
    Some(LinkView {
        choice: usize::try_from(index)
            .ok()
            .and_then(|index| choices.get(index).cloned())
            .unwrap_or_default(),
        index,
        choices,
        reason_shown: !reason.is_empty() && drawn.contains(&reason),
        warning_shown: !warning.is_empty() && drawn.contains(&warning),
        reason,
        warning,
        value: property(AccessibleStringProperty::Value),
        description: property(AccessibleStringProperty::Description),
    })
}

/// Whether the backend answered, after the trace `before` was taken, what a dialog's
/// destination does with links (`Event::TransferLinksInspected`). Read from the app's
/// event trace until the harness counts these events (audit H4).
pub fn link_facts_arrived(app: &App, before: &[String]) -> bool {
    app.recent_events()
        .iter()
        .any(|line| line.starts_with("TransferLinksInspected(") && !before.contains(line))
}

/// Wait (hang guard 10 s) until the open dialog's destination link facts arrived after
/// the trace `before` was taken; false when they were not seen.
pub fn wait_link_facts(app: &mut App, before: &[String]) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.settle();
        if link_facts_arrived(app, before) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        app.idle(Duration::from_millis(10));
    }
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
    /// Set "Preserve times and attributes" (false explicitly clears the default).
    pub preserve: bool,
    /// Make this Links choice ([`Self::link_input`] says how) once the destination's link
    /// facts arrived. The dialog must offer it: a choice it does not show fails.
    pub links: Option<LinkChoice>,
    pub link_input: LinkInput,
    /// The first confirmation must hold (the dialog explains and stays open, as for links
    /// kept where none can be stored or a typed folder whose link default differs) and
    /// the second start the transfer. Without it a held confirmation is a refusal.
    pub hold: bool,
    /// Confirm with the Links choice the destination's link facts decide: wait for them
    /// first (as a choice or a hold also does), and record what the dialog offered.
    pub offered_links: bool,
    /// Set "Wait for earlier transfers".
    pub wait: Option<bool>,
    /// Runs once the dialog is set up, just before it is confirmed (a change on disk
    /// while the dialog is open, such as a selected file deleted).
    pub before_confirm: Option<&'a dyn Fn() -> Result<(), String>>,
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
            preserve: false,
            links: None,
            link_input: LinkInput::default(),
            hold: false,
            offered_links: false,
            wait: None,
            before_confirm: None,
            choices: Vec::new(),
            shot: None,
            decision_shot: None,
            limit: None,
        }
    }

    /// The Links choice this run makes ([`Self::links`]).
    pub fn link_choice(&self) -> Option<LinkChoice> {
        self.links
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
pub const WAIT_OPTION: &str = "Wait for earlier transfers";
/// The Copy/Move dialog's option that keeps times and attributes.
pub const PRESERVE_OPTION: &str = "Preserve times and attributes";

/// The Copy/Move dialog's Links entries: the first copies the files links point to, the
/// second keeps links as links.
pub const COPY_LINKED_FILES: &str = "Copy the files links point to";
pub const KEEP_LINKS: &str = "Keep links as links";
/// The accessible label of the dialog's Links box.
pub const LINKS_LABEL: &str = "Links";

/// Whether the keyboard focus of `window` lies inside the visible control `role`/`label`.
pub fn focus_inside(window: &slint::Window, role: AccessibleRole, label: &str) -> bool {
    let focused = i_slint_core::window::WindowInner::from_pub(window)
        .focus_item
        .borrow()
        .upgrade();
    let Some(item) = focused else {
        return false;
    };
    // Any visible control of that name (the main window may show another one).
    let controls = inspect::find_all(window, role, label);
    if role == AccessibleRole::TextInput {
        // A long destination scrolls its native editor inside the field. The
        // center of the editor can lie outside the field while the caret is visible.
        return item.downcast::<TextInput>().is_some()
            && controls
                .iter()
                .any(|field| *field == item || inspect::is_ancestor(field, &item));
    }
    let center = inspect::center(&item);
    controls.iter().any(|area| {
        let (x, y, w, h) = inspect::rect(area);
        center.x >= x && center.x <= x + w && center.y >= y && center.y <= y + h
    })
}

/// Choose the open dialog's link handling as a user does, by `input`
/// ([`LinkInput`]). The dialog must stay open and admit nothing until it is confirmed:
/// Enter in the open list chooses the entry, it does not confirm the dialog.
pub fn choose_links(app: &mut App, wanted: LinkChoice, input: LinkInput) -> Result<(), String> {
    let jobs = app.copies.len();
    let token = app.window.get_copy_draft_token();
    if link_view(app).is_none() {
        return Err(format!(
            "{:?} was asked for, but the dialog offers no Links choice",
            wanted.label()
        ));
    }
    let step = if wanted == LinkChoice::CopyFiles {
        Key::UpArrow
    } else {
        Key::DownArrow
    };
    match input {
        LinkInput::ListKeys | LinkInput::Pointer => {
            let roots = inspect::roots(app.slint()).len();
            click_scrolled(app.slint(), AccessibleRole::Combobox, LINKS_LABEL)?;
            app.settle();
            if inspect::roots(app.slint()).len() <= roots {
                return Err("clicking the Links box opened no list".into());
            }
            if input == LinkInput::ListKeys {
                input::key(app.slint(), step);
                input::key(app.slint(), Key::Return);
            } else {
                let list = inspect::roots(app.slint()).pop().expect("the Links list");
                let mut entry = None;
                let mut offered = Vec::new();
                inspect::walk(&list, &mut |item| {
                    if item.accessible_role() == AccessibleRole::ListItem
                        && let Some(label) = inspect::label(item)
                    {
                        if label == wanted.label() && entry.is_none() {
                            entry = Some(item.clone());
                        }
                        offered.push(label);
                    }
                });
                let Some(entry) = entry else {
                    input::key(app.slint(), Key::Escape);
                    app.settle();
                    return Err(format!(
                        "the Links list offers {offered:?}, not {:?}",
                        wanted.label()
                    ));
                };
                input::click_at(app.slint(), inspect::center(&entry));
            }
            app.settle();
            if inspect::roots(app.slint()).len() > roots {
                return Err(format!(
                    "the Links list stayed open after choosing {:?}",
                    wanted.label()
                ));
            }
        }
        LinkInput::Keyboard => {
            for _ in 0..8 {
                if focus_inside(app.slint(), AccessibleRole::Combobox, LINKS_LABEL) {
                    break;
                }
                input::key(app.slint(), Key::Tab);
                app.settle();
            }
            if !focus_inside(app.slint(), AccessibleRole::Combobox, LINKS_LABEL) {
                return Err("Tab never reached the Links box".into());
            }
            input::key(app.slint(), step);
            app.settle();
        }
    }
    if app.window.get_copy_draft_token() != token || app.copies.len() != jobs {
        return Err(format!(
            "choosing {:?} ({input:?}) confirmed the dialog",
            wanted.label()
        ));
    }
    match link_view(app) {
        Some(view)
            if view.chosen() == Some(wanted) && view.value.as_deref() == Some(wanted.label()) =>
        {
            Ok(())
        }
        other => Err(format!(
            "after choosing {:?} ({input:?}) Links shows {other:?}",
            wanted.label()
        )),
    }
}

/// A confirmed dialog: where its job's reports start, when it was confirmed, its Links
/// choice and what it said when it held.
struct Confirmed {
    start: usize,
    confirmed: Instant,
    offered: Option<LinkView>,
    links: Option<LinkView>,
    hold: Option<Hold>,
}

/// Press F5/F6 on the focused panel, set the dialog up from `spec` and confirm with
/// Enter, once more when the dialog holds and `spec.hold` expects it. Returns once the
/// dialog closed (a job was admitted, or a notice says why not). The dialog's own
/// refusal, and a hold that was not expected (or one expected that did not come), is
/// an `Err`.
fn confirm_dialog(
    app: &mut App,
    context: &mut Context,
    spec: &TransferSpec<'_>,
) -> Result<Confirmed, String> {
    panel::wait_input(app);
    let start = app.copies.len();
    let trace = app.recent_events();
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
    set_check(app, PRESERVE_OPTION, spec.preserve, |app| {
        operation_form(app).get_option_checked()
    })?;
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
    // The Links choice, made once the destination's link facts are in (they decide
    // the default and whether keeping links holds), and last, so that Enter by
    // keyboard confirms from the Links box.
    let choice = spec.link_choice();
    let awaited = (choice.is_some() || spec.hold || spec.offered_links) && link_view(app).is_some();
    if awaited && !wait_link_facts(app, &trace) && (spec.hold || spec.offered_links) {
        return Err("the destination's link facts did not arrive within 10 s".into());
    }
    let offered = if awaited { link_view(app) } else { None };
    if let Some(choice) = choice {
        choose_links(app, choice, spec.link_input)?;
    }
    let links = link_view(app);
    if let Some(name) = spec.shot {
        app.screenshot(
            context,
            name,
            if moving { "Move dialog" } else { "Copy dialog" },
        );
    }
    if let Some(before_confirm) = spec.before_confirm {
        before_confirm()?;
    }
    let confirmed = Instant::now();
    let mut hold = None;
    loop {
        // Enter confirms from the destination field, the check boxes, the Links box
        // and the primary button.
        input::key(app.slint(), Key::Return);
        // Until the dialog closes (a job was admitted, or a notice says why not) or
        // says why it does not start: a typed folder is checked by the backend first.
        app.until("the Copy/Move dialog to start or explain", move |app| {
            app.window.get_copy_draft_token().is_empty()
                || !app.window.get_copy_draft_error().is_empty()
                || !operation_form(app).get_warning().is_empty()
                || app.copies.len() > start
        });
        // The presenter's message, or the window's own ("The transfer was not admitted").
        let error = Some(app.window.get_copy_draft_error().to_string())
            .filter(|error| !error.is_empty())
            .unwrap_or_else(|| operation_form(app).get_warning().to_string());
        if app.window.get_copy_draft_token().is_empty() || error.is_empty() {
            break;
        }
        if !spec.hold || hold.is_some() {
            return Err(format!("the dialog refused the transfer: {error}"));
        }
        hold = Some(Hold {
            shown: inspect::visible_texts(app.slint())
                .iter()
                .any(|text| text.contains(&error)),
            text: error,
            links: link_view(app),
        });
    }
    if spec.hold && hold.is_none() {
        return Err(format!(
            "the first confirmation started the transfer instead of holding (Links {links:?})"
        ));
    }
    Ok(Confirmed {
        start,
        confirmed,
        offered,
        links,
        hold,
    })
}

/// Confirm a Copy/Move (as [`transfer`] does) without waiting for it: returns the index
/// of its first report in `app.copies` once the job was admitted.
pub fn start_transfer(
    app: &mut App,
    context: &mut Context,
    spec: TransferSpec<'_>,
) -> Result<usize, String> {
    let start = confirm_dialog(app, context, &spec)?.start;
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
    let Confirmed {
        start,
        confirmed,
        offered,
        links,
        hold,
    } = confirm_dialog(app, context, &spec)?;
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
                offered,
                links,
                hold,
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
