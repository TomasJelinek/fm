//! Helpers of the cross-feature journey scenarios (`journeys_*.rs`): a journey record
//! that keeps going after a failed check and notes every step, waits that report
//! instead of panicking, keyboard-only and pointer-only moves, per-file answers to the
//! questions of a Copy, job windows and the Jobs list, and on-disk facts.
use crate::{
    Context,
    app::{self, App},
    encode,
    files_support::{self as files, Choice, Pending},
    input, inspect, panel,
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CopyOperationState, CopyProgress, PaneId};
use slint::platform::{Key, WindowAdapter};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

/// Tool windows are sized like desktop windows before they are read or clicked.
pub const TOOL_SIZE: (u32, u32) = (900, 640);

/// One journey's record: its steps in order (notes), failed checks and checks of
/// announced behavior that has not landed yet (pending).
#[derive(Default)]
pub struct Journey {
    steps: Vec<String>,
    failures: Vec<String>,
    pending: Pending,
}

impl Journey {
    /// Note a step that was done and checked.
    pub fn step(&mut self, text: impl Into<String>) {
        let number = self.steps.len() + 1;
        self.steps.push(format!("step {number}: {}", text.into()));
    }

    /// A check the journey goes on after; whether it held.
    pub fn check(&mut self, what: &str, result: Result<(), String>) -> bool {
        match result {
            Ok(()) => true,
            Err(error) => {
                self.failures.push(format!("{what}: {error}"));
                false
            }
        }
    }

    /// A check of announced behavior that has not landed yet: `reason` names it.
    #[allow(dead_code)] // No journey has such a check now; the quick drop became strict.
    pub fn pending(&mut self, what: &str, reason: &'static str, result: Result<(), String>) {
        self.pending.check(what, reason, result);
    }

    /// Report the journey: every step as a note, then its failures and pending checks.
    /// `ended` is how the journey's steps ended; an error there stopped it early.
    pub fn finish(
        mut self,
        context: &mut Context,
        ended: Result<(), String>,
    ) -> Result<(), String> {
        for step in &self.steps {
            context.note(step.clone());
        }
        if let Err(error) = ended {
            self.failures.push(format!(
                "the journey stopped after {} step(s): {error}",
                self.steps.len()
            ));
        }
        files::report_with(context, &self.failures, &self.pending)
    }
}

/// Ok when `found` holds exactly `wanted`.
pub fn same(found: &BTreeSet<String>, wanted: &[&str]) -> Result<(), String> {
    let wanted = names(wanted);
    if *found == wanted {
        Ok(())
    } else {
        Err(format!("found {found:?}, expected {wanted:?}"))
    }
}

pub fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// Run the application until `condition` holds; an error naming `what` after `limit`.
pub fn wait(
    app: &mut App,
    what: &str,
    limit: Duration,
    condition: impl Fn(&App) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + limit;
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what}: not within {limit:?} (left {:?}, right {:?}, notice {:?}, windows {:?})",
                app.location(PaneId::Left),
                app.location(PaneId::Right),
                files::shown_notice(app),
                tools::tool_titles(app)
            ));
        }
        app.pump(app::FRAME);
    }
}

// ---------------------------------------------------------------------------------------
// On disk

/// Write `data` to `path`, creating its folders.
pub fn write(path: &Path, data: impl AsRef<[u8]>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| files::io_error(parent, error))?;
    }
    fs::write(path, data).map_err(|error| files::io_error(path, error))
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("<{error}>"))
}

/// Ok when the file at `path` holds exactly `wanted`.
pub fn holds(path: &Path, wanted: &str) -> Result<(), String> {
    let found = read(path);
    if found == wanted {
        Ok(())
    } else {
        Err(format!(
            "{} holds {found:?}, expected {wanted:?}",
            path.display()
        ))
    }
}

/// Ok when nothing exists at `path` (links not followed).
pub fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(_) => Ok(()),
        Ok(_) => Err(format!("{} still exists", path.display())),
    }
}

/// The SHA-256 of a file, in hex.
pub fn sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| files::io_error(path, error))?;
    Ok(encode::hex(&encode::sha256(&bytes)))
}

/// Logical bytes of the regular files below `path`; links are not followed.
pub fn disk_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        0
    } else if metadata.is_dir() {
        fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| disk_size(&entry.path()))
            .sum()
    } else {
        metadata.len()
    }
}

