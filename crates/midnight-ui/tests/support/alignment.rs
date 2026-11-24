//! The UI polish gate's alignment checks (U-K32–U-K38 in
//! docs/developer/acceptance/ui-polish.md) on plain geometry, shared by the acceptance
//! gate (`crates/midnight-acceptance/tests/acceptance/scenarios/ui_gate.rs`) and the
//! `midnight-ui` layout tests (`window_polish.rs`). Both read a window's item tree into
//! [`Element`]s; this module compares neighbouring controls with each other, which the
//! per-element checks (clipping, overflow, contrast, focus) never do:
//!
//! - U-K32 row alignment: controls side by side (a field and its button, a label and
//!   its control, pickers, button rows) share their height and vertical centre;
//! - U-K33 caption centring: a button's, picker's or tab's caption is centred in it and
//!   not cut at its top or bottom, judged by its line box and, where the gate rendered
//!   it, by the glyphs of a caption without letters ("…", "×"), whose line box says
//!   nothing about where the glyphs sit;
//! - U-K34 edge alignment: controls and labels stacked in a column whose left or right
//!   edges almost line up (1–6 px apart) instead of sharing one edge;
//! - U-K35 companion button: an icon or symbol button right of a field or picker is
//!   square;
//! - U-K36 uneven row gaps: in a column of controls, one gap at least three times the
//!   column's usual gap (and 12 px more);
//! - U-K37 touching controls: controls side by side, or fields stacked, less than 2 px
//!   apart, or a control within 2 px of its dialog's edge;
//! - U-K38 elided with room: a label shortened with "…" although the free room beside
//!   it would show it whole;
//! - U-K39 option rows: check boxes, radio buttons and switches stacked one after the
//!   other in a column (nothing between them) keep one gap, within 1 px.
//!
//! Tolerances follow the request that introduced them: 1 px for heights, centres and
//! edges (layouts place on whole or half pixels), 1.5 px for a caption's line box, and
//! for a symbol caption's glyphs 2 px or an eighth of the font, whichever is more.
//! The crafted good and bad layouts at the end prove each check both ways.
#![allow(dead_code)]

/// A window rectangle: x, y, width, height (logical pixels).
pub type Rect = (f32, f32, f32, f32);

pub const ROW_ALIGNMENT: &str = "controls of one row not aligned";
pub const CAPTION_CENTRE: &str = "caption not centred";
pub const EDGE_ALIGNMENT: &str = "edges almost aligned";
pub const COMPANION_SIZE: &str = "companion button not square";
pub const ROW_GAPS: &str = "uneven gaps between rows";
pub const TOUCHING: &str = "controls touching";
pub const ELIDED_WITH_ROOM: &str = "text elided although there is room";
pub const OPTION_ROWS: &str = "uneven gaps between option rows";

/// Every check of this module, for the gate's summaries and red cases.
pub const KINDS: [&str; 8] = [
    ROW_ALIGNMENT,
    CAPTION_CENTRE,
    EDGE_ALIGNMENT,
    COMPANION_SIZE,
    ROW_GAPS,
    TOUCHING,
    ELIDED_WITH_ROOM,
    OPTION_ROWS,
];

/// What an element is, as far as alignment goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A single-line text field.
    Field,
    Button,
    /// A combo box or picker.
    Picker,
    /// A check box, radio button or switch: its height is its caption's.
    Toggle,
    Tab,
    Spin,
    Slider,
    /// A visible text outside any control.
    Label,
}

impl Role {
    /// A framed control whose height is a control height: a row of them shares it.
    fn boxed(self) -> bool {
        matches!(self, Self::Field | Self::Button | Self::Picker | Self::Spin)
    }

    fn control(self) -> bool {
        !matches!(self, Self::Label)
    }
}

/// A caption drawn in a control, or a label's own text.
#[derive(Clone, Debug, Default)]
pub struct Caption {
    /// The line box as its alignment places it inside its element.
    pub line: Rect,
    /// The single-line width and the height the text needs.
    pub needed: (f32, f32),
    pub font_size: f32,
    /// No letters or digits: "…", "×", "+", "›".
    pub symbol: bool,
    /// Shortened with "…" (`overflow: elide`).
    pub elide: bool,
    /// Left-aligned (a label's left edge is its glyphs' left edge).
    pub left_aligned: bool,
    /// The rendered glyphs' box, where the caller measured it in pixels.
    pub glyphs: Option<Rect>,
}

#[derive(Clone, Debug)]
pub struct Element {
    /// How findings name it: role, label and rectangle.
    pub name: String,
    pub role: Role,
    pub rect: Rect,
    /// Wholly shown: not clipped by a scroll viewport, not under another surface.
    pub whole: bool,
    /// Taller than one line on purpose: a list, a multi-line field, a wrapped text.
    pub multiline: bool,
    /// One segment of a segmented control (tabs, page buttons, a list's rows, a menu's
    /// entries): its neighbours touch it by design.
    pub segment: bool,
    /// The scroll viewport it lives in (0 for none): a column does not continue across
    /// viewports.
    pub scope: usize,
    pub caption: Option<Caption>,
    /// Shows no letters or digits: an icon or a symbol.
    pub icon_only: bool,
    /// The element's container (its parent item), for the room beside a label.
    pub container: Option<Rect>,
    /// Part of a list's or menu's row (a row, or a control inside one): rows are laid
    /// out by their list, and judged by its own tests.
    pub list_row: bool,
    /// How many shown texts with letters or digits it holds (a caption and its detail
    /// line are two: their block, not one caption, is centred).
    pub texts: usize,
}

