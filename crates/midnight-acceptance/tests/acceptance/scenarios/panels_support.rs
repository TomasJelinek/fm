//! Helpers of the panel scenarios (`panels_*.rs`): pointer gestures the harness has no
//! helper for (middle, right and double clicks), the window's physical-shortcut entry
//! that the native key hook calls, non-panicking waits, the facts of a panel the checks
//! compare, and the text geometry of an open dialog (is every line readable whole).
use crate::{
    app::App,
    files_support::{self as support, io_error},
    input, inspect,
};
use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
};
use midnight_core::PaneId;
use slint::{
    LogicalPosition,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const LEFT: PaneId = PaneId::Left;
pub const RIGHT: PaneId = PaneId::Right;

/// Labels of the close control in a dialog's header (`DialogHeader::close-label`).
pub const HEADER_CLOSE: [&str; 3] = ["Cancel dialog", "Close dialog", "Cancel file list"];

/// Pump the application until `condition` holds or `limit` passes; whether it held.
/// Unlike `App::until` this never panics, so a failing check can be recorded and the
/// scenario can go on with its other checks.
pub fn wait(app: &mut App, limit: Duration, condition: impl Fn(&App) -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        app.settle();
        if condition(app) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        app.idle(Duration::from_millis(10));
    }
}

/// Press and release one pointer button at `position`.
pub fn button_click(window: &slint::Window, position: LogicalPosition, button: PointerEventButton) {
    input::move_to(window, position);
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

pub fn middle_click(window: &slint::Window, position: LogicalPosition) {
    button_click(window, position, PointerEventButton::Middle);
}

pub fn right_click(window: &slint::Window, position: LogicalPosition) {
    button_click(window, position, PointerEventButton::Right);
}

/// Two left clicks in quick succession on one spot, after waiting out the
/// double-click interval since the previous click there.
pub fn double_click(app: &mut App, position: LogicalPosition) {
    if let Some((when, _)) = app.last_click
        && when.elapsed() < Duration::from_millis(650)
    {
        app.idle(Duration::from_millis(650).saturating_sub(when.elapsed()));
    }
    input::click_at(app.slint(), position);
    input::click_at(app.slint(), position);
    app.last_click = Some((Instant::now(), position));
}

/// A rubber-band selection: hold `modifiers`, press at `from`, move in small steps
/// while the application runs, release at `to`.
pub fn rubber_band(app: &mut App, from: LogicalPosition, to: LogicalPosition, modifiers: &[Key]) {
    let button = PointerEventButton::Left;
    for key in modifiers {
        input::press(app.slint(), *key);
    }
    input::move_to(app.slint(), from);
    app.slint().dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    const STEPS: u32 = 12;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        input::move_to(
            app.slint(),
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t),
        );
        app.idle(Duration::from_millis(16));
    }
    app.slint().dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    for key in modifiers.iter().rev() {
        input::release(app.slint(), *key);
    }
    app.settle();
}

/// The left panel's details strip: (label, value) pairs.
pub fn details(app: &App) -> Vec<(String, String)> {
    use slint::Model;
    app.window
        .get_left_details()
        .iter()
        .map(|row| (row.label.to_string(), row.value.to_string()))
        .collect()
}

/// A physical key (`Digit1`, `NumpadAdd`, `BracketLeft`, ...) as the native key hook
/// delivers it before Slint sees the typed character; whether a shortcut took it.
pub fn physical(app: &mut App, key: &str, control: bool, alt: bool, shift: bool) -> bool {
    let taken = app
        .window
        .invoke_physical_shortcut(key.into(), control, alt, shift, false);
    app.settle();
    taken
}

