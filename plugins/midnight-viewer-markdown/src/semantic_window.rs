//! Keep bounded layout units intact when a viewport anchor falls inside one.
use crate::markdown::Window;
use midnight_plugin_sdk::api::{document::*, viewer::SourceRange};
use std::ops::Range;

/// Paragraph wrapping and table widths must not depend on scroll direction.
/// The line count is a demand hint; the byte/block limits remain hard bounds.
pub(crate) fn crop(mut page: DocumentPage, window: &Window) -> Result<DocumentPage, DocumentPage> {
    let Some(selected) = select(&page, window) else {
        return Err(page);
    };
    let source = source(&page, &selected);
    page.has_before |= selected.start != 0;
    page.has_after |= selected.end != page.blocks.len();
    page.blocks = page.blocks.drain(selected).collect();
    page.source = source;
    Ok(page)
}

/// Borrow retained context and copy only the complete layout groups being returned.
pub(crate) fn crop_ref(page: &DocumentPage, window: &Window) -> Option<DocumentPage> {
    let selected = select(page, window)?;
    Some(DocumentPage {
        revision: page.revision,
        source: source(page, &selected),
        blocks: page.blocks[selected.clone()].to_vec(),
        has_before: page.has_before || selected.start != 0,
        has_after: page.has_after || selected.end != page.blocks.len(),
    })
}

fn select(page: &DocumentPage, window: &Window) -> Option<Range<usize>> {
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
    for index in order {
        let group = &groups[index];
        let blocks = &page.blocks[group.clone()];
        let group_bytes: usize = blocks.iter().map(|block| block.text.text.len()).sum();
        if bytes + group_bytes > window.max_bytes.min(65536)
            || selected.len() + group.len() > MAX_BLOCKS
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