impl Element {
    fn centre_y(&self) -> f32 {
        self.rect.1 + self.rect.3 / 2.0
    }

    fn right(&self) -> f32 {
        self.rect.0 + self.rect.2
    }

    fn bottom(&self) -> f32 {
        self.rect.1 + self.rect.3
    }

    /// The vertical centre that lines up with a neighbour: a label's line box.
    fn line_centre(&self) -> f32 {
        match (&self.role, &self.caption) {
            (Role::Label, Some(caption)) => caption.line.1 + caption.line.3 / 2.0,
            _ => self.centre_y(),
        }
    }

    /// Joined to `other` beside it on purpose: no gap, the same top and height (a split
    /// button, path segments, status fields, calendar cells).
    fn joined(&self, other: &Element) -> bool {
        ((other.rect.0 - self.right()).abs() <= 0.5 || (self.rect.0 - other.right()).abs() <= 0.5)
            && (self.rect.1 - other.rect.1).abs() <= 0.5
            && (self.rect.3 - other.rect.3).abs() <= 0.5
    }

    fn contains(&self, other: &Element) -> bool {
        let (a, b) = (self.rect, other.rect);
        b.0 >= a.0 - 0.5
            && b.1 >= a.1 - 0.5
            && b.0 + b.2 <= a.0 + a.2 + 0.5
            && b.1 + b.3 <= a.1 + a.3 + 0.5
    }
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub kind: &'static str,
    pub detail: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.detail)
    }
}

fn finding(kind: &'static str, detail: String) -> Finding {
    Finding { kind, detail }
}

/// How much two spans overlap.
fn overlap(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

fn vertical(element: &Element) -> (f32, f32) {
    (element.rect.1, element.bottom())
}

fn horizontal(element: &Element) -> (f32, f32) {
    (element.rect.0, element.right())
}

/// Every alignment finding of one surface's elements; `root` is the surface's own
/// rectangle (a dialog's popup or a tool window), or None where its edges are the
/// window's chrome (the main window).
pub fn check(elements: &[Element], root: Option<Rect>) -> Vec<Finding> {
    let shown: Vec<&Element> = elements.iter().filter(|element| element.whole).collect();
    let mut findings = Vec::new();
    rows(&shown, root.is_some(), &mut findings);
    captions(&shown, &mut findings);
    let led: Vec<bool> = shown
        .iter()
        .map(|element| {
            shown.iter().any(|other| {
                right_neighbour(other, &shown).is_some_and(|next| std::ptr::eq(next, *element))
            })
        })
        .collect();
    let trailing: Vec<bool> = shown
        .iter()
        .map(|element| right_neighbour(element, &shown).is_some())
        .collect();
    edges(&shown, &led, &trailing, &mut findings);
    row_gaps(&shown, root, &mut findings);
    touching(&shown, root, &mut findings);
    elided(&shown, &mut findings);
    let all: Vec<&Element> = elements.iter().collect();
    option_rows(&shown, &all, &mut findings);
    findings
}

/// The element right of `element` on its row: the nearest one starting at most 24 px
/// after it that shares at least half the smaller height, neither inside the other.
fn right_neighbour<'a>(element: &Element, elements: &[&'a Element]) -> Option<&'a Element> {
    elements
        .iter()
        .copied()
        .filter(|other| {
            let gap = other.rect.0 - element.right();
            let shared = overlap(vertical(element), vertical(other));
            !std::ptr::eq(*other, element)
                && (-0.5..=24.0).contains(&gap)
                && shared >= 0.5 * element.rect.3.min(other.rect.3)
                && !element.contains(other)
                && !other.contains(element)
        })
        .min_by(|a, b| a.rect.0.total_cmp(&b.rect.0))
}

