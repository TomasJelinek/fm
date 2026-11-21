//! The UI polish gate's automatic checks of one rendered window
//! (docs/developer/acceptance/ui-polish.md), on top of `inspect::check_layout`: framed
//! windows without their own close button, overflow that neither shows a scroll bar nor
//! scrolls, content cut off by a clipping container, text clipped by its own element,
//! accessible names, tooltips of icon-only buttons, overlapping text, text outside the
//! window, consistent button rows, a blank surface, and text contrast measured in the
//! rendered pixels (WCAG AA). Every finding becomes a review flag on the screenshot; the
//! gate scenarios fail on any flag that is not listed as an exemption with its reason
//! ([`EXEMPTIONS`]). `ui_red` proves that each check can fail.
use crate::{
    Context,
    app::{self, App},
    inspect::{self, Rect},
    report,
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ItemTreeRc},
    items::{
        AccessibleRole, BasicBorderRectangle, BorderRectangle, Clip, ComplexText, Flickable, Item,
        Opacity, Rectangle, SimpleText, TextHorizontalAlignment, TextInput, TextOverflow,
        TextVerticalAlignment, TextWrap, TooltipArea,
    },
    layout::Orientation,
    window::{WindowInner, WindowKind},
};
use slint::{
    Brush, Color, LogicalPosition, Rgb8Pixel,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{collections::HashMap, rc::Rc, time::Duration};

pub const FRAMED_CLOSE: &str = "internal close button in a framed window";
pub const OVERFLOW: &str = "overflow without scroll bar";
pub const CUT_OFF: &str = "content cut off without scrolling";
pub const TEXT_CLIPPED: &str = "text clipped without ellipsis";
pub const WRAP_CUT: &str = "wrapped text cut off";
pub const ELIDED_HIDDEN: &str = "elided text without full text";
pub const NO_NAME: &str = "control without accessible name";
pub const NO_TOOLTIP: &str = "icon-only button without tooltip";
pub const TEXT_OVERLAP: &str = "overlapping text";
pub const TEXT_OUTSIDE: &str = "text outside the window";
pub const BUTTON_HEIGHTS: &str = "uneven button heights";
pub const BUTTON_SPACING: &str = "uneven button spacing";
pub const DISMISS_ORDER: &str = "dismiss button not last";
pub const CONTRAST: &str = "low text contrast";
pub const BLANK: &str = "surface shows no text";
pub const OVERLAPPING: &str = "overlapping controls";
pub const BUTTON_TEXT: &str = "text does not fit its button";

/// One finding of a check.
#[derive(Clone, Debug)]
pub struct Issue {
    pub kind: &'static str,
    pub detail: String,
}

impl Issue {
    pub fn new(kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// A finding the gate accepts, with the reason recorded in the coverage record: the
/// surface (a substring of the capture's surface name, empty for any), the check, a
/// substring of the finding's detail and the reason.
pub struct Exemption {
    pub surface: &'static str,
    pub kind: &'static str,
    pub detail: &'static str,
    pub reason: &'static str,
}

/// Exemptions of the gate. Keep this list and the coverage record
/// (docs/developer/acceptance/ui-polish.md, "Exemptions") in step.
pub const EXEMPTIONS: &[Exemption] = &[];

/// Checks of announced behavior that other changes are implementing now: their flags
/// make the scenario PENDING instead of FAIL while nothing else is flagged. The
/// surface substring, the check (empty for every check) and the reason.
pub const PENDING: &[(&str, &str, &str)] = &[(
    "decision/",
    FRAMED_CLOSE,
    "the decision windows' own close button is being removed",
)];

/// Where a capture's window lives.
#[derive(Clone)]
pub enum Target {
    Main,
    Tool(Rc<MinimalSoftwareWindow>),
}

impl Target {
    pub fn window<'a>(&'a self, app: &'a App) -> &'a slint::Window {
        match self {
            Self::Main => app.slint(),
            Self::Tool(window) => window.window(),
        }
    }

    pub fn software(&self, app: &App) -> Rc<MinimalSoftwareWindow> {
        match self {
            Self::Main => app.main.clone(),
            Self::Tool(window) => window.clone(),
        }
    }

    /// A native window with the operating system's frame: a tool window whose Window
    /// element does not set `no-frame`.
    pub fn framed(&self) -> bool {
        match self {
            Self::Main => false,
            Self::Tool(window) => WindowInner::from_pub(window.window())
                .window_item()
                .is_some_and(|item| !item.as_pin_ref().no_frame()),
        }
    }
}

/// A software-rendered frame of a window, in physical pixels.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub scale: f32,
    pub pixels: Vec<Rgb8Pixel>,
}

impl Frame {
    pub fn of(window: &MinimalSoftwareWindow) -> Self {
        // A window that was never sized has nothing to render (and the renderer would
        // divide by its zero width).
        let size = window.window().size();
        if size.width == 0 || size.height == 0 {
            return Self {
                width: 0,
                height: 0,
                scale: window.window().scale_factor(),
                pixels: Vec::new(),
            };
        }
        let (width, height, pixels) = app::render(window);
        Self {
            width,
            height,
            scale: window.window().scale_factor(),
            pixels,
        }
    }

    pub fn at(&self, x: usize, y: usize) -> [u8; 3] {
        let pixel = self.pixels[y * self.width + x];
        [pixel.r, pixel.g, pixel.b]
    }

    /// The physical pixel bounds (x0, y0, x1, y1) of a logical rectangle, clamped.
    pub fn bounds(&self, (x, y, w, h): Rect) -> Option<(usize, usize, usize, usize)> {
        let scale = self.scale;
        let clamp = |value: f32, limit: usize| (value.max(0.0) as usize).min(limit);
        let (x0, y0) = (
            clamp((x * scale).round(), self.width),
            clamp((y * scale).round(), self.height),
        );
        let (x1, y1) = (
            clamp(((x + w) * scale).round(), self.width),
            clamp(((y + h) * scale).round(), self.height),
        );
        (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
    }

    /// The logical size of the window this frame shows.
    pub fn logical(&self) -> (f32, f32) {
        (
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        )
    }

    /// Pixels that differ from `other` inside `area` (same size frames only).
    pub fn changed(&self, other: &Frame, area: Rect) -> Vec<([u8; 3], [u8; 3])> {
        let mut changed = Vec::new();
        if self.width != other.width || self.height != other.height {
            return changed;
        }
        if let Some((x0, y0, x1, y1)) = self.bounds(area) {
            for y in y0..y1 {
                for x in x0..x1 {
                    let (before, after) = (other.at(x, y), self.at(x, y));
                    if before != after {
                        changed.push((before, after));
                    }
                }
            }
        }
        changed
    }

    /// A digest of `area`, skipping `except` (both logical).
    pub fn digest(&self, area: Rect, except: &[Rect]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let skipped: Vec<(usize, usize, usize, usize)> = except
            .iter()
            .filter_map(|rect| self.bounds(*rect))
            .collect();
        if let Some((x0, y0, x1, y1)) = self.bounds(area) {
            for y in y0..y1 {
                for x in x0..x1 {
                    if skipped
                        .iter()
                        .any(|(a, b, c, d)| x >= *a && x < *c && y >= *b && y < *d)
                    {
                        continue;
                    }
                    self.at(x, y).hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }
}

// ---------------------------------------------------------------------------------------
// Colours (WCAG 2.2 relative luminance and contrast ratio)

fn channel(value: u8) -> f64 {
    let value = f64::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub fn luminance([r, g, b]: [u8; 3]) -> f64 {
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter().zip(b).map(|(x, y)| u32::from(x.abs_diff(y))).sum()
}

fn rgb(color: Color) -> [u8; 3] {
    [color.red(), color.green(), color.blue()]
}

fn blend(top: [u8; 3], alpha: f32, under: [u8; 3]) -> [u8; 3] {
    let alpha = alpha.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) * alpha + f32::from(b) * (1.0 - alpha)).round() as u8;
    [
        mix(top[0], under[0]),
        mix(top[1], under[1]),
        mix(top[2], under[2]),
    ]
}

pub fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

// ---------------------------------------------------------------------------------------
// The item tree as nodes

fn cut(a: Rect, b: Rect) -> Rect {
    let (left, top) = (a.0.max(b.0), a.1.max(b.1));
    let (right, bottom) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

fn overlap(a: Rect, b: Rect) -> Option<Rect> {
    let both = cut(a, b);
    (both.2 > 0.0 && both.3 > 0.0).then_some(both)
}

fn contains_point((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x - 0.5 && py >= y - 0.5 && px <= x + w + 0.5 && py <= y + h + 0.5
}

fn middle((x, y, w, h): Rect) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

fn has_alnum(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

fn short(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() > 48 {
        format!("{}…", flat.chars().take(47).collect::<String>())
    } else {
        flat
    }
}

fn property(item: &ItemRc, which: AccessibleStringProperty) -> Option<String> {
    item.accessible_string_property(which)
        .map(|value| value.to_string())
}

pub fn interactive(role: AccessibleRole) -> bool {
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

#[derive(Clone)]
pub struct TextNode {
    pub text: String,
    pub font_size: f32,
    pub bold: bool,
    pub color: Color,
    pub gradient: bool,
    halign: TextHorizontalAlignment,
    valign: TextVerticalAlignment,
    pub wrap: bool,
    pub elide: bool,
    pub input: bool,
    /// A text field's selection colours (foreground, background) while it has one.
    pub selection: Option<(Color, Color)>,
    /// The single-line width and the height at the element's own width.
    pub preferred: (f32, f32),
}

#[derive(Clone)]
pub enum Kind {
    Other,
    Flickable {
        content: (f32, f32),
    },
    Tooltip,
    /// A filled rectangle: its background colour (alpha included).
    Filled(Color),
}

#[derive(Clone)]
pub struct Node {
    pub item: ItemRc,
    /// The index after this node's last descendant.
    pub end: usize,
    pub parent: Option<usize>,
    pub rect: Rect,
    /// The part inside every clipping ancestor; None when hidden or clipped away.
    pub shown: Option<Rect>,
    pub hidden: bool,
    /// Under an opaque element drawn later or under a popup (see [`occlude`]).
    pub occluded: bool,
    pub opacity: f32,
    pub disabled: bool,
    pub role: AccessibleRole,
    pub label: String,
    pub description: String,
    /// The control is in its active state: a checked, checkable control (an active
    /// page button) or a selected item (an active tab).
    pub active: bool,
    /// The control can be checked (a toggle or one segment of a segmented control).
    pub checkable: bool,
    pub text: Option<TextNode>,
    /// The nearest clipping ancestor that is not a Flickable, with no Flickable between.
    pub clip_owner: Option<usize>,
    /// The intersection of the clipping containers' own rectangles between the nearest
    /// Flickable (or the root) and this item: what cuts it without scrolling.
    pub fixed_clip: Option<Rect>,
    pub flick_owner: Option<usize>,
    pub kind: Kind,
}

#[derive(Clone, Copy)]
struct Walk {
    fixed: Option<Rect>,
    hidden: bool,
    opacity: f32,
    clip: Option<Rect>,
    clip_owner: Option<usize>,
    flick_owner: Option<usize>,
    disabled: bool,
}

fn default_font(window: &slint::Window) -> f32 {
    WindowInner::from_pub(window)
        .window_item()
        .map(|item| item.as_pin_ref().default_font_size().get())
        .filter(|size| *size > 0.0)
        .unwrap_or(12.0)
}

fn layout(item: &ItemRc, orientation: Orientation) -> Option<f32> {
    let adapter = item.window_adapter()?;
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(
            text.as_pin_ref()
                .layout_info(orientation, -1.0, &adapter, item)
                .preferred,
        );
    }
    if let Some(text) = item.downcast::<ComplexText>() {
        return Some(
            text.as_pin_ref()
                .layout_info(orientation, -1.0, &adapter, item)
                .preferred,
        );
    }
    if let Some(text) = item.downcast::<TextInput>() {
        return Some(
            text.as_pin_ref()
                .layout_info(orientation, -1.0, &adapter, item)
                .preferred,
        );
    }
    None
}

fn brush_color(brush: &Brush) -> (Color, bool) {
    match brush {
        Brush::SolidColor(color) => (*color, false),
        other => (other.color(), true),
    }
}

fn text_node(item: &ItemRc, default_size: f32) -> Option<TextNode> {
    let size = |value: f32| if value > 0.0 { value } else { default_size };
    let (text, font_size, weight, brush, halign, valign, wrap, elide, input, selection) =
        if let Some(text) = item.downcast::<SimpleText>() {
            let text = text.as_pin_ref();
            (
                text.text().to_string(),
                size(text.font_size().get()),
                text.font_weight(),
                text.color(),
                text.horizontal_alignment(),
                text.vertical_alignment(),
                false,
                false,
                false,
                None,
            )
        } else if let Some(text) = item.downcast::<ComplexText>() {
            let text = text.as_pin_ref();
            (
                text.text().to_string(),
                size(text.font_size().get()),
                text.font_weight(),
                text.color(),
                text.horizontal_alignment(),
                text.vertical_alignment(),
                !matches!(text.wrap(), TextWrap::NoWrap),
                matches!(text.overflow(), TextOverflow::Elide),
                false,
                None,
            )
        } else {
            let text = item.downcast::<TextInput>()?;
            let text = text.as_pin_ref();
            let selected = text.cursor_position_byte_offset() != text.anchor_position_byte_offset()
                && !text.text().is_empty();
            (
                text.text().to_string(),
                size(text.font_size().get()),
                text.font_weight(),
                text.color(),
                text.horizontal_alignment(),
                text.vertical_alignment(),
                !matches!(text.wrap(), TextWrap::NoWrap),
                false,
                true,
                selected.then(|| {
                    (
                        text.selection_foreground_color(),
                        text.selection_background_color(),
                    )
                }),
            )
        };
    let (color, gradient) = brush_color(&brush);
    let preferred = (
        layout(item, Orientation::Horizontal).unwrap_or(0.0),
        layout(item, Orientation::Vertical).unwrap_or(0.0),
    );
    Some(TextNode {
        text,
        font_size,
        bold: weight >= 700,
        color,
        gradient,
        halign,
        valign,
        wrap,
        elide,
        input,
        selection,
        preferred,
    })
}

fn kind_of(item: &ItemRc) -> Kind {
    if let Some(flick) = item.downcast::<Flickable>() {
        let flick = flick.as_pin_ref();
        return Kind::Flickable {
            content: (flick.content_width().get(), flick.content_height().get()),
        };
    }
    if item.downcast::<TooltipArea>().is_some() {
        return Kind::Tooltip;
    }
    let brush = if let Some(rectangle) = item.downcast::<Rectangle>() {
        Some(rectangle.as_pin_ref().background())
    } else if let Some(rectangle) = item.downcast::<BasicBorderRectangle>() {
        Some(rectangle.as_pin_ref().background())
    } else {
        item.downcast::<BorderRectangle>()
            .map(|rectangle| rectangle.as_pin_ref().background())
    };
    match brush {
        Some(brush) if !brush.is_transparent() => Kind::Filled(brush_color(&brush).0),
        _ => Kind::Other,
    }
}

fn collect(
    item: &ItemRc,
    parent: Option<usize>,
    walk: Walk,
    default_size: f32,
    nodes: &mut Vec<Node>,
) {
    let index = nodes.len();
    let rect = inspect::rect(item);
    let mut hidden = walk.hidden;
    let mut opacity = walk.opacity;
    if let Some(clip) = item.downcast::<Clip>() {
        let clip = clip.as_pin_ref();
        if clip.is_visibility_clip() && clip.clip() {
            hidden = true;
        }
    }
    if let Some(layer) = item.downcast::<Opacity>() {
        opacity *= layer.as_pin_ref().opacity();
        if opacity < 0.01 {
            hidden = true;
        }
    }
    let shown = if hidden || rect.2 <= 0.0 || rect.3 <= 0.0 {
        None
    } else {
        match walk.clip {
            Some(clip) => overlap(rect, clip),
            None => Some(rect),
        }
    };
    let role = item.accessible_role();
    let disabled = walk.disabled
        || property(item, AccessibleStringProperty::Enabled).as_deref() == Some("false")
        || item
            .downcast::<TextInput>()
            .is_some_and(|input| !input.as_pin_ref().enabled());
    nodes.push(Node {
        item: item.clone(),
        end: index + 1,
        parent,
        rect,
        shown,
        hidden,
        occluded: false,
        opacity,
        disabled,
        role,
        label: inspect::label(item).unwrap_or_default(),
        description: property(item, AccessibleStringProperty::Description).unwrap_or_default(),
        checkable: property(item, AccessibleStringProperty::Checkable).as_deref() == Some("true"),
        active: (property(item, AccessibleStringProperty::Checkable).as_deref() == Some("true")
            && property(item, AccessibleStringProperty::Checked).as_deref() == Some("true"))
            || property(item, AccessibleStringProperty::ItemSelected).as_deref() == Some("true"),
        text: text_node(item, default_size),
        clip_owner: walk.clip_owner,
        fixed_clip: walk.fixed,
        flick_owner: walk.flick_owner,
        kind: kind_of(item),
    });
    let clips = item.borrow().as_ref().clips_children();
    let flickable = item.downcast::<Flickable>().is_some();
    let child_walk = Walk {
        hidden,
        opacity,
        clip: if clips {
            Some(match walk.clip {
                Some(clip) => cut(clip, rect),
                None => rect,
            })
        } else {
            walk.clip
        },
        fixed: if flickable {
            None
        } else if clips {
            Some(match walk.fixed {
                Some(fixed) => cut(fixed, rect),
                None => rect,
            })
        } else {
            walk.fixed
        },
        clip_owner: if clips && !flickable {
            Some(index)
        } else if flickable {
            None
        } else {
            walk.clip_owner
        },
        flick_owner: if flickable {
            Some(index)
        } else {
            walk.flick_owner
        },
        disabled,
    };
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        collect(&current, Some(index), child_walk, default_size, nodes);
    }
    nodes[index].end = nodes.len();
}

/// Every item below `root` as nodes, depth first.
pub fn nodes(window: &slint::Window, root: &ItemRc) -> Vec<Node> {
    let mut nodes = Vec::new();
    collect(
        root,
        None,
        Walk {
            hidden: false,
            opacity: 1.0,
            clip: None,
            clip_owner: None,
            fixed: None,
            flick_owner: None,
            disabled: false,
        },
        default_font(window),
        &mut nodes,
    );
    nodes
}

/// Hide what the user cannot see: a node at least 90 % under an opaque fill drawn after
/// it (a later sibling's subtree, higher in z-order), or under a popup opened over its
/// root (`covers`). Occluded nodes are not shown to any check.
pub fn occlude(nodes: &mut [Node], covers: &[Rect]) {
    let fills: Vec<(usize, Rect)> = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| match node.kind {
            Kind::Filled(color) if color.alpha() >= 250 && !node.hidden => {
                node.shown.map(|rect| (index, rect))
            }
            _ => None,
        })
        .collect();
    let mut hidden = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some(shown) = node.shown else { continue };
        let area = match &node.text {
            Some(text) => overlap(ink(node, text), shown).unwrap_or(shown),
            None => shown,
        };
        let (cx, cy) = middle(area);
        let under_popup = covers.iter().any(|cover| contains_point(*cover, cx, cy));
        let under_fill = fills.iter().any(|(fill, rect)| {
            *fill >= node.end
                && overlap(*rect, area).is_some_and(|both| both.2 * both.3 >= 0.9 * area.2 * area.3)
        });
        if under_popup || under_fill {
            hidden.push(index);
        }
    }
    for index in hidden {
        nodes[index].shown = None;
        nodes[index].occluded = true;
    }
}

/// The roots the gate checks: the window's own tree, or only its open popups (dialogs,
/// menus) while any is open, as `inspect::check_layout` does; with `tooltips`, open
/// tooltips too.
pub fn checked_roots(window: &slint::Window, tooltips: bool) -> Vec<ItemRc> {
    let inner = WindowInner::from_pub(window);
    let popups: Vec<ItemRc> = inner
        .active_popups()
        .iter()
        .filter(|popup| tooltips || !matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| ItemRc::new_root(popup.component.clone()))
        .collect();
    let dialogs = inner
        .active_popups()
        .iter()
        .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip));
    if dialogs {
        popups
    } else {
        // A tool window that closed itself has no item tree any more.
        let mut roots: Vec<ItemRc> = inner
            .try_component()
            .map(ItemRc::new_root)
            .into_iter()
            .collect();
        roots.extend(popups);
        roots
    }
}

/// The text's glyph area inside its element: its single-line or wrapped extent placed
/// by its alignment.
pub fn ink(node: &Node, text: &TextNode) -> Rect {
    let (x, y, w, h) = node.rect;
    let width = if text.wrap || text.input {
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

pub fn describe(node: &Node) -> String {
    let (x, y, w, h) = node.rect;
    let name = if node.label.is_empty() {
        node.text
            .as_ref()
            .map(|text| short(&text.text))
            .unwrap_or_default()
    } else {
        short(&node.label)
    };
    format!("{:?} {name:?} at {x:.0},{y:.0} {w:.0}x{h:.0}", node.role)
}

/// The visible texts with letters or digits below `nodes[index]` (itself included).
fn subtree_texts(nodes: &[Node], index: usize) -> Vec<String> {
    nodes[index..nodes[index].end]
        .iter()
        .filter(|node| node.shown.is_some())
        .filter_map(|node| node.text.as_ref())
        .filter(|text| has_alnum(&text.text))
        .map(|text| text.text.clone())
        .collect()
}

/// Whether a visible text with letters or digits is drawn on the control `index`: inside
/// it, or a text whose glyph area's centre lies within its visible rectangle (a caption
/// drawn by a sibling element).
pub fn shows_text(nodes: &[Node], index: usize) -> bool {
    if !subtree_texts(nodes, index).is_empty() {
        return true;
    }
    let Some(area) = nodes[index].shown else {
        return false;
    };
    nodes.iter().any(|node| {
        let Some(text) = &node.text else {
            return false;
        };
        if node.shown.is_none() || !has_alnum(&text.text) {
            return false;
        }
        let (cx, cy) = middle(ink(node, text));
        contains_point(area, cx, cy)
    })
}

fn is_ancestor(nodes: &[Node], ancestor: usize, index: usize) -> bool {
    index > ancestor && index < nodes[ancestor].end
}

/// Whether a tooltip area covers the point: a TooltipArea covers its parent element.
pub fn tooltip_at(nodes: &[Node], px: f32, py: f32) -> bool {
    nodes.iter().any(|node| {
        if !matches!(node.kind, Kind::Tooltip) || node.hidden {
            return false;
        }
        let area = if node.rect.2 > 0.0 && node.rect.3 > 0.0 {
            node.rect
        } else {
            node.parent.map_or(node.rect, |parent| nodes[parent].rect)
        };
        contains_point(area, px, py)
    })
}

/// Whether a visible ellipsis text ("…" or "...") starts at the right edge of the text
/// `nodes[index]` cuts, on the same line.
fn ellipsis_after(nodes: &[Node], index: usize) -> bool {
    let (x, y, w, h) = nodes[index].rect;
    let edge = x + w;
    nodes.iter().enumerate().any(|(other, node)| {
        other != index
            && node.shown.is_some()
            && node
                .text
                .as_ref()
                .is_some_and(|text| matches!(text.text.trim(), "…" | "..."))
            && (node.rect.0 - edge).abs() <= 4.0
            && node.rect.1 < y + h
            && node.rect.1 + node.rect.3 > y
    })
}

/// Whether the full text is available to assistive technology or as a tooltip: the text
/// element's own label, an accessible ancestor's label or description, or a tooltip.
fn full_text_available(nodes: &[Node], index: usize, text: &str) -> bool {
    let node = &nodes[index];
    if node.role != AccessibleRole::None && node.label.contains(text.trim()) {
        return true;
    }
    let mut current = node.parent;
    while let Some(parent) = current {
        let ancestor = &nodes[parent];
        if ancestor.role != AccessibleRole::None
            && (ancestor.label.contains(text.trim()) || ancestor.description.contains(text.trim()))
        {
            return true;
        }
        current = ancestor.parent;
    }
    let (px, py) = middle(node.shown.unwrap_or(node.rect));
    tooltip_at(nodes, px, py)
}

// ---------------------------------------------------------------------------------------
// The static checks of one root

pub struct Analysis<'a> {
    pub nodes: &'a [Node],
    /// Overflowing viewports that scroll with the mouse wheel (they need no bar).
    pub scrolling: &'a [ItemRc],
    /// The window's logical size.
    pub size: (f32, f32),
    pub framed: bool,
    /// A dialog's root (a popup, or a tool window's own tree): button order applies.
    pub dialog: bool,
}

fn framed_close(analysis: &Analysis<'_>, issues: &mut Vec<Issue>) {
    if !analysis.framed {
        return;
    }
    let (width, height) = analysis.size;
    let band = (height * 0.25).clamp(64.0, 120.0);
    for (index, node) in analysis.nodes.iter().enumerate() {
        if node.role != AccessibleRole::Button {
            continue;
        }
        let Some(shown) = node.shown else { continue };
        let label = node.label.to_lowercase();
        let closing = label == "close"
            || label.starts_with("close ")
            || label == "cancel dialog"
            || label == "decide later"
            || label == "dismiss";
        let icon_only = !shows_text(analysis.nodes, index);
        if closing && icon_only && shown.1 < band && shown.0 + shown.2 > width * 0.6 {
            issues.push(Issue::new(
                FRAMED_CLOSE,
                format!(
                    "{} closes a window that has the system's own close button",
                    describe(node)
                ),
            ));
        }
    }
}

/// A scroll bar thumb beside `flick` (not inside it) along its right (vertical) or bottom
/// edge: a filled element shorter than the edge.
fn thumb(nodes: &[Node], flick: usize, vertical: bool) -> bool {
    let (fx, fy, fw, fh) = nodes[flick].rect;
    nodes.iter().enumerate().any(|(index, node)| {
        if is_ancestor(nodes, flick, index) || index == flick {
            return false;
        }
        let Kind::Filled(color) = node.kind else {
            return false;
        };
        if color.alpha() < 16 {
            return false;
        }
        let Some((x, y, w, h)) = node.shown else {
            return false;
        };
        if vertical {
            x >= fx + fw - 18.0
                && x + w <= fx + fw + 2.0
                && w <= 16.0
                && y >= fy - 1.0
                && y + h <= fy + fh + 1.0
                && h >= 6.0
                && h < fh - 1.0
        } else {
            y >= fy + fh - 18.0
                && y + h <= fy + fh + 2.0
                && h <= 16.0
                && x >= fx - 1.0
                && x + w <= fx + fw + 1.0
                && w >= 6.0
                && w < fw - 1.0
        }
    })
}

/// Overflowing scroll viewports: (node index, vertical) of each axis whose content does
/// not fit and that shows no scroll bar; viewports in `scrolling` are left out.
pub fn overflowing(nodes: &[Node], scrolling: &[ItemRc]) -> Vec<(usize, bool)> {
    let mut found = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let Kind::Flickable { content, .. } = node.kind else {
            continue;
        };
        if scrolling.contains(&node.item) {
            continue;
        }
        let Some(shown) = node.shown else { continue };
        if shown.2 < 8.0 || shown.3 < 8.0 {
            continue;
        }
        let (_, _, w, h) = node.rect;
        // Only content a user could reach matters: something visible must be inside.
        let has_content = nodes[index + 1..node.end]
            .iter()
            .any(|child| child.text.is_some() || interactive(child.role));
        if !has_content {
            continue;
        }
        for (vertical, needed, available) in [(true, content.1, h), (false, content.0, w)] {
            if needed > available + 1.5 && !thumb(nodes, index, vertical) {
                found.push((index, vertical));
            }
        }
    }
    found
}

/// Text or controls cut by a clipping container that does not scroll: (node index, how
/// far outside it reaches, the container, whether the cut is vertical).
pub fn cut_candidates(nodes: &[Node]) -> Vec<(usize, f32, Rect, bool)> {
    let mut found = Vec::new();
    let mut owners = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some(owner) = node.clip_owner else {
            continue;
        };
        if node.hidden || node.occluded {
            continue;
        }
        if node.rect.2 <= 0.5 || node.rect.3 <= 0.5 {
            continue;
        }
        let area = match &node.text {
            Some(text) if has_alnum(&text.text) && !text.input => ink(node, text),
            _ if interactive(node.role) => node.rect,
            _ => continue,
        };
        // A control inside an interactive control is part of it (the field of a combo
        // box); judge the outer control.
        if interactive(node.role)
            && node
                .parent
                .is_some_and(|parent| ancestor_control(nodes, parent).is_some())
        {
            continue;
        }
        // The containers' own bounds, not what an outer scroll view shows of them: a
        // container scrolled out of view cuts nothing.
        let Some(clip) = node.fixed_clip else {
            continue;
        };
        if scrolls_with_neighbour(nodes, clip, area) {
            continue;
        }
        let (x, y, w, h) = area;
        let horizontal = (clip.0 - x).max(0.0) + (x + w - clip.0 - clip.2).max(0.0);
        let vertical = (clip.1 - y).max(0.0) + (y + h - clip.1 - clip.3).max(0.0);
        if horizontal + vertical > 1.5 && !owners.contains(&owner) {
            owners.push(owner);
            found.push((index, horizontal + vertical, clip, vertical >= horizontal));
        }
    }
    found
}

fn cut_off(nodes: &[Node], scrolling: &[ItemRc], issues: &mut Vec<Issue>) {
    for (index, outside, clip, _) in cut_candidates(nodes) {
        if scrolling.contains(&nodes[index].item) {
            continue;
        }
        issues.push(Issue::new(
            CUT_OFF,
            format!(
                "{} reaches {outside:.0}px outside its clipping container at {:.0},{:.0} {:.0}x{:.0}, which does not scroll",
                describe(&nodes[index]),
                clip.0,
                clip.1,
                clip.2,
                clip.3
            ),
        ));
    }
}

/// A header kept in step with a scroll view: a viewport with the clip's horizontal
/// extent (its vertical bar's width aside) whose content reaches as far as `area`, or
/// one with its vertical extent for a vertical cut.
fn scrolls_with_neighbour(nodes: &[Node], clip: Rect, area: Rect) -> bool {
    nodes.iter().any(|node| {
        let Kind::Flickable { content } = node.kind else {
            return false;
        };
        let (fx, fy, fw, fh) = node.rect;
        let horizontal = (fx - clip.0).abs() <= 4.0
            && (fx + fw - clip.0 - clip.2).abs() <= 18.0
            && content.0 > fw + 1.0
            && content.0 + 18.0 >= area.0 + area.2 - clip.0;
        let vertical = (fy - clip.1).abs() <= 4.0
            && (fy + fh - clip.1 - clip.3).abs() <= 18.0
            && content.1 > fh + 1.0
            && content.1 + 18.0 >= area.1 + area.3 - clip.1;
        horizontal || vertical
    })
}

/// The nearest interactive ancestor-or-self of `index`.
fn ancestor_control(nodes: &[Node], index: usize) -> Option<usize> {
    let mut current = Some(index);
    while let Some(at) = current {
        if interactive(nodes[at].role) {
            return Some(at);
        }
        current = nodes[at].parent;
    }
    None
}

fn text_fit(nodes: &[Node], issues: &mut Vec<Issue>) {
    for (index, node) in nodes.iter().enumerate() {
        let Some(text) = &node.text else { continue };
        if text.input || node.shown.is_none() || !has_alnum(&text.text) {
            continue;
        }
        let (_, _, w, h) = node.rect;
        if w <= 0.5 || h <= 0.5 {
            continue;
        }
        let horizontal = !text.wrap && text.preferred.0 > w + 1.0;
        let vertical = text.wrap && text.preferred.1 > h + 1.0;
        if !horizontal && !vertical {
            continue;
        }
        // A label shortened by design: the clipped text is followed by a separate "…"
        // or "..." text at its cut edge (as path labels draw it).
        let marked = horizontal && ellipsis_after(nodes, index);
        if text.elide || marked {
            if !full_text_available(nodes, index, &text.text) {
                issues.push(Issue::new(
                    ELIDED_HIDDEN,
                    format!(
                        "{} is elided ({:.0}x{:.0} needed) and neither a tooltip nor an accessible label holds the full text",
                        describe(node),
                        text.preferred.0,
                        text.preferred.1
                    ),
                ));
            }
        } else if horizontal {
            issues.push(Issue::new(
                TEXT_CLIPPED,
                format!(
                    "{} needs {:.0}px, its element is {w:.0}px wide and does not elide",
                    describe(node),
                    text.preferred.0
                ),
            ));
        } else {
            issues.push(Issue::new(
                WRAP_CUT,
                format!(
                    "{} needs {:.0}px of height for its lines, its element has {h:.0}px",
                    describe(node),
                    text.preferred.1
                ),
            ));
        }
    }
}

/// Two shown interactive controls overlap by more than 2 px both ways, neither inside
/// the other (`inspect::check_layout`'s check, with occlusion: a control hidden under
/// an opaque element drawn later is not shown).
fn controls_overlap(nodes: &[Node], issues: &mut Vec<Issue>) {
    let controls: Vec<usize> = (0..nodes.len())
        .filter(|&index| interactive(nodes[index].role) && nodes[index].shown.is_some())
        .collect();
    let inside = |outer: Rect, inner: Rect| {
        inner.0 >= outer.0 - 0.5
            && inner.1 >= outer.1 - 0.5
            && inner.0 + inner.2 <= outer.0 + outer.2 + 0.5
            && inner.1 + inner.3 <= outer.1 + outer.3 + 0.5
    };
    for (position, first) in controls.iter().enumerate() {
        for second in &controls[position + 1..] {
            let (a, b) = (
                nodes[*first].shown.unwrap_or_default(),
                nodes[*second].shown.unwrap_or_default(),
            );
            let Some((_, _, w, h)) = overlap(a, b) else {
                continue;
            };
            if w <= 2.0
                || h <= 2.0
                || inside(a, b)
                || inside(b, a)
                || is_ancestor(nodes, *first, *second)
            {
                continue;
            }
            issues.push(Issue::new(
                OVERLAPPING,
                format!(
                    "{} and {} overlap by {w:.0}x{h:.0}",
                    describe(&nodes[*first]),
                    describe(&nodes[*second])
                ),
            ));
        }
    }
}

/// A file system path: elided in the middle by design (`inspect::check_layout`'s rule).
/// Path bar segments are wrapped in bidirectional isolates (U+2066 … U+2069), which are
/// not part of the path.
fn looks_like_path(text: &str) -> bool {
    let text = text.trim_start_matches([
        '…', '.', '\u{2066}', '\u{2067}', '\u{2068}', '\u{200e}', '\u{200f}',
    ]);
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with('~')
        || text.starts_with("\\\\")
        || (bytes.len() > 2 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/'))
}

/// A button's caption needs more width than it has (`inspect::check_layout`'s check):
/// paths are exempt, and so are the elided columns of a row that shows several texts
/// (their full text is checked by the elision check).
fn button_text(nodes: &[Node], issues: &mut Vec<Issue>) {
    for (index, node) in nodes.iter().enumerate() {
        if node.role != AccessibleRole::Button {
            continue;
        }
        let Some(shown) = node.shown else { continue };
        let texts: Vec<usize> = (index..node.end)
            .filter(|&child| {
                nodes[child].shown.is_some()
                    && nodes[child]
                        .text
                        .as_ref()
                        .is_some_and(|text| !text.text.trim().is_empty() && !text.input)
            })
            .collect();
        for child in &texts {
            let Some(text) = &nodes[*child].text else {
                continue;
            };
            if looks_like_path(&text.text) || (text.elide && texts.len() > 1) {
                continue;
            }
            let available = nodes[*child].rect.2.min(shown.2);
            // A caption whose own box holds it, cut at the edge of a view that scrolls
            // sideways or of a header kept in step with one (a table's last column
            // heading), is scrolled out of view, not squeezed (the elision check's rule
            // for labels, the cut-off check's for headers).
            let scrolled = nodes[*child].rect.2 + 1.0 >= text.preferred.0
                && (inspect::in_sideways_scroll(&nodes[*child].item)
                    || node.flick_owner.is_some_and(|owner| {
                        scrolls_with_neighbour(nodes, nodes[owner].rect, node.rect)
                    }));
            if text.preferred.0 > available + 1.0 && !text.wrap && !scrolled {
                issues.push(Issue::new(
                    BUTTON_TEXT,
                    format!(
                        "{:?} needs {:.0}px, has {available:.0}px ({})",
                        short(&text.text),
                        text.preferred.0,
                        describe(node)
                    ),
                ));
            }
        }
    }
}

fn names_and_tooltips(nodes: &[Node], issues: &mut Vec<Issue>) {
    for (index, node) in nodes.iter().enumerate() {
        if !interactive(node.role) || node.shown.is_none() {
            continue;
        }
        if node.label.trim().is_empty() {
            let nearby = subtree_texts(nodes, index);
            issues.push(Issue::new(
                NO_NAME,
                format!(
                    "{} has no accessible label (texts inside: {nearby:?})",
                    describe(node)
                ),
            ));
        }
        // A row of a scrolled list cut to a sliver at the viewport's edge shows none of
        // its texts, but it is not an icon-only button: scrolled in, it shows them.
        let mostly_hidden = node
            .shown
            .is_some_and(|shown| shown.2 < node.rect.2 * 0.5 || shown.3 < node.rect.3 * 0.5);
        if node.role == AccessibleRole::Button && !mostly_hidden && !shows_text(nodes, index) {
            let (px, py) = middle(node.shown.unwrap_or(node.rect));
            if !tooltip_at(nodes, px, py) {
                issues.push(Issue::new(
                    NO_TOOLTIP,
                    format!("{} shows no text and has no tooltip", describe(node)),
                ));
            }
        }
    }
}

fn text_overlap(nodes: &[Node], size: (f32, f32), issues: &mut Vec<Issue>) {
    let window = (0.0, 0.0, size.0, size.1);
    let mut boxes: Vec<(usize, Rect)> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some(text) = &node.text else { continue };
        let Some(shown) = node.shown else { continue };
        if !has_alnum(&text.text) || node.rect.2 <= 0.5 {
            continue;
        }
        let Some(glyphs) = overlap(ink(node, text), shown) else {
            continue;
        };
        let (x, y, w, h) = glyphs;
        let outside = (-x).max(0.0)
            + (-y).max(0.0)
            + (x + w - window.2).max(0.0)
            + (y + h - window.3).max(0.0);
        if outside > 1.5 {
            issues.push(Issue::new(
                TEXT_OUTSIDE,
                format!(
                    "{} reaches {outside:.0}px outside the {:.0}x{:.0} window",
                    describe(node),
                    size.0,
                    size.1
                ),
            ));
        }
        boxes.push((index, glyphs));
    }
    for (position, (first, a)) in boxes.iter().enumerate() {
        for (second, b) in &boxes[position + 1..] {
            let Some((_, _, w, h)) = overlap(*a, *b) else {
                continue;
            };
            if w <= 2.0 || h <= 2.0 {
                continue;
            }
            let (one, two) = (&nodes[*first], &nodes[*second]);
            let same = one.text.as_ref().map(|t| &t.text) == two.text.as_ref().map(|t| &t.text)
                && (one.rect.0 - two.rect.0).abs() < 0.5
                && (one.rect.1 - two.rect.1).abs() < 0.5;
            if same || is_ancestor(nodes, *first, *second) {
                continue;
            }
            issues.push(Issue::new(
                TEXT_OVERLAP,
                format!(
                    "{} and {} overlap by {w:.0}x{h:.0}",
                    describe(one),
                    describe(two)
                ),
            ));
        }
    }
}

fn is_dismiss(label: &str) -> bool {
    matches!(label.trim(), "Cancel" | "Close")
}

fn button_rows(analysis: &Analysis<'_>, issues: &mut Vec<Issue>) {
    let nodes = analysis.nodes;
    // Rows of dialog buttons only: the main window's toolbars and path bars are not
    // button rows, and path segment buttons are laid out as a path.
    if !analysis.dialog {
        return;
    }
    // Buttons that show text, as (label, shown rect).
    let buttons: Vec<(String, Rect)> = nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            node.role == AccessibleRole::Button
                && node
                    .shown
                    .is_some_and(|(_, _, w, h)| w >= 12.0 && h >= 12.0)
                && node.shown == Some(node.rect)
                && !subtree_texts(nodes, *index).is_empty()
                && !node
                    .label
                    .strip_prefix("Open ")
                    .is_some_and(|prefix| std::path::Path::new(prefix).is_absolute())
                && !node.checkable
                && !subtree_texts(nodes, *index)
                    .iter()
                    .any(|text| text.contains('/') || text.contains('\\'))
        })
        .map(|(_, node)| (node.label.clone(), node.rect))
        .collect();
    let mut rows: Vec<Vec<(String, Rect)>> = Vec::new();
    for button in buttons {
        let (_, cy) = middle(button.1);
        match rows.iter_mut().find(|row| {
            let (_, ry) = middle(row[0].1);
            (ry - cy).abs() <= 2.0
        }) {
            Some(row) => row.push(button),
            None => rows.push(vec![button]),
        }
    }
    let footer = rows
        .iter()
        .filter(|row| row.len() >= 2)
        .map(|row| middle(row[0].1).1)
        .fold(f32::MIN, f32::max);
    for row in &mut rows {
        if row.len() < 2 {
            continue;
        }
        row.sort_by(|a, b| a.1.0.total_cmp(&b.1.0));
        let heights: Vec<f32> = row.iter().map(|(_, rect)| rect.3).collect();
        let (low, high) = heights
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(*h), hi.max(*h)));
        if high - low > 1.5 {
            issues.push(Issue::new(
                BUTTON_HEIGHTS,
                format!(
                    "buttons in one row are {low:.0}–{high:.0}px high: {:?}",
                    row.iter()
                        .map(|(label, rect)| format!("{label} {:.0}px", rect.3))
                        .collect::<Vec<_>>()
                ),
            ));
        }
        // Gaps within clusters (gaps of at most 24px); wider gaps separate groups.
        let gaps: Vec<(f32, &str, &str)> = row
            .windows(2)
            .map(|pair| {
                (
                    pair[1].1.0 - (pair[0].1.0 + pair[0].1.2),
                    pair[0].0.as_str(),
                    pair[1].0.as_str(),
                )
            })
            .filter(|(gap, _, _)| *gap >= -0.5 && *gap <= 24.0)
            .collect();
        if let (Some(lowest), Some(highest)) = (
            gaps.iter().map(|g| g.0).reduce(f32::min),
            gaps.iter().map(|g| g.0).reduce(f32::max),
        ) && highest - lowest > 1.5
        {
            issues.push(Issue::new(
                BUTTON_SPACING,
                format!(
                    "gaps between neighbouring buttons differ ({lowest:.0}–{highest:.0}px): {:?}",
                    gaps.iter()
                        .map(|(gap, a, b)| format!("{a}|{b} {gap:.0}px"))
                        .collect::<Vec<_>>()
                ),
            ));
        }
        if analysis.dialog && (middle(row[0].1).1 - footer).abs() < 0.5 {
            let last = row.len() - 1;
            if let Some(position) = row.iter().position(|(label, _)| is_dismiss(label))
                && position != last
            {
                issues.push(Issue::new(
                    DISMISS_ORDER,
                    format!(
                        "{:?} is followed by {:?} in the footer row",
                        row[position].0,
                        row[position + 1..]
                            .iter()
                            .map(|(label, _)| label.as_str())
                            .collect::<Vec<_>>()
                    ),
                ));
            }
        }
    }
}

