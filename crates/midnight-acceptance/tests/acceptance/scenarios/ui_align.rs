//! The UI polish gate's alignment checks (U-K32–U-K38, docs/developer/acceptance/
//! ui-polish.md) on the gate's nodes: controls side by side share their height and
//! centre, captions sit in the middle of their controls (symbol captions judged by their
//! rendered glyphs), columns share their edges and their gaps, controls keep apart from
//! each other and from their dialog's edge, and labels are not shortened beside free
//! room. The checks themselves are plain geometry shared with the `midnight-ui` layout
//! tests, which also hold their crafted good and bad layouts
//! (`crates/midnight-ui/tests/support/alignment.rs`).
use super::ui_gate::{self as gate, Frame, Issue, Node};
use crate::inspect::Rect;
use i_slint_core::items::{AccessibleRole, TextInput};

#[path = "../../../../midnight-ui/tests/support/alignment.rs"]
pub mod alignment;

pub use alignment::{
    CAPTION_CENTRE, COMPANION_SIZE, EDGE_ALIGNMENT, ELIDED_WITH_ROOM, OPTION_ROWS, ROW_ALIGNMENT,
    ROW_GAPS, TOUCHING,
};

fn role(role: AccessibleRole) -> Option<alignment::Role> {
    use alignment::Role;
    Some(match role {
        AccessibleRole::Button => Role::Button,
        AccessibleRole::TextInput => Role::Field,
        AccessibleRole::Combobox => Role::Picker,
        AccessibleRole::Checkbox | AccessibleRole::RadioButton | AccessibleRole::Switch => {
            Role::Toggle
        }
        AccessibleRole::Tab => Role::Tab,
        AccessibleRole::Spinbox => Role::Spin,
        AccessibleRole::Slider => Role::Slider,
        _ => return None,
    })
}