/// U-K32 and U-K35 (only in a dialog or tool window: a toolbar's icon buttons are not
/// a field's companions).
fn rows(elements: &[&Element], dialog: bool, findings: &mut Vec<Finding>) {
    for element in elements {
        let Some(next) = right_neighbour(element, elements) else {
            continue;
        };
        if element.list_row || next.list_row {
            continue;
        }
        if element.multiline
            || next.multiline
            || element.role == Role::Tab
            || next.role == Role::Tab
        {
            continue;
        }
        // A tall strip beside a control (a splitter, a list) is not on its row.
        if element.rect.3.max(next.rect.3) > 2.0 * element.rect.3.min(next.rect.3) {
            continue;
        }
        if element.segment && next.segment {
            continue;
        }
        // Two labels in a row line up their baselines; only same-size ones their centres.
        if element.role == Role::Label && next.role == Role::Label {
            let sizes = (
                element.caption.as_ref().map(|caption| caption.font_size),
                next.caption.as_ref().map(|caption| caption.font_size),
            );
            // A wrapped value beside its label lines up its first line, not its middle.
            let lines = (
                element
                    .caption
                    .as_ref()
                    .map_or(0.0, |caption| caption.line.3),
                next.caption.as_ref().map_or(0.0, |caption| caption.line.3),
            );
            if sizes.0 != sizes.1 || lines.0.max(lines.1) > 1.5 * lines.0.min(lines.1) {
                continue;
            }
        }
        let mut problems = Vec::new();
        if element.role.boxed() && next.role.boxed() && (element.rect.3 - next.rect.3).abs() > 1.01
        {
            problems.push(format!(
                "heights {:.1}px and {:.1}px",
                element.rect.3, next.rect.3
            ));
        }
        let (first, second) = (element.line_centre(), next.line_centre());
        if (first - second).abs() > 1.01 {
            problems.push(format!("vertical centres {first:.1}px and {second:.1}px"));
        }
        if !problems.is_empty() {
            findings.push(finding(
                ROW_ALIGNMENT,
                format!(
                    "{} and {} beside it: {}",
                    element.name,
                    next.name,
                    problems.join(", ")
                ),
            ));
        }
        // A symbol or icon button that belongs to a field (or a picker at least twice as
        // wide as high, not a split button's arrow) is square.
        if dialog
            && (element.role == Role::Field
                || (element.role == Role::Picker && element.rect.2 >= 2.0 * element.rect.3))
            && !element.joined(next)
            && next.role == Role::Button
            && next.icon_only
            && (next.rect.2 - next.rect.3).abs() > 1.01
        {
            findings.push(finding(
                COMPANION_SIZE,
                format!(
                    "{} beside {} is {:.1}x{:.1}px, not square",
                    next.name, element.name, next.rect.2, next.rect.3
                ),
            ));
        }
    }
}

/// U-K33.
fn captions(elements: &[&Element], findings: &mut Vec<Finding>) {
    for element in elements {
        if !matches!(element.role, Role::Button | Role::Picker | Role::Tab)
            || element.multiline
            || element.texts > 1
        {
            continue;
        }
        let Some(caption) = &element.caption else {
            continue;
        };
        let centre = element.centre_y();
        let line = caption.line.1 + caption.line.3 / 2.0;
        // A list's or menu's row lays out its own caption (beside a separator line).
        if !element.list_row && (line - centre).abs() > 1.5 {
            findings.push(finding(
                CAPTION_CENTRE,
                format!(
                    "{}: its caption's line is centred at {line:.1}px, the control at {centre:.1}px",
                    element.name
                ),
            ));
        } else if !caption.symbol && caption.needed.1 > element.rect.3 + 1.0 {
            findings.push(finding(
                CAPTION_CENTRE,
                format!(
                    "{}: its caption needs {:.1}px of height, the control has {:.1}px",
                    element.name, caption.needed.1, element.rect.3
                ),
            ));
        }
        // Dots joined to text segments (a path's omission marker, a status field's "..")
        // sit on the text's baseline, as text does.
        let joined = elements
            .iter()
            .any(|other| !std::ptr::eq(*other, *element) && element.joined(other));
        if let Some(glyphs) = caption.glyphs
            && caption.symbol
            && !joined
        {
            let drawn = glyphs.1 + glyphs.3 / 2.0;
            let tolerance = 2.0_f32.max(caption.font_size / 8.0);
            if (drawn - centre).abs() > tolerance {
                findings.push(finding(
                    CAPTION_CENTRE,
                    format!(
                        "{}: its glyphs are drawn at {:.0}–{:.0}px, centred at {drawn:.1}px, the control's centre is {centre:.1}px ({:.1}px off, {tolerance:.1}px allowed)",
                        element.name,
                        glyphs.1,
                        glyphs.1 + glyphs.3,
                        (drawn - centre).abs()
                    ),
                ));
            }
        }
    }
}

/// Whether `outer` holds `inner` (within half a pixel).
fn holds(outer: Rect, inner: Rect) -> bool {
    inner.0 >= outer.0 - 0.5
        && inner.1 >= outer.1 - 0.5
        && inner.0 + inner.2 <= outer.0 + outer.2 + 0.5
        && inner.1 + inner.3 <= outer.1 + outer.3 + 0.5
}
/// The left edge an element shows: a label's glyphs start at its element only when it
/// is left-aligned.
fn left_edge(element: &Element) -> Option<f32> {
    match (&element.role, &element.caption) {
        (Role::Label, Some(caption)) if !caption.left_aligned => None,
        (Role::Label, None) => None,
        _ => Some(element.rect.0),
    }
}

/// The right edge to compare: a label's natural width ends anywhere.
fn right_edge(element: &Element) -> Option<f32> {
    (element.role != Role::Label).then(|| element.right())
}