/// The static checks of one root's nodes.
pub fn analyse(analysis: &Analysis<'_>) -> Vec<Issue> {
    let mut issues = Vec::new();
    let nodes = analysis.nodes;
    framed_close(analysis, &mut issues);
    for (index, vertical) in overflowing(nodes, analysis.scrolling) {
        let node = &nodes[index];
        let Kind::Flickable { content, .. } = node.kind else {
            continue;
        };
        issues.push(Issue::new(
            OVERFLOW,
            format!(
                "{} {} content of {:.0}px in {:.0}px shows no scroll bar",
                describe(node),
                if vertical { "vertical" } else { "horizontal" },
                if vertical { content.1 } else { content.0 },
                if vertical { node.rect.3 } else { node.rect.2 }
            ),
        ));
    }
    cut_off(nodes, analysis.scrolling, &mut issues);
    text_fit(nodes, &mut issues);
    names_and_tooltips(nodes, &mut issues);
    controls_overlap(nodes, &mut issues);
    button_text(nodes, &mut issues);
    text_overlap(nodes, analysis.size, &mut issues);
    button_rows(analysis, &mut issues);
    if !nodes.iter().any(|node| {
        node.shown.is_some() && node.text.as_ref().is_some_and(|text| has_alnum(&text.text))
    }) {
        issues.push(Issue::new(BLANK, "no visible text with letters or digits"));
    }
    issues
}

