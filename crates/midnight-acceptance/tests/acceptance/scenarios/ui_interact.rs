//! Interactive checks of the UI polish gate (docs/developer/acceptance/ui-polish.md):
//! Tab and Shift+Tab reach every control of a dialog and the focused control looks
//! focused (3:1 indicator), Escape closes, Enter triggers the default action, every
//! button has an observable effect, no unexpected window or notice follows a
//! dismissal, clickable controls react to hovering and icon-only buttons show their
//! tooltip. `red_cases` proves each check against a deliberately broken window.
use crate::{
    Context,
    app::App,
    files_support, input, inspect,
    scenarios::ui_gate::{self as gate, Frame, Issue, Kind, Node, Target},
    tools_support,
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, StyledTextItem, TooltipArea},
    styled_text,
    window::{WindowInner, WindowKind},
};
use midnight_core::PaneId;
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

pub const TAB_UNREACHABLE: &str = "control not reachable with Tab";
pub const TAB_ORDER: &str = "Shift+Tab does not reverse Tab";
pub const FOCUS_ESCAPES: &str = "focus leaves the dialog";
pub const FOCUS_INVISIBLE: &str = "focus not visible";
pub const FOCUS_FAINT: &str = "focus indicator below 3:1";
pub const ESCAPE: &str = "Escape does not close";
pub const ENTER: &str = "Enter has no effect";
pub const NO_EFFECT: &str = "button has no effect";
pub const EXTRA_SURFACE: &str = "unexpected window or notice";
pub const NO_HOVER: &str = "no hover feedback";
pub const TOOLTIP_MISSING: &str = "tooltip does not show";

/// Opens a surface from the idle state and returns where it lives.
pub type Opener<'a> = Box<dyn FnMut(&mut App) -> Result<Target, String> + 'a>;
/// Whether a surface is still open.
pub type Probe<'a> = Box<dyn Fn(&App, &Target) -> bool + 'a>;

/// A surface the interactive checks can open again and again from the idle state.
pub struct Surface<'a> {
    pub name: String,
    /// Open it from the idle state; returns where it lives.
    pub open: Opener<'a>,
    /// Whether it is still open.
    pub is_open: Probe<'a>,
    /// Bring the application back to the idle state.
    pub reset: Box<dyn FnMut(&mut App) + 'a>,
    /// Buttons that are not clicked, with the reason (recorded as exemptions).
    pub skip: Vec<(String, &'static str)>,
    /// Folders whose listings count as an effect (the panels' folders are always
    /// included).
    pub folders: Vec<PathBuf>,
    pub enter: bool,
    /// Text typed into the focused field before Enter (a dialog whose default action
    /// needs a name).
    pub enter_text: Option<&'static str>,
    pub buttons: bool,
    pub hover: bool,
    /// Popups of the main window the surface lies over (Options under its colour
    /// picker), which stay open when it closes.
    pub nested: usize,
}

impl<'a> Surface<'a> {
    pub fn new(
        name: impl Into<String>,
        open: impl FnMut(&mut App) -> Result<Target, String> + 'a,
        is_open: impl Fn(&App, &Target) -> bool + 'a,
        reset: impl FnMut(&mut App) + 'a,
    ) -> Self {
        Self {
            name: name.into(),
            open: Box::new(open),
            is_open: Box::new(is_open),
            reset: Box::new(reset),
            skip: Vec::new(),
            folders: Vec::new(),
            enter: true,
            enter_text: None,
            buttons: true,
            hover: true,
            nested: 0,
        }
    }
}

/// The outcome of the interactive checks of one surface.
#[derive(Default)]
pub struct Outcome {
    pub issues: Vec<Issue>,
    pub notes: Vec<String>,
}

fn focused(window: &slint::Window) -> Option<ItemRc> {
    WindowInner::from_pub(window).focus_item.borrow().upgrade()
}

/// The dialog's root: the topmost popup of the main window, or a tool window's tree.
fn dialog_root(window: &slint::Window, target: &Target) -> Option<ItemRc> {
    let roots = inspect::roots(window);
    match target {
        Target::Main if roots.len() > 1 => roots.last().cloned(),
        Target::Main => None,
        Target::Tool(_) => roots.first().cloned(),
    }
}

/// The nearest interactive ancestor-or-self.
fn control_of(nodes: &[Node], index: usize) -> usize {
    let mut current = Some(index);
    while let Some(at) = current {
        if gate::interactive(nodes[at].role) {
            return at;
        }
        current = nodes[at].parent;
    }
    index
}

/// Interactive controls a user can operate: shown, enabled and not inside another
/// interactive control.
fn operable(nodes: &[Node]) -> Vec<usize> {
    (0..nodes.len())
        .filter(|&index| {
            let node = &nodes[index];
            gate::interactive(node.role)
                && node.shown.is_some_and(|(_, _, w, h)| w >= 2.0 && h >= 2.0)
                && !node.disabled
                && node
                    .parent
                    .is_none_or(|parent| !gate::interactive(nodes[control_of(nodes, parent)].role))
        })
        .collect()
}

/// How many controls Tab can pass on its way round: the operable ones and those a scroll
/// view or a clipping container keeps out of sight (enabled, not hidden, not inside
/// another interactive control), which take Tab stops as well.
fn tab_stops(nodes: &[Node]) -> usize {
    (0..nodes.len())
        .filter(|&index| {
            let node = &nodes[index];
            gate::interactive(node.role)
                && !node.hidden
                && node.rect.2 >= 2.0
                && node.rect.3 >= 2.0
                && !node.disabled
                && node
                    .parent
                    .is_none_or(|parent| !gate::interactive(nodes[control_of(nodes, parent)].role))
        })
        .count()
}

fn inflate((x, y, w, h): inspect::Rect, by: f32) -> inspect::Rect {
    (x - by, y - by, w + 2.0 * by, h + 2.0 * by)
}

fn name(nodes: &[Node], index: usize) -> String {
    gate::describe(&nodes[index])
}