/// Every name the pane shows (all depths), in displayed order, without `..`.
pub fn shown(app: &App, pane: PaneId) -> Vec<String> {
    app.rows(pane)
        .iter()
        .filter(|row| !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect()
}

/// Top-level names the pane shows, in displayed order, without `..`.
pub fn top_level(app: &App, pane: PaneId) -> Vec<String> {
    app.rows(pane)
        .iter()
        .filter(|row| !row.parent_entry && row.tree_depth == 0)
        .map(|row| row.name.to_string())
        .collect()
}

/// The marked names, sorted.
pub fn marked(app: &App, pane: PaneId) -> Vec<String> {
    let mut names = support::selected_names(app, pane);
    names.sort();
    names
}

pub fn sorted(names: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    names.sort();
    names
}

pub fn cursor(app: &App, pane: PaneId) -> Option<String> {
    support::cursor_name(app, pane)
}

pub fn write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| io_error(path, error))
}

pub fn folder(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| io_error(path, error))
}

/// After a dialog closed: wait `settle` and report anything that appeared or stayed
/// (a notice, a window, a popup, held input) instead of plain browsing.
pub fn stays_idle(app: &mut App, settle: Duration) -> Result<(), String> {
    let deadline = Instant::now() + settle;
    while Instant::now() < deadline {
        app.idle(Duration::from_millis(20));
        if let Some(notice) = support::shown_notice(app) {
            return Err(format!("a notice appeared: {notice:?}"));
        }
    }
    if !support::main_idle(app) {
        return Err(format!(
            "the main window is not idle: modal {}, popups {}, tool windows {}, input held {}, operation {:?}, copy draft {:?}, metadata {:?}",
            app.window.get_embedded_modal_open(),
            inspect::roots(app.slint()).len() - 1,
            app.tool_windows().len(),
            app.window.get_mutation_input_blocked(),
            app.window.get_operation_token().to_string(),
            app.window.get_copy_draft_token().to_string(),
            app.window.get_metadata_token().to_string(),
        ));
    }
    Ok(())
}

/// Visible buttons of the open popups (dialogs, menus), topmost last.
pub fn popup_buttons(app: &App) -> Vec<(String, ItemRc)> {
    let mut found = Vec::new();
    for root in inspect::roots(app.slint()).iter().skip(1) {
        inspect::walk(root, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && inspect::visible_rect(item).is_some()
            {
                found.push((inspect::label(item).unwrap_or_default(), item.clone()));
            }
        });
    }
    found
}

/// Click the visible popup button labelled `label` (the topmost one).
pub fn click_popup_button(app: &mut App, label: &str) -> Result<(), String> {
    let buttons = popup_buttons(app);
    let item = buttons
        .iter()
        .rev()
        .find(|(text, _)| text == label)
        .map(|(_, item)| item.clone())
        .ok_or_else(|| {
            format!(
                "no {label:?} button in the open dialog; buttons {:?}",
                buttons.iter().map(|(text, _)| text).collect::<Vec<_>>()
            )
        })?;
    let (x, y, w, h) = inspect::visible_rect(&item).unwrap_or_else(|| inspect::rect(&item));
    let position = LogicalPosition::new(x + w / 2.0, y + h / 2.0);
    input::click_at(app.slint(), position);
    // A later click on the same spot within the double-click interval waits it out.
    app.last_click = Some((Instant::now(), position));
    app.settle();
    Ok(())
}

/// A visible main-window button whose label starts with `prefix` (toolbars, footer).
pub fn button_starting(app: &App, prefix: &str) -> Option<(String, ItemRc)> {
    inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, item)| label.starts_with(prefix) && inspect::visible_rect(item).is_some())
}

/// Click the visible main-window button whose label starts with `prefix`.
pub fn click_starting(app: &mut App, prefix: &str) -> Result<String, String> {
    let (label, item) = button_starting(app, prefix).ok_or_else(|| {
        format!(
            "no visible button starting with {prefix:?}; buttons {:?}",
            inspect::labels(app.slint(), AccessibleRole::Button)
        )
    })?;
    let (x, y, w, h) = inspect::visible_rect(&item).unwrap_or_else(|| inspect::rect(&item));
    let position = LogicalPosition::new(x + w / 2.0, y + h / 2.0);
    input::click_at(app.slint(), position);
    app.last_click = Some((Instant::now(), position));
    app.settle();
    Ok(label)
}