/// WCAG AA contrast of every visible text against the rendered background(s) under its
/// glyphs: 4.5:1, or 3:1 for large text (24px, or 18.67px bold) and for symbols.
/// Text of disabled controls is exempt (WCAG 1.4.3).
pub fn contrast_issues(frame: &Frame, nodes: &[Node]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for node in nodes {
        let Some(text) = &node.text else { continue };
        if node.disabled || text.gradient || text.text.trim().is_empty() || node.rect.2 <= 0.5 {
            continue;
        }
        let Some(shown) = node.shown else { continue };
        let inked = ink(node, text);
        let Some(area) = overlap(inked, shown) else {
            continue;
        };
        // Text scrolled to a sliver at a viewport's edge shows no readable glyphs, and
        // what lies under the sliver is the viewport's frame, not the text's background.
        if area.3 < inked.3 * 0.5 {
            continue;
        }
        if let Some(issue) = text_contrast(frame, node, text, area) {
            issues.push(issue);
        }
    }
    issues
}

fn text_contrast(frame: &Frame, node: &Node, text: &TextNode, area: Rect) -> Option<Issue> {
    let (x0, y0, x1, y1) = frame.bounds(area)?;
    let total = (x1 - x0) * (y1 - y0);
    if total < 12 {
        return None;
    }
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for y in y0..y1 {
        for x in x0..x1 {
            *counts.entry(frame.at(x, y)).or_default() += 1;
        }
    }
    let declared = rgb(text.color);
    let alpha = f32::from(text.color.alpha()) / 255.0 * node.opacity;
    if alpha < 0.05 {
        return None;
    }
    let mut colours: Vec<([u8; 3], usize)> = counts.into_iter().collect();
    colours.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let near_text = |colour: [u8; 3]| {
        distance(colour, declared) <= 30
            || text
                .selection
                .is_some_and(|(foreground, _)| distance(colour, rgb(foreground)) <= 30)
    };
    let mut backgrounds: Vec<[u8; 3]> = colours
        .iter()
        .filter(|(colour, count)| *count * 100 >= total * 15 && *count >= 6 && !near_text(*colour))
        .map(|(colour, _)| *colour)
        .collect();
    if backgrounds.is_empty() {
        let fallback = colours
            .iter()
            .find(|(colour, count)| *count * 100 >= total * 5 && !near_text(*colour))?;
        backgrounds.push(fallback.0);
    }
    // Something must be drawn over the main background, or the text is not shown here.
    let main = backgrounds[0];
    let drawn = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| distance(frame.at(*x, *y), main) > 60)
        .take(3)
        .count();
    if drawn < 3 {
        return None;
    }
    let large = text.font_size >= 24.0 || (text.bold && text.font_size >= 18.66);
    let needed = if large || !has_alnum(&text.text) {
        3.0
    } else {
        4.5
    };
    for background in backgrounds {
        let foreground = match text.selection {
            Some((foreground, selected)) if distance(background, rgb(selected)) <= 24 => {
                rgb(foreground)
            }
            _ => declared,
        };
        let shown = blend(foreground, alpha, background);
        let ratio = contrast(shown, background);
        if ratio + 0.005 < needed {
            return Some(Issue::new(
                CONTRAST,
                format!(
                    "{} is {} on {} ({ratio:.2}:1, needs {needed}:1{})",
                    describe(node),
                    hex(shown),
                    hex(background),
                    if text.selection.is_some() {
                        "; selected text"
                    } else {
                        ""
                    }
                ),
            ));
        }
    }
    None
}