/// The most common colour of a thin ring just outside `area`.
fn ring_colour(frame: &Frame, area: inspect::Rect) -> Option<[u8; 3]> {
    let outer = inflate(area, 3.0);
    let (x0, y0, x1, y1) = frame.bounds(outer)?;
    let inner = frame.bounds(area);
    let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
    for y in y0..y1 {
        for x in x0..x1 {
            if inner.is_some_and(|(a, b, c, d)| x >= a && x < c && y >= b && y < d) {
                continue;
            }
            *counts.entry(frame.at(x, y)).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(colour, count)| (*count, *colour))
        .map(|(colour, _)| colour)
}

/// Tab through the dialog and back with Shift+Tab: every operable control is reached
/// (one member of a tab or radio group reaches the group), focus stays inside the
/// dialog, Shift+Tab visits the stops in reverse, and every stop looks focused with an
/// indicator of at least 3:1.
pub fn focus_checks(app: &mut App, target: &Target) -> Vec<Issue> {
    let _phase = crate::timing::Span::new("interact: focus checks");
    let mut issues = Vec::new();
    let software = target.software(app);
    let window = software.window();
    gate::pointer_out(window);
    app.idle(Duration::from_millis(150));
    let Some(root) = dialog_root(window, target) else {
        return vec![Issue::new(FOCUS_ESCAPES, "no dialog is open to check")];
    };
    let nodes = gate::nodes(window, &root);
    let wanted = operable(&nodes);
    if wanted.is_empty() {
        return issues;
    }
    let stop = |window: &slint::Window| -> Option<Option<usize>> {
        let item = focused(window)?;
        Some(
            nodes
                .iter()
                .position(|node| node.item == item)
                .map(|index| control_of(&nodes, index)),
        )
    };
    // A hang guard, not the measure of a cycle: the walk ends when the first stop comes
    // back, and its budget counts the controls scrolled out of sight too, which Tab passes
    // on its way round (a long page such as Options › Archives).
    let limit = tab_stops(&nodes).max(wanted.len()) * 2 + 6;
    let mut forward: Vec<Option<usize>> = Vec::new();
    let mut previous = Frame::of(&software);
    let mut escaped = false;
    // The stop that had the focus before each Tab: a Tab that leaves the focus where it
    // was (a dialog whose only stop already has it) shows nothing to compare.
    let mut current = stop(window).flatten();
    for _ in 0..limit {
        input::key(window, Key::Tab);
        app.idle(Duration::from_millis(160));
        let frame = Frame::of(&software);
        match stop(window) {
            None => {}
            Some(None) => {
                if !escaped {
                    issues.push(Issue::new(
                        FOCUS_ESCAPES,
                        "Tab moved the keyboard focus out of the dialog",
                    ));
                }
                escaped = true;
                forward.push(None);
            }
            Some(Some(index)) => {
                forward.push(Some(index));
                let node = &nodes[index];
                let moved = current != Some(index);
                current = Some(index);
                if moved && let Some(shown) = node.shown {
                    let area = inflate(shown, 4.0);
                    let changed = frame.changed(&previous, area);
                    if changed.is_empty() {
                        issues.push(Issue::new(
                            FOCUS_INVISIBLE,
                            if gate::interactive(node.role) {
                                format!("{} looks the same focused and unfocused", name(&nodes, index))
                            } else {
                                format!(
                                    "Tab stops on {}, which is not a control, and nothing shows the focus",
                                    name(&nodes, index)
                                )
                            },
                        ));
                    } else {
                        let ring = ring_colour(&frame, area);
                        let best = changed
                            .iter()
                            .map(|(before, after)| {
                                gate::contrast(*before, *after)
                                    .max(ring.map_or(0.0, |ring| gate::contrast(*after, ring)))
                            })
                            .fold(0.0_f64, f64::max);
                        if best < 3.0 {
                            issues.push(Issue::new(
                                FOCUS_FAINT,
                                format!(
                                    "{}: its focus indicator reaches {best:.2}:1",
                                    name(&nodes, index)
                                ),
                            ));
                        }
                    }
                }
            }
        }
        previous = frame;
        // The cycle closes when the first stop comes back.
        if forward.len() > 1 && forward.last() == forward.first() {
            break;
        }
    }
    let cycle: Vec<Option<usize>> = if forward.len() > 1 && forward.last() == forward.first() {
        forward[..forward.len() - 1].to_vec()
    } else {
        forward.clone()
    };
    let reached: Vec<usize> = cycle.iter().flatten().copied().collect();
    // Tabs and radio buttons are reached as a group: the nearest container holding
    // more than one of them.
    let group = |index: usize| -> Option<usize> {
        let role = nodes[index].role;
        if !matches!(role, AccessibleRole::Tab | AccessibleRole::RadioButton) {
            return None;
        }
        let mut current = nodes[index].parent;
        while let Some(at) = current {
            let members = nodes[at..nodes[at].end]
                .iter()
                .filter(|node| node.role == role)
                .count();
            if members >= 2 {
                return Some(at);
            }
            current = nodes[at].parent;
        }
        None
    };
    let missing: Vec<String> = wanted
        .iter()
        .filter(|index| !reached.contains(index))
        .filter(|index| {
            group(**index).is_none_or(|container| {
                !reached.iter().any(|other| {
                    nodes[*other].role == nodes[**index].role && group(*other) == Some(container)
                })
            })
        })
        .map(|index| name(&nodes, *index))
        .collect();
    if !missing.is_empty() {
        issues.push(Issue::new(
            TAB_UNREACHABLE,
            format!(
                "{} control(s) never get the focus: {}",
                missing.len(),
                missing.join(", ")
            ),
        ));
    }
    // Shift+Tab from the first stop must visit the cycle backwards.
    if cycle.len() >= 2 && forward.last() == forward.first() {
        let mut backward = Vec::new();
        for _ in 0..cycle.len() {
            input::chord(window, &[Key::Shift], Key::Tab);
            app.idle(Duration::from_millis(60));
            backward.push(stop(window).flatten());
        }
        let expected: Vec<Option<usize>> = (0..cycle.len())
            .map(|step| cycle[(cycle.len() * 2 - 1 - step) % cycle.len()])
            .collect();
        if backward != expected {
            let describe = |stops: &[Option<usize>]| {
                stops
                    .iter()
                    .map(|stop| {
                        stop.map_or("outside".to_owned(), |index| nodes[index].label.clone())
                    })
                    .collect::<Vec<_>>()
                    .join(" → ")
            };
            issues.push(Issue::new(
                TAB_ORDER,
                format!(
                    "Shift+Tab visits {} instead of {}",
                    describe(&backward),
                    describe(&expected)
                ),
            ));
        }
    }
    issues
}

/// Hovering every operable control changes how it looks (or its surroundings). Each
/// control is judged against two frames taken just before it is hovered, with the
/// pointer away: pixels that change between those two (a caret blinking, progress) are
/// left out, so that only the hover's own change counts.
pub fn hover_checks(app: &mut App, target: &Target) -> Vec<Issue> {
    let _phase = crate::timing::Span::new("interact: hover checks");
    let mut issues = Vec::new();
    let software = target.software(app);
    let window = software.window();
    gate::pointer_out(window);
    app.idle(Duration::from_millis(200));
    let roots = match dialog_root(window, target) {
        Some(root) => vec![root],
        None => inspect::roots(window).into_iter().take(1).collect(),
    };
    for root in roots {
        let nodes = gate::nodes(window, &root);
        for index in operable(&nodes) {
            let node = &nodes[index];
            if !matches!(
                node.role,
                AccessibleRole::Button
                    | AccessibleRole::Tab
                    | AccessibleRole::Checkbox
                    | AccessibleRole::RadioButton
                    | AccessibleRole::Combobox
                    | AccessibleRole::Switch
            ) {
                continue;
            }
            // The active page or tab needs no hover state.
            if node.active {
                continue;
            }
            let Some(shown) = node.shown else { continue };
            let (x, y, w, h) = shown;
            let area = inflate(shown, 2.0);
            let before = Frame::of(&software);
            app.idle(Duration::from_millis(180));
            let base = Frame::of(&software);
            gate::hover(window, x + w / 2.0, y + h / 2.0);
            app.idle(Duration::from_millis(180));
            let frame = Frame::of(&software);
            if !changed_beyond(&frame, &base, &before, area) {
                issues.push(Issue::new(
                    NO_HOVER,
                    format!("{} does not change when hovered", gate::describe(node)),
                ));
            }
            gate::pointer_out(window);
            app.idle(Duration::from_millis(120));
        }
    }
    issues
}

/// Whether `after` differs from `base` inside `area` at a pixel that did not already
/// change between `before` and `base` (an animation in progress).
fn changed_beyond(after: &Frame, base: &Frame, before: &Frame, area: inspect::Rect) -> bool {
    if after.width != base.width
        || after.height != base.height
        || before.width != base.width
        || before.height != base.height
    {
        return !after.changed(base, area).is_empty();
    }
    let Some((x0, y0, x1, y1)) = after.bounds(area) else {
        return false;
    };
    (y0..y1).any(|y| {
        (x0..x1).any(|x| after.at(x, y) != base.at(x, y) && before.at(x, y) == base.at(x, y))
    })
}

/// Texts of the window's open tooltips: the plain texts of custom tooltip content and
/// the styled text a `Tooltip { text: ... }` shows, which `tools_support::tooltip_texts`
/// does not read.
fn tooltip_texts(window: &slint::Window) -> Vec<String> {
    let roots: Vec<ItemRc> = WindowInner::from_pub(window)
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| ItemRc::new_root(popup.component.clone()))
        .collect();
    let mut texts = tools_support::tooltip_texts(window);
    for root in &roots {
        inspect::walk(root, &mut |item| {
            if let Some(styled) = item.downcast::<StyledTextItem>()
                && item.is_visible()
            {
                let text = styled_text::get_raw_text(&styled.as_pin_ref().text()).into_owned();
                if !text.trim().is_empty() {
                    texts.push(text);
                }
            }
        });
    }
    texts
}

