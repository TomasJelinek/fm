//! Unwrapped pages of rows as a stack of whole-row groups (`TextRenderer::layout_rows`).
//!
//! A Text or Source page of long unwrapped rows holds hundreds of kilobytes, and a refill
//! keeps most rows of the page before it. Shaped as one layout, every refill would shape the
//! whole page again, however few rows it adds. A page of rows is therefore a composite page
//! (`tables::Composite`, as tables use) of groups of whole rows, each its own layout. A page
//! that repeats a group of the page before it (the same text, colours, styles and options)
//! keeps that group's shaping: a refill shapes only the rows it adds and the group that its
//! new edge cuts.
//!
//! Each group is broken into lines at its place in the page (parley's line `y`, from the
//! sum of the line heights above it, in page order), so its lines lie where one layout of
//! the page puts them, bit for bit, whatever groups the page reused. Every group but the
//! last ends after a line break, after which its own layout has an empty line that the next
//! group's first row covers; the queries leave it out and pick a group by its rows' edges as
//! one layout picks a line. The composite's carets, hit tests, selections, lines and scroll
//! anchors are those of one layout (`rows_tests.rs`).
//!
//! Each row is a paragraph of its own: a row that takes bidirectional resolution (it holds a
//! right-to-left letter or an Arabic number) is a group of its own, so that its direction
//! comes from its own first strong character, and no other row resolves with it. Parley
//! resolves one layout as one paragraph, its direction from the layout's first strong
//! character; a group of several such rows would order them by whichever row the group, and
//! so the scroll history, started with. Rows without them need no resolution and group
//! freely; a page of them answers exactly as one layout.
use crate::tables::{Composite, PlacedPage, Styles, clipped_colors, colors_in};
use crate::{ColorSpan, LayoutError, StyleSpan, TextOptions, TextPage, TextRenderer};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

/// Bytes of whole rows a group holds at most; a longer row is a group of its own.
pub(crate) const GROUP_BYTES: usize = 8 * 1024;
/// The most bytes one page of row groups lays out.
pub const MAX_ROWS_BYTES: usize = 1024 * 1024;
/// The most colour or style spans one page of row groups takes (each group takes at most the
/// one-layout limits).
pub const MAX_ROWS_SPANS: usize = 16 * 4096;

/// What a page of row groups shaped and what it reused (`TextRenderer::layout_rows`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowLayout {
    pub groups: usize,
    pub reused_groups: usize,
    pub reused_bytes: usize,
    pub shaped_bytes: usize,
}

/// A composite page that is a stack of row groups: what decided each group's shaping besides
/// its text, and where its rows lie.
pub(crate) struct RowGroups {
    keys: Vec<u64>,
    edges: Vec<Edges>,
    /// Where each group's lines start: parley's line `y` of its first line.
    places: Vec<f64>,
}

/// Where a group's rows lie, from its lines (`group_edges`).
#[derive(Clone, Copy, Debug)]
struct Edges {
    /// The top of its first row.
    top: f32,
    /// The top and bottom of its last row.
    last_row: [f32; 2],
    /// The top of its own empty last line after its last line break, where the next group's
    /// first row lies instead; past every point for the page's last group.
    end: f32,
}

impl RowGroups {
    /// The group, and the `y` to hit-test in it, that answer a hit test at `y` as one layout
    /// of the page does: parley takes the last line whose top is at or above `y`, or the
    /// first line (`Layout::line_for_offset`, a binary search over the line tops).
    pub(crate) fn hit_at(&self, y: f32) -> (usize, f32) {
        let index = self
            .edges
            .partition_point(|edges| edges.top <= y)
            .saturating_sub(1);
        let edges = self.edges[index];
        // A group's own empty last line is not one of the page's lines: its last row is.
        (index, if y >= edges.end { edges.last_row[0] } else { y })
    }

    /// The group whose line one layout of the page gives at `y` (`TextPage::line_at_y`: the
    /// first line whose bottom lies below `y`, or the last line).
    pub(crate) fn line_at(&self, y: f32) -> usize {
        self.edges
            .partition_point(|edges| edges.last_row[1] <= y)
            .min(self.edges.len() - 1)
    }
}

