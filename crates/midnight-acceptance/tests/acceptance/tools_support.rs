//! Helpers of the tools scenarios (`scenarios/tools_*.rs`): tool windows, popups and
//! tooltips, accessible states, the in-process clipboard's copies, fitted dialog footers
//! with More…, context menus, the rows of selectable result lists and their cursor, file
//! times and sparse files, and independent SHA-1 and SHA-512 implementations for checking
//! the Checksums window.
use super::{
    app::{self, App},
    input, inspect,
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText, StyledTextItem},
    window::{WindowInner, WindowKind},
};
use midnight_core::FileClipboard;
use midnight_ui::NoticePresentation;
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{
        Key, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::MinimalSoftwareWindow,
    },
};
use std::{
    fs,
    path::Path,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

pub type Tool = Rc<MinimalSoftwareWindow>;

/// The notice the main window shows (title, body), if one is open. `App::notice`
/// keeps the last title after the notice was dismissed; the presentation's token
/// says whether it is still shown.
pub fn shown_notice(app: &App) -> Option<(String, String)> {
    let notice = app.window.global::<NoticePresentation>();
    (!notice.get_token().is_empty()).then(|| {
        (
            notice.get_title().to_string(),
            notice.get_body().to_string(),
        )
    })
}

/// A folder notice's Copy details starts with what the notice shows: its title, its
/// reason and, when it names one, the folder (`Title\n\nReason\n\nFolder\n<path>`),
/// before any diagnostic report. Compares the in-process clipboard with the notice.
pub fn notice_copy_starts_as_shown(app: &App) -> Result<(), String> {
    let notice = app.window.global::<NoticePresentation>();
    let mut start = format!("{}\n\n{}", notice.get_title(), notice.get_body());
    let folder = notice.get_folder_path().to_string();
    if !folder.is_empty() {
        start.push_str(&format!("\n\nFolder\n{folder}"));
    }
    let text = app
        .clipboard
        .as_ref()
        .and_then(|clipboard| clipboard.text())
        .unwrap_or_default();
    if text.starts_with(&start) {
        Ok(())
    } else {
        Err(format!(
            "Copy details copied {:?}, which does not start with the notice's title, reason \
             and folder {start:?}",
            text.chars()
                .take(start.chars().count() + 80)
                .collect::<String>()
        ))
    }
}

/// Click the shown (not transparent, not clipped away) button with this label that
/// comes last in tree order: the topmost one a user can see.
pub fn click_shown(window: &slint::Window, label: &str) -> Result<(), String> {
    let item = inspect::find_all(window, AccessibleRole::Button, label)
        .into_iter()
        .rfind(|item| inspect::visible_rect(item).is_some())
        .ok_or_else(|| {
            format!(
                "no shown {label:?} button; visible text: {:?}",
                inspect::visible_texts(window)
            )
        })?;
    let (x, y, w, h) = inspect::visible_rect(&item).unwrap_or_else(|| inspect::rect(&item));
    input::click_at(window, LogicalPosition::new(x + w / 2.0, y + h / 2.0));
    Ok(())
}

/// Scroll the list that shows `anchor` with the mouse wheel until a control with this
/// role and label is in view (down first, then up); returns it.
pub fn scroll_to(
    app: &mut App,
    window: &slint::Window,
    anchor: &ItemRc,
    role: AccessibleRole,
    label: &str,
) -> Result<ItemRc, String> {
    if let Some(item) = inspect::find(window, role, label) {
        return Ok(item);
    }
    let clip = inspect::clip_rect(anchor).unwrap_or_else(|| inspect::rect(anchor));
    let position = LogicalPosition::new(clip.0 + clip.2 / 2.0, clip.1 + clip.3 / 2.0);
    for (delta, steps) in [(-40.0, 60), (40.0, 120)] {
        for _ in 0..steps {
            window.dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: delta,
            });
            app.settle();
            inspect::settle_tree(window);
            if let Some(item) = inspect::find(window, role, label) {
                return Ok(item);
            }
        }
    }
    Err(format!("no {role:?} {label:?} after scrolling the list"))
}

/// Labels of every control with this role in the scrolling list that shows `anchor`,
/// in order: the list is scrolled to its top, then down until nothing new appears.
/// `keep` filters the labels (and the items considered) inside the topmost popup.
pub fn scrolled_labels(
    app: &mut App,
    window: &slint::Window,
    anchor: &ItemRc,
    role: AccessibleRole,
    keep: impl Fn(&str) -> bool,
) -> Vec<String> {
    let clip = inspect::clip_rect(anchor).unwrap_or_else(|| inspect::rect(anchor));
    let position = LogicalPosition::new(clip.0 + clip.2 / 2.0, clip.1 + clip.3 / 2.0);
    let scroll = |app: &mut App, delta: f32| {
        window.dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: delta,
        });
        app.settle();
        inspect::settle_tree(window);
    };
    let shown = |window: &slint::Window| -> Vec<String> {
        let mut labels = Vec::new();
        if let Some(top) = inspect::roots(window).last() {
            inspect::walk(top, &mut |item| {
                if item.accessible_role() == role
                    && item.is_visible()
                    && let Some(label) = inspect::label(item)
                    && keep(&label)
                {
                    labels.push(label);
                }
            });
        }
        labels
    };
    for _ in 0..15 {
        scroll(app, 300.0);
    }
    let mut labels: Vec<String> = Vec::new();
    let mut quiet = 0;
    while quiet < 3 && labels.len() < 1000 {
        let before = labels.len();
        for label in shown(window) {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        quiet = if labels.len() == before { quiet + 1 } else { 0 };
        scroll(app, -60.0);
    }
    labels
}

