//! Keep bounded layout units intact when a viewport anchor falls inside one.
use crate::markdown::Window;
use midnight_plugin_sdk::api::{
    document::*,
    viewer::{MAX_ANCHORS, MAX_SPANS, SourceRange},
    wire::MAX_ITEMS,
};
use std::ops::Range;

/// Conservative public wire cost of complete blocks. CBOR maps cost one item
/// per map, key and scalar value (see the API's bounded codec): a page is 15
/// items, a block 31 (+2 per absent optional field, counted as if present),
/// an anchor 5, a span 9, a link or table cell 7, a strikethrough range 5.
/// Exact anchors and table geometry are required; styles and link metadata are
/// optional decoration that a dense page may still discard.
#[derive(Clone, Copy, Default)]
struct Cost {
    required: usize,
    optional: usize,
    anchors: usize,
    cells: usize,
    spans: usize,
    links: usize,
    decorations: usize,
    targets: usize,
}

const PAGE_ITEMS: usize = 15;
const LINK_TARGET_BYTES: usize = 64 * 1024;

impl Cost {
    fn of(blocks: &[Block]) -> Self {
        let mut cost = Self::default();
        for block in blocks {
            let anchors = block.text.anchors.len();
            let cells = block.table.as_ref().map_or(0, |row| row.cells.len());
            let ranges = block.strikethrough.as_ref().map_or(0, Vec::len);
            cost.required += 35 + 5 * anchors + block.table.as_ref().map_or(0, |_| 6 + 7 * cells);
            cost.optional += 9 * block.text.spans.len()
                + 7 * block.links.len()
                + block.strikethrough.as_ref().map_or(0, |_| 2 + 5 * ranges);
            cost.anchors += anchors;
            cost.cells += cells;
            cost.spans += block.text.spans.len();
            cost.links += block.links.len();
            cost.decorations += ranges;
            cost.targets += block
                .links
                .iter()
                .map(|link| link.target.len())
                .sum::<usize>();
        }
        cost
    }

    fn plus(self, other: Self) -> Self {
        Self {
            required: self.required + other.required,
            optional: self.optional + other.optional,
            anchors: self.anchors + other.anchors,
            cells: self.cells + other.cells,
            spans: self.spans + other.spans,
            links: self.links + other.links,
            decorations: self.decorations + other.decorations,
            targets: self.targets + other.targets,
        }
    }

    /// The complete decorated page fits every public budget.
    fn fits_all(&self) -> bool {
        PAGE_ITEMS + self.required + self.optional <= MAX_ITEMS
            && self.anchors <= MAX_ANCHORS
            && self.cells <= MAX_TABLE_CELLS
            && self.spans <= MAX_SPANS
            && self.links <= MAX_SPANS
            && self.decorations <= MAX_SPANS
            && self.targets <= LINK_TARGET_BYTES
    }
}

/// Borrow retained context and copy only the complete layout groups being returned.
pub(crate) fn crop_ref(
    page: &DocumentPage,
    window: &Window,
    proven_end: u64,
) -> Option<DocumentPage> {
    let selected = select(page, window, proven_end)?;
    Some(slice_ref(page, selected))
}

pub(crate) fn slice_ref(page: &DocumentPage, selected: Range<usize>) -> DocumentPage {
    DocumentPage {
        revision: page.revision,
        source: source(page, &selected),
        blocks: page.blocks[selected.clone()].to_vec(),
        has_before: page.has_before || selected.start != 0,
        has_after: page.has_after || selected.end != page.blocks.len(),
    }
}

pub(crate) fn groups(page: &DocumentPage) -> Vec<Range<usize>> {
    let mut groups: Vec<Range<usize>> = Vec::new();
    for (index, block) in page.blocks.iter().enumerate() {
        if block.table.as_ref().is_some_and(|row| !row.header)
            && index > 0
            && page.blocks[index - 1].table.is_some()
            && let Some(group) = groups.last_mut()
        {
            group.end = index + 1;
        } else {
            groups.push(index..index + 1);
        }
    }
    groups
}