impl TextRenderer {
    /// Lay out `text`, whole rows without wrapping, as a stack of row groups. A group of
    /// `previous`, a page of row groups whose offset `o` is `o + shift` here, keeps its
    /// shaping when this page repeats it whole at that place with the same colours, styles
    /// and options.
    pub fn layout_rows(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
        styles: &[StyleSpan],
        previous: Option<(&TextPage, isize)>,
    ) -> Result<(TextPage, RowLayout), LayoutError> {
        self.layout_rows_with(text, options, spans, styles, previous, GROUP_BYTES)
    }

    /// [`Self::layout_rows`] with groups of at most `group_bytes` bytes of whole rows.
    pub(crate) fn layout_rows_with(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
        styles: &[StyleSpan],
        previous: Option<(&TextPage, isize)>,
        group_bytes: usize,
    ) -> Result<(TextPage, RowLayout), LayoutError> {
        if options.wrap_width.is_some() {
            return Err(LayoutError::InvalidGeometry);
        }
        if text.len() > MAX_ROWS_BYTES
            || spans.len() > MAX_ROWS_SPANS
            || styles.len() > MAX_ROWS_SPANS
        {
            return Err(LayoutError::LimitExceeded);
        }
        validate_ranges(text, spans, styles)?;
        let styles = Styles::new(styles);
        let options = TextOptions { ..*options };
        let options_key = options_key(&options);
        let key = |range: &Range<usize>| {
            group_key(
                options_key,
                &clipped_colors(spans, range),
                &styles.clip(range),
            )
        };
        // What a group of `range` holds of one layout's limits besides bytes and lines.
        let marks = |range: &Range<usize>| (colors_in(spans, range), styles.count(range));
        let mut reusable = previous.map_or_else(Vec::new, |(page, shift)| {
            reusable_groups(page, shift, text, &key)
        });
        reusable.sort_by_key(|(range, ..)| range.start);
        let mut next = reusable.into_iter().peekable();
        let mut parts = Vec::new();
        let mut keys = Vec::new();
        let mut edges = Vec::new();
        let mut places = Vec::new();
        let mut layout = RowLayout::default();
        let mut width = 0.0_f32;
        // Where the next group's first row starts, summed as one layout sums its lines.
        let mut y = 0.0_f64;
        let mut position = 0;
        while position < text.len() || parts.is_empty() {
            while next
                .peek()
                .is_some_and(|(range, ..)| range.start < position)
            {
                next.next();
            }
            let (range, page, group) = match next.next_if(|(range, ..)| range.start == position) {
                Some((range, mut page, group, from)) => {
                    layout.reused_groups += 1;
                    layout.reused_bytes += range.len();
                    place(&mut page, from, y);
                    (range, page, group)
                }
                None => {
                    let limit = next.peek().map_or(text.len(), |(range, ..)| range.start);
                    let end = group_end(text, position, limit, group_bytes, &marks);
                    let range = position..end;
                    let page = self.layout_at(
                        &text[range.clone()],
                        &options,
                        &clipped_colors(spans, &range),
                        &styles.clip(&range),
                        crate::runs::RUN_BYTES,
                        y,
                    )?;
                    layout.shaped_bytes += range.len();
                    let group = key(&range);
                    (range, page, group)
                }
            };
            let top = y;
            let (group_edges, below) = group_edges(&page, top, range.end == text.len());
            y = below;
            width = width.max(page.width());
            position = range.end;
            parts.push(PlacedPage {
                range,
                table_row: None,
                origin: [0.0, 0.0],
                width: page.width(),
                page,
            });
            keys.push(group);
            edges.push(group_edges);
            places.push(top);
            if text.is_empty() {
                break;
            }
        }
        layout.groups = parts.len();
        let mut result = self.layout_with_styles("", &options, &[], &[])?;
        result.text = text.into();
        result.composite = Some(Composite {
            parts,
            backgrounds: Vec::new(),
            borders: Vec::new(),
            border_color: [0; 4],
            width,
            height: y as f32,
            rows: Some(RowGroups {
                keys,
                edges,
                places,
            }),
        });
        Ok((result, layout))
    }
}