/// A tool window's title, as its Window element declares it.
pub fn title(window: &MinimalSoftwareWindow) -> String {
    WindowInner::from_pub(window.window())
        .window_item()
        .map(|item| item.as_pin_ref().title().to_string())
        .unwrap_or_default()
}

/// The visible tool window with this title, if any.
pub fn tool_titled(app: &App, wanted: &str) -> Option<Tool> {
    app.tool_windows()
        .into_iter()
        .find(|window| title(window) == wanted)
}

/// Titles of the visible tool windows.
pub fn tool_titles(app: &App) -> Vec<String> {
    app.tool_windows()
        .iter()
        .map(|window| title(window))
        .collect()
}

/// Press a shortcut on the main window and wait for a new tool window, sized for the
/// scenario.
#[track_caller]
pub fn open_tool(app: &mut App, what: &str, modifiers: &[Key], key: Key, size: (u32, u32)) -> Tool {
    let before = app.tool_windows().len();
    input::chord(app.slint(), modifiers, key);
    wait_tool(app, what, before, size)
}

/// Wait for a tool window beyond the `before` already open and size it.
#[track_caller]
pub fn wait_tool(app: &mut App, what: &str, before: usize, size: (u32, u32)) -> Tool {
    app.until(what, move |app| app.tool_windows().len() > before);
    let window = app.tool_windows().pop().expect("tool window");
    app::layout_tool(&window, size.0, size.1);
    app.settle();
    window
}

/// Close a tool window with Escape and wait until it is gone.
#[track_caller]
pub fn close_tool(app: &mut App, window: &Tool, what: &str) {
    input::key(window.window(), Key::Escape);
    let watched = window.clone();
    app.until(&format!("{what} to close"), move |_| {
        !watched.window().is_visible()
    });
}

/// The text of a Text item, or of a styled text (a text-mode tooltip).
pub fn text_of(item: &ItemRc) -> Option<String> {
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    if let Some(text) = item.downcast::<ComplexText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    // A Tooltip in text mode (`Tooltip { text: @markdown(...) }`, as the viewers use) draws a
    // StyledText: its raw text, without the markup.
    item.downcast::<StyledTextItem>()
        .map(|text| i_slint_core::styled_text::get_raw_text(&text.as_pin_ref().text()).into_owned())
}

/// Every non-empty visible text below `item`, in tree order.
pub fn item_texts(item: &ItemRc) -> Vec<String> {
    let mut texts = Vec::new();
    inspect::walk(item, &mut |child| {
        if child.is_visible()
            && let Some(text) = text_of(child)
            && !text.trim().is_empty()
        {
            texts.push(text);
        }
    });
    texts
}

/// Texts of the window's open tooltips (hover popups).
pub fn tooltip_texts(window: &slint::Window) -> Vec<String> {
    let inner = WindowInner::from_pub(window);
    let roots: Vec<ItemRc> = inner
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| ItemRc::new_root(popup.component.clone()))
        .collect();
    roots.iter().flat_map(item_texts).collect()
}

/// Hover the middle of `item` and wait until a tooltip shows; returns its texts.
pub fn hover_tooltip(app: &mut App, window: &slint::Window, item: &ItemRc) -> Vec<String> {
    let position = inspect::visible_rect(item).map_or_else(
        || inspect::center(item),
        |(x, y, w, h)| LogicalPosition::new(x + w / 2.0, y + h / 2.0),
    );
    input::move_to(window, LogicalPosition::new(position.x - 2.0, position.y));
    input::move_to(window, position);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.idle(Duration::from_millis(50));
        let texts = tooltip_texts(window);
        if !texts.is_empty() || Instant::now() >= deadline {
            return texts;
        }
    }
}

fn property(item: &ItemRc, which: AccessibleStringProperty) -> Option<String> {
    item.accessible_string_property(which)
        .map(|value| value.to_string())
}

pub fn checked(item: &ItemRc) -> bool {
    property(item, AccessibleStringProperty::Checked).as_deref() == Some("true")
}

pub fn enabled(item: &ItemRc) -> bool {
    property(item, AccessibleStringProperty::Enabled).as_deref() != Some("false")
}

pub fn value(item: &ItemRc) -> String {
    property(item, AccessibleStringProperty::Value).unwrap_or_default()
}