/// U-K34: an element and the next one below it in its column (overlapping horizontally,
/// at most 48 px lower) whose left or right edges differ by more than 1 and at most
/// 6 px. An element that follows another on its row (`led`) starts where that one
/// ends, not at the column's edge: its left edge is not compared.
fn edges(elements: &[&Element], led: &[bool], trailing: &[bool], findings: &mut Vec<Finding>) {
    for (position, upper) in elements.iter().enumerate() {
        if upper.segment || upper.list_row {
            continue;
        }
        let below = elements
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, lower)| {
                let distance = lower.rect.1 - upper.bottom();
                !lower.segment
                    && !lower.list_row
                    && (-0.5..=48.0).contains(&distance)
                    && overlap(horizontal(upper), horizontal(lower)) > 0.0
            })
            .min_by(|a, b| a.1.rect.1.total_cmp(&b.1.rect.1));
        let Some((below_position, lower)) = below else {
            continue;
        };
        // Separate bars (a window's toolbar and a panel's) line up only by chance: an
        // element is compared with one whose container holds, or lies in, its own.
        if let (Some(a), Some(b)) = (upper.container, lower.container)
            && !holds(a, b)
            && !holds(b, a)
        {
            continue;
        }
        let mut problems = Vec::new();
        // A row of buttons followed by more buttons (a footer) is placed from its end:
        // where its first one starts is the sum of their widths, not an edge.
        let button_row =
            |at: usize, element: &Element| element.role == Role::Button && trailing[at];
        if !led[position]
            && !led[below_position]
            && !button_row(position, upper)
            && !button_row(below_position, lower)
            && let (Some(a), Some(b)) = (left_edge(upper), left_edge(lower))
        {
            let difference = (a - b).abs();
            if difference > 1.01 && difference <= 6.0 {
                problems.push(format!("left edges {a:.1}px and {b:.1}px"));
            }
        }
        if !trailing[position]
            && !trailing[below_position]
            && let (Some(a), Some(b)) = (right_edge(upper), right_edge(lower))
        {
            let difference = (a - b).abs();
            if difference > 1.01 && difference <= 6.0 {
                problems.push(format!("right edges {a:.1}px and {b:.1}px"));
            }
        }
        if !problems.is_empty() {
            findings.push(finding(
                EDGE_ALIGNMENT,
                format!(
                    "{} and {} under it: {}",
                    upper.name,
                    lower.name,
                    problems.join(", ")
                ),
            ));
        }
    }
}

/// U-K36: the columns of controls (the same left edge within 1 px, the same viewport);
/// with at least three gaps between consecutive members that nothing else fills, a gap
/// at least three times the median gap and 12 px more is an outlier. The surface's last
/// row (its footer) may sit lower: a window larger than its content leaves room there.
fn row_gaps(elements: &[&Element], root: Option<Rect>, findings: &mut Vec<Finding>) {
    let controls: Vec<&Element> = elements
        .iter()
        .copied()
        .filter(|element| element.role.control() && !element.segment && !element.list_row)
        .collect();
    let footer = controls
        .iter()
        .map(|element| element.rect.1)
        .fold(f32::MIN, f32::max);
    let mut seen: Vec<usize> = Vec::new();
    for (index, first) in controls.iter().enumerate() {
        if seen.contains(&index) {
            continue;
        }
        let mut column: Vec<(usize, &Element)> = controls
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, other)| {
                other.scope == first.scope && (other.rect.0 - first.rect.0).abs() <= 1.0
            })
            .collect();
        seen.extend(column.iter().map(|(index, _)| *index));
        column.sort_by(|a, b| a.1.rect.1.total_cmp(&b.1.rect.1));
        let mut gaps: Vec<(f32, &Element, &Element)> = Vec::new();
        for pair in column.windows(2) {
            let (upper, lower) = (pair[0].1, pair[1].1);
            let gap = lower.rect.1 - upper.bottom();
            if gap < 0.0 {
                continue;
            }
            let span = (
                upper.rect.0.min(lower.rect.0),
                upper.right().max(lower.right()),
            );
            let filled = elements.iter().any(|other| {
                other.rect.1 >= upper.bottom() - 0.5
                    && other.bottom() <= lower.rect.1 + 0.5
                    && overlap(horizontal(other), span) > 0.0
            });
            let last_row = root.is_some() && (lower.rect.1 - footer).abs() < 0.5;
            if !filled && !last_row {
                gaps.push((gap, upper, lower));
            }
        }
        if gaps.len() < 3 {
            continue;
        }
        let mut sizes: Vec<f32> = gaps.iter().map(|gap| gap.0).collect();
        sizes.sort_by(f32::total_cmp);
        let median = sizes[sizes.len() / 2];
        for (gap, upper, lower) in &gaps {
            if *gap >= 3.0 * median && *gap >= median + 12.0 {
                findings.push(finding(
                    ROW_GAPS,
                    format!(
                        "{} and {} under it are {gap:.0}px apart; the column's other rows are {median:.0}px apart",
                        upper.name, lower.name
                    ),
                ));
            }
        }
    }
}