/// Every icon-only button with a tooltip area shows a tooltip with text on hovering.
pub fn tooltip_checks(app: &mut App, target: &Target) -> Vec<Issue> {
    let _phase = crate::timing::Span::new("interact: tooltip checks");
    let mut issues = Vec::new();
    let software = target.software(app);
    let window = software.window();
    let roots = match dialog_root(window, target) {
        Some(root) => vec![root],
        None => inspect::roots(window).into_iter().take(1).collect(),
    };
    for root in roots {
        let nodes = gate::nodes(window, &root);
        for index in operable(&nodes) {
            let node = &nodes[index];
            if node.role != AccessibleRole::Button {
                continue;
            }
            let has_text = nodes[index..node.end].iter().any(|child| {
                child.shown.is_some()
                    && child
                        .text
                        .as_ref()
                        .is_some_and(|text| text.text.chars().any(char::is_alphanumeric))
            });
            if has_text {
                continue;
            }
            let Some(shown) = node.shown else { continue };
            let (cx, cy) = (shown.0 + shown.2 / 2.0, shown.1 + shown.3 / 2.0);
            // Buttons without a tooltip area are flagged by the static check.
            if !gate::tooltip_at(&nodes, cx, cy) {
                continue;
            }
            let delay = nodes
                .iter()
                .filter(|other| matches!(other.kind, Kind::Tooltip))
                .filter_map(|other| other.item.downcast::<TooltipArea>())
                .map(|area| area.as_pin_ref().delay())
                .max();
            let Some(delay) = delay else { continue };
            gate::pointer_out(window);
            app.settle();
            gate::hover(window, cx - 1.0, cy);
            gate::hover(window, cx, cy);
            let deadline = Instant::now() + Duration::from_millis(delay.max(0) as u64 + 600);
            let mut texts = Vec::new();
            while Instant::now() < deadline {
                app.idle(Duration::from_millis(60));
                texts = tooltip_texts(window);
                if texts
                    .iter()
                    .any(|text| text.chars().any(char::is_alphanumeric))
                {
                    break;
                }
            }
            if !texts
                .iter()
                .any(|text| text.chars().any(char::is_alphanumeric))
            {
                issues.push(Issue::new(
                    TOOLTIP_MISSING,
                    format!("{} shows no tooltip text on hovering", gate::describe(node)),
                ));
            }
            gate::pointer_out(window);
            app.idle(Duration::from_millis(60));
        }
    }
    issues
}