// ---------------------------------------------------------------------------------------
// Capturing a window and recording its flags

/// Analyse the window as rendered in `frame`: `inspect::check_layout`, the static checks
/// of every checked root and the contrast of its texts. Viewports in `scrolling` were
/// seen to scroll with the wheel.
pub fn inspect_window(
    window: &slint::Window,
    frame: &Frame,
    framed: bool,
    scrolling: &[ItemRc],
) -> Vec<Issue> {
    let size = frame.logical();
    // The gate has its own occlusion-aware versions of these two checks.
    let mut issues: Vec<Issue> = inspect::check_layout(window, size.0, size.1)
        .into_iter()
        .filter(|finding| finding.kind != OVERLAPPING && finding.kind != BUTTON_TEXT)
        .map(|finding| Issue::new(finding.kind, finding.detail))
        .collect();
    let dialogs = WindowInner::from_pub(window)
        .active_popups()
        .iter()
        .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip));
    let roots = checked_roots(window, false);
    let rects: Vec<Rect> = roots.iter().map(inspect::rect).collect();
    for (position, root) in roots.iter().enumerate() {
        let mut nodes = nodes(window, root);
        occlude(&mut nodes, &rects[position + 1..]);
        let analysis = Analysis {
            nodes: &nodes,
            scrolling,
            size,
            framed: framed && position == 0 && !dialogs,
            dialog: framed || dialogs,
        };
        issues.extend(analyse(&analysis));
        issues.extend(contrast_issues(frame, &nodes));
    }
    issues
}