pub fn description(item: &ItemRc) -> String {
    property(item, AccessibleStringProperty::Description).unwrap_or_default()
}

pub fn item_count(item: &ItemRc) -> Option<usize> {
    property(item, AccessibleStringProperty::ItemCount)?
        .parse()
        .ok()
}

/// The visible control with this role and label, or an error naming what is visible.
pub fn control(
    window: &slint::Window,
    role: AccessibleRole,
    label: &str,
) -> Result<ItemRc, String> {
    inspect::find(window, role, label).ok_or_else(|| {
        format!(
            "no visible {role:?} {label:?}; visible text: {:?}",
            inspect::visible_texts(window)
        )
    })
}

/// Whether a visible control with this role and label exists.
pub fn has(window: &slint::Window, role: AccessibleRole, label: &str) -> bool {
    inspect::find(window, role, label).is_some()
}

/// The checked state of the visible check box with this label.
pub fn is_checked(window: &slint::Window, label: &str) -> Result<bool, String> {
    control(window, AccessibleRole::Checkbox, label).map(|item| checked(&item))
}

/// Click a check box's own box (the native control under a drawn indicator is
/// transparent, so it is clicked where it is, without the scroll-into-view search).
pub fn click_check(window: &slint::Window, label: &str) -> Result<(), String> {
    let item = control(window, AccessibleRole::Checkbox, label)?;
    let (x, y, w, h) = inspect::rect(&item);
    let size = window.size().to_logical(window.scale_factor());
    let position = LogicalPosition::new(x + w.min(34.0) / 2.0, y + h / 2.0);
    if position.x < 0.0 || position.y < 0.0 || position.x >= size.width || position.y >= size.height
    {
        // Out of view inside a scroll view: let the generic click scroll it in.
        return input::click(window, AccessibleRole::Checkbox, label);
    }
    input::click_at(window, position);
    Ok(())
}

/// Set a check box to `wanted` by clicking it when it differs; verifies the result.
pub fn set_check(
    app: &mut App,
    window: &slint::Window,
    label: &str,
    wanted: bool,
) -> Result<(), String> {
    if is_checked(window, label)? != wanted {
        click_check(window, label)?;
        app.settle();
        inspect::settle_tree(window);
    }
    let now = is_checked(window, label)?;
    if now != wanted {
        return Err(format!(
            "clicking {label:?} left it checked={now}, wanted {wanted}"
        ));
    }
    Ok(())
}

/// Choose `wanted` in a combo box whose items are `items` (in order): the combo box is
/// clicked open and the item is reached with the arrow keys and Return.
pub fn choose_combo(
    app: &mut App,
    window: &slint::Window,
    label: &str,
    items: &[&str],
    wanted: &str,
) -> Result<(), String> {
    let item = control(window, AccessibleRole::Combobox, label)?;
    let current = value(&item);
    if current == wanted {
        return Ok(());
    }
    let from = items
        .iter()
        .position(|candidate| *candidate == current)
        .ok_or_else(|| format!("{label:?} shows {current:?}, not one of {items:?}"))?;
    let to = items
        .iter()
        .position(|candidate| *candidate == wanted)
        .ok_or_else(|| format!("{wanted:?} is not one of {items:?}"))?;
    input::click(window, AccessibleRole::Combobox, label)?;
    app.settle();
    for _ in 0..from.abs_diff(to) {
        input::key(
            window,
            if to > from {
                Key::DownArrow
            } else {
                Key::UpArrow
            },
        );
    }
    input::key(window, Key::Return);
    app.settle();
    let now = value(&control(window, AccessibleRole::Combobox, label)?);
    if now != wanted {
        return Err(format!("{label:?} shows {now:?} after choosing {wanted:?}"));
    }
    Ok(())
}

/// Click the last visible button with this label (footers come after page tabs).
pub fn click_last(window: &slint::Window, label: &str) -> Result<(), String> {
    let item = inspect::find_all(window, AccessibleRole::Button, label)
        .pop()
        .ok_or_else(|| {
            format!(
                "no {label:?} button; visible text: {:?}",
                inspect::visible_texts(window)
            )
        })?;
    if !enabled(&item) {
        return Err(format!("the {label:?} button is disabled"));
    }
    input::click_at(window, inspect::center(&item));
    Ok(())
}

/// Whether the window has an open (non-tooltip) popup.
pub fn popup_open(window: &slint::Window) -> bool {
    inspect::roots(window).len() > 1
}

/// Wait until a visible text of `window` satisfies `wanted`; returns that text.
#[track_caller]
pub fn wait_text(
    app: &mut App,
    window: &Tool,
    what: &str,
    wanted: impl Fn(&str) -> bool,
) -> String {
    let watched = window.clone();
    app.until(what, |_| {
        inspect::visible_texts(watched.window())
            .iter()
            .any(|text| wanted(text))
    });
    inspect::visible_texts(window.window())
        .into_iter()
        .find(|text| wanted(text))
        .unwrap_or_default()
}