// ---------------------------------------------------------------------------------------
// Observable state: what a click or a key may change

#[derive(Clone, PartialEq, Debug)]
struct State {
    windows: Vec<String>,
    notice: Option<(String, String)>,
    popups: usize,
    texts: Vec<String>,
    controls: Vec<String>,
    folders: u64,
    commands: Vec<String>,
    picker: usize,
    jobs: (usize, usize, usize),
    panels: String,
    pixels: u64,
    /// Copies to the in-process clipboard so far (Copy details, a copied path).
    clipboard: u64,
}

fn folder_digest(folders: &[PathBuf]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for folder in folders {
        let mut entries: Vec<(String, u64, u64, bool)> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| {
                let metadata = entry.metadata().ok();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    metadata.as_ref().map_or(0, |m| m.len()),
                    metadata
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |time| time.as_nanos() as u64),
                    metadata.as_ref().is_some_and(|m| m.is_dir()),
                )
            })
            .collect();
        entries.sort();
        entries.hash(&mut hasher);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
                if let Ok(metadata) = entry.metadata() {
                    (entry.file_name(), metadata.permissions().mode()).hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

fn control_states(window: &slint::Window) -> Vec<String> {
    let mut states = Vec::new();
    for root in inspect::roots(window) {
        inspect::walk(&root, &mut |item| {
            if gate::interactive(item.accessible_role()) && item.is_visible() {
                let get = |which| {
                    item.accessible_string_property(which)
                        .map(|value| value.to_string())
                        .unwrap_or_default()
                };
                states.push(format!(
                    "{:?}|{}|{}|{}|{}|{}",
                    item.accessible_role(),
                    get(AccessibleStringProperty::Label),
                    get(AccessibleStringProperty::Checked),
                    get(AccessibleStringProperty::Value),
                    get(AccessibleStringProperty::Enabled),
                    get(AccessibleStringProperty::Expanded)
                ));
            }
        });
    }
    states
}

fn windows_of(app: &App) -> Vec<Rc<MinimalSoftwareWindow>> {
    let mut windows = vec![app.main.clone()];
    windows.extend(app.tool_windows());
    windows
}

fn state(app: &App, folders: &[PathBuf], skip: &[(usize, inspect::Rect)]) -> State {
    let _phase = crate::timing::Span::new("interact: state sample");
    let windows = windows_of(app);
    let mut texts = Vec::new();
    let mut controls = Vec::new();
    let mut popups = 0;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (position, window) in windows.iter().enumerate() {
        let slint_window = window.window();
        texts.push(format!("#{position}"));
        texts.extend(inspect::visible_texts(slint_window));
        controls.extend(control_states(slint_window));
        popups += inspect::roots(slint_window).len() - 1;
        let frame = Frame::of(window);
        let (width, height) = frame.logical();
        let except: Vec<inspect::Rect> = skip
            .iter()
            .filter(|(at, _)| *at == position)
            .map(|(_, rect)| *rect)
            .collect();
        frame
            .digest((0.0, 0.0, width, height), &except)
            .hash(&mut hasher);
    }
    let mut all_folders: Vec<PathBuf> = folders.to_vec();
    for pane in [PaneId::Left, PaneId::Right] {
        if let Some(location) = app.location(pane) {
            all_folders.push(location);
        }
    }
    State {
        windows: app
            .tool_windows()
            .iter()
            .map(|window| tools_support::title(window))
            .collect(),
        notice: tools_support::shown_notice(app),
        popups,
        texts,
        controls,
        folders: folder_digest(&all_folders),
        // The harness's own acknowledgement of the visible listing is sent in the
        // background after any refresh: never a button's effect.
        commands: app
            .recent_events()
            .into_iter()
            .filter(|line| line.starts_with('→') && !line.contains("PaneCommitted"))
            .collect(),
        picker: app.picker.requests().len(),
        jobs: (
            app.copies.len(),
            app.operations.len(),
            app.checksum_runs.len() + app.manifests.len(),
        ),
        panels: format!(
            "{:?}|{:?}|{}|{}|{:?}|{:?}",
            app.location(PaneId::Left),
            app.location(PaneId::Right),
            app.cursor(PaneId::Left),
            app.cursor(PaneId::Right),
            files_support::selected_names(app, PaneId::Left).len(),
            files_support::selected_names(app, PaneId::Right).len()
        ),
        pixels: hasher.finish(),
        clipboard: app
            .clipboard
            .as_ref()
            .map_or(0, |clipboard| clipboard.writes()),
    }
}

/// The fields still changing between the two samples before an action: they cannot
/// count as its effect, which the outcome's notes then say.
fn unsettled(noisy: &State, quiet: &State) -> Vec<&'static str> {
    let mut found = Vec::new();
    macro_rules! field {
        ($name:ident) => {
            if noisy.$name != quiet.$name {
                found.push(stringify!($name));
            }
        };
    }
    field!(windows);
    field!(notice);
    field!(popups);
    field!(texts);
    field!(controls);
    field!(folders);
    field!(picker);
    field!(jobs);
    field!(panels);
    field!(pixels);
    field!(clipboard);
    field!(commands);
    found
}

