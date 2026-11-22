//! The UI polish gate's layout checks for a window at its minimum size, as
//! `crates/midnight-acceptance/tests/acceptance/inspect.rs` (`check_layout`) and
//! `scenarios/ui_gate.rs` judge it, over the surfaces a user sees: the open popups (a
//! menu, an embedded dialog and its pickers), or the window itself when none is open.
//!
//! - a control outside the window (U-K16);
//! - text or a control cut by a container that does not scroll (U-K03), or lying beyond
//!   the content of its scroll view, where no scrolling brings it into view (stricter
//!   than the gate, which judges only what is shown);
//! - two texts drawn over each other (U-K09);
//! - two controls overlapping, neither part of the other (U-K17);
//! - a text shown narrower than about one character while it needs more (U-K19), unless
//!   its own box holds it and a view that scrolls sideways cuts it, as the gate exempts.
#![allow(dead_code)]
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{
        AccessibleRole, Clip, ComplexText, Flickable, Item, Opacity, SimpleText,
        TextHorizontalAlignment, TextVerticalAlignment, TextWrap,
    },
    layout::Orientation,
    window::{WindowInner, WindowKind},
};

/// A window rectangle: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.0 >= outer.0 - 0.5
        && inner.1 >= outer.1 - 0.5
        && inner.0 + inner.2 <= outer.0 + outer.2 + 0.5
        && inner.1 + inner.3 <= outer.1 + outer.3 + 0.5
}

pub fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
        .unwrap_or_default()
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

struct Text {
    text: String,
    wrap: bool,
    halign: TextHorizontalAlignment,
    valign: TextVerticalAlignment,
    preferred: (f32, f32),
    font_size: f32,
}

fn text_of(item: &ItemRc) -> Option<Text> {
    let adapter = item.window_adapter()?;
    let preferred = |orientation| {
        if let Some(text) = item.downcast::<SimpleText>() {
            text.as_pin_ref()
                .layout_info(orientation, -1.0, &adapter, item)
                .preferred
        } else if let Some(text) = item.downcast::<ComplexText>() {
            text.as_pin_ref()
                .layout_info(orientation, -1.0, &adapter, item)
                .preferred
        } else {
            0.0
        }
    };
    let preferred = (
        preferred(Orientation::Horizontal),
        preferred(Orientation::Vertical),
    );
    if let Some(text) = item.downcast::<SimpleText>() {
        let text = text.as_pin_ref();
        return Some(Text {
            text: text.text().to_string(),
            wrap: false,
            halign: text.horizontal_alignment(),
            valign: text.vertical_alignment(),
            preferred,
            font_size: text.font_size().get(),
        });
    }
    let text = item.downcast::<ComplexText>()?;
    let text = text.as_pin_ref();
    Some(Text {
        text: text.text().to_string(),
        wrap: !matches!(text.wrap(), TextWrap::NoWrap),
        halign: text.horizontal_alignment(),
        valign: text.vertical_alignment(),
        preferred,
        font_size: text.font_size().get(),
    })
}

/// The glyph area of a text inside its element.
fn ink((x, y, w, h): Rect, text: &Text) -> Rect {
    let width = if text.wrap {
        w
    } else {
        text.preferred.0.min(w)
    };
    let height = if text.preferred.1 > 0.0 {
        text.preferred.1.min(h)
    } else {
        h
    };
    let left = match text.halign {
        TextHorizontalAlignment::Center => x + (w - width) / 2.0,
        TextHorizontalAlignment::Right | TextHorizontalAlignment::End => x + w - width,
        _ => x,
    };
    let top = match text.valign {
        TextVerticalAlignment::Center => y + (h - height) / 2.0,
        TextVerticalAlignment::Bottom => y + h - height,
        _ => y,
    };
    (left, top, width, height)
}

/// What clips an item as the walk reaches it.
#[derive(Clone, Copy)]
struct Walk {
    hidden: bool,
    /// Every clipping ancestor: what a user can see of an item.
    clip: Option<Rect>,
    /// The clipping containers below the nearest scroll view: they cut without scrolling.
    fixed: Option<Rect>,
    /// The nearest scroll view's content: what lies outside it cannot be scrolled into
    /// view (a view that scrolls only up and down has content as wide as itself).
    extent: Option<Rect>,
    /// Inside an interactive control (a caption, a combo box's field).
    in_control: bool,
    /// Inside a scroll view that scrolls sideways: what its edge cuts is scrolled out of
    /// view, not elided.
    sideways: bool,
}

struct Node {
    item: ItemRc,
    rect: Rect,
    /// The glyph area of a text, else the element.
    area: Rect,
    shown: Option<Rect>,
    text: Option<Text>,
    role: AccessibleRole,
    walk: Walk,
}