/// The digits of a number shown with locale grouping ("3,000" or "3 000").
pub fn number(text: &str) -> Option<u64> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The number before `unit` in a status line such as "Completed · 3,000 found · …".
pub fn count_before(text: &str, unit: &str) -> Option<u64> {
    text.split(" · ")
        .find_map(|part| part.strip_suffix(unit).map(str::trim).and_then(number))
}

// ---------------------------------------------------------------------------------------
// The in-process clipboard

/// How many copies the in-process clipboard (`Config::clipboard`) has taken so far, of
/// text or files: take it before an action, and [`copied`] or [`wait_copied`] see that
/// action's copy only, also when it puts the same text there again. 0 without one.
pub fn clipboard_writes(app: &App) -> u64 {
    app.clipboard
        .as_ref()
        .map_or(0, |clipboard| clipboard.writes())
}

/// The text on the in-process clipboard when something was copied since `before` (a
/// [`clipboard_writes`] count); None while nothing was.
pub fn copied(app: &App, before: u64) -> Option<String> {
    let clipboard = app.clipboard.as_ref()?;
    if clipboard.writes() > before {
        clipboard.text()
    } else {
        None
    }
}

/// Wait for a copy of text since `before`; returns it. The limit is a hang guard: a copy
/// goes through the Engine's command queue and arrives within a few frames.
pub fn wait_copied(app: &mut App, before: u64, what: &str) -> Result<String, String> {
    wait_copied_within(app, before, what, Duration::from_secs(15))
}

/// [`wait_copied`] with its own hang guard, for copies of many megabytes.
pub fn wait_copied_within(
    app: &mut App,
    before: u64,
    what: &str,
    limit: Duration,
) -> Result<String, String> {
    if app.clipboard.is_none() {
        return Err(format!(
            "{what}: the scenario installed no clipboard (Config::clipboard)"
        ));
    }
    let deadline = Instant::now() + limit;
    loop {
        app.settle();
        if let Some(text) = copied(app, before) {
            return Ok(text);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} copied nothing (the clipboard holds {:?})",
                app.clipboard
                    .as_ref()
                    .and_then(|clipboard| clipboard.text())
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

/// Exactly one copy since `before`, of the text `expected`. An action that must copy
/// nothing is proven that way without a fixed wait: a known copy after it (the barrier)
/// is the only one, because the Engine takes the presenter's copies in the order they
/// were asked for.
pub fn only_copy(app: &mut App, before: u64, expected: &str, what: &str) -> Result<(), String> {
    let text = wait_copied(app, before, what)?;
    let copies = clipboard_writes(app) - before;
    if copies != 1 || text != expected {
        return Err(format!(
            "{what}: {copies} copies, the clipboard holds {text:?}, expected one copy of {expected:?}"
        ));
    }
    Ok(())
}

/// Wait for a copy of files since `before` (Ctrl+C or Ctrl+X in a panel); returns them.
pub fn wait_copied_files(app: &mut App, before: u64, what: &str) -> Result<FileClipboard, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.settle();
        if let Some(clipboard) = app.clipboard.as_ref()
            && clipboard.writes() > before
            && let Some(files) = clipboard.files()
        {
            return Ok(files);
        }
        if Instant::now() >= deadline {
            return Err(format!("{what} put no files on the clipboard"));
        }
        app.idle(Duration::from_millis(10));
    }
}

// ---------------------------------------------------------------------------------------
// Fitted dialog footers, More… and context menus

/// A button of a window or of one of its menus.
#[derive(Clone, Debug)]
pub struct Action {
    pub label: String,
    pub enabled: bool,
    /// Its tooltip and accessible description (which also says why it is disabled).
    pub description: String,
    /// Window coordinates.
    pub rect: inspect::Rect,
}

impl Action {
    fn of(item: &ItemRc) -> Self {
        Self {
            label: inspect::label(item).unwrap_or_default(),
            enabled: enabled(item),
            description: description(item),
            rect: inspect::visible_rect(item).unwrap_or_else(|| inspect::rect(item)),
        }
    }
}

/// What a fitted dialog footer (`DialogFooter` with `footer-actions`) shows: the actions
/// that are buttons, left to right, More… when some do not fit and the actions in its
/// menu, and the dismiss button at the end of the row.
#[derive(Clone, Debug)]
pub struct Footer {
    pub buttons: Vec<Action>,
    pub more: Option<Action>,
    /// The actions in the More… menu, in order (opened with a click and closed with
    /// Escape to read them).
    pub menu: Vec<Action>,
    pub dismiss: Action,
}

impl Footer {
    pub fn labels(&self) -> Vec<String> {
        self.buttons
            .iter()
            .map(|action| action.label.clone())
            .collect()
    }

    pub fn menu_labels(&self) -> Vec<String> {
        self.menu
            .iter()
            .map(|action| action.label.clone())
            .collect()
    }

    /// Every action the footer offers, as a button or in More….
    pub fn all(&self) -> impl Iterator<Item = &Action> {
        self.buttons.iter().chain(&self.menu)
    }
}