fn alphanumeric(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// Whether `index` lies below an interactive control (a caption, a field's text).
fn inside_control(nodes: &[Node], index: usize) -> bool {
    let mut current = nodes[index].parent;
    while let Some(parent) = current {
        if gate::interactive(nodes[parent].role) {
            return true;
        }
        current = nodes[parent].parent;
    }
    false
}

/// Whether `index` lies in a list or menu row (an ancestor is a list, list item, table
/// or tree).
fn in_list(nodes: &[Node], index: usize) -> bool {
    let mut current = nodes[index].parent;
    while let Some(parent) = current {
        if matches!(
            nodes[parent].role,
            AccessibleRole::List
                | AccessibleRole::ListItem
                | AccessibleRole::Table
                | AccessibleRole::Tree
        ) {
            return true;
        }
        current = nodes[parent].parent;
    }
    false
}

/// How many shown texts with letters or digits `index` holds.
fn text_count(nodes: &[Node], index: usize) -> usize {
    nodes[index..nodes[index].end]
        .iter()
        .filter(|node| node.shown.is_some())
        .filter(|node| {
            node.text
                .as_ref()
                .is_some_and(|text| !text.input && alphanumeric(&text.text))
        })
        .count()
}

/// A control's caption: its first shown one-line text with letters or digits, else its
/// first shown symbol; and whether it shows no letters or digits at all.
fn caption(
    nodes: &[Node],
    index: usize,
    frame: Option<&Frame>,
) -> (Option<alignment::Caption>, bool) {
    let texts: Vec<&Node> = nodes[index..nodes[index].end]
        .iter()
        .filter(|node| node.shown.is_some())
        .filter(|node| {
            node.text
                .as_ref()
                .is_some_and(|text| !text.input && !text.wrap && !text.text.trim().is_empty())
        })
        .collect();
    let has_text = |node: &Node| {
        node.text
            .as_ref()
            .is_some_and(|text| alphanumeric(&text.text))
    };
    let icon_only = !texts.iter().any(|node| has_text(node));
    let Some(chosen) = texts
        .iter()
        .find(|node| has_text(node))
        .or_else(|| texts.first())
    else {
        return (None, icon_only);
    };
    let Some(text) = &chosen.text else {
        return (None, icon_only);
    };
    let symbol = !alphanumeric(&text.text);
    let glyphs = match frame {
        Some(frame) if symbol && frame.width > 0 => {
            let (x, y, w, h) = nodes[index].rect;
            alignment::drawn_box(
                &|x, y| frame.at(x, y),
                (frame.width, frame.height),
                frame.scale,
                (x + 2.0, y + 2.0, w - 4.0, h - 4.0),
            )
        }
        _ => None,
    };
    (
        Some(alignment::Caption {
            line: gate::ink(chosen, text),
            needed: text.preferred,
            font_size: text.font_size,
            symbol,
            elide: text.elide,
            left_aligned: text.left_aligned(),
            glyphs,
        }),
        icon_only,
    )
}

/// A text field that shows more than one line.
fn multi_line_field(nodes: &[Node], index: usize) -> bool {
    nodes[index..nodes[index].end].iter().any(|node| {
        node.item
            .downcast::<TextInput>()
            .is_some_and(|input| !input.as_pin_ref().single_line())
    })
}

/// The gate's nodes as the alignment checks see them: every shown control outside
/// another control, and every shown text with letters or digits outside a control,
/// marked whole or cut by its viewport.
pub fn elements(nodes: &[Node], frame: Option<&Frame>) -> Vec<alignment::Element> {
    let mut elements = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        // A control under a fully transparent layer is an input overlay (a check box's
        // native control over its drawn box): it takes the place of its nearest shown
        // ancestor's part.
        let overlay = || {
            if node.opacity >= 0.01 || node.occluded || role(node.role).is_none() {
                return None;
            }
            let mut current = node.parent;
            while let Some(parent) = current {
                if let Some(shown) = nodes[parent].shown {
                    let whole = shown == nodes[parent].rect
                        && node.rect.0 >= shown.0 - 0.5
                        && node.rect.1 >= shown.1 - 0.5
                        && node.rect.0 + node.rect.2 <= shown.0 + shown.2 + 0.5
                        && node.rect.1 + node.rect.3 <= shown.1 + shown.3 + 0.5;
                    return whole.then_some(node.rect);
                }
                current = nodes[parent].parent;
            }
            None
        };
        let Some(shown) = node.shown.or_else(overlay) else {
            continue;
        };
        if inside_control(nodes, index) {
            continue;
        }
        let whole = !node.occluded
            && (shown.0 - node.rect.0).abs() < 0.5
            && (shown.1 - node.rect.1).abs() < 0.5
            && (shown.2 - node.rect.2).abs() < 0.5
            && (shown.3 - node.rect.3).abs() < 0.5;
        let container = node.parent.map(|parent| nodes[parent].rect);
        let scope = node.flick_owner.map_or(0, |owner| owner + 1);
        if let Some(kind) = role(node.role) {
            let (caption, icon_only) = caption(nodes, index, frame);
            let viewport = node.flick_owner.map(|owner| nodes[owner].rect);
            let row_of_list = viewport.is_some_and(|viewport| node.rect.2 >= 0.9 * viewport.2);
            let multiline = match kind {
                alignment::Role::Field => multi_line_field(nodes, index),
                _ => caption
                    .as_ref()
                    .is_some_and(|caption| node.rect.3 > 2.5 * caption.needed.1.max(1.0)),
            };
            elements.push(alignment::Element {
                name: gate::describe(node),
                role: kind,
                rect: node.rect,
                whole,
                multiline,
                segment: kind == alignment::Role::Tab || node.checkable || row_of_list,
                scope,
                caption,
                icon_only,
                container,
                list_row: row_of_list || in_list(nodes, index),
                texts: text_count(nodes, index),
            });
        } else if let Some(text) = &node.text
            && !text.input
            && alphanumeric(&text.text)
        {
            // A text its viewport cuts is not judged (`check` judges whole elements only),
            // but it still makes its row taller: a wrapped description running past a
            // page's edge exempts its check box's gap (U-K39), as a whole one does.
            elements.push(alignment::Element {
                name: gate::describe(node),
                role: alignment::Role::Label,
                rect: node.rect,
                whole,
                multiline: text.wrap && node.rect.3 > 1.5 * text.preferred.1.max(1.0),
                segment: false,
                scope,
                caption: Some(alignment::Caption {
                    line: gate::ink(node, text),
                    needed: text.preferred,
                    font_size: text.font_size,
                    symbol: false,
                    elide: text.elide,
                    left_aligned: text.left_aligned(),
                    glyphs: None,
                }),
                icon_only: false,
                container,
                list_row: in_list(nodes, index),
                texts: 1,
            });
        }
    }
    elements
}

/// The alignment issues of one checked root: `dialog` roots (a popup or a tool window's
/// own tree) also keep their controls off their edges.
pub fn issues(nodes: &[Node], dialog: bool, frame: Option<&Frame>) -> Vec<Issue> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let root: Option<Rect> = dialog
        .then_some(nodes[0].rect)
        .filter(|rect| rect.2 > 0.0 && rect.3 > 0.0);
    alignment::check(&elements(nodes, frame), root)
        .into_iter()
        .map(|finding| Issue::new(finding.kind, finding.detail))
        .collect()
}
