//! The UI polish gate's measurements (docs/developer/acceptance/ui-polish.md) in the
//! form the job, Jobs, Checksums, Compare and Copy/Move tests need (of a window, or of an
//! embedded dialog's popup): a window's own minimum size,
//! text or a control cut by a container that does not scroll (U-K03), one-line text
//! clipped without an ellipsis (U-K04), wrapped text cut off (U-K05), a caption that
//! does not fit the visible part of its button (U-K20), and how a focus change shows
//! (U-K24, U-K25). The rules follow `crates/midnight-acceptance/tests/acceptance/
//! scenarios/ui_gate.rs` and `ui_interact.rs`.
#![allow(dead_code)]
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ItemTreeRc, ParentItemTraversalMode},
    items::{
        AccessibleRole, Clip, ComplexText, Flickable, Item, Opacity, SimpleText,
        TextHorizontalAlignment, TextOverflow, TextVerticalAlignment, TextWrap,
    },
    layout::Orientation,
    window::WindowInner,
};

/// A window rectangle: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

/// The window's minimum logical size, from its root layout, as the gate sizes it.
pub fn minimum_size(window: &slint::Window) -> (u32, u32) {
    let component = WindowInner::from_pub(window).component();
    let component = ItemTreeRc::borrow_pin(&component);
    let width = component.as_ref().layout_info(Orientation::Horizontal).min;
    let height = component.as_ref().layout_info(Orientation::Vertical).min;
    (width.max(1.0).ceil() as u32, height.max(1.0).ceil() as u32)
}

pub fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