/// The buttons of the window's topmost popup (a menu), in order: those in view, or with
/// `scrolled` also those its scrolling list holds out of view (a long menu in a short
/// window scrolls; its entries exist all the same).
fn menu_entries(window: &slint::Window, scrolled: bool) -> Vec<Action> {
    let roots = inspect::roots(window);
    let mut found = Vec::new();
    if roots.len() > 1
        && let Some(top) = roots.last()
    {
        inspect::walk(top, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && (scrolled || item.is_visible())
                && inspect::label(item).is_some_and(|label| !label.is_empty())
            {
                found.push(Action::of(item));
            }
        });
    }
    found
}

/// The buttons in view in the window's topmost popup (a menu), in order.
pub fn popup_actions(window: &slint::Window) -> Vec<Action> {
    menu_entries(window, false)
}

/// The window's fitted footer: the row of buttons that ends with the dismiss button
/// (`dismiss`, its last button with that label); More… is opened with a click to read
/// its menu, then closed with Escape.
pub fn footer_actions(
    app: &mut App,
    window: &slint::Window,
    dismiss: &str,
) -> Result<Footer, String> {
    if popup_open(window) {
        return Err("a popup is open over the footer".into());
    }
    let main = inspect::roots(window)
        .into_iter()
        .next()
        .ok_or("the window has no item tree")?;
    let mut buttons = Vec::new();
    inspect::walk(&main, &mut |item| {
        if item.accessible_role() == AccessibleRole::Button
            && item.is_visible()
            && inspect::visible_rect(item).is_some()
        {
            buttons.push(Action::of(item));
        }
    });
    let close = buttons
        .iter()
        .rfind(|action| action.label == dismiss)
        .cloned()
        .ok_or_else(|| {
            format!(
                "no {dismiss:?} button; buttons {:?}",
                buttons
                    .iter()
                    .map(|action| &action.label)
                    .collect::<Vec<_>>()
            )
        })?;
    let middle = close.rect.1 + close.rect.3 / 2.0;
    let mut row: Vec<Action> = buttons
        .into_iter()
        .filter(|action| {
            action.rect.1 <= middle
                && action.rect.1 + action.rect.3 >= middle
                && action.rect.0 + action.rect.2 <= close.rect.0 + 0.5
        })
        .collect();
    row.sort_by(|a, b| a.rect.0.total_cmp(&b.rect.0));
    let more = row.iter().position(|action| action.label == "More…");
    let more = more.map(|index| row.remove(index));
    let mut menu = Vec::new();
    if let Some(more) = &more {
        input::click_at(
            window,
            LogicalPosition::new(
                more.rect.0 + more.rect.2 / 2.0,
                more.rect.1 + more.rect.3 / 2.0,
            ),
        );
        app.settle();
        if !popup_open(window) {
            return Err("a click on More… opened no menu".into());
        }
        // A short window's menu scrolls: its entries out of view count too.
        menu = menu_entries(window, true);
        input::key(window, Key::Escape);
        app.settle();
        if popup_open(window) {
            return Err("Escape did not close the More… menu".into());
        }
    }
    Ok(Footer {
        buttons: row,
        more,
        menu,
        dismiss: close,
    })
}

/// The window rectangle of the item that has the keyboard focus, if any.
pub fn focused_rect(window: &slint::Window) -> Option<inspect::Rect> {
    WindowInner::from_pub(window)
        .focus_item
        .borrow()
        .upgrade()
        .map(|item| inspect::rect(&item))
}

/// Whether the keyboard focus is on (inside) the area `rect`.
pub fn focus_in(window: &slint::Window, rect: inspect::Rect) -> bool {
    focused_rect(window).is_some_and(|(x, y, w, h)| {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        cx >= rect.0 && cx <= rect.0 + rect.2 && cy >= rect.1 && cy <= rect.1 + rect.3
    })
}

