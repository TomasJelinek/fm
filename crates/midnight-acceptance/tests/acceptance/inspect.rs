//! Read-only queries of a window's live item tree: controls by accessible role and
//! label, visible text, and the UI review's automatic layout checks.
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ComplexText, Flickable, Item, Opacity, SimpleText},
    layout::Orientation,
    window::{WindowInner, WindowKind},
};
use slint::LogicalPosition;

/// The window's root item tree followed by its open non-tooltip popups (embedded
/// dialogs and menus live in popups on the same adapter), topmost last.
pub fn roots(window: &slint::Window) -> Vec<ItemRc> {
    let inner = WindowInner::from_pub(window);
    // A tool window that closed itself (a completed job's) has no item tree any more.
    let Some(component) = inner.try_component() else {
        return Vec::new();
    };
    let mut roots = vec![ItemRc::new_root(component)];
    for popup in inner.active_popups().iter() {
        if !matches!(popup.window_kind, WindowKind::ToolTip) {
            roots.push(ItemRc::new_root(popup.component.clone()));
        }
    }
    roots
}

/// Visit every item below `item`, depth first, in tree order.
pub fn walk(item: &ItemRc, visit: &mut dyn FnMut(&ItemRc)) {
    visit(item);
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        walk(&current, visit);
    }
}

pub fn label(item: &ItemRc) -> Option<String> {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|value| value.to_string())
}