/// U-K37.
fn touching(elements: &[&Element], root: Option<Rect>, findings: &mut Vec<Finding>) {
    let controls: Vec<&Element> = elements
        .iter()
        .copied()
        .filter(|element| element.role.control() && !element.list_row)
        .collect();
    for element in &controls {
        if !element.segment
            && let Some(next) = right_neighbour(element, &controls)
            && !next.segment
            && !element.joined(next)
            && element.rect.3.max(next.rect.3) <= 2.0 * element.rect.3.min(next.rect.3)
        {
            let gap = next.rect.0 - element.right();
            if gap < 2.0 {
                findings.push(finding(
                    TOUCHING,
                    format!(
                        "{} and {} beside it are {gap:.1}px apart",
                        element.name, next.name
                    ),
                ));
            }
        }
        if matches!(element.role, Role::Field | Role::Picker | Role::Spin)
            && let Some(lower) = controls
                .iter()
                .copied()
                .filter(|lower| {
                    matches!(lower.role, Role::Field | Role::Picker | Role::Spin)
                        && overlap(horizontal(element), horizontal(lower)) > 0.0
                        && (-0.5..2.0).contains(&(lower.rect.1 - element.bottom()))
                })
                .min_by(|a, b| a.rect.1.total_cmp(&b.rect.1))
        {
            findings.push(finding(
                TOUCHING,
                format!(
                    "{} and {} under it are {:.1}px apart",
                    element.name,
                    lower.name,
                    lower.rect.1 - element.bottom()
                ),
            ));
        }
        // A joined group (calendar cells) is judged as one control, by its tests.
        let joined = controls
            .iter()
            .any(|other| !std::ptr::eq(*other, *element) && element.joined(other));
        if let Some((x, y, w, h)) = root
            && !element.segment
            && !joined
            && element.rect.2 < 0.9 * w
        {
            let margins = [
                element.rect.0 - x,
                element.rect.1 - y,
                x + w - element.right(),
                y + h - element.bottom(),
            ];
            let closest = margins.into_iter().fold(f32::MAX, f32::min);
            if (-0.5..2.0).contains(&closest) {
                findings.push(finding(
                    TOUCHING,
                    format!(
                        "{} is {closest:.1}px from the edge of its surface",
                        element.name
                    ),
                ));
            }
        }
    }
}

/// U-K38: an elided label whose free room to its right (to its row's next element, or
/// its container's edge less 16 px of padding) would hold the text whole.
fn elided(elements: &[&Element], findings: &mut Vec<Finding>) {
    for element in elements {
        let Some(caption) = &element.caption else {
            continue;
        };
        if element.role != Role::Label || !caption.elide || caption.needed.0 <= element.rect.2 + 1.0
        {
            continue;
        }
        let Some(container) = element.container else {
            continue;
        };
        let next = elements
            .iter()
            .filter(|other| {
                other.rect.0 >= element.right() - 0.5
                    && overlap(vertical(element), vertical(other)) > 0.0
            })
            .map(|other| other.rect.0)
            .fold(container.0 + container.2 - 16.0, f32::min);
        let room = next - element.right();
        let missing = caption.needed.0 - element.rect.2;
        if room >= missing {
            findings.push(finding(
                ELIDED_WITH_ROOM,
                format!(
                    "{} needs {:.0}px and has {:.0}px, with {room:.0}px free beside it",
                    element.name, caption.needed.0, element.rect.2
                ),
            ));
        }
    }
}