/// A right click (press and release of the right button) at `position`.
pub fn right_click_at(window: &slint::Window, position: LogicalPosition) {
    let button = PointerEventButton::Right;
    input::move_to(window, position);
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// How a context menu is opened.
#[derive(Clone, Copy, Debug)]
pub enum MenuBy {
    /// A right click on this point (a row).
    Pointer(LogicalPosition),
    /// Shift+F10 on the focused list.
    ShiftF10,
    /// The Menu (application) key.
    MenuKey,
}

/// Open a context menu `by` the pointer or a key; returns its entries (the menu stays
/// open for the caller to choose one or press Escape).
pub fn context_menu(
    app: &mut App,
    window: &slint::Window,
    by: MenuBy,
) -> Result<Vec<Action>, String> {
    match by {
        MenuBy::Pointer(position) => right_click_at(window, position),
        MenuBy::ShiftF10 => input::chord(window, &[Key::Shift], Key::F10),
        MenuBy::MenuKey => input::key(window, Key::Menu),
    }
    app.settle();
    if !popup_open(window) {
        return Err(format!("{by:?} opened no context menu"));
    }
    Ok(popup_actions(window))
}

/// Choose the entry `label` of the open menu with a click.
pub fn choose(app: &mut App, window: &slint::Window, label: &str) -> Result<(), String> {
    let entry = popup_actions(window)
        .into_iter()
        .find(|action| action.label == label)
        .ok_or_else(|| {
            format!(
                "the menu has no {label:?}: {:?}",
                popup_actions(window)
                    .iter()
                    .map(|action| &action.label)
                    .collect::<Vec<_>>()
            )
        })?;
    if !entry.enabled {
        return Err(format!("the menu's {label:?} is disabled"));
    }
    let (x, y, w, h) = entry.rect;
    input::click_at(window, LogicalPosition::new(x + w / 2.0, y + h / 2.0));
    app.settle();
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Rows of selectable result lists (Find results, Checksum results)

/// A row a virtualized list has instantiated, with its accessible state.
#[derive(Clone, Debug)]
pub struct ListRow {
    pub index: usize,
    pub label: String,
    pub selected: bool,
    /// Window coordinates of the whole row.
    pub rect: inspect::Rect,
    /// Whether the whole row is inside the list's visible area.
    pub whole: bool,
}

impl ListRow {
    /// The first field of the label: the name of a Find or Checksum result.
    pub fn name(&self) -> &str {
        self.label.split(" · ").next().unwrap_or_default()
    }
}

/// The rows of the list labelled `list` that are in view, in order. A virtualized list
/// instantiates rows for its viewport while it is drawn, so the window is drawn first.
pub fn list_rows(window: &MinimalSoftwareWindow, list: &str) -> Vec<ListRow> {
    let _ = app::render(window);
    let mut rows = Vec::new();
    if let Some(found) = inspect::find(window.window(), AccessibleRole::List, list) {
        let area = inspect::visible_rect(&found);
        inspect::walk(&found, &mut |item| {
            if item.accessible_role() == AccessibleRole::ListItem
                && item.is_visible()
                && inspect::visible_rect(item).is_some()
                && let Some(label) = inspect::label(item)
            {
                let index = property(item, AccessibleStringProperty::ItemIndex)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(usize::MAX);
                let rect = inspect::rect(item);
                let whole = area.is_some_and(|(x, y, w, h)| {
                    rect.1 >= y - 0.5
                        && rect.1 + rect.3 <= y + h + 0.5
                        && rect.0 >= x - 0.5
                        && rect.0 < x + w
                });
                rows.push(ListRow {
                    index,
                    label,
                    selected: property(item, AccessibleStringProperty::ItemSelected).as_deref()
                        == Some("true"),
                    rect,
                    whole,
                });
            }
        });
    }
    rows.sort_by_key(|row| row.index);
    rows.dedup_by_key(|row| row.index);
    rows
}

/// The names of the selected rows in view, in order.
pub fn selected_names(window: &MinimalSoftwareWindow, list: &str) -> Vec<String> {
    list_rows(window, list)
        .into_iter()
        .filter(|row| row.selected)
        .map(|row| row.name().to_owned())
        .collect()
}

/// The row that has the list's cursor, as the list draws it: the cursor's row has a
/// full-height stroke at its left edge (3 px of `Theme.cursor-edge`), which no other row
/// has; a selected row's square sits in the middle of the edge only. Reads the rendered
/// pixels just inside the row's top edge, on the stroke and right of it: they differ on
/// the cursor's row alone. None when no whole row in view, or more than one, shows it.
pub fn cursor_row(window: &MinimalSoftwareWindow, list: &str) -> Option<ListRow> {
    let rows = list_rows(window, list);
    let (width, height, pixels) = app::render(window);
    let at = |x: f32, y: f32| -> Option<slint::Rgb8Pixel> {
        let (x, y) = (x.floor() as i64, y.floor() as i64);
        (x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height)
            .then(|| pixels[y as usize * width + x as usize])
    };
    let differs = |a: slint::Rgb8Pixel, b: slint::Rgb8Pixel| {
        a.r.abs_diff(b.r) as u32 + a.g.abs_diff(b.g) as u32 + a.b.abs_diff(b.b) as u32 > 48
    };
    let mut found = rows.into_iter().filter(|row| {
        let (x, y, _, h) = row.rect;
        row.whole
            && h >= 10.0
            && [y + 2.0, y + h - 3.0].into_iter().all(|line| {
                matches!((at(x + 1.0, line), at(x + 6.0, line)), (Some(edge), Some(inside)) if differs(edge, inside))
            })
    });
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

pub fn set_modified(path: &Path, time: SystemTime) -> Result<(), String> {
    times_handle(path)
        .and_then(|file| file.set_modified(time))
        .map_err(|error| format!("set the time of {}: {error}", path.display()))
}

/// A handle that may change the times of the file or folder `path`. Unix: the file
/// opened for writing, a folder for reading (its owner may set its times). Windows: one
/// with only the right to write attributes, which a read-only file grants too and a
/// folder gives with backup semantics.
pub fn times_handle(path: &Path) -> std::io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        if fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
            fs::File::open(path)
        } else {
            fs::File::options().write(true).open(path)
        }
    }
}