/// One text item of a window: its text and the part of it a user sees.
pub struct ShownText {
    pub text: String,
    /// Its laid-out rectangle (x, y, width, height).
    pub full: inspect::Rect,
    /// The part inside its clipping ancestors, if any.
    pub visible: Option<inspect::Rect>,
}

impl ShownText {
    /// The text is laid out taller than the part that shows: some lines can only be
    /// read by scrolling, or not at all.
    pub fn cut(&self) -> bool {
        self.visible
            .is_none_or(|visible| visible.3 + 1.0 < self.full.3 || visible.2 + 1.0 < self.full.2)
    }
}

fn text_of(item: &ItemRc) -> Option<String> {
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    if let Some(text) = item.downcast::<ComplexText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    None
}

/// Non-empty visible texts of the topmost popup (an open dialog or notice).
pub fn dialog_texts(app: &App) -> Vec<ShownText> {
    let roots = inspect::roots(app.slint());
    let Some(root) = roots.iter().skip(1).last() else {
        return Vec::new();
    };
    let mut texts = Vec::new();
    inspect::walk(root, &mut |item| {
        if let Some(text) = text_of(item)
            && !text.trim().is_empty()
            && item.is_visible()
        {
            texts.push(ShownText {
                text,
                full: inspect::rect(item),
                visible: inspect::visible_rect(item),
            });
        }
    });
    texts
}

/// Gives a path's read access back when dropped, so a failed check never leaves an
/// unreadable folder behind.
pub struct ReadGuard(pub PathBuf);

impl Drop for ReadGuard {
    fn drop(&mut self) {
        let _ = crate::fixtures::allow_read(&self.0);
    }
}

/// Make `path` unreadable until the guard drops (`fixtures::deny_read`: mode 000 on
/// Unix, a deny entry for this account on Windows).
pub fn lock(path: &Path) -> Result<ReadGuard, String> {
    crate::fixtures::deny_read(path)?;
    Ok(ReadGuard(path.to_path_buf()))
}

/// Seconds since the epoch of a civil UTC date and time (Howard Hinnant's algorithm).
pub fn civil_seconds(date: (i64, i64, i64), time: (i64, i64, i64)) -> i64 {
    let (year, month, day) = date;
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    days * 86_400 + time.0 * 3_600 + time.1 * 60 + time.2
}

/// The instant a civil date and time at `offset` minutes east of UTC denote.
pub fn instant(
    date: (i64, i64, i64),
    time: (i64, i64, i64),
    nanos: u32,
    offset: i64,
) -> std::time::SystemTime {
    let seconds = civil_seconds(date, time) - offset * 60;
    std::time::UNIX_EPOCH + Duration::new(seconds as u64, nanos)
}

/// Click the Attributes dialog's three-state box `label` (Unchanged, Set, Clear, in
/// turn) until it shows `choice`: Windows' Ctrl+F2 has attribute boxes where Unix has
/// permission boxes and an octal mode.
pub fn set_attribute(app: &mut App, label: &str, choice: &str) -> Result<(), String> {
    let state = |app: &App| {
        inspect::find(app.slint(), AccessibleRole::Checkbox, label)
            .map(|item| crate::tools_support::value(&item))
    };
    for _ in 0..3 {
        if state(app).as_deref() == Some(choice) {
            return Ok(());
        }
        input::click(app.slint(), AccessibleRole::Checkbox, label)?;
        app.settle();
    }
    match state(app) {
        Some(shown) if shown == choice => Ok(()),
        shown => Err(format!(
            "the {label:?} attribute shows {shown:?}, not {choice:?}"
        )),
    }
}