/// U-K39: options stacked in a column (check boxes, radio buttons, switches with the
/// same left edge in one viewport, nothing else between two neighbours) keep one gap:
/// each gap is within 1 px of the column's most common one.
fn option_rows(elements: &[&Element], all: &[&Element], findings: &mut Vec<Finding>) {
    let toggles: Vec<&Element> = elements
        .iter()
        .copied()
        .filter(|element| element.role == Role::Toggle && !element.list_row)
        .collect();
    let mut seen: Vec<usize> = Vec::new();
    for (index, first) in toggles.iter().enumerate() {
        if seen.contains(&index) {
            continue;
        }
        let mut column: Vec<(usize, &Element)> = toggles
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, other)| {
                other.scope == first.scope && (other.rect.0 - first.rect.0).abs() <= 1.0
            })
            .collect();
        seen.extend(column.iter().map(|(index, _)| *index));
        column.sort_by(|a, b| a.1.rect.1.total_cmp(&b.1.rect.1));
        let mut gaps: Vec<(f32, &Element, &Element)> = Vec::new();
        for pair in column.windows(2) {
            let (upper, lower) = (pair[0].1, pair[1].1);
            let gap = lower.rect.1 - upper.bottom();
            let between = all.iter().any(|other| {
                !std::ptr::eq(*other, upper)
                    && !std::ptr::eq(*other, lower)
                    // Reaching into the gap: a row's wrapped detail line makes it taller.
                    && other.bottom() > upper.bottom() + 0.5
                    && other.rect.1 < lower.rect.1 - 0.5
                    && other.right() > upper.rect.0
            });
            if gap >= -0.5 && !between {
                gaps.push((gap, upper, lower));
            }
        }
        if gaps.len() < 2 {
            continue;
        }
        // The most common gap (to a whole pixel): the column's rhythm.
        let mut counts: Vec<(i32, usize)> = Vec::new();
        for (gap, _, _) in &gaps {
            let key = gap.round() as i32;
            match counts.iter_mut().find(|(value, _)| *value == key) {
                Some(entry) => entry.1 += 1,
                None => counts.push((key, 1)),
            }
        }
        let usual = counts
            .iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map_or(0.0, |(value, _)| *value as f32);
        for (gap, upper, lower) in &gaps {
            if (gap - usual).abs() > 1.01 {
                findings.push(finding(
                    OPTION_ROWS,
                    format!(
                        "{} and {} under it are {gap:.0}px apart; the column's options are {usual:.0}px apart",
                        upper.name, lower.name
                    ),
                ));
            }
        }
    }
}
/// The box of the pixels inside `area` (in physical pixels of a `width`×`height` frame,
/// `scale` physical per logical pixel) that differ clearly from the area's most common
/// colour: where a caption's glyphs are drawn. `pixel(x, y)` reads the frame. Returns
/// logical coordinates, or None when nothing is drawn.
pub fn drawn_box(
    pixel: &dyn Fn(usize, usize) -> [u8; 3],
    size: (usize, usize),
    scale: f32,
    area: Rect,
) -> Option<Rect> {
    let left = (area.0 * scale).ceil().max(0.0) as usize;
    let top = (area.1 * scale).ceil().max(0.0) as usize;
    let right = (((area.0 + area.2) * scale).floor().max(0.0) as usize).min(size.0);
    let bottom = (((area.1 + area.3) * scale).floor().max(0.0) as usize).min(size.1);
    if right <= left || bottom <= top {
        return None;
    }
    let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    for y in top..bottom {
        for x in left..right {
            *counts.entry(pixel(x, y)).or_default() += 1;
        }
    }
    let background = counts.into_iter().max_by_key(|(_, count)| *count)?.0;
    let distance = |a: [u8; 3], b: [u8; 3]| {
        a.iter()
            .zip(b)
            .map(|(a, b)| u32::from(a.abs_diff(b)))
            .sum::<u32>()
    };
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in top..bottom {
        for x in left..right {
            if distance(pixel(x, y), background) > 90 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x0 < x1).then(|| {
        (
            x0 as f32 / scale,
            y0 as f32 / scale,
            (x1 - x0) as f32 / scale,
            (y1 - y0) as f32 / scale,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(name: &str, role: Role, rect: Rect) -> Element {
        Element {
            name: name.to_owned(),
            role,
            rect,
            whole: true,
            multiline: false,
            segment: false,
            scope: 0,
            caption: None,
            icon_only: false,
            container: None,
            list_row: false,
            texts: 1,
        }
    }

    fn captioned(name: &str, role: Role, rect: Rect, line: Rect, symbol: bool) -> Element {
        Element {
            caption: Some(Caption {
                line,
                needed: (line.2, line.3),
                font_size: 12.0,
                symbol,
                elide: false,
                left_aligned: role == Role::Label,
                glyphs: None,
            }),
            icon_only: symbol,
            ..element(name, role, rect)
        }
    }

    fn kinds(findings: &[Finding]) -> Vec<&'static str> {
        findings.iter().map(|finding| finding.kind).collect()
    }

    /// The Checksums Verify page as the owner saw it: a 24 px field and a 32 px "…"
    /// button beside it, top-aligned, its dots drawn near the button's bottom.
    fn verify_row(button_height: f32, glyph_top: f32) -> Vec<Element> {
        let field = element(
            "TextInput \"Checksum file to verify\"",
            Role::Field,
            (16.0, 40.0, 300.0, 24.0),
        );
        let mut browse = captioned(
            "Button \"Browse checksum file\"",
            Role::Button,
            (324.0, 40.0, 24.0, button_height),
            (330.0, 40.0 + (button_height - 16.0) / 2.0, 12.0, 16.0),
            true,
        );
        if let Some(caption) = &mut browse.caption {
            caption.glyphs = Some((331.0, glyph_top, 10.0, 2.0));
        }
        vec![field, browse]
    }

    #[test]
    fn the_owners_checksum_row_is_flagged_and_its_fix_passes() {
        let broken = check(&verify_row(32.0, 58.0), Some((0.0, 0.0, 400.0, 200.0)));
        let found = kinds(&broken);
        assert!(found.contains(&ROW_ALIGNMENT), "{broken:#?}");
        assert!(found.contains(&COMPANION_SIZE), "{broken:#?}");
        assert!(found.contains(&CAPTION_CENTRE), "{broken:#?}");
        assert!(
            broken
                .iter()
                .any(|finding| finding.detail.contains("heights 24.0px and 32.0px")),
            "{broken:#?}"
        );
        // The same button 24 px square with its glyph centred.
        let fixed = check(&verify_row(24.0, 51.0), Some((0.0, 0.0, 400.0, 200.0)));
        assert!(fixed.is_empty(), "{fixed:#?}");
    }

    #[test]
    fn a_label_lines_up_with_its_control_by_its_line_box() {
        let label = |top: f32| {
            captioned(
                "Text \"Links\"",
                Role::Label,
                (16.0, top, 40.0, 16.0),
                (16.0, top, 40.0, 16.0),
                false,
            )
        };
        let picker = element(
            "Combobox \"Links\"",
            Role::Picker,
            (64.0, 40.0, 120.0, 24.0),
        );
        assert!(check(&[label(44.0), picker.clone()], None).is_empty());
        let off = check(&[label(40.0), picker], None);
        assert_eq!(kinds(&off), [ROW_ALIGNMENT], "{off:#?}");
    }

    #[test]
    fn multi_line_controls_and_tabs_are_not_row_aligned() {
        let list = Element {
            multiline: true,
            ..element("List", Role::Field, (16.0, 40.0, 200.0, 120.0))
        };
        let button = element("Button \"Add\"", Role::Button, (224.0, 40.0, 60.0, 24.0));
        assert!(check(&[list, button], None).is_empty());
    }

    #[test]
    fn a_caption_off_centre_or_cut_is_flagged() {
        let centred = captioned(
            "Button \"OK\"",
            Role::Button,
            (0.0, 0.0, 60.0, 30.0),
            (20.0, 7.0, 20.0, 16.0),
            false,
        );
        assert!(check(&[centred], None).is_empty());
        let low = captioned(
            "Button \"OK\"",
            Role::Button,
            (0.0, 0.0, 60.0, 30.0),
            (20.0, 12.0, 20.0, 16.0),
            false,
        );
        assert_eq!(kinds(&check(&[low], None)), [CAPTION_CENTRE]);
        let mut cut = captioned(
            "Button \"OK\"",
            Role::Button,
            (0.0, 0.0, 60.0, 14.0),
            (20.0, -1.0, 20.0, 16.0),
            false,
        );
        if let Some(caption) = &mut cut.caption {
            caption.line = (20.0, -1.0, 20.0, 16.0);
            caption.needed = (20.0, 16.0);
        }
        assert_eq!(kinds(&check(&[cut], None)), [CAPTION_CENTRE]);
        // Letters are judged by their line box, not by where their ink happens to sit.
        let mut letters = captioned(
            "Button \"Copy\"",
            Role::Button,
            (0.0, 0.0, 60.0, 30.0),
            (16.0, 7.0, 28.0, 16.0),
            false,
        );
        if let Some(caption) = &mut letters.caption {
            caption.glyphs = Some((16.0, 10.0, 28.0, 14.0));
        }
        assert!(check(&[letters], None).is_empty());
    }

    #[test]
    fn near_miss_edges_in_a_column_are_flagged_and_shared_or_distinct_ones_pass() {
        let field = element("Field", Role::Field, (16.0, 40.0, 300.0, 24.0));
        let aligned = element("Picker", Role::Picker, (16.0, 70.0, 300.0, 24.0));
        assert!(check(&[field.clone(), aligned], None).is_empty());
        let near = element("Picker", Role::Picker, (19.0, 70.0, 296.0, 24.0));
        let found = check(&[field.clone(), near], None);
        assert_eq!(kinds(&found), [EDGE_ALIGNMENT], "{found:#?}");
        assert!(
            found[0].detail.contains("left edges 16.0px and 19.0px"),
            "{found:#?}"
        );
        // An indented option is meant to stand apart.
        let indented = element("Check box", Role::Toggle, (40.0, 70.0, 200.0, 20.0));
        assert!(check(&[field, indented], None).is_empty());
        // A status after a progress bar and a field after its caption start where their
        // row's first element ends, 3 px apart by chance.
        let row = |left: f32, top: f32| (left, top, 40.0, 16.0);
        let bar = element("Bar", Role::Slider, row(10.0, 10.0));
        let status = captioned(
            "Status",
            Role::Label,
            (58.0, 10.0, 200.0, 16.0),
            (58.0, 10.0, 200.0, 16.0),
            false,
        );
        let caption = captioned(
            "Command",
            Role::Label,
            row(10.0, 30.0),
            row(10.0, 30.0),
            false,
        );
        let command = element("Command line", Role::Field, (55.0, 26.0, 300.0, 24.0));
        let found = check(&[bar, status, caption, command], None);
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn a_gap_three_times_the_columns_others_is_flagged() {
        let at = |name: &str, top: f32| element(name, Role::Field, (16.0, top, 200.0, 20.0));
        let even = [at("A", 0.0), at("B", 26.0), at("C", 52.0), at("D", 78.0)];
        assert!(check(&even, None).is_empty());
        let gap = [at("A", 0.0), at("B", 26.0), at("C", 52.0), at("D", 112.0)];
        let found = check(&gap, None);
        assert_eq!(kinds(&found), [ROW_GAPS], "{found:#?}");
        // A dialog's footer may sit lower in a window taller than its content.
        let footer = [
            at("A", 0.0),
            at("B", 26.0),
            at("C", 52.0),
            at("D", 78.0),
            at("E", 300.0),
        ];
        assert!(check(&footer, Some((0.0, -16.0, 400.0, 400.0))).is_empty());
    }

    #[test]
    fn touching_controls_and_edges_are_flagged() {
        let a = element("Button \"A\"", Role::Button, (16.0, 16.0, 60.0, 24.0));
        let b = element("Button \"B\"", Role::Button, (77.0, 16.0, 60.0, 24.0));
        assert_eq!(kinds(&check(&[a.clone(), b], None)), [TOUCHING]);
        let apart = element("Button \"B\"", Role::Button, (84.0, 16.0, 60.0, 24.0));
        assert!(check(&[a.clone(), apart.clone()], None).is_empty());
        let edge = element("Button \"C\"", Role::Button, (0.5, 16.0, 60.0, 24.0));
        assert_eq!(
            kinds(&check(&[edge], Some((0.0, 0.0, 300.0, 100.0)))),
            [TOUCHING]
        );
        // Segments of one control touch by design.
        let tabs = [
            Element {
                segment: true,
                ..element("Tab 1", Role::Button, (16.0, 16.0, 60.0, 24.0))
            },
            Element {
                segment: true,
                ..element("Tab 2", Role::Button, (76.0, 16.0, 60.0, 24.0))
            },
        ];
        assert!(check(&tabs, Some((0.0, 0.0, 300.0, 100.0))).is_empty());
        // A split button's halves, path segments and status fields are joined on purpose:
        // no gap, one top and height.
        let split = [
            element("Button \"Back\"", Role::Button, (16.0, 16.0, 25.0, 22.0)),
            element(
                "Combobox \"Back history\"",
                Role::Picker,
                (41.0, 16.0, 21.0, 22.0),
            ),
        ];
        assert!(check(&split, None).is_empty());
        let fields = [
            element("Field 1", Role::Field, (16.0, 16.0, 200.0, 24.0)),
            element("Field 2", Role::Field, (16.0, 40.5, 200.0, 24.0)),
        ];
        assert_eq!(kinds(&check(&fields, None)), [TOUCHING]);
    }

    #[test]
    fn an_elided_label_with_room_beside_it_is_flagged() {
        let label = |width: f32| Element {
            caption: Some(Caption {
                line: (16.0, 10.0, width, 16.0),
                needed: (180.0, 16.0),
                font_size: 12.0,
                elide: true,
                left_aligned: true,
                ..Caption::default()
            }),
            container: Some((0.0, 0.0, 400.0, 40.0)),
            ..element(
                "Text \"Current folder\"",
                Role::Label,
                (16.0, 10.0, width, 16.0),
            )
        };
        let found = check(&[label(120.0)], None);
        assert_eq!(kinds(&found), [ELIDED_WITH_ROOM], "{found:#?}");
        let button = element("Button \"Go\"", Role::Button, (150.0, 6.0, 60.0, 24.0));
        let crowded = check(&[label(120.0), button], None);
        assert!(!kinds(&crowded).contains(&ELIDED_WITH_ROOM), "{crowded:#?}");
    }

    #[test]
    fn option_rows_keep_one_gap() {
        let at = |name: &str, top: f32| element(name, Role::Toggle, (16.0, top, 200.0, 24.0));
        let even = [at("A", 0.0), at("B", 28.0), at("C", 56.0), at("D", 84.0)];
        assert!(check(&even, None).is_empty());
        let uneven = [at("A", 0.0), at("B", 28.0), at("C", 56.0), at("D", 92.0)];
        let found = check(&uneven, None);
        assert_eq!(kinds(&found), [OPTION_ROWS], "{found:#?}");
        // A heading between two groups starts a new run.
        let heading = captioned(
            "Text \"Group\"",
            Role::Label,
            (16.0, 62.0, 80.0, 16.0),
            (16.0, 62.0, 80.0, 16.0),
            false,
        );
        let grouped = [
            at("A", 0.0),
            at("B", 28.0),
            at("C", 84.0),
            at("D", 112.0),
            heading,
        ];
        assert!(!kinds(&check(&grouped, None)).contains(&OPTION_ROWS));
    }
    #[test]
    fn drawn_box_finds_the_glyphs_on_a_plain_fill() {
        let (width, height) = (40, 30);
        let mut frame = vec![[40_u8, 40, 40]; width * height];
        for y in 22..24 {
            for x in 10..30 {
                frame[y * width + x] = [230, 230, 230];
            }
        }
        let pixel = |x: usize, y: usize| frame[y * width + x];
        let found = drawn_box(&pixel, (width, height), 1.0, (0.0, 0.0, 40.0, 30.0));
        assert_eq!(found, Some((10.0, 22.0, 20.0, 2.0)));
        let halved = drawn_box(&pixel, (width, height), 2.0, (0.0, 0.0, 20.0, 15.0));
        assert_eq!(halved, Some((5.0, 11.0, 10.0, 1.0)));
    }
}