/// The topmost visible control with this role and label (popups first).
pub fn find(window: &slint::Window, role: AccessibleRole, wanted: &str) -> Option<ItemRc> {
    for root in roots(window).iter().rev() {
        let mut found = None;
        walk(root, &mut |item| {
            if found.is_none()
                && item.accessible_role() == role
                && label(item).is_some_and(|value| value == wanted)
                && item.is_visible()
            {
                found = Some(item.clone());
            }
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Every visible control with this role and label, in tree order (popups last).
pub fn find_all(window: &slint::Window, role: AccessibleRole, wanted: &str) -> Vec<ItemRc> {
    controls(window, role)
        .into_iter()
        .filter(|(label, _)| label == wanted)
        .map(|(_, item)| item)
        .collect()
}

/// Labels of the visible items with this role (for Text items: their text).
pub fn labels(window: &slint::Window, role: AccessibleRole) -> Vec<String> {
    controls(window, role)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

/// Every visible control with this role, as (label, item), popups last.
pub fn controls(window: &slint::Window, role: AccessibleRole) -> Vec<(String, ItemRc)> {
    let mut found = Vec::new();
    for root in roots(window) {
        walk(&root, &mut |item| {
            if item.accessible_role() == role && item.is_visible() {
                found.push((label(item).unwrap_or_default(), item.clone()));
            }
        });
    }
    found
}

/// A file system path. Paths are elided in the middle by design, with the full path in
/// their tooltip and accessible label, so they are not captions that must fit whole.
fn looks_like_path(text: &str) -> bool {
    let text = text.trim_start_matches(['…', '.']);
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with('~')
        || text.starts_with("\\\\")
        || (bytes.len() > 2 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
}

fn text_of(item: &ItemRc) -> Option<(String, f32)> {
    if let Some(text) = item.downcast::<SimpleText>() {
        let text = text.as_pin_ref();
        return Some((text.text().to_string(), text.font_size().get()));
    }
    if let Some(text) = item.downcast::<ComplexText>() {
        let text = text.as_pin_ref();
        return Some((text.text().to_string(), text.font_size().get()));
    }
    None
}

/// Every non-empty visible text in the window and its popups.
pub fn visible_texts(window: &slint::Window) -> Vec<String> {
    let mut texts = Vec::new();
    for root in roots(window) {
        walk(&root, &mut |item| {
            if let Some((text, _)) = text_of(item)
                && !text.trim().is_empty()
                && item.is_visible()
            {
                texts.push(text);
            }
        });
    }
    texts
}

/// Window coordinates of an item's rectangle: (x, y, width, height).
pub fn rect(item: &ItemRc) -> Rect {
    let geometry = item.geometry();
    let origin = item.map_to_native_window(geometry.origin);
    (
        origin.x,
        origin.y,
        geometry.size.width,
        geometry.size.height,
    )
}

pub fn center(item: &ItemRc) -> LogicalPosition {
    let (x, y, width, height) = rect(item);
    LogicalPosition::new(x + width / 2.0, y + height / 2.0)
}

fn preferred_width(item: &ItemRc) -> Option<f32> {
    let adapter = item.window_adapter()?;
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(
            text.as_pin_ref()
                .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
                .preferred,
        );
    }
    if let Some(text) = item.downcast::<ComplexText>() {
        return Some(
            text.as_pin_ref()
                .layout_info(Orientation::Horizontal, -1.0, &adapter, item)
                .preferred,
        );
    }
    None
}

fn interactive(role: AccessibleRole) -> bool {
    matches!(
        role,
        AccessibleRole::Button
            | AccessibleRole::Checkbox
            | AccessibleRole::Combobox
            | AccessibleRole::Slider
            | AccessibleRole::Spinbox
            | AccessibleRole::Tab
            | AccessibleRole::TextInput
            | AccessibleRole::Switch
            | AccessibleRole::RadioButton
    )
}

/// A window rectangle: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

/// The intersection of the item's clipping ancestors (scroll views, clipped
/// containers) in window coordinates; None when nothing clips it.
pub fn clip_rect(item: &ItemRc) -> Option<Rect> {
    let mut clip: Option<Rect> = None;
    let mut current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = current {
        if ancestor.borrow().as_ref().clips_children() {
            let bounds = rect(&ancestor);
            clip = Some(match clip {
                Some(inner) => intersect(inner, bounds).unwrap_or((bounds.0, bounds.1, 0.0, 0.0)),
                None => bounds,
            });
        }
        current = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    clip
}

/// Whether a scroll view whose content is wider than itself holds `item`.
fn in_sideways_scroll(item: &ItemRc) -> bool {
    let mut current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = current {
        if let Some(flickable) = ancestor.downcast::<Flickable>()
            && flickable.as_pin_ref().content_width().get() > rect(&ancestor).2 + 1.0
        {
            return true;
        }
        current = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

/// Whether the item or an ancestor is a fully transparent layer.
fn transparent(item: &ItemRc) -> bool {
    let mut current = Some(item.clone());
    while let Some(node) = current {
        if let Some(layer) = node.downcast::<Opacity>()
            && layer.as_pin_ref().opacity() < 0.01
        {
            return true;
        }
        current = node.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

/// The part of the item a user can see: its rectangle within its clipping ancestors;
/// None when it is clipped away, transparent or has no area.
pub fn visible_rect(item: &ItemRc) -> Option<Rect> {
    if transparent(item) {
        return None;
    }
    placed_rect(item)
}

/// Where the item lies within its clipping ancestors, drawn or not: for a control that
/// its siblings draw, such as a WrappingCheckBox, whose accessible native check box
/// sits under an opacity-0 layer beside the drawn box and caption. None when it is
/// hidden, clipped away or has no area.
pub fn placed_rect(item: &ItemRc) -> Option<Rect> {
    if !item.is_visible() {
        return None;
    }
    let full = rect(item);
    if full.2 <= 0.0 || full.3 <= 0.0 {
        return None;
    }
    match clip_rect(item) {
        Some(clip) => intersect(full, clip),
        None => Some(full),
    }
}

/// Run what a frame runs before painting (timers, lazy instantiation) so geometry is
/// current after synthetic input.
pub fn settle_tree(window: &slint::Window) {
    slint::platform::update_timers_and_animations();
    WindowInner::from_pub(window).ensure_tree_instantiated();
}

/// One automatic layout finding.
pub struct Finding {
    pub kind: &'static str,
    pub detail: String,
}

/// The UI review's checks for one rendered window of `width` x `height`: interactive
/// controls outside the window, overlapping interactive controls, zero-size or
/// elided-to-nothing labels, and button text wider than its button.
///
/// Only what a user sees is measured: parts clipped by scroll views and transparent
/// layers do not count. While a dialog or menu is open (a popup on this window), only
/// the popups are checked, each on its own; the window beneath was checked without it.
pub fn check_layout(window: &slint::Window, width: f32, height: f32) -> Vec<Finding> {
    let all = roots(window);
    let checked = if all.len() > 1 {
        all[1..].to_vec()
    } else {
        all
    };
    let mut findings = Vec::new();
    for root in checked {
        check_root(&root, width, height, &mut findings);
    }
    findings
}

fn check_root(root: &ItemRc, width: f32, height: f32, findings: &mut Vec<Finding>) {
    let mut controls: Vec<(String, AccessibleRole, Rect, ItemRc)> = Vec::new();
    walk(root, &mut |item| {
        let role = item.accessible_role();
        if interactive(role)
            && let Some(bounds) = visible_rect(item)
        {
            controls.push((label(item).unwrap_or_default(), role, bounds, item.clone()));
        }
        let Some((text, font_size)) = text_of(item) else {
            return;
        };
        if text.trim().is_empty() || !item.is_visible() || transparent(item) {
            return;
        }
        let short: String = text.chars().take(48).collect();
        let (_, _, text_width, text_height) = rect(item);
        if text_width <= 0.0 || text_height <= 0.0 {
            if clip_rect(item).is_none_or(|clip| clip.2 > 0.0 && clip.3 > 0.0) {
                findings.push(Finding {
                    kind: "zero-size label",
                    detail: format!("{short:?} is {text_width:.0}x{text_height:.0}"),
                });
            }
        } else if let Some((_, _, shown, _)) = visible_rect(item)
            && let Some(preferred) = preferred_width(item)
            && preferred > shown + 1.0
            && shown < font_size.max(8.0) * 1.2
            // A text whose own box holds it, cut at the edge of a view that scrolls
            // sideways (a table's last column), is scrolled out of view, not elided.
            && !(text_width + 1.0 >= preferred && in_sideways_scroll(item))
        {
            findings.push(Finding {
                kind: "label elided to nothing",
                detail: format!("{short:?} shows {shown:.0}px of {preferred:.0}px"),
            });
        }
    });
    for (name, role, (x, y, w, h), item) in &controls {
        let describe = || format!("{role:?} {name:?} at {x:.0},{y:.0} {w:.0}x{h:.0}");
        if *x < -0.5 || *y < -0.5 || x + w > width + 0.5 || y + h > height + 0.5 {
            findings.push(Finding {
                kind: "control clipped by window",
                detail: format!("{} in a {width:.0}x{height:.0} window", describe()),
            });
        }
        if *role == AccessibleRole::Button {
            walk(item, &mut |child| {
                if let Some((text, _)) = text_of(child)
                    && !text.trim().is_empty()
                    && !looks_like_path(&text)
                    && visible_rect(child).is_some()
                    && let Some(preferred) = preferred_width(child)
                {
                    let available = rect(child).2.min(*w);
                    if preferred > available + 1.0 {
                        let short: String = text.chars().take(48).collect();
                        findings.push(Finding {
                            kind: "text does not fit its button",
                            detail: format!(
                                "{short:?} needs {preferred:.0}px, has {available:.0}px ({})",
                                describe()
                            ),
                        });
                    }
                }
            });
        }
    }
    for (index, (first_name, first_role, a, first)) in controls.iter().enumerate() {
        for (second_name, second_role, b, second) in &controls[index + 1..] {
            let Some((_, _, overlap_x, overlap_y)) = intersect(*a, *b) else {
                continue;
            };
            if overlap_x <= 2.0 || overlap_y <= 2.0 {
                continue;
            }
            // A control inside another (a clear button inside a text field) is a
            // composition, not an overlap.
            if contains(a, b)
                || contains(b, a)
                || is_ancestor(first, second)
                || is_ancestor(second, first)
            {
                continue;
            }
            findings.push(Finding {
                kind: "overlapping controls",
                detail: format!(
                    "{first_role:?} {first_name:?} and {second_role:?} {second_name:?} overlap by {overlap_x:.0}x{overlap_y:.0}"
                ),
            });
        }
    }
}

fn contains(outer: &Rect, inner: &Rect) -> bool {
    inner.0 >= outer.0 - 0.5
        && inner.1 >= outer.1 - 0.5
        && inner.0 + inner.2 <= outer.0 + outer.2 + 0.5
        && inner.1 + inner.3 <= outer.1 + outer.3 + 0.5
}

fn is_ancestor(ancestor: &ItemRc, item: &ItemRc) -> bool {
    let mut current = item.parent_item(ParentItemTraversalMode::FindAllParents);
    while let Some(parent) = current {
        if parent == *ancestor {
            return true;
        }
        current = parent.parent_item(ParentItemTraversalMode::FindAllParents);
    }
    false
}