fn exemption(surface: &str, issue: &Issue) -> Option<&'static Exemption> {
    EXEMPTIONS.iter().find(|exemption| {
        surface.contains(exemption.surface)
            && exemption.kind == issue.kind
            && issue.detail.contains(exemption.detail)
    })
}

fn pending(surface: &str, issue: &Issue) -> Option<&'static str> {
    PENDING
        .iter()
        .find(|(where_, kind, _)| {
            surface.contains(where_) && (kind.is_empty() || *kind == issue.kind)
        })
        .map(|(_, _, reason)| *reason)
}

/// Record issues against a screenshot: exempt ones as notes (with their reason), the rest
/// as review flags (pending ones marked). Returns the number of flags.
pub fn record(context: &mut Context, shot: &str, surface: &str, issues: Vec<Issue>) -> usize {
    let mut flagged = 0;
    let mut seen: Vec<(&'static str, String)> = Vec::new();
    for issue in issues {
        if seen
            .iter()
            .any(|(kind, detail)| *kind == issue.kind && *detail == issue.detail)
        {
            continue;
        }
        seen.push((issue.kind, issue.detail.clone()));
        if let Some(exempt) = exemption(surface, &issue) {
            context.note(format!(
                "exempt ({}): {surface}: {}: {}",
                exempt.reason, issue.kind, issue.detail
            ));
            continue;
        }
        let detail = match pending(surface, &issue) {
            Some(reason) => format!("[pending: {reason}] {surface}: {}", issue.detail),
            None => format!("{surface}: {}", issue.detail),
        };
        context.outcome.flags.push(report::Flag {
            shot: shot.to_owned(),
            kind: issue.kind,
            detail,
        });
        flagged += 1;
    }
    if let Some(recorded) = context
        .outcome
        .shots
        .iter_mut()
        .find(|recorded| recorded.file == shot)
    {
        recorded.flags += flagged;
    }
    flagged
}

/// Save a frame as the scenario's screenshot `name`; returns its relative path.
pub fn save(context: &mut Context, frame: &Frame, name: &str, caption: &str) -> String {
    let relative = format!("shots/{}/{name}.png", app::context_name());
    let path = context.run_dir.join(&relative);
    if let Err(error) = report::save_png(&path, frame.width, frame.height, &frame.pixels) {
        context.note(format!("screenshot {name} failed: {error}"));
    } else if !context
        .outcome
        .shots
        .iter()
        .any(|shot| shot.file == relative)
    {
        context.outcome.shots.push(report::Shot {
            file: relative.clone(),
            caption: caption.to_owned(),
            flags: 0,
        });
    }
    relative
}

fn offset(item: &ItemRc) -> (f32, f32) {
    item.downcast::<Flickable>().map_or((0.0, 0.0), |flick| {
        let flick = flick.as_pin_ref();
        (flick.content_x().get(), flick.content_y().get())
    })
}

/// Whether the viewport `item` scrolls when the wheel turns over `at`; it is scrolled
/// back afterwards.
pub fn scrolls(
    app: &mut App,
    window: &slint::Window,
    item: &ItemRc,
    vertical: bool,
    at: (f32, f32),
) -> bool {
    let wheel = |amount: f32| {
        window.dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(at.0, at.1),
            delta_x: if vertical { 0.0 } else { amount },
            delta_y: if vertical { amount } else { 0.0 },
        });
    };
    let before = offset(item);
    let mut moved = false;
    for amount in [-60.0_f32, 60.0] {
        wheel(amount);
        app.idle(Duration::from_millis(150));
        if offset(item) != before {
            wheel(-amount);
            app.idle(Duration::from_millis(150));
            moved = true;
            break;
        }
    }
    pointer_out(window);
    app.settle();
    moved
}