/// What changed from `quiet` to `after` that had not changed between the two quiet
/// samples `noisy` and `quiet` (ongoing progress is not an effect).
fn evidence(noisy: &State, quiet: &State, after: &State) -> Vec<&'static str> {
    let mut found = Vec::new();
    macro_rules! field {
        ($name:ident) => {
            if noisy.$name == quiet.$name && quiet.$name != after.$name {
                found.push(stringify!($name));
            }
        };
    }
    field!(windows);
    field!(notice);
    field!(popups);
    field!(texts);
    field!(controls);
    field!(folders);
    field!(picker);
    field!(jobs);
    field!(panels);
    field!(pixels);
    field!(clipboard);
    if noisy.commands == quiet.commands && quiet.commands != after.commands {
        found.push("commands");
    }
    found
}

fn is_dismissal(label: &str) -> bool {
    let label = label.trim().to_lowercase();
    matches!(
        label.as_str(),
        "cancel" | "close" | "cancel dialog" | "close dialog" | "decide later" | "close window"
    ) || label.starts_with("close ")
}

/// After a dismissal: no window or notice beyond those open before the surface, and no
/// main-window popup beyond the `nested` ones it lay over.
fn extra_surfaces(app: &App, baseline: &[String], what: &str, nested: usize) -> Option<Issue> {
    let mut windows: Vec<String> = app
        .tool_windows()
        .iter()
        .map(|window| tools_support::title(window))
        .collect();
    for title in baseline {
        if let Some(position) = windows.iter().position(|window| window == title) {
            windows.remove(position);
        }
    }
    let notice = tools_support::shown_notice(app);
    let popups = inspect::roots(app.slint()).len() > 1 + nested;
    if windows.is_empty() && notice.is_none() && !popups {
        return None;
    }
    Some(Issue::new(
        EXTRA_SURFACE,
        format!(
            "after {what}: windows {windows:?}, notice {:?}, main window popup {popups}",
            notice.map(|(title, body)| format!(
                "{title}: {}",
                body.chars().take(80).collect::<String>()
            ))
        ),
    ))
}

fn titles(app: &App) -> Vec<String> {
    app.tool_windows()
        .iter()
        .map(|window| tools_support::title(window))
        .collect()
}

/// Wait up to `limit` for the surface to close.
fn wait_closed(app: &mut App, surface: &Surface<'_>, target: &Target, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        app.idle(Duration::from_millis(40));
        if !(surface.is_open)(app, target) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
    }
}

/// The software window index (0 is the main window) a target lives in.
fn window_index(app: &App, target: &Target) -> usize {
    match target {
        Target::Main => 0,
        Target::Tool(window) => windows_of(app)
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, window))
            .unwrap_or(0),
    }
}

/// A button of the dialog: role, label and its occurrence among equal ones.
#[derive(Clone)]
struct ButtonRef {
    role: AccessibleRole,
    label: String,
    occurrence: usize,
}

fn dialog_buttons(app: &App, target: &Target) -> Vec<ButtonRef> {
    let window = target.window(app);
    let Some(root) = dialog_root(window, target) else {
        return Vec::new();
    };
    let nodes = gate::nodes(window, &root);
    let mut buttons: Vec<ButtonRef> = Vec::new();
    for index in operable(&nodes) {
        let node = &nodes[index];
        if !matches!(node.role, AccessibleRole::Button | AccessibleRole::Tab) {
            continue;
        }
        // Selecting the active page or tab again is a no-op by design.
        if node.active {
            continue;
        }
        // Counted as `locate` finds it again: among every inactive control of this role
        // and label, such as the Checksums footer's Verify after the Verify page tab.
        let occurrence = nodes[..index]
            .iter()
            .filter(|other| other.role == node.role && other.label == node.label && !other.active)
            .count();
        buttons.push(ButtonRef {
            role: node.role,
            label: node.label.clone(),
            occurrence,
        });
    }
    buttons
}

/// Find the button again in a freshly opened surface and scroll it into view.
fn locate(app: &mut App, target: &Target, button: &ButtonRef) -> Option<(ItemRc, inspect::Rect)> {
    let software = target.software(app);
    let window = software.window();
    for _ in 0..24 {
        let root = dialog_root(window, target)?;
        let nodes = gate::nodes(window, &root);
        let node = nodes
            .iter()
            .filter(|node| node.role == button.role && node.label == button.label && !node.active)
            .nth(button.occurrence)
            .cloned()?;
        let full = node.rect;
        match node.shown {
            Some(shown) if shown.3 >= full.3 * 0.9 && shown.2 >= full.2.min(48.0) * 0.9 => {
                return Some((node.item.clone(), shown));
            }
            _ => {}
        }
        // Scroll the nearest scrolling ancestor towards it.
        let flick = node.flick_owner?;
        let area = nodes[flick].shown?;
        let below = full.1 + full.3 / 2.0 > area.1 + area.3 / 2.0;
        window.dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(area.0 + area.2 / 2.0, area.1 + area.3 / 2.0),
            delta_x: 0.0,
            delta_y: if below { -48.0 } else { 48.0 },
        });
        app.idle(Duration::from_millis(60));
    }
    None
}

/// Whether `button` can still be pressed in the freshly reopened surface. A button that
/// clears a list comes after the buttons it would empty the list for, and an earlier one
/// can leave it nothing to do: with every finished job clean, Jobs' Clear finished without
/// errors empties the list, and the reopened Jobs disables Clear all finished.
fn still_enabled(app: &mut App, surface: &mut Surface<'_>, button: &ButtonRef) -> bool {
    let Ok(target) = (surface.open)(app) else {
        return true;
    };
    let window = target.window(app);
    let enabled = dialog_root(window, &target).is_none_or(|root| {
        gate::nodes(window, &root)
            .iter()
            .filter(|node| node.role == button.role && node.label == button.label && !node.active)
            .nth(button.occurrence)
            .is_none_or(|node| !node.disabled)
    });
    (surface.reset)(app);
    enabled
}