/// Paragraph wrapping and table widths must not depend on scroll direction.
/// The line count is a demand hint; the byte/block limits remain hard bounds.
/// Only units before `proven_end` are complete: a unit cut by the read edge
/// could change its interpretation, so an anchor in such a unit gets `None`.
pub(crate) fn select(
    page: &DocumentPage,
    window: &Window,
    proven_end: u64,
) -> Option<Range<usize>> {
    let after_proven = if window.backwards {
        window.anchor > proven_end
    } else {
        window.anchor >= proven_end
    };
    if after_proven {
        return None;
    }
    let mut groups = groups(page);
    let complete = groups
        .iter()
        .take_while(|group| page.blocks[group.start].text.source.offset < proven_end)
        .count();
    groups.truncate(complete);
    let first = groups.iter().position(|range| {
        let block = &page.blocks[range.end - 1];
        block.text.source.offset + block.text.source.length > window.anchor
            || (block.kind == BLOCK_RULE && block.text.source.offset == window.anchor)
    });
    let pivot = if window.backwards {
        groups
            .iter()
            .rposition(|range| page.blocks[range.start].text.source.offset < window.anchor)
    } else {
        first
    }?;
    let order: Box<dyn Iterator<Item = usize>> = if window.backwards {
        Box::new((0..=pivot).rev())
    } else {
        Box::new(pivot..groups.len())
    };
    let mut selected = groups[pivot].start..groups[pivot].start;
    let mut bytes = 0;
    let mut lines = 0;
    let mut cost = Cost::default();
    for index in order {
        let group = &groups[index];
        let blocks = &page.blocks[group.clone()];
        let group_bytes: usize = blocks.iter().map(|block| block.text.text.len()).sum();
        let next = cost.plus(Cost::of(blocks));
        // An oversized pivot unit keeps the bounded fallbacks of crop_context.
        // Every further unit must keep the whole decorated page within the
        // public anchor/item budgets, so ordinary prose pages stop early
        // instead of losing their styles, links and table geometry.
        let wire = selected.is_empty() || next.fits_all();
        if bytes + group_bytes > window.max_bytes.min(65536)
            || selected.len() + group.len() > MAX_BLOCKS
            || !wire
        {
            let pivot_range = &groups[pivot];
            let first = &page.blocks[pivot_range.start];
            let last = &page.blocks[pivot_range.end - 1];
            if selected == *pivot_range
                && window.anchor > first.text.source.offset
                && window.anchor < last.text.source.offset + last.text.source.length
            {
                // Retaining all preceding/following context would return the
                // same full-budget unit forever for an overlapping wheel anchor.
                // Use bounded scalar paging for this oversized layout context.
                return None;
            }
            break;
        }
        if lines >= window.max_lines {
            break;
        }
        selected.start = selected.start.min(group.start);
        selected.end = selected.end.max(group.end);
        bytes += group_bytes;
        cost = next;
        lines += blocks
            .iter()
            .map(|block| {
                if index != pivot {
                    return block.text.text.lines().count().max(1);
                }
                let at = block
                    .text
                    .anchors
                    .iter()
                    .find(|anchor| anchor.source_offset >= window.anchor)
                    .map_or(block.text.text.len(), |anchor| anchor.text_offset as usize);
                let visible = if window.backwards {
                    &block.text.text[..at]
                } else {
                    &block.text.text[at..]
                };
                visible.lines().count()
            })
            .sum::<usize>();
    }
    if selected.is_empty() {
        // Oversized individual blocks retain the bounded scalar-safe fallback.
        return None;
    }
    Some(selected)
}

fn source(page: &DocumentPage, selected: &Range<usize>) -> SourceRange {
    let old_end = page.source.offset + page.source.length;
    let start = if selected.start == 0 {
        page.source.offset
    } else {
        page.blocks[selected.start].text.source.offset
    };
    let end = if selected.end == page.blocks.len() {
        old_end
    } else {
        page.blocks[selected.end].text.source.offset
    };
    SourceRange {
        offset: start,
        length: end - start,
    }
}