fn cut((x, y, w, h): Rect, by: Rect) -> Rect {
    intersect((x, y, w, h), by).unwrap_or((x, y, 0.0, 0.0))
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

/// A file system path: elided in the middle by design, so the gate exempts it.
fn looks_like_path(text: &str) -> bool {
    let text = text.trim_start_matches(['…', '.']);
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with('~')
        || text.starts_with("\\\\")
        || (bytes.len() > 2 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
}

struct TextNode {
    text: String,
    wrap: bool,
    elide: bool,
    halign: TextHorizontalAlignment,
    valign: TextVerticalAlignment,
    preferred: (f32, f32),
}

fn text_node(item: &ItemRc) -> Option<TextNode> {
    let adapter = item.window_adapter()?;
    if let Some(text) = item.downcast::<SimpleText>() {
        let text = text.as_pin_ref();
        return Some(TextNode {
            text: text.text().to_string(),
            wrap: false,
            elide: false,
            halign: text.horizontal_alignment(),
            valign: text.vertical_alignment(),
            preferred: (
                text.layout_info(Orientation::Horizontal, -1.0, &adapter, item)
                    .preferred,
                text.layout_info(Orientation::Vertical, -1.0, &adapter, item)
                    .preferred,
            ),
        });
    }
    let text = item.downcast::<ComplexText>()?;
    let text = text.as_pin_ref();
    Some(TextNode {
        text: text.text().to_string(),
        wrap: !matches!(text.wrap(), TextWrap::NoWrap),
        elide: matches!(text.overflow(), TextOverflow::Elide),
        halign: text.horizontal_alignment(),
        valign: text.vertical_alignment(),
        preferred: (
            text.layout_info(Orientation::Horizontal, -1.0, &adapter, item)
                .preferred,
            text.layout_info(Orientation::Vertical, -1.0, &adapter, item)
                .preferred,
        ),
    })
}

/// The glyph area of a text inside its element (the gate's `ink`).
fn ink((x, y, w, h): Rect, text: &TextNode) -> Rect {
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

#[derive(Clone, Copy)]
struct Walk {
    hidden: bool,
    /// Every clipping ancestor: what the user can see of an item.
    clip: Option<Rect>,
    /// The clipping containers below the nearest scroll viewport: what cuts an item
    /// without scrolling.
    fixed: Option<Rect>,
    /// Inside an interactive control (a caption, a combo box's field).
    in_control: bool,
}

fn shown(rect: Rect, walk: &Walk) -> Option<Rect> {
    if walk.hidden || rect.2 <= 0.0 || rect.3 <= 0.0 {
        return None;
    }
    match walk.clip {
        Some(clip) => intersect(rect, clip),
        None => Some(rect),
    }
}

/// What a walk of the item tree found. One-line texts clipped by their element are
/// judged once the whole tree is seen: a label shortened by design draws a separate
/// "…" or "..." text at its cut edge (as path labels do), which the gate accepts.
#[derive(Default)]
struct Found {
    findings: Vec<String>,
    /// A clipped one-line text: its element, its text and the width it needs.
    clipped: Vec<(Rect, String, f32)>,
    /// The shown "…" and "..." texts.
    markers: Vec<Rect>,
}

impl Found {
    /// The gate's `ellipsis_after`: a shown marker starts at the clipped text's right
    /// edge (within 4 px), on the same line.
    fn marked(&self, (x, y, w, h): Rect) -> bool {
        let edge = x + w;
        self.markers.iter().any(|marker| {
            (marker.0 - edge).abs() <= 4.0 && marker.1 < y + h && marker.1 + marker.3 > y
        })
    }
}

fn audit(item: &ItemRc, walk: Walk, found: &mut Found) {
    let rect = bounds(item);
    let mut hidden = walk.hidden;
    if item.downcast::<Clip>().is_some_and(|clip| {
        let clip = clip.as_pin_ref();
        clip.is_visibility_clip() && clip.clip()
    }) || item
        .downcast::<Opacity>()
        .is_some_and(|layer| layer.as_pin_ref().opacity() < 0.01)
    {
        hidden = true;
    }
    let here = Walk { hidden, ..walk };
    let visible = shown(rect, &here);
    let role = item.accessible_role();
    let has_text = |text: &TextNode| text.text.chars().any(char::is_alphanumeric);
    // U-K03: text or a control that reaches outside a container that does not scroll.
    if !hidden
        && rect.2 > 0.5
        && rect.3 > 0.5
        && let Some(fixed) = walk.fixed
    {
        let area = match text_node(item) {
            Some(text) if has_text(&text) => Some((ink(rect, &text), text.text)),
            _ if interactive(role) && !walk.in_control => Some((rect, label(item))),
            _ => None,
        };
        if let Some(((x, y, w, h), name)) = area {
            let outside = (fixed.0 - x).max(0.0)
                + (x + w - fixed.0 - fixed.2).max(0.0)
                + (fixed.1 - y).max(0.0)
                + (y + h - fixed.1 - fixed.3).max(0.0);
            if outside > 1.5 {
                found.findings.push(format!(
                    "cut off: {role:?} {name:?} at {x:.0},{y:.0} {w:.0}x{h:.0} reaches {outside:.0}px outside {fixed:?}"
                ));
            }
        }
    }
    // U-K04 and U-K05: a shown text that needs more room than its element has.
    if visible.is_some()
        && let Some(text) = text_node(item)
        && matches!(text.text.trim(), "…" | "...")
    {
        found.markers.push(rect);
    }
    if visible.is_some()
        && let Some(text) = text_node(item)
        && has_text(&text)
    {
        let (_, _, w, h) = rect;
        if !text.wrap && !text.elide && text.preferred.0 > w + 1.0 {
            found
                .clipped
                .push((rect, text.text.clone(), text.preferred.0));
        }
        if text.wrap && text.preferred.1 > h + 1.0 {
            found.findings.push(format!(
                "wrapped text cut: {:?} needs {:.0}px of height, has {h:.0}px",
                text.text, text.preferred.1
            ));
        }
    }
    // U-K20: a button whose caption needs more than the shown part of the button.
    if role == AccessibleRole::Button
        && let Some(button) = visible
    {
        let mut captions = Vec::new();
        collect_captions(item, here, &mut captions);
        for (text, width) in &captions {
            if looks_like_path(&text.text) || (text.elide && captions.len() > 1) || text.wrap {
                continue;
            }
            let available = width.min(button.2);
            if text.preferred.0 > available + 1.0 {
                found.findings.push(format!(
                    "caption cut: {:?} needs {:.0}px, has {available:.0}px of button {:?} at {button:?}",
                    text.text,
                    text.preferred.0,
                    label(item)
                ));
            }
        }
    }
    let clips = item.borrow().as_ref().clips_children();
    let flickable = item.downcast::<Flickable>().is_some();
    let child = Walk {
        hidden,
        clip: if clips {
            Some(walk.clip.map_or(rect, |clip| cut(clip, rect)))
        } else {
            walk.clip
        },
        fixed: if flickable {
            None
        } else if clips {
            Some(walk.fixed.map_or(rect, |fixed| cut(fixed, rect)))
        } else {
            walk.fixed
        },
        in_control: walk.in_control || interactive(role),
    };
    let mut next = item.first_child();
    while let Some(current) = next {
        next = current.next_sibling();
        audit(&current, child, found);
    }
}

/// The shown, non-empty texts inside a button, with their element widths.
fn collect_captions(item: &ItemRc, walk: Walk, found: &mut Vec<(TextNode, f32)>) {
    let mut next = item.first_child();
    while let Some(current) = next {
        next = current.next_sibling();
        let rect = bounds(&current);
        let hidden = walk.hidden
            || current.downcast::<Clip>().is_some_and(|clip| {
                let clip = clip.as_pin_ref();
                clip.is_visibility_clip() && clip.clip()
            });
        let here = Walk { hidden, ..walk };
        if shown(rect, &here).is_some()
            && let Some(text) = text_node(&current)
            && !text.text.trim().is_empty()
        {
            found.push((text, rect.2));
        }
        collect_captions(&current, here, found);
    }
}

/// Every finding of the layout checks in the window's item tree.
pub fn layout_findings(window: &slint::Window) -> Vec<String> {
    item_findings(&ItemRc::new_root(WindowInner::from_pub(window).component()))
}

/// Every finding of the layout checks below `root`, such as an embedded dialog's popup.
pub fn item_findings(root: &ItemRc) -> Vec<String> {
    let mut found = Found::default();
    audit(
        root,
        Walk {
            hidden: false,
            clip: None,
            fixed: None,
            in_control: false,
        },
        &mut found,
    );
    for (rect, text, needed) in &found.clipped {
        if !found.marked(*rect) {
            found.findings.push(format!(
                "clipped without ellipsis: {text:?} needs {needed:.0}px, has {:.0}px",
                rect.2
            ));
        }
    }
    found.findings
}

/// The control that has the keyboard focus: the focus item or its nearest ancestor
/// with an accessible role.
pub fn focused_control(window: &slint::Window) -> Option<ItemRc> {
    let mut item = WindowInner::from_pub(window).focus_item.borrow().upgrade();
    while let Some(current) = item {
        if current.accessible_role() != AccessibleRole::None {
            return Some(current);
        }
        item = current.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

/// The focus item itself.
pub fn focus_item(window: &slint::Window) -> Option<ItemRc> {
    WindowInner::from_pub(window).focus_item.borrow().upgrade()
}

fn luminance(pixel: slint::Rgb8Pixel) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(pixel.r) + 0.7152 * linear(pixel.g) + 0.0722 * linear(pixel.b)
}

/// WCAG contrast of two colours.
pub fn contrast(first: slint::Rgb8Pixel, second: slint::Rgb8Pixel) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// The pixel indices of `area` grown by `margin`, inside a `width`×`height` frame.
fn pixels_of((x, y, w, h): Rect, margin: f32, width: usize, height: usize) -> Vec<usize> {
    let left = (x - margin).floor().max(0.0) as usize;
    let top = (y - margin).floor().max(0.0) as usize;
    let right = ((x + w + margin).ceil().max(0.0) as usize).min(width);
    let bottom = ((y + h + margin).ceil().max(0.0) as usize).min(height);
    (top..bottom)
        .flat_map(|row| (left..right).map(move |column| row * width + column))
        .collect()
}

/// How a focus change shows on `area` (grown by 4 px, as the gate measures it) between
/// two frames of a `width`×`height` window: None when no pixel changed, else the best
/// contrast of a changed pixel against its earlier colour.
pub fn focus_contrast(
    before: &[slint::Rgb8Pixel],
    after: &[slint::Rgb8Pixel],
    width: usize,
    height: usize,
    area: Rect,
) -> Option<f64> {
    pixels_of(area, 4.0, width, height)
        .into_iter()
        .filter(|&index| before[index] != after[index])
        .map(|index| contrast(before[index], after[index]))
        .reduce(f64::max)
}