/// Write `data` to `path` (creating parents) and give it the modification time `time`.
pub fn write_at(path: &Path, data: &[u8], time: SystemTime) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    fs::write(path, data).map_err(|error| format!("{}: {error}", path.display()))?;
    set_modified(path, time)
}

/// A sparse file: `length` bytes that read as zeros and use (almost) no disk space.
/// NTFS allocates the whole length of an ordinary file it extends, so on Windows a file
/// of 1 MiB or more is marked sparse first (`fsutil sparse setflag`, which needs no
/// privilege); smaller ones take their length.
pub fn sparse(path: &Path, length: u64) -> Result<(), String> {
    let error = |error: std::io::Error| format!("{}: {error}", path.display());
    let file = fs::File::create(path).map_err(error)?;
    #[cfg(windows)]
    let file = if length >= 1 << 20 {
        drop(file);
        let output = std::process::Command::new("fsutil")
            .args(["sparse", "setflag"])
            .arg(path)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|failure| format!("fsutil sparse setflag {}: {failure}", path.display()))?;
        if !output.status.success() {
            return Err(format!(
                "fsutil sparse setflag {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
        fs::File::options().write(true).open(path).map_err(error)?
    } else {
        file
    };
    file.set_len(length).map_err(error)
}

#[cfg(unix)]
pub fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("chmod {mode:o} {}: {error}", path.display()))
}

/// The Windows equivalent of the owner bits of `mode` that the scenarios use: no owner
/// access (`000`) denies reading (`fixtures::deny_read`), a folder without owner write
/// (`5xx`) denies creating entries in it (`fixtures::deny_write`), a file without
/// owner write (`4xx`, `5xx`) gets the read-only attribute, and owner write (`6xx`,
/// `7xx`) gives everything back. Other combinations (a listable folder whose entries
/// cannot be reached, `4xx` on a folder) have no Windows equivalent and are refused.
#[cfg(windows)]
pub fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let folder = metadata.is_dir();
    let readonly = |value: bool| {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(value);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("read-only attribute {value} {}: {error}", path.display()))
    };
    match (mode & 0o700, folder) {
        (owner, _) if owner & 0o200 != 0 => {
            crate::fixtures::allow_read(path)?;
            readonly(false)
        }
        (0, _) => crate::fixtures::deny_read(path),
        (0o500, true) => crate::fixtures::deny_write(path),
        (0o400 | 0o500, false) => readonly(true),
        _ => Err(format!(
            "mode {mode:o} on {} has no Windows equivalent in this suite",
            path.display()
        )),
    }
}

/// SHA-1 (FIPS 180-4), for checking the application's results independently.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut words = [0_u32; 80];
        for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e]) {
            *value = value.wrapping_add(add);
        }
    }
    let mut digest = [0_u8; 20];
    for (bytes, value) in digest.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *bytes = value.to_be_bytes();
    }
    digest
}