/// Whether `item` moves when the wheel turns over `at`; it is scrolled back afterwards.
fn moves(
    app: &mut App,
    window: &slint::Window,
    item: &ItemRc,
    vertical: bool,
    at: (f32, f32),
) -> bool {
    let wheel = |amount: f32| {
        window.dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(at.0, at.1),
            delta_x: if vertical { 0.0 } else { amount },
            delta_y: if vertical { amount } else { 0.0 },
        });
    };
    let before = inspect::rect(item);
    let mut moved = false;
    for amount in [-60.0_f32, 60.0] {
        wheel(amount);
        app.idle(Duration::from_millis(150));
        if inspect::rect(item) != before {
            wheel(-amount);
            app.idle(Duration::from_millis(150));
            moved = true;
            break;
        }
    }
    pointer_out(window);
    app.settle();
    moved
}

/// What the wheel scrolls although no scroll bar shows it: overflowing viewports without
/// a scroll bar, and content cut by a container that moves under the wheel.
pub fn scrolling_viewports(app: &mut App, software: &MinimalSoftwareWindow) -> Vec<ItemRc> {
    let window = software.window();
    let mut found = Vec::new();
    for root in checked_roots(window, false) {
        let nodes = nodes(window, &root);
        for (index, vertical) in overflowing(&nodes, &found) {
            let node = &nodes[index];
            let Some(shown) = node.shown else { continue };
            if scrolls(app, window, &node.item, vertical, middle(shown)) {
                found.push(node.item.clone());
            }
        }
        for (index, _, clip, vertical) in cut_candidates(&nodes) {
            if clip.2 < 4.0 || clip.3 < 4.0 {
                continue;
            }
            if moves(app, window, &nodes[index].item, vertical, middle(clip)) {
                found.push(nodes[index].item.clone());
            }
        }
    }
    found
}

