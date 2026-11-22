//! Options and customisation support: a settings store that outlives one application
//! run (so a scenario can restart the application on what it saved), and input helpers
//! for the Options dialog's pages, pickers, check boxes, fields and footer.
//!
//! Every helper acts through synthetic keyboard and pointer input on the visible
//! controls. The store is read only to verify what the application saved; scenarios
//! never write preferences behind the application's back after it started.
use super::{
    Context,
    app::{self, App},
    input, inspect, report,
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Clip, ComplexText, Flickable, SimpleText},
    window::WindowInner,
};
use midnight_core::{ComparatorValidationProof, FsError, PaneId, SettingsService, UserPreferences};
use midnight_ui::NoticePresentation;
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Font families the store reports as installed. The build image ships DejaVu; on a
/// machine without them Slint falls back to its default font, and the scenarios check
/// the selection and what was saved rather than glyph shapes.
pub const FONT_FAMILIES: [&str; 3] = ["DejaVu Sans", "DejaVu Sans Mono", "DejaVu Serif"];

type SeedEdit = Box<dyn FnOnce(&mut UserPreferences) + Send>;

#[derive(Default)]
struct StoreState {
    /// What the next application run loads; `None` until the first run seeds it.
    stored: Option<UserPreferences>,
    /// The scenario's change to the first run's preferences.
    seed: Option<SeedEdit>,
    /// Completed saves.
    saves: usize,
    /// How long each load takes: a slow disk (`SettingsStore::delay`).
    delay: Option<Duration>,
}

/// Settings shared by successive application runs of one scenario (opt-in through
/// `Config::settings`). The first run seeds it with the preferences its `Config`
/// describes (after the optional [`SettingsStore::seeded`] edit); every save replaces
/// them; later runs load what was saved, like a restart on the same profile.
#[derive(Default)]
pub struct SettingsStore {
    state: Mutex<StoreState>,
}

impl SettingsStore {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// A store whose first run starts from its `Config` changed by `edit`.
    pub fn seeded(edit: impl FnOnce(&mut UserPreferences) + Send + 'static) -> Arc<Self> {
        let store = Self::default();
        store.lock().seed = Some(Box::new(edit));
        Arc::new(store)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StoreState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The preferences the application saved last (or the seed before any save).
    pub fn current(&self) -> UserPreferences {
        self.lock().stored.clone().unwrap_or_default()
    }

    /// Completed saves so far.
    pub fn saves(&self) -> usize {
        self.lock().saves
    }

    /// Make every later load take `delay` (`None`: immediate again), as a slow disk does:
    /// the application's settings deadline then shows the defaults first.
    #[allow(dead_code)] // For O10 (slow settings) once `App::start` arms the deadline (H6).
    pub fn delay(&self, delay: Option<Duration>) {
        self.lock().delay = delay;
    }

    /// The settings service an application run installs; `initial` is what its
    /// `Config` describes and seeds an unused store.
    pub fn service(self: &Arc<Self>, initial: UserPreferences) -> Arc<dyn SettingsService> {
        let mut state = self.lock();
        if state.stored.is_none() {
            let mut preferences = initial;
            if let Some(edit) = state.seed.take() {
                edit(&mut preferences);
            }
            state.stored = Some(preferences);
        }
        drop(state);
        Arc::new(StoreService(self.clone()))
    }
}

struct StoreService(Arc<SettingsStore>);

impl SettingsService for StoreService {
    fn font_families(&self) -> Result<Vec<String>, FsError> {
        Ok(FONT_FAMILIES
            .iter()
            .map(|name| (*name).to_owned())
            .collect())
    }

    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        // A backend worker loads settings, so a slow load blocks only that worker.
        let delay = self.0.lock().delay;
        if let Some(delay) = delay {
            std::thread::sleep(delay);
        }
        Ok(self.0.current())
    }

    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        let mut state = self.0.lock();
        state.stored = Some(preferences);
        state.saves += 1;
        Ok(())
    }

    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.load_preferences()
    }

    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        if !proof.matches(&preferences.file_comparators) {
            return Err(FsError::from_io(
                "save settings",
                Path::new(""),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "comparator proof mismatch",
                ),
            ));
        }
        self.save_preferences(preferences)
    }
}

