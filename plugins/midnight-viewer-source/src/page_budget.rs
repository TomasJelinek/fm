//! Keep a directional fragment and its styles within the aggregate CBOR item budget.
use midnight_plugin_sdk::{
    Output,
    api::{
        viewer::*,
        wire::{self, WireError},
    },
};

pub(crate) fn output(mut page: TextPage, backwards: bool) -> Result<Output, WireError> {
    if fits(&page, 0, page.text.len(), page.anchors.len()) {
        return Output::text(&page);
    }
    // Prefix/suffix cost is monotonic. Search scalar anchors, never raw UTF-8 bytes.
    let mut low = 1;
    let mut high = page.anchors.len().saturating_sub(1);
    let mut keep = 0;
    while low <= high {
        let mid = low + (high - low) / 2;
        let (start, end) = if backwards {
            (
                page.anchors[page.anchors.len() - 1 - mid].text_offset as usize,
                page.text.len(),
            )
        } else {
            (0, page.anchors[mid].text_offset as usize)
        };
        if fits(&page, start, end, mid + 1) {
            keep = mid;
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    if keep == 0 {
        return Err(WireError::Limit);
    }
    let last = page.anchors.len() - 1;
    let (first, last) = if backwards {
        (last - keep, last)
    } else {
        (0, keep)
    };
    let start = page.anchors[first].text_offset as usize;
    let end = page.anchors[last].text_offset as usize;
    crop(&mut page, start, end)?;
    Output::text(&page)
}

pub(crate) fn crop(page: &mut TextPage, start: usize, end: usize) -> Result<(), WireError> {
    let first = page
        .anchors
        .binary_search_by_key(&(start as u32), |a| a.text_offset)
        .map_err(|_| WireError::Malformed)?;
    let last = page
        .anchors
        .binary_search_by_key(&(end as u32), |a| a.text_offset)
        .map_err(|_| WireError::Malformed)?;
    page.has_before |= first > 0;
    page.has_after |= last + 1 < page.anchors.len();
    page.partial_line |= (start > 0 && !page.text[..start].ends_with('\n'))
        || (end < page.text.len() && !page.text[..end].ends_with('\n'));
    page.source.offset = page.anchors[first].source_offset;
    page.source.length = page.anchors[last].source_offset - page.source.offset;
    page.spans.retain_mut(|span| {
        if span.end as usize <= start || span.start as usize >= end {
            return false;
        }
        span.start = (span.start as usize).max(start).saturating_sub(start) as u32;
        span.end = ((span.end as usize).min(end) - start) as u32;
        true
    });
    page.anchors.truncate(last + 1);
    page.anchors.drain(..first);
    for anchor in &mut page.anchors {
        anchor.text_offset -= start as u32;
    }
    page.text.truncate(end);
    page.text.drain(..start);
    Ok(())
}

fn fits(page: &TextPage, start: usize, end: usize, anchors: usize) -> bool {
    let first = page.spans.partition_point(|s| s.end as usize <= start);
    let last = page.spans.partition_point(|s| (s.start as usize) < end);
    let spans = last.saturating_sub(first);
    // Each span map consumes 9 items and each anchor map 5; 32 conservatively
    // covers TextPage/SourceRange maps, array headers and scalar fields.
    anchors <= MAX_ANCHORS
        && spans <= MAX_SPANS
        && end - start <= MAX_PAGE_BYTES
        && 32 + anchors * 5 + spans * 9 <= wire::MAX_ITEMS
}