/// Every check on the window as it is now: the wheel probe of overflowing viewports,
/// then one frame for the static and the contrast checks.
pub fn settled_issues(app: &mut App, target: &Target) -> Vec<Issue> {
    let software = target.software(app);
    inspect::settle_tree(software.window());
    let scrolling = scrolling_viewports(app, &software);
    let frame = Frame::of(&software);
    inspect_window(software.window(), &frame, target.framed(), &scrolling)
}

/// Settle, render, save and check one window; flags are recorded against the
/// screenshot under `surface` (the combination and the surface's name).
pub fn capture(
    app: &mut App,
    context: &mut Context,
    target: &Target,
    surface: &str,
    name: &str,
    caption: &str,
) -> usize {
    // Theme colours and hover states animate briefly; let them settle first.
    app.idle(Duration::from_millis(250));
    let software = target.software(app);
    inspect::settle_tree(software.window());
    let scrolling = scrolling_viewports(app, &software);
    let frame = Frame::of(&software);
    let shot = save(context, &frame, name, caption);
    let issues = inspect_window(software.window(), &frame, target.framed(), &scrolling);
    record(context, &shot, surface, issues)
}

/// The gate's verdict over the scenario's flags: PASS without flags, PENDING when every
/// flag belongs to a pending check, FAIL otherwise, with the counts per check.
pub fn verdict(context: &Context, what: &str) -> Result<(), String> {
    let flags = &context.outcome.flags;
    if flags.is_empty() {
        return Ok(());
    }
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for flag in flags {
        match counts.iter_mut().find(|(kind, _)| *kind == flag.kind) {
            Some(entry) => entry.1 += 1,
            None => counts.push((flag.kind, 1)),
        }
    }
    let summary = counts
        .iter()
        .map(|(kind, count)| format!("{count} × {kind}"))
        .collect::<Vec<_>>()
        .join(", ");
    let pending = flags
        .iter()
        .filter(|flag| flag.detail.starts_with("[pending:"))
        .count();
    let first: Vec<String> = flags
        .iter()
        .filter(|flag| !flag.detail.starts_with("[pending:"))
        .take(10)
        .map(|flag| format!("{}: {}", flag.kind, flag.detail))
        .collect();
    if pending == flags.len() {
        Err(format!(
            "PENDING: {what}: {pending} flag(s) of checks whose product change is in progress ({summary})"
        ))
    } else {
        Err(format!(
            "{what}: {} polish flag(s) in {} screenshot(s) ({summary}); first:\n{}",
            flags.len() - pending,
            context.outcome.shots.len(),
            first.join("\n")
        ))
    }
}