/// Options pages in their displayed order with the dialog's topic numbers
/// (`OptionsTopics` in options-navigation.slint; 2 is retired).
pub const PAGES: [(&str, i32); 10] = [
    ("General", 0),
    ("Appearance", 1),
    ("Colors", 8),
    ("Previews", 6),
    ("Archives", 7),
    ("Editors", 4),
    ("Comparators", 10),
    ("Custom Commands", 9),
    ("Keyboard", 3),
    ("Plugins", 5),
];

fn topic(title: &str) -> Result<i32, String> {
    PAGES
        .iter()
        .find(|(name, _)| *name == title)
        .map(|(_, topic)| *topic)
        .ok_or_else(|| format!("no Options page {title:?}"))
}

/// Open Options with the toolbar's Options button and wait until its settings are
/// ready for input.
pub fn open(app: &mut App) -> Result<(), String> {
    super::panel::wait_input(app);
    input::click_button(app.slint(), "Options")?;
    app.until("Options to open", |app| {
        app.window.get_options_open()
            && app.window.get_preferences_ready()
            && !app.window.get_options_busy()
    });
    Ok(())
}

/// Show an Options page with its topic button (or, in narrow layouts, the topic
/// picker).
pub fn page(app: &mut App, title: &str) -> Result<(), String> {
    let wanted = topic(title)?;
    if app.window.get_options_topic_index() == wanted {
        return Ok(());
    }
    if find_any(app, AccessibleRole::Button, title).is_some() {
        press(app, title)?;
    } else {
        choose(app, "Options topic", title)?;
    }
    app.until(&format!("the {title} Options page"), move |app| {
        app.window.get_options_topic_index() == wanted
    });
    // Pages instantiate lazily; let their first layout and focus settle.
    app.idle(Duration::from_millis(60));
    Ok(())
}

/// The text of the Options error line, if one is shown.
pub fn error_text(app: &App) -> Option<String> {
    inspect::find(app.slint(), AccessibleRole::Text, "Options error")
        .and_then(|item| item.accessible_string_property(AccessibleStringProperty::Value))
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
}

/// Press OK and wait until Options closed. Returns whether settings were saved.
pub fn ok(app: &mut App, store: &SettingsStore) -> Result<bool, String> {
    let before = store.saves();
    if !is_enabled(app, AccessibleRole::Button, "OK") {
        return Err(format!(
            "the Options OK button is disabled; error: {:?}; visible text: {:?}",
            error_text(app),
            inspect::visible_texts(app.slint())
        ));
    }
    input::click_button(app.slint(), "OK")?;
    let mut closed = false;
    for _ in 0..400 {
        app.idle(Duration::from_millis(15));
        if !app.window.get_options_open() {
            closed = true;
            break;
        }
        if !app.window.get_options_busy()
            && let Some(error) = error_text(app)
        {
            return Err(format!("OK kept Options open: {error}"));
        }
    }
    if !closed {
        return Err(format!(
            "Options did not close after OK; error: {:?}; visible text: {:?}",
            error_text(app),
            inspect::visible_texts(app.slint())
        ));
    }
    app.settle();
    Ok(store.saves() > before)
}

/// Press Cancel and wait until Options closed.
pub fn cancel(app: &mut App) -> Result<(), String> {
    input::click_button(app.slint(), "Cancel")?;
    app.until("Options to close after Cancel", |app| {
        !app.window.get_options_open()
    });
    app.settle();
    Ok(())
}