/// The digits of a shown number ("4,105" or "4 105").
pub fn digits(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The permission bits of `path` (links not followed).
#[cfg(unix)]
pub fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path).map_or(0, |metadata| metadata.permissions().mode() & 0o7777)
}

/// The bytes of the member `name` of an uncompressed ustar archive.
pub fn tar_member(tar: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut offset = 0;
    while offset + 512 <= tar.len() {
        let header = &tar[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            return None;
        }
        let field = |from: usize, to: usize| {
            String::from_utf8_lossy(&header[from..to])
                .trim_end_matches('\0')
                .to_owned()
        };
        let prefix = field(345, 500);
        let path = if prefix.is_empty() {
            field(0, 100)
        } else {
            format!("{prefix}/{}", field(0, 100))
        };
        let size = usize::from_str_radix(field(124, 136).trim_matches([' ', '\0']), 8).ok()?;
        let start = offset + 512;
        if path == name {
            return tar.get(start..start + size).map(<[u8]>::to_vec);
        }
        offset = start + size.div_ceil(512) * 512;
    }
    None
}

// ---------------------------------------------------------------------------------------
// Panels

/// The names a pane lists at its top level, without `..`.
pub fn shown(app: &App, pane: PaneId) -> BTreeSet<String> {
    app.rows(pane)
        .iter()
        .filter(|row| !row.parent_entry && row.tree_depth == 0)
        .map(|row| row.name.to_string())
        .collect()
}

/// The marked names of a pane.
pub fn marked(app: &App, pane: PaneId) -> BTreeSet<String> {
    files::selected_names(app, pane).into_iter().collect()
}