/// What one action showed: the evidence of an effect, an unexpected surface after a
/// dismissal, and the fields that were still changing before it (never evidence).
type Acted = (Vec<&'static str>, Option<Issue>, Vec<&'static str>);

/// Open the surface fresh and measure one action (a click on `button`, or Enter).
fn act(
    app: &mut App,
    surface: &mut Surface<'_>,
    what: &str,
    button: Option<&ButtonRef>,
    dismissal: bool,
) -> Result<Acted, String> {
    let baseline = titles(app);
    let phase = crate::timing::Span::new("act: open");
    let target = (surface.open)(app)?;
    gate::pointer_out(target.window(app));
    app.idle(Duration::from_millis(150));
    drop(phase);
    let phase = crate::timing::Span::new("act: before the action");
    if button.is_none()
        && let Some(text) = surface.enter_text
    {
        input::replace_text(target.window(app), text);
        app.idle(Duration::from_millis(150));
    }
    let index = window_index(app, &target);
    let clicked = match button {
        Some(button) => Some(
            locate(app, &target, button)
                .ok_or_else(|| format!("{:?} {:?} not found", button.role, button.label))?
                .1,
        ),
        None => None,
    };
    // The focused control and the clicked one change by focus and hover alone.
    let mut skip: Vec<(usize, inspect::Rect)> = focused(target.window(app))
        .map(|item| (index, inflate(inspect::rect(&item), 6.0)))
        .into_iter()
        .collect();
    if let Some(rect) = clicked {
        skip.push((index, inflate(rect, 6.0)));
    }
    let mut noisy = state(app, &surface.folders, &skip);
    app.idle(Duration::from_millis(250));
    let mut quiet = state(app, &surface.folders, &skip);
    // A surface still settling (a page detecting its file, a panel re-reading a folder)
    // changes between the two samples, and a change there never counts as the click's:
    // wait up to 2 s more for two equal samples.
    for _ in 0..8 {
        if noisy == quiet {
            break;
        }
        noisy = quiet;
        app.idle(Duration::from_millis(250));
        quiet = state(app, &surface.folders, &skip);
    }
    drop(phase);
    let phase = crate::timing::Span::new("act: after the action");
    match clicked {
        Some((x, y, w, h)) => input::click_at(
            target.window(app),
            LogicalPosition::new(x + w / 2.0, y + h / 2.0),
        ),
        None => input::key(target.window(app), Key::Return),
    }
    gate::pointer_out(target.window(app));
    let mut found = Vec::new();
    for wait in [400_u64, 900] {
        app.idle(Duration::from_millis(wait));
        let after = state(app, &surface.folders, &skip);
        found = evidence(&noisy, &quiet, &after);
        if !found.is_empty() {
            break;
        }
    }
    drop(phase);
    let phase = crate::timing::Span::new("act: dismissal wait");
    let extra = if dismissal {
        app.idle(Duration::from_millis(500));
        extra_surfaces(app, &baseline, what, surface.nested)
    } else {
        None
    };
    drop(phase);
    let _phase = crate::timing::Span::new("act: reset");
    (surface.reset)(app);
    Ok((found, extra, unsettled(&noisy, &quiet)))
}

/// Run every interactive check on a surface.
pub fn check_surface(app: &mut App, surface: &mut Surface<'_>) -> Outcome {
    let mut outcome = Outcome::default();
    let baseline = titles(app);
    // Focus, hover and tooltips on one opening; then Escape closes it.
    match (surface.open)(app) {
        Err(error) => {
            outcome.issues.push(Issue::new(
                "surface did not open",
                format!("{}: {error}", surface.name),
            ));
            (surface.reset)(app);
            return outcome;
        }
        Ok(target) => {
            outcome.issues.extend(focus_checks(app, &target));
            if surface.hover {
                outcome.issues.extend(hover_checks(app, &target));
                outcome.issues.extend(tooltip_checks(app, &target));
            }
            let _phase = crate::timing::Span::new("interact: Escape");
            let window = target.software(app);
            input::key(window.window(), Key::Escape);
            if !wait_closed(app, surface, &target, Duration::from_secs(3)) {
                outcome.issues.push(Issue::new(
                    ESCAPE,
                    format!("{} stays open after Escape", surface.name),
                ));
            } else {
                app.idle(Duration::from_millis(500));
                if let Some(issue) = extra_surfaces(app, &baseline, "Escape", surface.nested) {
                    outcome.issues.push(issue);
                }
            }
            (surface.reset)(app);
        }
    }
    if surface.enter {
        match act(app, surface, "Enter", None, false) {
            Ok((found, _, unsettled)) if found.is_empty() => outcome.issues.push(Issue::new(
                ENTER,
                format!(
                    "Enter in {} changes nothing observable{}",
                    surface.name,
                    if unsettled.is_empty() {
                        String::new()
                    } else {
                        format!(" (still changing before it: {unsettled:?})")
                    }
                ),
            )),
            Ok((found, _, _)) => outcome
                .notes
                .push(format!("{}: Enter changed {found:?}", surface.name)),
            Err(error) => outcome.issues.push(Issue::new(
                "surface did not open",
                format!("{} (Enter): {error}", surface.name),
            )),
        }
    }
    if surface.buttons {
        let buttons = match (surface.open)(app) {
            Ok(target) => {
                let mut found = dialog_buttons(app, &target);
                // A button that clears a list (Jobs) leaves nothing for the buttons after
                // it, since a surface reopens on the same list: those come last, the one
                // that clears everything at the very end.
                found.sort_by_key(|button| {
                    (
                        button.label.starts_with("Clear"),
                        button.label.starts_with("Clear all"),
                    )
                });
                (surface.reset)(app);
                found
            }
            Err(error) => {
                outcome.issues.push(Issue::new(
                    "surface did not open",
                    format!("{} (buttons): {error}", surface.name),
                ));
                (surface.reset)(app);
                Vec::new()
            }
        };
        for button in buttons {
            if let Some((_, reason)) = surface
                .skip
                .iter()
                .find(|(label, _)| *label == button.label)
            {
                outcome.notes.push(format!(
                    "exempt ({reason}): {}: button {:?} not clicked",
                    surface.name, button.label
                ));
                continue;
            }
            if button.label.starts_with("Clear") && !still_enabled(app, surface, &button) {
                outcome.notes.push(format!(
                    "{}: button {:?} is disabled once the buttons before it ran: not clicked",
                    surface.name, button.label
                ));
                continue;
            }
            let dismissal = is_dismissal(&button.label);
            let what = format!("clicking {:?}", button.label);
            match act(app, surface, &what, Some(&button), dismissal) {
                Ok((found, extra, unsettled)) => {
                    if found.is_empty() {
                        outcome.issues.push(Issue::new(
                            NO_EFFECT,
                            format!(
                                "{}: {:?} {:?} changes nothing observable{}",
                                surface.name,
                                button.role,
                                button.label,
                                if unsettled.is_empty() {
                                    String::new()
                                } else {
                                    format!(" (still changing before the click: {unsettled:?})")
                                }
                            ),
                        ));
                    } else {
                        outcome.notes.push(format!(
                            "{}: {:?} changed {found:?}",
                            surface.name, button.label
                        ));
                    }
                    if let Some(issue) = extra {
                        outcome.issues.push(Issue::new(
                            issue.kind,
                            format!("{}: {}", surface.name, issue.detail),
                        ));
                    }
                }
                Err(error) => outcome.issues.push(Issue::new(
                    NO_EFFECT,
                    format!(
                        "{}: {:?} could not be clicked: {error}",
                        surface.name, button.label
                    ),
                )),
            }
        }
    }
    outcome
}