const SHA512_K: [u64; 80] = [
    0x428a_2f98_d728_ae22,
    0x7137_4491_23ef_65cd,
    0xb5c0_fbcf_ec4d_3b2f,
    0xe9b5_dba5_8189_dbbc,
    0x3956_c25b_f348_b538,
    0x59f1_11f1_b605_d019,
    0x923f_82a4_af19_4f9b,
    0xab1c_5ed5_da6d_8118,
    0xd807_aa98_a303_0242,
    0x1283_5b01_4570_6fbe,
    0x2431_85be_4ee4_b28c,
    0x550c_7dc3_d5ff_b4e2,
    0x72be_5d74_f27b_896f,
    0x80de_b1fe_3b16_96b1,
    0x9bdc_06a7_25c7_1235,
    0xc19b_f174_cf69_2694,
    0xe49b_69c1_9ef1_4ad2,
    0xefbe_4786_384f_25e3,
    0x0fc1_9dc6_8b8c_d5b5,
    0x240c_a1cc_77ac_9c65,
    0x2de9_2c6f_592b_0275,
    0x4a74_84aa_6ea6_e483,
    0x5cb0_a9dc_bd41_fbd4,
    0x76f9_88da_8311_53b5,
    0x983e_5152_ee66_dfab,
    0xa831_c66d_2db4_3210,
    0xb003_27c8_98fb_213f,
    0xbf59_7fc7_beef_0ee4,
    0xc6e0_0bf3_3da8_8fc2,
    0xd5a7_9147_930a_a725,
    0x06ca_6351_e003_826f,
    0x1429_2967_0a0e_6e70,
    0x27b7_0a85_46d2_2ffc,
    0x2e1b_2138_5c26_c926,
    0x4d2c_6dfc_5ac4_2aed,
    0x5338_0d13_9d95_b3df,
    0x650a_7354_8baf_63de,
    0x766a_0abb_3c77_b2a8,
    0x81c2_c92e_47ed_aee6,
    0x9272_2c85_1482_353b,
    0xa2bf_e8a1_4cf1_0364,
    0xa81a_664b_bc42_3001,
    0xc24b_8b70_d0f8_9791,
    0xc76c_51a3_0654_be30,
    0xd192_e819_d6ef_5218,
    0xd699_0624_5565_a910,
    0xf40e_3585_5771_202a,
    0x106a_a070_32bb_d1b8,
    0x19a4_c116_b8d2_d0c8,
    0x1e37_6c08_5141_ab53,
    0x2748_774c_df8e_eb99,
    0x34b0_bcb5_e19b_48a8,
    0x391c_0cb3_c5c9_5a63,
    0x4ed8_aa4a_e341_8acb,
    0x5b9c_ca4f_7763_e373,
    0x682e_6ff3_d6b2_b8a3,
    0x748f_82ee_5def_b2fc,
    0x78a5_636f_4317_2f60,
    0x84c8_7814_a1f0_ab72,
    0x8cc7_0208_1a64_39ec,
    0x90be_fffa_2363_1e28,
    0xa450_6ceb_de82_bde9,
    0xbef9_a3f7_b2c6_7915,
    0xc671_78f2_e372_532b,
    0xca27_3ece_ea26_619c,
    0xd186_b8c7_21c0_c207,
    0xeada_7dd6_cde0_eb1e,
    0xf57d_4f7f_ee6e_d178,
    0x06f0_67aa_7217_6fba,
    0x0a63_7dc5_a2c8_98a6,
    0x113f_9804_bef9_0dae,
    0x1b71_0b35_131c_471b,
    0x28db_77f5_2304_7d84,
    0x32ca_ab7b_40c7_2493,
    0x3c9e_be0a_15c9_bebc,
    0x431d_67c4_9c10_0d4c,
    0x4cc5_d4be_cb3e_42b6,
    0x597f_299c_fc65_7e2a,
    0x5fcb_6fab_3ad6_faec,
    0x6c44_198c_4a47_5817,
];

/// SHA-512 (FIPS 180-4), for checking the application's results independently.
pub fn sha512(data: &[u8]) -> [u8; 64] {
    let mut state: [u64; 8] = [
        0x6a09_e667_f3bc_c908,
        0xbb67_ae85_84ca_a73b,
        0x3c6e_f372_fe94_f82b,
        0xa54f_f53a_5f1d_36f1,
        0x510e_527f_ade6_82d1,
        0x9b05_688c_2b3e_6c1f,
        0x1f83_d9ab_fb41_bd6b,
        0x5be0_cd19_137e_2179,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 128 != 112 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u128).wrapping_mul(8)).to_be_bytes());
    for block in message.as_chunks::<128>().0 {
        let mut words = [0_u64; 80];
        for (word, bytes) in words.iter_mut().zip(block.as_chunks::<8>().0) {
            *word = u64::from_be_bytes(*bytes);
        }
        for index in 16..80 {
            let previous = words[index - 15];
            let recent = words[index - 2];
            let s0 = previous.rotate_right(1) ^ previous.rotate_right(8) ^ (previous >> 7);
            let s1 = recent.rotate_right(19) ^ recent.rotate_right(61) ^ (recent >> 6);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let mut v = state;
        for (constant, word) in SHA512_K.iter().zip(words) {
            let [a, b, c, d, e, f, g, h] = v;
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(*constant)
                .wrapping_add(word);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (value, add) in state.iter_mut().zip(v) {
            *value = value.wrapping_add(add);
        }
    }
    let mut digest = [0_u8; 64];
    for (bytes, value) in digest.as_chunks_mut::<8>().0.iter_mut().zip(state) {
        *bytes = value.to_be_bytes();
    }
    digest
}

/// Known answers of the SHA-1 and SHA-512 helpers (FIPS 180 examples and coreutils).
pub fn self_test() -> Result<(), String> {
    use super::encode::hex;
    let thousand = vec![b'a'; 1000];
    let cases: [(&str, String, &str); 7] = [
        (
            "SHA-1 abc",
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d",
        ),
        (
            "SHA-1 empty",
            hex(&sha1(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709",
        ),
        (
            "SHA-1 two blocks",
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1",
        ),
        (
            "SHA-1 1000 a",
            hex(&sha1(&thousand)),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba",
        ),
        (
            "SHA-512 abc",
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            "SHA-512 two blocks",
            hex(&sha512(
                b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            )),
            "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
        ),
        (
            "SHA-512 1000 a",
            hex(&sha512(&thousand)),
            "67ba5535a46e3f86dbfbed8cbbaf0125c76ed549ff8b0b9e03e0c88cf90fa634fa7b12b47d77b694de488ace8d9a65967dc96df599727d3292a8d9d447709c97",
        ),
    ];
    for (name, actual, expected) in cases {
        if actual != expected {
            return Err(format!(
                "digest self-test {name}: {actual}, expected {expected}"
            ));
        }
    }
    Ok(())
}