/// Break `page`, a group shaped and placed at `from`, into lines from `top` down, as one
/// layout of the page breaks them there. Its lines' paint reach moves with them: it only
/// culls painting, by an ink reach wider than any glyph (`glyphs::line_bounds`).
fn place(page: &mut TextPage, from: f64, top: f64) {
    let mut lines = page.layout.break_lines();
    lines.state_mut().set_line_y(top);
    lines.break_remaining(f32::MAX);
    let shift = (top - from) as f32;
    for bounds in &mut page.line_bounds {
        *bounds = bounds.map(|edge| edge + shift);
    }
}

/// Where the rows of `page`, a group placed at `top`, lie, and where the next group starts:
/// below its last row, or for the page's last group (`last`) below its every line, as one
/// layout's height counts them.
fn group_edges(page: &TextPage, top: f64, last: bool) -> (Edges, f64) {
    let layout = &page.layout;
    let count = layout.len();
    // Every group but the last ends with a line break, after which its own layout has an
    // empty line where the next group's first row lies instead.
    let end = count
        .checked_sub(1)
        .filter(|&index| !last && index > 0)
        .and_then(|index| layout.get(index))
        .filter(|line| line.text_range().is_empty())
        .map(|line| line.metrics().block_min_coord);
    let rows = count - usize::from(end.is_some());
    let mut below = top;
    for line in layout.lines().take(rows) {
        below += f64::from(line.metrics().line_height);
    }
    let first = layout.get(0).map(|line| line.metrics().block_min_coord);
    let last_row = rows
        .checked_sub(1)
        .and_then(|index| layout.get(index))
        .map(|line| {
            [
                line.metrics().block_min_coord,
                line.metrics().block_max_coord,
            ]
        });
    let edges = Edges {
        top: first.unwrap_or(top as f32),
        last_row: last_row.unwrap_or([top as f32; 2]),
        end: end.unwrap_or(f32::INFINITY),
    };
    (edges, below)
}

/// The groups of `page`, a page of row groups whose offset `o` is `o + shift` in `text`,
/// that `text` repeats whole and that `key` still describes: their range in `text`, their
/// shaped layout (to break into lines at their new place), key and place in `page`.
fn reusable_groups(
    page: &TextPage,
    shift: isize,
    text: &str,
    key: &dyn Fn(&Range<usize>) -> u64,
) -> Vec<(Range<usize>, TextPage, u64, f64)> {
    let Some(grid) = &page.composite else {
        return Vec::new();
    };
    let Some(rows) = &grid.rows else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    for ((part, previous), from) in grid.parts.iter().zip(&rows.keys).zip(&rows.places) {
        let Some(start) = part.range.start.checked_add_signed(shift) else {
            continue;
        };
        let end = start + part.range.len();
        let (Some(here), Some(there)) = (text.get(start..end), page.text.get(part.range.clone()))
        else {
            continue;
        };
        // A group is whole rows: here too it starts a row, and its last row ends where it
        // ended (with a line break, or at both pages' ends).
        let starts_row = start == 0 || text.as_bytes()[start - 1] == b'\n';
        let ends_row = there.ends_with('\n') || end == text.len();
        if part.range.is_empty() || !starts_row || !ends_row || here != there {
            continue;
        }
        let range = start..end;
        if key(&range) == *previous {
            let shaped = TextPage {
                text: part.page.text.clone(),
                layout: part.page.layout.clone(),
                line_bounds: part.page.line_bounds.clone(),
                source_map: part.page.source_map.clone(),
                composite: None,
            };
            groups.push((range, shaped, *previous, *from));
        }
    }
    groups
}