// ---------------------------------------------------------------------------------------
// Window size and scale

/// Size the main window to `logical` at `scale` (1 or 2). The App's frame buffer is
/// physical, so it is resized to the physical size first.
pub fn scale_main(app: &mut App, logical: (usize, usize), scale: f32) {
    let physical = (
        (logical.0 as f32 * scale).round() as usize,
        (logical.1 as f32 * scale).round() as usize,
    );
    app.resize(physical.0, physical.1);
    if (scale - 1.0).abs() > f32::EPSILON {
        let window = app.slint();
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        window.dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(logical.0 as f32, logical.1 as f32),
        });
    }
    app.settle();
}

/// Size a tool window to `logical` at `scale` and settle its layout.
pub fn size_tool(window: &MinimalSoftwareWindow, logical: (f32, f32), scale: f32) {
    window.set_size(slint::PhysicalSize::new(
        (logical.0 * scale).round() as u32,
        (logical.1 * scale).round() as u32,
    ));
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(logical.0, logical.1),
    });
    WindowInner::from_pub(window.window()).ensure_tree_instantiated();
}

/// A window's minimum and preferred logical sizes, from its root layout.
pub fn window_sizes(window: &slint::Window) -> ((f32, f32), (f32, f32)) {
    // A tool window that closed itself has no layout any more: one pixel each way.
    let Some(component) = WindowInner::from_pub(window).try_component() else {
        return ((1.0, 1.0), (1.0, 1.0));
    };
    let component = ItemTreeRc::borrow_pin(&component);
    let horizontal = component.as_ref().layout_info(Orientation::Horizontal);
    let vertical = component.as_ref().layout_info(Orientation::Vertical);
    (
        (horizontal.min.max(1.0), vertical.min.max(1.0)),
        (
            horizontal.preferred_bounded().max(1.0),
            vertical.preferred_bounded().max(1.0),
        ),
    )
}

/// Move the pointer out of every window (no hover state is left behind).
pub fn pointer_out(window: &slint::Window) {
    window.dispatch_event(WindowEvent::PointerExited);
}

/// Move the pointer over a point of the window.
pub fn hover(window: &slint::Window, x: f32, y: f32) {
    window.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
}