/// Whether the file `path` has the Windows Read-only attribute (never on Unix).
pub fn read_only_attribute(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const READ_ONLY: u32 = 0x1;
        std::fs::metadata(path).is_ok_and(|metadata| metadata.file_attributes() & READ_ONLY != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Open Ctrl+F2 (or Ctrl+F7 with `case`) on the focused panel and wait until the
/// editor has loaded its captured items.
pub fn open_metadata(app: &mut App, case: bool) -> Result<(), String> {
    crate::panel::wait_input(app);
    input::chord(
        app.slint(),
        &[Key::Control],
        if case { Key::F7 } else { Key::F2 },
    );
    if !wait(app, Duration::from_secs(20), |app| {
        // Usable once the captured items' capabilities and current modes arrived.
        !app.window.get_metadata_token().is_empty()
            && !app.window.get_metadata_busy()
            && app.window.get_metadata_can_modified()
            && (!cfg!(unix) || app.window.get_metadata_unix_known_mask() != 0)
    }) {
        return Err(format!(
            "Ctrl+{} did not open a loaded editor (token {:?}, busy {})",
            if case { "F7" } else { "F2" },
            app.window.get_metadata_token().to_string(),
            app.window.get_metadata_busy()
        ));
    }
    Ok(())
}

/// Review changes, Apply and wait for the metadata job; the editor must close with
/// nothing else left open. The job's terminal report.
/// Also returns the reviewed summary and preview the editor showed before Apply.
pub fn apply_metadata(
    app: &mut App,
) -> Result<(std::sync::Arc<midnight_core::OperationProgress>, String), String> {
    use midnight_core::{MutationKind, OperationState};
    let start = app.operations.len();
    input::click_button(app.slint(), "Review changes")?;
    if !wait(app, Duration::from_secs(20), |app| {
        app.window.get_metadata_preview_current()
            && !app.window.get_metadata_busy()
            && inspect::find(app.slint(), AccessibleRole::Button, "Apply").is_some()
    }) {
        return Err(format!(
            "Review changes did not offer Apply (error {:?})",
            app.window.get_metadata_error().to_string()
        ));
    }
    let error = app.window.get_metadata_error().to_string();
    if !error.is_empty() {
        return Err(format!("the review reported {error:?}"));
    }
    let reviewed = format!(
        "{} / {}",
        app.window.get_metadata_summary(),
        app.window.get_metadata_preview_text()
    );
    input::click_button(app.slint(), "Apply")?;
    let finished = |app: &App| {
        app.operations[start..]
            .iter()
            .find(|progress| {
                progress.kind == MutationKind::EditMetadata && progress.state.is_terminal()
            })
            .cloned()
    };
    if !wait(app, Duration::from_secs(30), |app| finished(app).is_some()) {
        return Err("the metadata job never finished".into());
    }
    let progress = finished(app).ok_or("no metadata report")?;
    if !wait(app, Duration::from_secs(10), support::main_idle) {
        return Err("the editor did not close after Apply".into());
    }
    stays_idle(app, Duration::from_millis(300))?;
    if progress.state != OperationState::Completed {
        return Err(format!(
            "the metadata job ended {:?}: {:?}",
            progress.state, progress.errors
        ));
    }
    Ok((progress, reviewed))
}

/// The accessible value (a field's text, a spin box's value) of a control.
pub fn value(app: &App, role: AccessibleRole, label: &str) -> Option<String> {
    inspect::find(app.slint(), role, label)?
        .accessible_string_property(i_slint_core::accessibility::AccessibleStringProperty::Value)
        .map(|value| value.to_string())
}

/// Whether a check box is ticked (its accessible checked state).
pub fn checked(app: &App, label: &str) -> Option<bool> {
    inspect::find(app.slint(), AccessibleRole::Checkbox, label)?
        .accessible_string_property(i_slint_core::accessibility::AccessibleStringProperty::Checked)
        .map(|value| value == "true")
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