/// Record a surface's interactive findings against its screenshot.
pub fn record(context: &mut Context, surface: &str, shot: &str, outcome: Outcome) -> usize {
    for note in outcome.notes {
        context.note(note);
    }
    gate::record(context, shot, surface, outcome.issues)
}

// ---------------------------------------------------------------------------------------
// Red cases of the interactive checks

slint::slint! {
    component RedButton inherits Rectangle {
        in property <string> caption;
        in property <bool> focusable: true;
        in property <bool> ring: true;
        in property <color> ring-color: #1a3fb4;
        in property <bool> hover-look: true;
        out property <bool> focused: focus.has-focus;
        callback clicked();
        width: 96px; height: 30px;
        background: area.has-hover && root.hover-look ? #c0d0ff : #d8d8d8;
        border-width: 2px;
        border-color: focus.has-focus && root.ring ? root.ring-color : #d8d8d8;
        accessible-role: button; accessible-label: root.caption;
        accessible-action-default => { root.clicked(); }
        forward-focus: focus;
        focus := FocusScope {
            enabled: root.focusable;
            key-pressed(event) => {
                if event.text == " " { root.clicked(); return accept; }
                reject
            }
            area := TouchArea {
                clicked => {
                    if root.focusable { focus.focus(); }
                    root.clicked();
                }
            }
        }
        Text {
            x: 0px; y: 0px; width: parent.width; height: parent.height;
            horizontal-alignment: center; vertical-alignment: center;
            text: root.caption; color: #000000; accessible-role: none;
        }
    }

    // An icon-only button (case 11); its tooltip comes with the two variants below.
    component RedIconBase inherits Rectangle {
        callback clicked();
        width: 30px; height: 30px;
        background: area.has-hover ? #c0d0ff : #d8d8d8;
        border-width: 2px;
        border-color: focus.has-focus ? #1a3fb4 : #d8d8d8;
        accessible-role: button; accessible-label: "Refresh";
        accessible-action-default => { root.clicked(); }
        forward-focus: focus;
        focus := FocusScope {
            key-pressed(event) => {
                if event.text == " " { root.clicked(); return accept; }
                reject
            }
            area := TouchArea {
                clicked => { focus.focus(); root.clicked(); }
            }
        }
        // The icon.
        Rectangle { x: 9px; y: 9px; width: 12px; height: 12px; background: #000000; }
    }

    // The corrected twin: a tooltip that names the button's action.
    component RedIconButton inherits RedIconBase {
        Tooltip { text: @markdown("Refresh the status"); }
    }

    // The broken one: a tooltip area whose tooltip shows no text.
    component RedIconButtonMute inherits RedIconBase {
        Tooltip { text: @markdown(" "); }
    }

    // One dialog with one deliberately broken behaviour per case.
    export component RedDialog inherits Window {
        in property <int> case;
        in property <bool> broken: true;
        in-out property <string> status: "Ready";
        in-out property <bool> details;
        callback closed();
        callback extra();
        title: "Red dialog";
        background: #ffffff;
        default-font-size: 14px;
        preferred-width: 480px;
        preferred-height: 260px;
        public function focus-first() { elsewhere.close(); first.focus(); }
        init => { first.focus(); }
        // Case 10: where the broken dialog's Tab sends the focus, outside the dialog.
        elsewhere := PopupWindow {
            x: 300px; y: 60px; width: 160px; height: 60px;
            close-policy: no-auto-close;
            forward-focus: elsewhere-focus;
            Rectangle {
                background: #f0f0f0;
                Text { x: 8px; y: 8px; text: "Somewhere else"; color: #000000; }
            }
            elsewhere-focus := FocusScope {
                key-pressed(event) => { reject }
            }
        }
        FocusScope {
            x: 0px; y: 0px; width: root.width; height: root.height;
            focus-on-tab-navigation: false;
            capture-key-pressed(event) => {
                // Case 9: Shift+Tab moves forwards, like Tab.
                if root.case == 9 && root.broken && event.text == Key.Tab && event.modifiers.shift {
                    if first.focused { second.focus(); }
                    else if second.focused { third.focus(); }
                    else { first.focus(); }
                    return accept;
                }
                // Case 10: Tab from the last button leaves the dialog.
                if root.case == 10 && root.broken && event.text == Key.Tab && !event.modifiers.shift
                    && third.focused {
                    elsewhere.show();
                    return accept;
                }
                reject
            }
            key-pressed(event) => {
                if event.text == Key.Escape {
                    if !(root.case == 3 && root.broken) { root.closed(); }
                    return accept;
                }
                if event.text == Key.Return {
                    if !(root.case == 4 && root.broken) { root.status = "Applied"; }
                    return accept;
                }
                reject
            }
            Text { x: 12px; y: 10px; text: "Red dialog " + root.case; color: #000000; }
            Text { x: 12px; y: 40px; text: root.status; color: #000000; }
            if root.details: Text { x: 12px; y: 70px; text: "Details: 3 files, 2 folders"; color: #000000; }
            first := RedButton {
                x: 12px; y: 200px; caption: "Apply";
                ring: !(root.case == 2 && root.broken);
                ring-color: root.case == 8 && root.broken ? #cfcfcf : #1a3fb4;
                hover-look: !(root.case == 7 && root.broken);
                clicked => { root.status = "Applied"; }
            }
            second := RedButton {
                x: 116px; y: 200px; caption: "Details";
                focusable: !(root.case == 1 && root.broken);
                clicked => { if !(root.case == 5 && root.broken) { root.details = !root.details; } }
            }
            third := RedButton {
                x: 220px; y: 200px; caption: "Cancel";
                clicked => {
                    root.closed();
                    if root.case == 6 && root.broken { root.extra(); }
                }
            }
            // Case 11: the broken tooltip area shows no text.
            if root.case == 11 && !root.broken : RedIconButton {
                x: 330px; y: 200px;
                clicked => { root.status = "Refreshed"; }
            }
            if root.case == 11 && root.broken : RedIconButtonMute {
                x: 330px; y: 200px;
                clicked => { root.status = "Refreshed"; }
            }
        }
    }

    // What the broken dialog of case 6 opens after Cancel.
    export component RedNotice inherits Window {
        title: "Red notice";
        background: #ffffff;
        preferred-width: 300px;
        preferred-height: 120px;
        Text { x: 12px; y: 12px; text: "Something happened"; color: #000000; }
    }
}

/// The interactive red cases: case, broken behaviour, the check that must flag it.
const INTERACTIVE: [(i32, &str); 11] = [
    (1, TAB_UNREACHABLE),
    (2, FOCUS_INVISIBLE),
    (3, ESCAPE),
    (4, ENTER),
    (5, NO_EFFECT),
    (6, EXTRA_SURFACE),
    (7, NO_HOVER),
    (8, FOCUS_FAINT),
    (9, TAB_ORDER),
    (10, FOCUS_ESCAPES),
    (11, TOOLTIP_MISSING),
];

/// Prove every interactive check against the red dialog; returns the failures.
pub fn red_cases(app: &mut App, context: &mut Context) -> Result<(), String> {
    let dialog = RedDialog::new().map_err(|error| format!("red dialog: {error}"))?;
    let notice = RedNotice::new().map_err(|error| format!("red notice: {error}"))?;
    let weak = dialog.as_weak();
    dialog.on_closed(move || {
        if let Some(dialog) = weak.upgrade() {
            let _ = dialog.hide();
        }
    });
    let extra = notice.as_weak();
    dialog.on_extra(move || {
        if let Some(notice) = extra.upgrade() {
            let _ = notice.show();
        }
    });
    let mut failures = Vec::new();
    for (case, kind) in INTERACTIVE {
        for broken in [true, false] {
            let handle = dialog.as_weak();
            let open = move |app: &mut App| -> Result<Target, String> {
                let dialog = handle.upgrade().ok_or("red dialog gone")?;
                dialog.set_case(case);
                dialog.set_broken(broken);
                dialog.set_status("Ready".into());
                dialog.set_details(false);
                let before = app.tool_windows().len();
                dialog.show().map_err(|error| error.to_string())?;
                app.settle();
                let window = app
                    .tool_windows()
                    .into_iter()
                    .find(|window| tools_support::title(window) == "Red dialog")
                    .ok_or_else(|| format!("red dialog not shown ({before} windows before)"))?;
                gate::size_tool(&window, (480.0, 260.0), 1.0);
                window
                    .window()
                    .dispatch_event(WindowEvent::WindowActiveChanged(true));
                app.settle();
                // A fresh opening starts with the focus on the first button.
                dialog.invoke_focus_first();
                app.settle();
                Ok(Target::Tool(window))
            };
            let is_open = |_: &App, target: &Target| match target {
                Target::Tool(window) => window.window().is_visible(),
                Target::Main => false,
            };
            let close_dialog = dialog.as_weak();
            let close_notice = notice.as_weak();
            let reset = move |app: &mut App| {
                if let Some(dialog) = close_dialog.upgrade() {
                    let _ = dialog.hide();
                }
                if let Some(notice) = close_notice.upgrade() {
                    let _ = notice.hide();
                }
                app.settle();
            };
            let mut surface = Surface::new(format!("red dialog {case}"), open, is_open, reset);
            let outcome = check_surface(app, &mut surface);
            let flagged: Vec<String> = outcome
                .issues
                .iter()
                .map(|issue| format!("{}: {}", issue.kind, issue.detail))
                .collect();
            if broken && !outcome.issues.iter().any(|issue| issue.kind == kind) {
                failures.push(format!(
                    "interactive case {case} ({kind}): the broken dialog was not flagged; found: {flagged:?}"
                ));
            }
            if !broken && !outcome.issues.is_empty() {
                failures.push(format!(
                    "interactive case {case} ({kind}): the corrected dialog was flagged: {flagged:?}"
                ));
            }
            if broken {
                context.note(format!("red interactive case {case} {kind}: {flagged:?}"));
            }
        }
    }
    files_support::dismiss_all(app);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