/// Whether no ancestor hides the item, wherever it is scrolled: Slint lowers
/// `visible: false` to a visibility clip that clips everything, while scroll views and
/// other clips only move an item out of view.
fn shown(item: &ItemRc) -> bool {
    let mut current = Some(item.clone());
    while let Some(node) = current {
        if let Some(clip) = node.downcast::<Clip>() {
            let clip = clip.as_pin_ref();
            if clip.is_visibility_clip() && clip.clip() {
                return false;
            }
        }
        current = node.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    true
}

/// The first shown item matching `wanted`, searching the topmost popup first, whether
/// or not it is scrolled into view.
pub fn find_where(app: &App, wanted: &dyn Fn(&ItemRc) -> bool) -> Option<ItemRc> {
    for root in inspect::roots(app.slint()).iter().rev() {
        let mut found = None;
        inspect::walk(root, &mut |item| {
            if found.is_none() && wanted(item) && shown(item) {
                found = Some(item.clone());
            }
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The first shown control with this role and label (topmost popup first), scrolled
/// into view or not.
pub fn find_any(app: &App, role: AccessibleRole, label: &str) -> Option<ItemRc> {
    find_where(app, &|item| {
        item.accessible_role() == role && inspect::label(item).as_deref() == Some(label)
    })
}

/// A point of `area` (inside `flickable`) that is not over a scroll view nested in it,
/// which would take the wheel instead.
fn wheel_point(flickable: &ItemRc, area: inspect::Rect) -> Option<slint::LogicalPosition> {
    let mut nested = Vec::new();
    inspect::walk(flickable, &mut |candidate| {
        if candidate != flickable
            && candidate.downcast::<Flickable>().is_some()
            && let Some(rect) = inspect::visible_rect(candidate)
        {
            nested.push(rect);
        }
    });
    for fy in [0.5, 0.3, 0.7, 0.1, 0.9, 0.02, 0.98] {
        for fx in [0.5, 0.04, 0.96, 0.25, 0.75] {
            let (x, y) = (area.0 + area.2 * fx, area.1 + area.3 * fy);
            if !nested
                .iter()
                .any(|r| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3)
            {
                return Some(slint::LogicalPosition::new(x, y));
            }
        }
    }
    None
}

fn fully_shown(item: &ItemRc) -> bool {
    let full = inspect::rect(item);
    inspect::visible_rect(item)
        .is_some_and(|(_, _, w, h)| h >= full.3 * 0.9 - 0.5 && w >= full.2.min(48.0) * 0.9)
}

/// Scroll the item `find` returns into view with the mouse wheel, as a user would:
/// over the innermost scroll view that does not already show it, never over a nested
/// one. Returns the item, shown as far as its scroll views allow.
pub fn reveal_where(
    app: &mut App,
    what: &str,
    find: &dyn Fn(&App) -> Option<ItemRc>,
) -> Result<ItemRc, String> {
    let missing = |app: &App| {
        format!(
            "no {what} is shown; visible text: {:?}",
            inspect::visible_texts(app.slint())
        )
    };
    let mut item = find(app).ok_or_else(|| missing(app))?;
    let mut sign = 1.0_f32;
    let mut previous: Option<f32> = None;
    for _ in 0..80 {
        if fully_shown(&item) {
            return Ok(item);
        }
        let full = inspect::rect(&item);
        let mut scrolled = None;
        let mut current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        while let Some(ancestor) = current {
            current = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
            if ancestor.downcast::<Flickable>().is_none() {
                continue;
            }
            // Scroll the innermost view whose own viewport does not hold the item.
            let view = inspect::rect(&ancestor);
            let span = full.3.min(view.3);
            let target = if full.3 <= view.3 {
                full.1 + full.3 / 2.0
            } else {
                full.1 + view.3 / 2.0
            };
            if target >= view.1 + span / 2.0 - 0.5 && target <= view.1 + view.3 - span / 2.0 + 0.5 {
                continue;
            }
            let Some(area) = inspect::visible_rect(&ancestor) else {
                continue;
            };
            let Some(point) = wheel_point(&ancestor, area) else {
                continue;
            };
            let gap = target - (view.1 + view.3 / 2.0);
            scrolled = Some((point, gap));
            break;
        }
        let Some((point, gap)) = scrolled else {
            break;
        };
        if let Some(before) = previous {
            if (gap.abs() - before.abs()).abs() < 0.5 {
                // Nothing moved: the view cannot scroll further.
                break;
            }
            if gap.abs() > before.abs() {
                sign = -sign;
            }
        }
        previous = Some(gap);
        app.slint().dispatch_event(WindowEvent::PointerScrolled {
            position: point,
            delta_x: 0.0,
            delta_y: sign * (-gap).clamp(-240.0, 240.0),
        });
        inspect::settle_tree(app.slint());
        app.settle();
        item = find(app).ok_or_else(|| missing(app))?;
    }
    Ok(item)
}

/// Labels of every shown control with this role (scrolled into view or not).
pub fn labels(app: &App, role: AccessibleRole) -> Vec<String> {
    let mut labels = Vec::new();
    for root in inspect::roots(app.slint()) {
        inspect::walk(&root, &mut |item| {
            if item.accessible_role() == role && shown(item) {
                labels.push(inspect::label(item).unwrap_or_default());
            }
        });
    }
    labels
}

/// Scroll a control with this role and label into view and return it.
pub fn reveal(app: &mut App, role: AccessibleRole, label: &str) -> Result<ItemRc, String> {
    let wanted = label.to_owned();
    reveal_where(app, &format!("{role:?} {label:?}"), &move |app| {
        find_any(app, role, &wanted)
    })
}

/// Click the middle of the visible part of an item.
pub fn click_item(app: &mut App, item: &ItemRc) -> Result<(), String> {
    let (x, y, w, h) = inspect::visible_rect(item).ok_or_else(|| {
        format!(
            "{:?} {:?} cannot be scrolled into view",
            item.accessible_role(),
            inspect::label(item)
        )
    })?;
    input::click_at(
        app.slint(),
        slint::LogicalPosition::new(x + w / 2.0, y + h / 2.0),
    );
    app.settle();
    Ok(())
}

/// Whether an item accepts input: controls that do not declare `accessible-enabled`
/// (plain clickable rows and tabs) are enabled.
fn enabled(item: &ItemRc) -> bool {
    item.accessible_string_property(AccessibleStringProperty::Enabled)
        .is_none_or(|value| value == "true")
}

/// Whether the control with this role and label is shown and enabled.
pub fn is_enabled(app: &App, role: AccessibleRole, label: &str) -> bool {
    find_any(app, role, label).is_some_and(|item| enabled(&item))
}

/// Whether the control with this role and label is enabled; `None` when it is not shown
/// (a negative check must not pass because the control is missing).
pub fn enabled_state(app: &App, role: AccessibleRole, label: &str) -> Option<bool> {
    find_any(app, role, label).map(|item| enabled(&item))
}

/// Pump the application until `condition` holds or `limit` has passed; whether it
/// held. The limit is a hang guard for a positive condition, or the observation window
/// of a negative one (then pair it with a positive control that shows within it).
pub fn poll(app: &mut App, limit: Duration, condition: impl Fn(&App) -> bool) -> bool {
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

/// The item that has the keyboard focus in the main window or one of its popups.
pub fn focus_item(app: &App) -> Option<ItemRc> {
    WindowInner::from_pub(app.slint())
        .focus_item
        .borrow()
        .upgrade()
}

/// The role and label of the control that has the keyboard focus: the focused item or
/// its nearest ancestor with an accessible role and a label (a topic row's focus scope
/// names its row, a combo box's inner focus scope its combo box).
pub fn focus(app: &App) -> Option<(AccessibleRole, String)> {
    let mut current = focus_item(app);
    while let Some(item) = current {
        let role = item.accessible_role();
        if role != AccessibleRole::None
            && let Some(label) = inspect::label(&item).filter(|label| !label.is_empty())
        {
            return Some((role, label));
        }
        current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

/// Press Tab (or Shift+Tab) until the focused control is one `accept` takes, at most
/// `limit` times. Returns the controls visited on the way (the last is the accepted
/// one); an error names them when none was accepted.
pub fn tab_until(
    app: &mut App,
    backwards: bool,
    limit: usize,
    what: &str,
    accept: &dyn Fn(AccessibleRole, &str) -> bool,
) -> Result<Vec<String>, String> {
    let mut visited = Vec::new();
    for _ in 0..limit {
        if backwards {
            input::chord(app.slint(), &[Key::Shift], Key::Tab);
        } else {
            input::key(app.slint(), Key::Tab);
        }
        app.settle();
        match focus(app) {
            Some((role, label)) => {
                visited.push(format!("{role:?} {label:?}"));
                if accept(role, &label) {
                    return Ok(visited);
                }
            }
            None => visited.push("(nothing)".to_owned()),
        }
    }
    Err(format!(
        "{} {limit} times did not reach {what}; focus went to {visited:?}",
        if backwards { "Shift+Tab" } else { "Tab" }
    ))
}

/// The accessible value of a control (a picker's shown choice, a field's text).
pub fn value(app: &App, role: AccessibleRole, label: &str) -> Option<String> {
    find_any(app, role, label)
        .and_then(|item| item.accessible_string_property(AccessibleStringProperty::Value))
        .map(|value| value.to_string())
}

/// Open the picker labelled `label` and click the first popup item `accept` takes
/// (list items for combo boxes, buttons for compact pickers), scrolling the popup
/// list as needed. Returns the clicked item's label.
fn pick_where(
    app: &mut App,
    label: &str,
    describe: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Result<String, String> {
    let picker = reveal(app, AccessibleRole::Combobox, label)?;
    let popups = inspect::roots(app.slint()).len();
    click_item(app, &picker)?;
    if inspect::roots(app.slint()).len() <= popups {
        return Err(format!("clicking the {label:?} picker opened no popup"));
    }
    let popup = inspect::roots(app.slint()).pop().expect("the picker popup");
    let mut found = None;
    let mut offered = Vec::new();
    inspect::walk(&popup, &mut |candidate| {
        if matches!(
            candidate.accessible_role(),
            AccessibleRole::ListItem | AccessibleRole::Button
        ) && shown(candidate)
            && let Some(name) = inspect::label(candidate)
        {
            if found.is_none() && accept(&name) {
                found = Some(candidate.clone());
            }
            offered.push(name);
        }
    });
    let Some(item) = found else {
        // Escape goes to the open list, which has the keyboard, and closes only it.
        input::key(app.slint(), Key::Escape);
        app.settle();
        let left_open = inspect::roots(app.slint()).len() > popups;
        return Err(format!(
            "the {label:?} picker offers no {describe}; it offers {offered:?}{}",
            if left_open {
                " (Escape left its list open)"
            } else {
                ""
            }
        ));
    };
    let name = inspect::label(&item).unwrap_or_default();
    let role = item.accessible_role();
    let wanted = name.clone();
    let item = reveal_where(app, &format!("the {name:?} choice"), &move |app| {
        inspect::roots(app.slint()).pop().and_then(|popup| {
            let mut found = None;
            inspect::walk(&popup, &mut |candidate| {
                if found.is_none()
                    && candidate.accessible_role() == role
                    && inspect::label(candidate).as_deref() == Some(wanted.as_str())
                {
                    found = Some(candidate.clone());
                }
            });
            found
        })
    })?;
    click_item(app, &item)?;
    Ok(name)
}

/// Choose `item` in the picker labelled `label` and check that it shows it.
pub fn choose(app: &mut App, label: &str, item: &str) -> Result<(), String> {
    if value(app, AccessibleRole::Combobox, label).as_deref() == Some(item) {
        return Ok(());
    }
    pick_where(app, label, &format!("{item:?}"), &|name| name == item)?;
    let shown = value(app, AccessibleRole::Combobox, label);
    if shown.as_deref() != Some(item) {
        return Err(format!(
            "the {label:?} picker shows {shown:?} after choosing {item:?}"
        ));
    }
    Ok(())
}

/// Open the picker labelled `label` and click the item that is `item` or starts with
/// `item` and a space (shortcut hints follow some items). The picker may run an action
/// instead of showing a value, so nothing else is checked. Returns the item's label.
pub fn pick(app: &mut App, label: &str, item: &str) -> Result<String, String> {
    let prefix = format!("{item} ");
    pick_where(app, label, &format!("{item:?}"), &|name| {
        name == item || name.starts_with(&prefix)
    })
}

/// Whether the check box labelled `label` is checked (None when not shown).
pub fn checked(app: &App, label: &str) -> Option<bool> {
    find_any(app, AccessibleRole::Checkbox, label)
        .and_then(|item| item.accessible_string_property(AccessibleStringProperty::Checked))
        .map(|value| value == "true")
}

/// Set a check box by clicking its caption (Options check boxes keep a transparent
/// native box under a wrapping caption, as a user sees them), or the box itself.
pub fn set_checked(app: &mut App, caption: &str, wanted: bool) -> Result<(), String> {
    let before =
        checked(app, caption).ok_or_else(|| format!("no check box {caption:?} is shown"))?;
    if before == wanted {
        return Ok(());
    }
    let target = if find_any(app, AccessibleRole::Text, caption).is_some() {
        reveal(app, AccessibleRole::Text, caption)?
    } else {
        reveal(app, AccessibleRole::Checkbox, caption)?
    };
    click_item(app, &target)?;
    let after = checked(app, caption);
    if after != Some(wanted) {
        return Err(format!(
            "clicking {caption:?} left the check box at {after:?} instead of {wanted}"
        ));
    }
    Ok(())
}

/// Replace a text field's content by clicking it and typing, then let validation
/// timers run.
pub fn fill(app: &mut App, label: &str, text: &str) -> Result<(), String> {
    let field = reveal(app, AccessibleRole::TextInput, label)?;
    click_item(app, &field)?;
    input::replace_text(app.slint(), text);
    app.idle(Duration::from_millis(200));
    Ok(())
}

/// Scroll a button into view, check that it is enabled and click it.
pub fn press(app: &mut App, label: &str) -> Result<(), String> {
    let button = reveal(app, AccessibleRole::Button, label)?;
    if !enabled(&button) {
        return Err(format!(
            "the {label:?} button is disabled; visible text: {:?}",
            inspect::visible_texts(app.slint())
        ));
    }
    click_item(app, &button)?;
    app.idle(Duration::from_millis(60));
    Ok(())
}

/// The text of a Text item.
pub fn text_of(item: &ItemRc) -> Option<String> {
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    item.downcast::<ComplexText>()
        .map(|text| text.as_pin_ref().text().to_string())
}

/// Every non-empty text of the window and its popups that is shown or can be
/// scrolled to.
pub fn texts(app: &App) -> Vec<String> {
    let mut texts = Vec::new();
    for root in inspect::roots(app.slint()) {
        inspect::walk(&root, &mut |item| {
            if let Some(text) = text_of(item)
                && !text.trim().is_empty()
                && shown(item)
            {
                texts.push(text);
            }
        });
    }
    texts
}

/// The texts (shown or scrolled away) containing `needle`.
pub fn texts_with(app: &App, needle: &str) -> Vec<String> {
    texts(app)
        .into_iter()
        .filter(|text| text.contains(needle))
        .collect()
}

/// The shown items of the list labelled `list`: (label, window rectangle), top to
/// bottom, including rows scrolled out of view.
pub fn list_rows(app: &App, list: &str) -> Vec<(String, inspect::Rect)> {
    let mut rows = Vec::new();
    for root in inspect::roots(app.slint()) {
        inspect::walk(&root, &mut |item| {
            if item.accessible_role() == AccessibleRole::List
                && inspect::label(item).as_deref() == Some(list)
            {
                inspect::walk(item, &mut |row| {
                    if row.accessible_role() == AccessibleRole::ListItem && shown(row) {
                        rows.push((inspect::label(row).unwrap_or_default(), inspect::rect(row)));
                    }
                });
            }
        });
    }
    rows.sort_by(|a, b| a.1.1.total_cmp(&b.1.1));
    rows
}

/// Scroll the row of the list labelled `list` that `accept` takes into view and
/// click it. Returns its label.
pub fn click_row(
    app: &mut App,
    list: &str,
    describe: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Result<String, String> {
    let find = |app: &App| -> Option<ItemRc> {
        let mut found = None;
        for root in inspect::roots(app.slint()) {
            inspect::walk(&root, &mut |item| {
                if found.is_none()
                    && item.accessible_role() == AccessibleRole::List
                    && inspect::label(item).as_deref() == Some(list)
                {
                    inspect::walk(item, &mut |row| {
                        if found.is_none()
                            && row.accessible_role() == AccessibleRole::ListItem
                            && shown(row)
                            && inspect::label(row).is_some_and(|label| accept(&label))
                        {
                            found = Some(row.clone());
                        }
                    });
                }
            });
        }
        found
    };
    let row = reveal_where(app, &format!("the {describe} row of {list:?}"), &find)?;
    let label = inspect::label(&row).unwrap_or_default();
    click_item(app, &row)?;
    app.idle(Duration::from_millis(60));
    Ok(label)
}

/// One rendered frame of the main window.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<slint::Rgb8Pixel>,
}

impl Frame {
    pub fn of(app: &mut App) -> Self {
        app.settle();
        let (width, height, pixels) = app::render(&app.main);
        Self {
            width,
            height,
            pixels,
        }
    }

    fn clamp(&self, (x, y, w, h): inspect::Rect) -> (usize, usize, usize, usize) {
        let left = (x.max(0.0) as usize).min(self.width);
        let top = (y.max(0.0) as usize).min(self.height);
        let right = ((x + w).max(0.0) as usize).min(self.width);
        let bottom = ((y + h).max(0.0) as usize).min(self.height);
        (left, top, right, bottom)
    }

    /// Mean luma (0–255) of a window rectangle.
    pub fn luma(&self, area: inspect::Rect) -> f32 {
        let (left, top, right, bottom) = self.clamp(area);
        let mut sum = 0.0_f64;
        let mut count = 0usize;
        for y in top..bottom {
            for pixel in &self.pixels[y * self.width + left..y * self.width + right] {
                sum += 0.299 * f64::from(pixel.r)
                    + 0.587 * f64::from(pixel.g)
                    + 0.114 * f64::from(pixel.b);
                count += 1;
            }
        }
        if count == 0 {
            0.0
        } else {
            (sum / count as f64) as f32
        }
    }

    /// Mean luma of the whole frame.
    pub fn mean_luma(&self) -> f32 {
        self.luma((0.0, 0.0, self.width as f32, self.height as f32))
    }

    /// FNV-1a digest of a window rectangle's pixels.
    pub fn digest(&self, area: inspect::Rect) -> u64 {
        let (left, top, right, bottom) = self.clamp(area);
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for y in top..bottom {
            for pixel in &self.pixels[y * self.width + left..y * self.width + right] {
                for byte in [pixel.r, pixel.g, pixel.b] {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        hash
    }

    /// How many pixels of a rectangle are close to `color` (each channel within 40).
    pub fn count_near(&self, area: inspect::Rect, color: (u8, u8, u8)) -> usize {
        let (left, top, right, bottom) = self.clamp(area);
        let mut count = 0;
        for y in top..bottom {
            for pixel in &self.pixels[y * self.width + left..y * self.width + right] {
                if pixel.r.abs_diff(color.0) <= 40
                    && pixel.g.abs_diff(color.1) <= 40
                    && pixel.b.abs_diff(color.2) <= 40
                {
                    count += 1;
                }
            }
        }
        count
    }
}

/// The window rectangle of a pane's file list.
pub fn list_area(app: &App, pane: PaneId) -> inspect::Rect {
    let window = &app.window;
    match pane {
        PaneId::Left => (
            window.get_left_list_left(),
            window.get_left_list_top(),
            window.get_left_list_width(),
            window.get_left_list_height(),
        ),
        PaneId::Right => (
            window.get_right_list_left(),
            window.get_right_list_top(),
            window.get_right_list_width(),
            window.get_right_list_height(),
        ),
    }
}

/// Screenshot the main window (with any open dialog) and run the UI review's layout
/// checks on it; findings become review flags of this scenario.
pub fn review_shot(app: &mut App, context: &mut Context, name: &str, caption: &str) {
    // Theme colours animate briefly after changes; let them settle before rendering.
    app.idle(Duration::from_millis(250));
    let shot = app.screenshot(context, name, caption);
    let findings = inspect::check_layout(app.slint(), app.width as f32, app.height as f32);
    if let Some(recorded) = context
        .outcome
        .shots
        .iter_mut()
        .find(|recorded| recorded.file == shot)
    {
        recorded.flags = findings.len();
    }
    for finding in findings {
        context.outcome.flags.push(report::Flag {
            shot: shot.clone(),
            kind: finding.kind,
            detail: finding.detail,
        });
    }
}

/// Wait (bounded) until `path` exists and its size holds still across two looks 50 ms
/// apart (a writer has finished its bytes), pumping the application meanwhile.
pub fn wait_file(app: &mut App, path: &Path, what: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut size: Option<u64> = None;
    loop {
        let now = std::fs::metadata(path).ok().map(|metadata| metadata.len());
        if now.is_some() && now == size {
            return Ok(());
        }
        size = now;
        if Instant::now() >= deadline {
            return Err(format!(
                "{what}: {} did not appear and hold still (size {size:?}); notice: {:?}",
                path.display(),
                app.notice()
            ));
        }
        app.idle(Duration::from_millis(50));
    }
}

/// Wait (bounded) until the button labelled `label` is enabled.
pub fn wait_enabled(app: &mut App, label: &str) -> Result<(), String> {
    for _ in 0..250 {
        if is_enabled(app, AccessibleRole::Button, label) {
            return Ok(());
        }
        app.idle(Duration::from_millis(20));
    }
    Err(format!(
        "the {label:?} button stays disabled; visible text: {:?}",
        inspect::visible_texts(app.slint())
    ))
}

/// Click the middle of a window rectangle.
pub fn click_rect(app: &mut App, (x, y, w, h): inspect::Rect) {
    input::click_at(
        app.slint(),
        slint::LogicalPosition::new(x + w / 2.0, y + h / 2.0),
    );
    app.idle(Duration::from_millis(60));
}

/// Whether the notice dialog is open (`App::notice` keeps the last title after it
/// closes; the dialog's presentation token says whether it is shown).
pub fn notice_open(app: &App) -> bool {
    !app.window
        .global::<NoticePresentation>()
        .get_token()
        .is_empty()
}

/// Wait (bounded) for the notice dialog and return its title and body.
pub fn wait_notice(app: &mut App, what: &str) -> Result<(String, String), String> {
    for _ in 0..400 {
        if notice_open(app) {
            let notice = app.window.global::<NoticePresentation>();
            return Ok((notice.get_title().into(), notice.get_body().into()));
        }
        app.idle(Duration::from_millis(25));
    }
    Err(format!("{what}: no notice appeared"))
}

/// Close the notice dialog with Escape and wait until panel input is accepted again.
pub fn dismiss_notice(app: &mut App) -> Result<(), String> {
    for _ in 0..3 {
        if !notice_open(app) {
            break;
        }
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(100));
    }
    if notice_open(app) {
        return Err(format!(
            "Escape did not close the notice {:?}",
            app.window.global::<NoticePresentation>().get_title()
        ));
    }
    super::panel::wait_input(app);
    Ok(())
}