/// Mark exactly `names` in `pane`: Ctrl+D, then Insert on each.
pub fn mark(app: &mut App, pane: PaneId, names: &[&str]) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    for name in names {
        panel::select_row(app, pane, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    same(&marked(app, pane), names)
}

/// Wait until `pane` lists exactly `wanted` (top level, without `..`) and is not loading.
pub fn wait_shown(
    app: &mut App,
    pane: PaneId,
    wanted: &[&str],
    limit: Duration,
) -> Result<(), String> {
    let set = names(wanted);
    wait(
        app,
        &format!("the {pane:?} panel to list {wanted:?}"),
        limit,
        move |app| !app.loading(pane) && shown(app, pane) == set,
    )
    .map_err(|error| format!("{error}; it lists {:?}", shown(app, pane)))
}

/// Open the row `name` of `pane` with Enter and wait until the panel lists `wanted`
/// (inside an archive the panel has no native location to wait for).
pub fn open_row(
    app: &mut App,
    pane: PaneId,
    name: &str,
    wanted: &[&str],
    limit: Duration,
) -> Result<(), String> {
    panel::select_row(app, pane, name)?;
    input::key(app.slint(), Key::Return);
    wait_shown(app, pane, wanted, limit).map_err(|error| format!("Enter on {name}: {error}"))
}

/// The path bar's text of `pane` (inside an archive the panel has no native location).
pub fn path_text(app: &App, pane: PaneId) -> String {
    match pane {
        PaneId::Left => app.window.get_left_path(),
        PaneId::Right => app.window.get_right_path(),
    }
    .to_string()
}

/// Backspace until `pane` shows the native `folder` (at most `levels` presses).
pub fn back_to(app: &mut App, pane: PaneId, folder: &Path, levels: usize) -> Result<(), String> {
    for _ in 0..levels {
        if app.location(pane).as_deref() == Some(folder) && !app.loading(pane) {
            return Ok(());
        }
        panel::focus_list(app, pane)?;
        let before = path_text(app, pane);
        input::key(app.slint(), Key::Backspace);
        let _ = wait(
            app,
            "Backspace to leave the folder",
            Duration::from_secs(20),
            |app| path_text(app, pane) != before && !app.loading(pane),
        );
    }
    let wanted = folder.to_path_buf();
    wait(
        app,
        &format!("Backspace back to {}", folder.display()),
        Duration::from_secs(20),
        move |app| app.location(pane).as_ref() == Some(&wanted) && !app.loading(pane),
    )
}

// ---------------------------------------------------------------------------------------
// Keyboard only

/// Give `pane` the keyboard focus with Tab.
pub fn key_focus(app: &mut App, pane: PaneId) -> Result<(), String> {
    for _ in 0..3 {
        panel::wait_input(app);
        if app.window.get_focused_pane() == pane.index() as i32 {
            return Ok(());
        }
        input::key(app.slint(), Key::Tab);
        app.settle();
    }
    Err(format!(
        "Tab did not give the {pane:?} panel the focus (focused pane {})",
        app.window.get_focused_pane()
    ))
}

/// Put the cursor of the focused `pane` on `name` with Home and Down only.
pub fn key_cursor(app: &mut App, pane: PaneId, name: &str) -> Result<(), String> {
    panel::wait_input(app);
    if app.window.get_focused_pane() != pane.index() as i32 {
        return Err(format!(
            "the {pane:?} panel does not have the keyboard focus (focused pane {})",
            app.window.get_focused_pane()
        ));
    }
    let index = panel::index_of(app, pane, name)?;
    input::key(app.slint(), Key::Home);
    for _ in 0..index {
        input::key(app.slint(), Key::DownArrow);
    }
    app.settle();
    match files::cursor_name(app, pane) {
        Some(found) if found == name => Ok(()),
        other => Err(format!(
            "Home and {index} × Down put the cursor on {other:?}, not {name:?}"
        )),
    }
}

/// The item that has the keyboard focus in `window`, if any.
pub fn focused(window: &slint::Window) -> Option<i_slint_core::item_tree::ItemRc> {
    i_slint_core::window::WindowInner::from_pub(window)
        .focus_item
        .borrow()
        .upgrade()
}

/// Whether the focused item of `window` lies inside the visible control `role`/`label`.
pub fn focus_inside(window: &slint::Window, role: AccessibleRole, label: &str) -> bool {
    let (Some(item), Some(area)) = (focused(window), inspect::find(window, role, label)) else {
        return false;
    };
    let (x, y, w, h) = inspect::rect(&area);
    let center = inspect::center(&item);
    center.x >= x && center.x <= x + w && center.y >= y && center.y <= y + h
}

// ---------------------------------------------------------------------------------------
// Pointer only

/// Click the footer cell of the function key `key` ("F5"), which must be enabled.
pub fn click_footer(app: &mut App, key: &str) -> Result<(), String> {
    panel::wait_input(app);
    let prefix = format!("{key} ");
    let (label, item) = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, _)| label == key || label.starts_with(&prefix))
        .ok_or_else(|| format!("no footer cell {key}"))?;
    if !tools::enabled(&item) {
        return Err(format!("the footer cell {label:?} is disabled"));
    }
    input::click_at(app.slint(), inspect::center(&item));
    app.settle();
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Copy and Move questions, job windows and Jobs

/// Answer each question of the Copy/Move whose reports start at `start` in
/// `app.copies` with what `decide` says for the question's texts, until the job ends;
/// its last report and the texts of every question answered.
pub fn answer_job(
    app: &mut App,
    start: usize,
    limit: Duration,
    decide: impl Fn(&[String]) -> Option<Choice>,
) -> Result<(Arc<CopyProgress>, Vec<Vec<String>>), String> {
    let deadline = Instant::now() + limit;
    let mut answered = Vec::new();
    loop {
        wait(
            app,
            "the job to end or ask",
            deadline.saturating_duration_since(Instant::now()),
            |app| {
                files::job(app, start).is_some_and(|job| job.state.is_terminal())
                    || files::decision_window(app).is_some()
            },
        )?;
        if let Some(job) = files::job(app, start).filter(|job| job.state.is_terminal()) {
            app.settle();
            return Ok((job, answered));
        }
        let window = files::decision_window(app).ok_or("the question closed by itself")?;
        app::layout_tool(&window, 820, 560);
        app.settle();
        let texts = inspect::visible_texts(window.window());
        let choice = decide(&texts).ok_or_else(|| format!("an unexpected question: {texts:?}"))?;
        let token = app.window.get_copy_token();
        files::answer(app, &window, &choice)?;
        answered.push(texts);
        let watched = window.clone();
        wait(
            app,
            &format!("the answer {choice:?} to be taken"),
            Duration::from_secs(30),
            move |app| !watched.window().is_visible() || app.window.get_copy_token() != token,
        )?;
    }
}