/// Hidden by `visible: false` (which Slint lowers to a clipping wrapper) or by an
/// invisible layer. An item that is only clipped away is not hidden: it may lie where
/// no scrolling brings it into view (`ItemRc::is_visible` counts it as invisible).
fn hides(item: &ItemRc) -> bool {
    item.downcast::<Clip>().is_some_and(|clip| {
        let clip = clip.as_pin_ref();
        clip.is_visibility_clip() && clip.clip()
    }) || item
        .downcast::<Opacity>()
        .is_some_and(|layer| layer.as_pin_ref().opacity() < 0.01)
}

fn collect(item: &ItemRc, walk: Walk, nodes: &mut Vec<Node>, flickables: &mut Vec<(Rect, f32)>) {
    let rect = bounds(item);
    let hidden = walk.hidden || hides(item);
    let here = Walk { hidden, ..walk };
    let text = text_of(item).filter(|text| text.text.chars().any(char::is_alphanumeric));
    let role = item.accessible_role();
    if !hidden && rect.2 > 0.5 && rect.3 > 0.5 && (text.is_some() || interactive(role)) {
        let area = text.as_ref().map_or(rect, |text| ink(rect, text));
        let shown = match walk.clip {
            Some(clip) => intersect(area, clip),
            None => Some(area),
        };
        nodes.push(Node {
            item: item.clone(),
            rect,
            area,
            shown,
            text,
            role,
            walk: here,
        });
    }
    let flickable = item.downcast::<Flickable>();
    if let Some(flickable) = &flickable {
        flickables.push((rect, flickable.as_pin_ref().content_width().get()));
    }
    let clips = item.borrow().as_ref().clips_children();
    let sideways = flickable
        .as_ref()
        .is_some_and(|flickable| flickable.as_pin_ref().content_width().get() > rect.2 + 1.0);
    let child = Walk {
        hidden,
        clip: if clips {
            Some(walk.clip.map_or(rect, |clip| {
                intersect(clip, rect).unwrap_or((rect.0, rect.1, 0.0, 0.0))
            }))
        } else {
            walk.clip
        },
        fixed: if flickable.is_some() {
            None
        } else if clips {
            Some(walk.fixed.map_or(rect, |fixed| {
                intersect(fixed, rect).unwrap_or((rect.0, rect.1, 0.0, 0.0))
            }))
        } else {
            walk.fixed
        },
        extent: flickable.as_ref().map_or(walk.extent, |flickable| {
            let flickable = flickable.as_pin_ref();
            Some((
                rect.0 + flickable.content_x().get(),
                rect.1 + flickable.content_y().get(),
                flickable.content_width().get(),
                flickable.content_height().get(),
            ))
        }),
        in_control: walk.in_control || interactive(role),
        sideways: walk.sideways || sideways,
    };
    let mut next = item.first_child();
    while let Some(current) = next {
        next = current.next_sibling();
        collect(&current, child, nodes, flickables);
    }
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

/// A header kept in step with a list that scrolls sideways (a table's column headings):
/// what its clip cuts at the list's edge is scrolled out of view, as the gate's
/// `scrolls_with_neighbour` judges it.
fn scrolls_with_list(flickables: &[(Rect, f32)], clip: Rect, area: Rect) -> bool {
    flickables.iter().any(|&((fx, _, fw, _), content)| {
        (fx - clip.0).abs() <= 4.0
            && (fx + fw - clip.0 - clip.2).abs() <= 18.0
            && content > fw + 1.0
            && content + 18.0 >= area.0 + area.2 - clip.0
    })
}

fn describe(node: &Node) -> String {
    let (x, y, w, h) = node.rect;
    let name = node.text.as_ref().map_or_else(
        || label(&node.item),
        |text| text.text.chars().take(48).collect(),
    );
    format!("{:?} {name:?} at {x:.0},{y:.0} {w:.0}x{h:.0}", node.role)
}

fn check(root: &ItemRc, size: (f32, f32), findings: &mut Vec<String>) {
    let mut nodes = Vec::new();
    let mut flickables = Vec::new();
    collect(
        root,
        Walk {
            hidden: false,
            clip: None,
            fixed: None,
            extent: None,
            in_control: false,
            sideways: false,
        },
        &mut nodes,
        &mut flickables,
    );
    for node in &nodes {
        let control = interactive(node.role) && !node.walk.in_control;
        // Beyond its scroll view's content, shown or not: no scrolling reaches it.
        if let Some(extent) = node.walk.extent
            && (node.text.is_some() || control)
        {
            let (x, y, w, h) = node.area;
            let outside = (extent.0 - x).max(0.0)
                + (x + w - extent.0 - extent.2).max(0.0)
                + (extent.1 - y).max(0.0)
                + (y + h - extent.1 - extent.3).max(0.0);
            if outside > 1.5 {
                findings.push(format!(
                    "beyond what its scroll view reaches: {} reaches {outside:.0}px outside {extent:?}",
                    describe(node)
                ));
            }
        }
        let Some(shown) = node.shown else { continue };
        // U-K16: a control outside the window.
        if control
            && (shown.0 < -0.5
                || shown.1 < -0.5
                || shown.0 + shown.2 > size.0 + 0.5
                || shown.1 + shown.3 > size.1 + 0.5)
        {
            findings.push(format!(
                "control clipped by window: {} in a {:.0}x{:.0} window",
                describe(node),
                size.0,
                size.1
            ));
        }
        if node.text.is_none() && !control {
            continue;
        }
        let (x, y, w, h) = node.area;
        // U-K03: cut by a container that does not scroll.
        if let Some(fixed) = node.walk.fixed
            && !scrolls_with_list(&flickables, fixed, node.area)
        {
            let outside = (fixed.0 - x).max(0.0)
                + (x + w - fixed.0 - fixed.2).max(0.0)
                + (fixed.1 - y).max(0.0)
                + (y + h - fixed.1 - fixed.3).max(0.0);
            if outside > 1.5 {
                findings.push(format!(
                    "content cut off without scrolling: {} reaches {outside:.0}px outside {fixed:?}",
                    describe(node)
                ));
            }
        }
        // U-K19: shown narrower than about one character while it needs more. A text
        // whose own box holds it, cut at the edge of a view that scrolls sideways, is
        // scrolled out of view, not elided (the gate's exemption).
        if let Some(text) = &node.text {
            let font = if text.font_size > 0.0 {
                text.font_size
            } else {
                12.0
            };
            let scrolled = node.walk.sideways && node.rect.2 + 1.0 >= text.preferred.0;
            if text.preferred.0 > shown.2 + 1.0 && shown.2 < font.max(8.0) * 1.2 && !scrolled {
                findings.push(format!(
                    "label elided to nothing: {:?} shows {:.0}px of {:.0}px",
                    text.text, shown.2, text.preferred.0
                ));
            }
        }
    }
    // U-K09 and U-K17: texts drawn over each other, controls overlapping.
    for (index, first) in nodes.iter().enumerate() {
        let Some(a) = first.shown else { continue };
        for second in &nodes[index + 1..] {
            let Some(b) = second.shown else { continue };
            let Some((_, _, w, h)) = intersect(a, b) else {
                continue;
            };
            if w <= 2.0 || h <= 2.0 {
                continue;
            }
            if is_ancestor(&first.item, &second.item) || is_ancestor(&second.item, &first.item) {
                continue;
            }
            match (&first.text, &second.text) {
                (Some(one), Some(two)) => {
                    let same = one.text == two.text
                        && (first.rect.0 - second.rect.0).abs() < 0.5
                        && (first.rect.1 - second.rect.1).abs() < 0.5;
                    if !same {
                        findings.push(format!(
                            "overlapping text: {} and {} overlap by {w:.0}x{h:.0}",
                            describe(first),
                            describe(second)
                        ));
                    }
                }
                (None, None)
                    if interactive(first.role)
                        && interactive(second.role)
                        && !first.walk.in_control
                        && !second.walk.in_control
                        && !contains(a, b)
                        && !contains(b, a) =>
                {
                    findings.push(format!(
                        "overlapping controls: {} and {} overlap by {w:.0}x{h:.0}",
                        describe(first),
                        describe(second)
                    ));
                }
                _ => {}
            }
        }
    }
}

/// Every finding of these checks: the open popups' (tooltips aside), or the window's
/// own when no popup is open, as the gate checks a surface.
pub fn findings(window: &slint::Window) -> Vec<String> {
    let inner = WindowInner::from_pub(window);
    let size = window.size().to_logical(window.scale_factor());
    let mut roots = vec![ItemRc::new_root(inner.component())];
    for popup in inner.active_popups().iter() {
        if !matches!(popup.window_kind, WindowKind::ToolTip) {
            roots.push(ItemRc::new_root(popup.component.clone()));
        }
    }
    let checked = if roots.len() > 1 {
        roots[1..].to_vec()
    } else {
        roots
    };
    let mut findings = Vec::new();
    for root in &checked {
        check(root, (size.width, size.height), &mut findings);
    }
    findings
}

/// What of `item` is shown: its rectangle within its clipping ancestors (popups stop
/// the walk), None when clipped away.
pub fn shown(item: &ItemRc) -> Option<Rect> {
    let full = bounds(item);
    let mut clip: Option<Rect> = None;
    let mut current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = current {
        if ancestor.borrow().as_ref().clips_children() {
            let rect = bounds(&ancestor);
            clip = Some(match clip {
                Some(inner) => intersect(inner, rect).unwrap_or((rect.0, rect.1, 0.0, 0.0)),
                None => rect,
            });
        }
        current = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    match clip {
        Some(clip) => intersect(full, clip),
        None => Some(full),
    }
}