/// The end of a group of whole rows from `start`: before the row that would bring it past
/// `group_bytes` or past one layout's colour spans, styles or lines (`marks` counts a
/// range's colour spans and styles), at most `limit` (a row start or the text's end). A
/// longer row is a group of its own, and so is a row that takes bidirectional resolution.
fn group_end(
    text: &str,
    start: usize,
    limit: usize,
    group_bytes: usize,
    marks: &dyn Fn(&Range<usize>) -> (usize, usize),
) -> usize {
    let bytes = text.as_bytes();
    let mut end = start;
    let mut lines = 1;
    while end < limit {
        let row_end = bytes[end..limit]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(limit, |at| end + at + 1);
        let row = &text[end..row_end];
        let paragraph = bidirectional(row);
        lines += crate::hard_line_count(row) - 1;
        if end > start {
            let (colors, styles) = marks(&(start..row_end));
            if row_end - start > group_bytes
                || paragraph
                || lines > crate::MAX_LINES
                || colors > crate::MAX_SPANS
                || styles > crate::MAX_STYLE_SPANS
            {
                break;
            }
        }
        end = row_end;
        if paragraph {
            break;
        }
    }
    end
}

/// Whether `row` may hold a character that parley resolves bidirectionally: a right-to-left
/// letter, an Arabic number or an explicit embedding, override or isolate
/// (`parley::bidi::BIDI_MASK`). The blocks whose characters have those classes, unassigned
/// ones included, stand for them.
fn bidirectional(row: &str) -> bool {
    !row.is_ascii()
        && row.chars().any(|character| {
            matches!(
                u32::from(character),
                0x0590..=0x08FF
                    | 0x200F
                    | 0x202A..=0x202E
                    | 0x2066..=0x2069
                    | 0xFB1D..=0xFDFF
                    | 0xFE70..=0xFEFF
                    | 0x1_0800..=0x1_0FFF
                    | 0x1_E800..=0x1_EFFF
            )
        })
}

/// Spans and styles inside `text` on character boundaries, spans sorted and disjoint (each
/// group checks the rest of its own limits).
fn validate_ranges(
    text: &str,
    spans: &[ColorSpan],
    styles: &[StyleSpan],
) -> Result<(), LayoutError> {
    let mut previous_end = 0;
    for span in spans {
        if span.range.start < previous_end || text.get(span.range.clone()).is_none() {
            return Err(LayoutError::InvalidRange);
        }
        previous_end = span.range.end;
    }
    if styles
        .iter()
        .any(|style| style.range.is_empty() || text.get(style.range.clone()).is_none())
    {
        return Err(LayoutError::InvalidRange);
    }
    Ok(())
}

/// Everything in `options` that decides a layout.
fn options_key(options: &TextOptions<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    (
        options.family,
        options.literal_family,
        options.size.to_bits(),
        options.scale.to_bits(),
        options.weight,
        options.italic,
        options.tabular_numbers,
        options.tab_width,
        options.foreground,
        options.wrap_width.map(f32::to_bits),
        options.line_height.map(f32::to_bits),
    )
        .hash(&mut hasher);
    hasher.finish()
}

/// Everything besides its text that decides a group's shaping: the options and its own
/// colours and styles.
fn group_key(options: u64, colors: &[ColorSpan], styles: &[StyleSpan]) -> u64 {
    let mut hasher = DefaultHasher::new();
    options.hash(&mut hasher);
    for span in colors {
        (&span.range, span.color).hash(&mut hasher);
    }
    for style in styles {
        (
            &style.range,
            style.weight,
            style.italic,
            style.underline,
            style.strikethrough,
            style.monospace,
            style.size.to_bits(),
        )
            .hash(&mut hasher);
    }
    hasher.finish()
}

impl TextPage {
    /// The byte ranges of the groups of a page of row groups (`TextRenderer::layout_rows`),
    /// in order; `None` for any other page.
    pub fn row_groups(&self) -> Option<Vec<Range<usize>>> {
        let grid = self.composite.as_ref()?;
        grid.rows.as_ref()?;
        Some(grid.parts.iter().map(|part| part.range.clone()).collect())
    }
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