/// The visible tool windows titled `title`, oldest first.
pub fn titled(app: &App, title: &str) -> Vec<Tool> {
    app.tool_windows()
        .into_iter()
        .filter(|window| tools::title(window) == title)
        .collect()
}

/// Wait for one more window titled `title` than the `before` already open; sized.
pub fn new_window(app: &mut App, title: &str, before: usize) -> Result<Tool, String> {
    let wanted = title.to_owned();
    wait(
        app,
        &format!("a new {title} window"),
        Duration::from_secs(30),
        move |app| titled(app, &wanted).len() > before,
    )?;
    let window = titled(app, title)
        .pop()
        .ok_or_else(|| format!("the {title} window closed at once"))?;
    app::layout_tool(&window, TOOL_SIZE.0, TOOL_SIZE.1);
    app.settle();
    Ok(window)
}

/// Every visible text of a tool window, joined by line breaks.
pub fn window_text(window: &Tool) -> String {
    inspect::visible_texts(window.window()).join("\n")
}

/// Close a tool window with Escape, or else with its Close button; an error when it
/// stays open.
pub fn close_window(app: &mut App, window: &Tool, what: &str) -> Result<(), String> {
    if !window.window().is_visible() {
        return Ok(());
    }
    input::key(window.window(), Key::Escape);
    let watched = window.clone();
    let escaped = wait(
        app,
        &format!("Escape to close {what}"),
        Duration::from_secs(10),
        move |_| !watched.window().is_visible(),
    );
    if escaped.is_ok() {
        return Ok(());
    }
    tools::click_last(window.window(), "Close")?;
    let watched = window.clone();
    wait(
        app,
        &format!("Close to close {what}"),
        Duration::from_secs(10),
        move |_| !watched.window().is_visible(),
    )
    .map_err(|error| format!("Escape left {what} open, and then {error}"))
}

/// Pause the job `start` with its window's Pause button once it copies, and wait until
/// the window says Paused.
pub fn pause_job(app: &mut App, window: &Tool, start: usize, what: &str) -> Result<(), String> {
    wait(
        app,
        &format!("{what} to copy"),
        Duration::from_secs(60),
        move |app| {
            files::job(app, start).is_some_and(|job| {
                job.state.is_terminal()
                    || (job.state == CopyOperationState::Running
                        && (job.copied_bytes > 0 || job.files_completed > 0))
            })
        },
    )?;
    if files::job(app, start).is_some_and(|job| job.state.is_terminal()) {
        return Err(format!("{what} ended before it could be paused"));
    }
    tools::click_last(window.window(), "Pause")?;
    let watched = window.clone();
    wait(
        app,
        &format!("{what}'s window to say Paused"),
        Duration::from_secs(30),
        move |_| {
            inspect::visible_texts(watched.window())
                .iter()
                .any(|text| text == "Paused")
        },
    )?;
    match files::job(app, start) {
        Some(job) if job.paused && !job.state.is_terminal() => Ok(()),
        other => Err(format!(
            "{what} is not paused: {:?}",
            other.map(|job| (job.state, job.paused))
        )),
    }
}

/// Open Jobs with the toolbar's Jobs button; the window, sized.
pub fn open_jobs(app: &mut App) -> Result<Tool, String> {
    panel::wait_input(app);
    let before = titled(app, "Jobs").len();
    input::click_button(app.slint(), "Jobs")?;
    new_window(app, "Jobs", before)
}

/// The Jobs list's rows, top to bottom: (accessible label, every visible text).
pub fn job_rows(window: &Tool) -> Vec<(String, Vec<String>)> {
    let _ = app::render(window);
    let mut rows: Vec<(f32, String, Vec<String>)> =
        inspect::controls(window.window(), AccessibleRole::ListItem)
            .into_iter()
            .map(|(label, item)| (inspect::rect(&item).1, label, tools::item_texts(&item)))
            .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter()
        .map(|(_, label, texts)| (label, texts))
        .collect()
}

/// The first Jobs row that shows `title` and every one of `texts`.
pub fn job_row<'a>(
    rows: &'a [(String, Vec<String>)],
    title: &str,
    texts: &[&str],
) -> Option<&'a (String, Vec<String>)> {
    rows.iter().find(|(label, shown)| {
        (label.contains(title) || shown.iter().any(|text| text == title))
            && texts
                .iter()
                .all(|wanted| shown.iter().any(|text| text == wanted))
    })
}
